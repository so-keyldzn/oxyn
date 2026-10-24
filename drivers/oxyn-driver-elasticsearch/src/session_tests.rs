use std::sync::{Arc, Mutex, PoisonError};

use oxyn_core::{ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits, StatementIntent};
use oxyn_driver::Credentials;

use super::*;
use crate::catalog::Product;
use crate::driver::elasticsearch_metadata;
use crate::http::tests::canned;
use crate::options::ConnectSpec;

fn session_to(port: u16) -> ElasticsearchSession {
    let config = ConnectionConfig::new("test", DriverId::elasticsearch())
        .with_environment(Environment::Local)
        .with_param("url", format!("http://127.0.0.1:{port}"));
    let spec = ConnectSpec::from_config(&elasticsearch_metadata(), &config, &Credentials::new())
        .expect("a valid configuration");
    let http = Arc::new(Http::new(spec).expect("a client"));
    let capabilities = Capabilities::SEARCH_DSL | Capabilities::SERVER_SIDE_CANCEL;
    let catalog = ElasticsearchCatalog::new(
        Arc::clone(&http),
        Product {
            name: "Elasticsearch",
            version: "9.1.0".to_owned(),
        },
        capabilities,
    );
    ElasticsearchSession::new(http, catalog, capabilities)
}

fn answer(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn request(text: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SearchDsl, text).with_limits(ExecLimits::default())
}

#[test]
fn session_proves_only_a_parseable_search_dsl_request() {
    let session = session_to(9);
    assert_eq!(
        session.classify_intent(&request("POST /logs/_search\n{}")),
        Some(StatementIntent::Read)
    );
    assert_eq!(
        session.classify_intent(&request("POST /future_route\n{}")),
        Some(StatementIntent::Unknown)
    );
    assert_eq!(session.classify_intent(&request("not a request")), None);
    let sql = ExecRequest::new(QueryLanguage::SQL, "SELECT 1");
    assert_eq!(session.classify_intent(&sql), None);
}

#[test]
fn session_answers_the_risk_of_what_it_classifies() {
    let session = session_to(9);
    assert_eq!(
        session.classify_risk(&request("DELETE /logs")),
        Some(MutationRisk::DropObject)
    );
    assert_eq!(
        session.classify_risk(&request(
            "POST /logs/_delete_by_query\n{\"query\":{\"match_all\":{}}}"
        )),
        Some(MutationRisk::UnboundedDelete)
    );
    assert_eq!(
        session.classify_risk(&ExecRequest::new(QueryLanguage::SQL, "DELETE /logs")),
        None
    );
}

#[tokio::test]
async fn a_read_only_execution_refuses_what_it_cannot_prove_reads_before_sending() {
    // Nothing listens on port 9: a request that left would fail differently.
    let session = session_to(9);
    for text in ["DELETE /logs", "POST /_future\n{}", "GET /_refresh"] {
        let refused = session
            .execute(request(text), &CancelToken::new())
            .await
            .err()
            .expect("refused");
        assert!(
            refused.to_string().contains("read-only"),
            "{text}: {refused}"
        );
    }
}

#[tokio::test]
async fn bound_values_are_refused_rather_than_substituted() {
    let session = session_to(9);
    let with_params =
        request("POST /_search\n{}").with_params(vec![oxyn_core::ScalarValue::Int64(1)]);
    assert!(matches!(
        session.execute(with_params, &CancelToken::new()).await,
        Err(OxynError::NotSupported { .. })
    ));
}

#[tokio::test]
async fn a_bulk_answered_200_with_failed_items_is_not_a_success() {
    let port = canned(answer(
        r#"{"took":1,"errors":true,"items":[{"index":{"status":201}},{"index":{"status":400,"error":{"type":"mapper_parsing_exception","reason":"failed to parse field [n]"}}}]}"#,
    ))
    .await;
    let session = session_to(port);
    let mut write = request("POST /_bulk\n{\"index\":{\"_index\":\"a\"}}\n{\"n\":\"x\"}");
    write.limits.read_only = false;
    let failed = session
        .execute(write, &CancelToken::new())
        .await
        .err()
        .expect("a partial failure");
    let text = failed.to_string();
    assert!(text.contains("1 of 2 `_bulk` items failed"), "{text}");
    assert!(text.contains("mapper_parsing_exception"), "{text}");
    assert_eq!(
        failed.class(),
        ErrorClass::Permanent,
        "never replayed as transient"
    );
}

#[tokio::test]
async fn a_by_query_operation_that_timed_out_is_not_a_success() {
    let port = canned(answer(
        r#"{"took":30001,"timed_out":true,"total":1000,"updated":400,"deleted":0,"batches":1,"version_conflicts":0,"noops":0,"failures":[]}"#,
    ))
    .await;
    let session = session_to(port);
    let mut write = request("POST /logs/_update_by_query\n{\"query\":{\"term\":{\"a\":1}}}");
    write.limits.read_only = false;
    let failed = session
        .execute(write, &CancelToken::new())
        .await
        .err()
        .expect("a partial outcome");
    let text = failed.to_string();
    assert!(text.contains("timed_out"), "{text}");
    assert!(text.contains("of the 1000 it matched"), "{text}");
}

