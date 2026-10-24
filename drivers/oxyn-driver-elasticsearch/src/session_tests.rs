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
