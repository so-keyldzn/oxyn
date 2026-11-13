//! Streaming protocol and lifetime regressions over real loopback HTTP.

use std::sync::Arc;
use std::time::Duration;

use arrow::array::AsArray as _;
use arrow::datatypes::Int64Type;
use oxyn_core::{CancelToken, DriverId, ExecRequest, OxynError, StatementIntent};
use oxyn_driver::Session;
use serde_json::json;
use tokio::sync::Notify;

use crate::http::Http;
use crate::options::ConnectSpec;
use crate::session::TursoSession;
use crate::testserver::{Reply, Server, TOKEN};

fn session(server: &Server) -> Box<TursoSession> {
    let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN)).expect("test HTTP client");
    Box::new(TursoSession::new(
        Arc::new(http),
        DriverId::turso(),
        "3.54.1".into(),
    ))
}

fn request() -> ExecRequest {
    ExecRequest::new(crate::variant::LANGUAGE, "SELECT id, label FROM a_view")
        .with_intent(StatementIntent::Read)
}

fn closed() -> Reply {
    Reply::Json(
        200,
        json!({"baton":null,"base_url":null,"results":[{"type":"ok","response":{"type":"close"}}]})
            .to_string(),
    )
}

async fn server(rows: usize, value_bytes: usize, gate: Option<Arc<Notify>>) -> Server {
    Server::start(move |request| {
        assert_eq!(
            request.authorization.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        match request.path.as_str() {
            "/v3" => Reply::Raw(200, Vec::new()),
            "/v3/cursor" => {
                assert!(request.body["baton"].is_null());
                assert_eq!(
                    request.body["batch"]["steps"][0]["stmt"]["sql"],
                    "SELECT id, label FROM a_view"
                );
                Reply::Cursor {
                    rows,
                    value_bytes,
                    before_header: gate.clone(),
                }
            }
            "/v3/pipeline" => {
                assert_eq!(request.body["baton"], "cursor-private-identity");
                assert_eq!(request.body["requests"][0]["type"], "close");
                closed()
            }
            _ => panic!("unexpected protocol path"),
        }
    })
    .await
}

#[tokio::test]
async fn a_cursor_stream_exceeds_the_pipeline_budget_in_byte_bounded_batches() {
    // More than 32 MiB overall, generated one row at a time by the fixture.
    let server = server(2400, 16 * 1024, None).await;
    let session = session(&server);
    let mut cursor = session
        .execute_preview(request(), &CancelToken::new())
        .await
        .expect("stream opens before the whole response is read");
    let mut rows = 0;
    let mut batches = 0;
    while let Some(batch) = cursor.next_batch().await.expect("next batch") {
        assert!(batch.get_array_memory_size() <= 1024 * 1024 + 32 * 1024);
        assert_eq!(batch.column(0).as_primitive::<Int64Type>().value(0), rows);
        rows += i64::try_from(batch.num_rows()).expect("bounded rows");
        batches += 1;
    }
    assert_eq!(rows, 2400);
    assert!(batches > 10);
    assert_eq!(cursor.stats().rows, 2400);
    drop(cursor);
    session.close().await.expect("cleanup joined");
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.path == "/v3/cursor")
            .count(),
        1
    );
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.path == "/v3/pipeline")
            .count(),
        1
    );
}

#[tokio::test]
async fn abandoning_the_opener_still_closes_a_late_baton() {
    let gate = Arc::new(Notify::new());
    let server = server(2, 8, Some(Arc::clone(&gate))).await;
    let session = session(&server);
    let cancel = CancelToken::new();
    {
        let opening = session.execute_preview(request(), &cancel);
        tokio::pin!(opening);
        tokio::select! {
            result = &mut opening => panic!("the header is still gated: {}", result.is_ok()),
            () = server.requests_seen(2) => {}
        }
        cancel.cancel();
        assert!(matches!(opening.await, Err(OxynError::Cancelled)));
    }
    gate.notify_one();
    tokio::time::timeout(Duration::from_secs(5), session.close())
        .await
        .expect("close does not orphan its worker")
        .expect("closed");
    assert!(server.requests().iter().any(|r| r.path == "/v3/pipeline"));
}

#[tokio::test]
async fn dropping_an_unexhausted_cursor_closes_its_baton() {
    let server = server(100_000, 256, None).await;
    let session = session(&server);
    let cursor = session
        .execute_preview(request(), &CancelToken::new())
        .await
        .expect("schema");
    drop(cursor);
    tokio::time::timeout(Duration::from_secs(5), session.close())
        .await
        .expect("bounded cleanup")
        .expect("closed");
    assert!(server.requests().iter().any(|r| r.path == "/v3/pipeline"));
}

#[tokio::test]
async fn a_cursor_rejects_incomplete_framing_and_scrubs_remote_identity() {
    for tail in [
        "{\"type\":\"row\",\"row\":[{\"type\":\"integer\",\"value\":\"1\"}]}\n",
        "{\"type\":\"step_error\",\"step\":0,\"error\":{\"message\":\"cursor-private-identity eyJ-test-token-0123456789 failed\"}}\n",
    ] {
        let body = format!(
            "{{\"baton\":\"cursor-private-identity\",\"base_url\":null}}\n{{\"type\":\"step_begin\",\"step\":0,\"cols\":[{{\"name\":\"id\",\"decltype\":\"INTEGER\"}}]}}\n{tail}"
        );
        let server = Server::start(move |request| match request.path.as_str() {
            "/v3" => Reply::Raw(200, Vec::new()),
            "/v3/cursor" => Reply::Raw(200, body.as_bytes().to_vec()),
            "/v3/pipeline" => closed(),
            _ => panic!("unexpected path"),
        })
        .await;
        let session = session(&server);
        let error = match session
            .execute_preview(request(), &CancelToken::new())
            .await
        {
            Ok(_) => panic!("incomplete or failed stream was accepted"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(!message.contains(TOKEN));
        assert!(!message.contains("cursor-private-identity"));
        session.close().await.expect("closed");
        assert!(server.requests().iter().any(|r| r.path == "/v3/pipeline"));
    }
}

#[tokio::test]
async fn cursor_routing_cannot_forward_authentication_to_an_untrusted_origin() {
    let server = Server::start(|request| match request.path.as_str() {
        "/v3" => Reply::Raw(200, Vec::new()),
        "/v3/cursor" => Reply::Raw(
            200,
            b"{\"baton\":\"cursor-private-identity\",\"base_url\":\"https://attacker.invalid/\"}\n"
                .to_vec(),
        ),
        "/v3/pipeline" => closed(),
        _ => panic!("unexpected path"),
    })
    .await;
    let session = session(&server);
    assert!(matches!(
        session
            .execute_preview(request(), &CancelToken::new())
            .await,
        Err(OxynError::Config(_))
    ));
    session.close().await.expect("cleanup at original endpoint");
    assert!(server.requests().iter().any(|r| r.path == "/v3/pipeline"));
}

#[path = "protocol_tests.rs"]
mod protocol;
