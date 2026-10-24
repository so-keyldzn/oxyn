//! Tests against a real Elasticsearch or OpenSearch cluster.
//!
//! Every test is `#[ignore]`: run with `--ignored` and
//! `OXYN_ELASTICSEARCH_TEST_URL=http://host:port` — a disposable single-node
//! cluster with security disabled, which this suite fills and empties. Without
//! the variable a test returns at once and prints `not set: test skipped`;
//! `make test-serveurs` refuses a CI run where that line appears. Reading the
//! variable is the test's doing, never the driver's.
//!
//! Every index a test creates is named `oxyn-it-…` and deleted by the test;
//! the data is bounded (the streaming test writes about 300 MB of a highly
//! compressible `_source`, a few MB on disk). The tests share the cluster:
//! nextest runs them one at a time (`.config/nextest.toml`).
//!
//! | Module | What it holds |
//! |---|---|
//! | here | opening, errors, read-only, classification against the server, results |
//! | [`catalog`] | indices, aliases, data streams, mappings, hostile names, preview |
//! | [`stop`] | the two tests the checklist imposes |

mod catalog;
mod stop;

use arrow::array::{Array, StringArray};
use arrow::record_batch::RecordBatch;
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits, ExecRequest,
    OxynError, QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::ElasticsearchDriver;

/// The variable that carries the test cluster URL.
const VARIABLE: &str = "OXYN_ELASTICSEARCH_TEST_URL";

/// The test cluster's connection, or `None` when the variable is absent.
fn target() -> Option<ConnectionConfig> {
    let Ok(url) = std::env::var(VARIABLE) else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    Some(
        ConnectionConfig::new("elasticsearch-it", DriverId::elasticsearch())
            .with_environment(Environment::Local)
            .with_param("url", url),
    )
}

/// A session on the test cluster, or `None` when the variable is absent.
pub(super) async fn session() -> Option<Box<dyn Session>> {
    let config = target()?;
    Some(
        ElasticsearchDriver::new()
            .connect(&config, &Credentials::new(), &CancelToken::new())
            .await
            .expect("connect to the disposable cluster"),
    )
}

/// A request that may write: a test's own setup.
pub(super) fn write(text: &str) -> ExecRequest {
    let mut request = ExecRequest::new(QueryLanguage::SearchDsl, text)
        .with_limits(ExecLimits::default().with_max_rows(None));
    request.limits.read_only = false;
    request
}

/// A read, bounded as the console bounds it.
pub(super) fn read(text: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SearchDsl, text)
}

/// Runs a request to its end and returns its batches.
pub(super) async fn run(session: &dyn Session, request: ExecRequest) -> Vec<RecordBatch> {
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("the request runs");
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await.expect("a batch") {
        batches.push(batch);
    }
    batches
}

/// Applies a setup request, `?refresh=true` left to the caller.
pub(super) async fn apply(session: &dyn Session, text: &str) {
    run(session, write(text)).await;
}