#[tokio::test]
async fn a_read_answered_partial_is_refused_by_the_session() {
    let port = canned(answer(
        r#"{"took":1,"timed_out":false,"_shards":{"total":2,"successful":1,"skipped":0,"failed":1,"failures":[{"shard":0,"index":"b","reason":{"type":"script_exception","reason":"runtime error"}}]},"hits":{"hits":[]}}"#,
    ))
    .await;
    let session = session_to(port);
    let refused = session
        .execute(request("POST /a,b/_search\n{}"), &CancelToken::new())
        .await
        .err()
        .expect("a partial read");
    let text = refused.to_string();
    assert!(text.contains("1 of the 2 shards"), "{text}");
    assert!(text.contains("script_exception"), "{text}");
    assert!(session.registry.all().is_empty(), "the handle is released");
}

/// A loopback server that answers its connections with `pages` in order,
/// then `{}`, and keeps every request body it read.
async fn scripted(pages: Vec<&'static str>) -> (u16, Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let port = listener.local_addr().expect("an address").port();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&bodies);
    tokio::spawn(async move {
        let mut pages = pages.into_iter();
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut read = Vec::new();
            let mut buffer = [0_u8; 4096];
            let body = loop {
                match socket.read(&mut buffer).await {
                    Ok(0) | Err(_) => break String::new(),
                    Ok(n) => read.extend_from_slice(buffer.get(..n).unwrap_or_default()),
                }
                let text = String::from_utf8_lossy(&read).into_owned();
                let Some((head, body)) = text.split_once("\r\n\r\n") else {
                    continue;
                };
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break body.to_owned();
                }
            };
            kept.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(body);
            let _ = socket
                .write_all(&answer(pages.next().unwrap_or("{}")))
                .await;
            let _ = socket.shutdown().await;
        }
    });
    (port, bodies)
}

#[tokio::test]
async fn an_sql_cursor_asks_its_later_pages_columnar_again() {
    let (port, bodies) = scripted(vec![
        r#"{"columns":[{"name":"a","type":"long"}],"values":[[1,2]],"cursor":"c1"}"#,
        r#"{"values":[[3]]}"#,
    ])
    .await;
    let session = session_to(port);
    let mut cursor = session
        .execute(
            request("POST /_sql?format=json\n{\"query\":\"SELECT a FROM t\",\"fetch_size\":2,\"columnar\":true}"),
            &CancelToken::new(),
        )
        .await
        .expect("a cursor");
    let mut values = Vec::new();
    while let Some(batch) = cursor.next_batch().await.expect("a batch") {
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("int64");
        values.extend(column.values().iter().copied());
    }
    assert_eq!(values, [1, 2, 3]);
    let bodies = bodies
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let later: serde_json::Value =
        serde_json::from_str(bodies.get(1).expect("a second request")).expect("JSON");
    assert_eq!(later, serde_json::json!({"cursor": "c1", "columnar": true}));
}

#[tokio::test]
async fn a_point_in_time_page_that_timed_out_stops_the_stream_with_an_error() {
    let (port, _) = scripted(vec![
        r#"{"pit_id":"p2","timed_out":false,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[{"_index":"a","_id":"1","_score":null,"_source":{"x":1},"sort":[1]}]}}"#,
        r#"{"pit_id":"p3","timed_out":true,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]}}"#,
    ])
    .await;
    let session = session_to(port);
    let mut cursor = session
        .execute(
            request("POST /_search\n{\"size\":1,\"pit\":{\"id\":\"p1\",\"keep_alive\":\"1m\"},\"sort\":[{\"_shard_doc\":\"asc\"}]}"),
            &CancelToken::new(),
        )
        .await
        .expect("a cursor");
    assert!(cursor.next_batch().await.expect("the first page").is_some());
    let stopped = cursor.next_batch().await.expect_err("the partial page");
    assert!(stopped.to_string().contains("timed_out"), "{stopped}");
}

#[tokio::test]
async fn a_search_answer_streams_its_rows_within_the_row_limit() {
    let port = canned(answer(
        r#"{"took":2,"hits":{"hits":[{"_index":"a","_id":"1","_score":1,"_source":{"x":1}},{"_index":"a","_id":"2","_score":1,"_source":{"x":2}},{"_index":"a","_id":"3","_score":1,"_source":{"x":3}}]}}"#,
    ))
    .await;
    let session = session_to(port);
    let read =
        request("POST /a/_search\n{}").with_limits(ExecLimits::default().with_max_rows(Some(2)));
    let mut cursor = session
        .execute(read, &CancelToken::new())
        .await
        .expect("a cursor");
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await.expect("a batch") {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 2);
    assert!(cursor.stats().truncated);
    assert_eq!(cursor.stats().server_time, Some(Duration::from_millis(2)));
}

#[tokio::test]
async fn a_cancelled_read_is_cancelled_and_its_handle_released() {
    // Accepts and never answers: the execution waits until it is stopped.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let port = listener.local_addr().expect("an address").port();
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let session = session_to(port);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let outcome = session
        .execute(request("POST /a/_search\n{}"), &cancel)
        .await;
    assert!(
        matches!(outcome, Err(OxynError::Cancelled)),
        "{:?}",
        outcome.err()
    );
    assert!(session.registry.all().is_empty());
    holder.abort();
}