/// The text cells of `column`, every batch in order.
pub(super) fn column(batches: &[RecordBatch], name: &str) -> Vec<Option<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            let column = batch
                .column_by_name(name)
                .unwrap_or_else(|| panic!("a column `{name}`"));
            let column = column
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a text column")
                .clone();
            (0..column.len())
                .map(move |row| (!column.is_null(row)).then(|| column.value(row).to_owned()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Deletes the test's indices, whatever state the test left them in.
pub(super) async fn clean(session: &dyn Session, pattern: &str) {
    let _ = session
        .execute(
            write(&format!("DELETE /{pattern}?ignore_unavailable=true")),
            &CancelToken::new(),
        )
        .await;
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn the_product_and_its_version_come_from_the_cluster() {
    let Some(session) = session().await else {
        return;
    };
    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("server information");
    assert!(
        info.product == "Elasticsearch" || info.product == "OpenSearch",
        "{}",
        info.product
    );
    assert!(info.version.split('.').count() >= 2, "{}", info.version);
    assert!(
        session
            .capabilities()
            .contains(oxyn_core::Capabilities::SERVER_SIDE_CANCEL),
        "a security-disabled cluster lets the account list tasks"
    );
    session.ping().await.expect("a ping");
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_refusal_is_the_servers_own_words_and_permanent() {
    let Some(session) = session().await else {
        return;
    };
    let error = session
        .execute(read("GET /oxyn-it-absent/_search"), &CancelToken::new())
        .await
        .err()
        .expect("no such index");
    assert!(
        error.to_string().contains("index_not_found_exception"),
        "{error}"
    );
    assert_eq!(error.class(), ErrorClass::Permanent);

    let error = session
        .execute(
            read("POST /_search\n{\"query\":{\"no_such_query\":{}}}"),
            &CancelToken::new(),
        )
        .await
        .err()
        .expect("an unknown query");
    assert!(error.to_string().contains("HTTP 400"), "{error}");
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_read_only_execution_never_sends_a_write() {
    let Some(session) = session().await else {
        return;
    };
    clean(&*session, "oxyn-it-ro").await;
    let refused = session
        .execute(read("PUT /oxyn-it-ro"), &CancelToken::new())
        .await
        .err()
        .expect("refused before sending");
    assert!(refused.to_string().contains("read-only"), "{refused}");
    let exists = run(&*session, read("HEAD /oxyn-it-ro")).await;
    let status = exists[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::UInt16Array>()
        .expect("a status")
        .value(0);
    assert_eq!(status, 404, "the index was never created");
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn documents_aggregations_and_cat_read_as_tables() {
    let Some(session) = session().await else {
        return;
    };
    let index = "oxyn-it-results";
    clean(&*session, index).await;
    apply(
        &*session,
        &format!(
            "PUT /{index}\n{{\"mappings\":{{\"properties\":{{\"status\":{{\"type\":\"keyword\"}},\"big\":{{\"type\":\"unsigned_long\"}},\"at\":{{\"type\":\"date\"}}}}}}}}"
        ),
    )
    .await;
    apply(
        &*session,
        &format!(
            "POST /_bulk?refresh=true\n\
             {{\"index\":{{\"_index\":\"{index}\",\"_id\":\"1\"}}}}\n\
             {{\"status\":\"500\",\"big\":18446744073709551615,\"at\":\"2024-01-01T00:00:00Z\",\"tags\":[\"a\",\"b\"]}}\n\
             {{\"index\":{{\"_index\":\"{index}\",\"_id\":\"2\"}}}}\n\
             {{\"status\":\"200\",\"big\":9007199254740993}}"
        ),
    )
    .await;

    let hits = run(
        &*session,
        read(&format!(
            "POST /{index}/_search\n{{\"sort\":[{{\"status\":\"desc\"}}]}}"
        )),
    )
    .await;
    assert_eq!(
        column(&hits, "_id"),
        [Some("1".to_owned()), Some("2".to_owned())]
    );
    assert_eq!(
        column(&hits, "big"),
        [
            Some("18446744073709551615".to_owned()),
            Some("9007199254740993".to_owned())
        ],
        "beyond 2^53, exact"
    );
    assert_eq!(column(&hits, "tags")[0].as_deref(), Some(r#"["a","b"]"#));

    let buckets = run(
        &*session,
        read(&format!(
            "POST /{index}/_search\n{{\"size\":0,\"aggs\":{{\"by_status\":{{\"terms\":{{\"field\":\"status\"}}}}}}}}"
        )),
    )
    .await;
    assert_eq!(column(&buckets, "by_status").len(), 2);

    let cat = run(
        &*session,
        read(&format!("GET /_cat/indices/{index}?v&h=index,docs.count")),
    )
    .await;
    assert_eq!(column(&cat, "index"), [Some(index.to_owned())]);
    assert_eq!(column(&cat, "docs.count"), [Some("2".to_owned())]);
    clean(&*session, index).await;
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_bulk_with_a_failed_item_is_reported_and_the_rest_applied() {
    let Some(session) = session().await else {
        return;
    };
    let index = "oxyn-it-bulk";
    clean(&*session, index).await;
    apply(
        &*session,
        &format!(
            "PUT /{index}\n{{\"mappings\":{{\"properties\":{{\"n\":{{\"type\":\"long\"}}}}}}}}"
        ),
    )
    .await;
    let failed = session
        .execute(
            write(&format!(
                "POST /_bulk?refresh=true\n\
                 {{\"index\":{{\"_index\":\"{index}\",\"_id\":\"ok\"}}}}\n{{\"n\":1}}\n\
                 {{\"index\":{{\"_index\":\"{index}\",\"_id\":\"bad\"}}}}\n{{\"n\":\"not a number\"}}"
            )),
            &CancelToken::new(),
        )
        .await
        .err()
        .expect("a failed item");
    assert!(
        failed.to_string().contains("1 of 2 `_bulk` items failed"),
        "{failed}"
    );
    let count = run(&*session, read(&format!("GET /{index}/_count"))).await;
    assert_eq!(column(&count, "count"), [Some("1".to_owned())]);
    clean(&*session, index).await;
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_version_conflict_is_permanent() {
    let Some(session) = session().await else {
        return;
    };
    let index = "oxyn-it-conflict";
    clean(&*session, index).await;
    apply(
        &*session,
        &format!("PUT /{index}/_doc/1?refresh=true\n{{\"a\":1}}"),
    )
    .await;
    let conflict = session
        .execute(
            write(&format!("PUT /{index}/_create/1\n{{\"a\":2}}")),
            &CancelToken::new(),
        )
        .await
        .err()
        .expect("already exists");
    assert!(
        conflict
            .to_string()
            .contains("version_conflict_engine_exception"),
        "{conflict}"
    );
    assert_eq!(conflict.class(), ErrorClass::Permanent);
    assert!(!matches!(conflict, OxynError::OutcomeUnknown(_)));
    clean(&*session, index).await;
}
