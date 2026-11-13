//! The driver end to end, against a local server that speaks Hrana.
//!
//! Every test runs the real path — `open`, `Session::execute`, the cursor, the
//! catalog — over a real socket, so what is checked is what a Turso database
//! would see and answer, not a mock of the driver's own functions. Nothing
//! waits on the wall clock: tests wait on events the server raises.

use std::sync::Arc;
use std::time::Duration;

use arrow::array::AsArray as _;
use arrow::datatypes::{DataType, Int64Type};
use arrow::record_batch::RecordBatch;
use oxyn_catalog::CatalogPath;
use oxyn_core::{
    CancelToken, DriverId, ErrorClass, ExecLimits, ExecRequest, OxynError, PreviewShape, Result,
    ScalarValue, StatementIntent,
};
use oxyn_driver::{Cursor, Session};
use serde_json::json;

use crate::convert::{BATCH_ROWS, META_DECLARED_TYPE};
use crate::driver::open;
use crate::error::RESULT_TOO_LARGE;
use crate::http::Http;
use crate::options::ConnectSpec;
use crate::testserver::{Recorded, Reply, Server, TOKEN, affected, failure, rows, value};
use crate::variant::LANGUAGE;

const VERSION_PROBE: &str = "SELECT sqlite_version()";

/// Answers the connection probe, and hands every other request to `rest`.
fn database(
    rest: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
) -> impl Fn(&Recorded) -> Reply + Send + Sync + 'static {
    move |request| {
        if request.sql() == VERSION_PROBE {
            rows(
                &[("sqlite_version()", None)],
                vec![vec![value::text("3.45.1")]],
            )
        } else {
            rest(request)
        }
    }
}

async fn session_over(http: Http) -> Result<Box<dyn Session>> {
    open(Arc::new(http), DriverId::turso(), &CancelToken::new()).await
}

async fn session(server: &Server) -> Box<dyn Session> {
    let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN)).expect("a client");
    session_over(http).await.expect("a session")
}

fn read(sql: &str) -> ExecRequest {
    ExecRequest::new(LANGUAGE, sql).with_intent(StatementIntent::Read)
}

fn write(sql: &str) -> ExecRequest {
    let limits = ExecLimits {
        read_only: false,
        ..ExecLimits::default()
    };
    ExecRequest::new(LANGUAGE, sql)
        .with_intent(StatementIntent::Write)
        .with_limits(limits)
}

fn failed(outcome: Result<Box<dyn Cursor>>) -> OxynError {
    match outcome {
        Err(error) => error,
        Ok(_) => panic!("the execution should have failed"),
    }
}

async fn drain(cursor: &mut Box<dyn Cursor>) -> Vec<RecordBatch> {
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await.expect("a batch") {
        batches.push(batch);
    }
    batches
}

/// Opt-in counterpart to the wire fixtures, against a disposable libSQL
/// server. The plaintext client is test-only and restricted to the loopback.
#[tokio::test]
#[ignore = "requires a disposable local libSQL server in OXYN_TURSO_TEST_URL"]
async fn a_real_libsql_server_streams_tables_and_views_at_each_desktop_size() {
    let base = std::env::var("OXYN_TURSO_TEST_URL").expect("a disposable server URL is set");
    let endpoint = url::Url::parse(&base).expect("a local test URL");
    assert_eq!(endpoint.scheme(), "http", "a test-only plaintext client");
    assert_eq!(endpoint.host_str(), Some("127.0.0.1"), "only the loopback");
    assert!(endpoint.username().is_empty() && endpoint.password().is_none());
    let http = Http::new(ConnectSpec::plaintext(&base, TOKEN)).expect("a local test client");
    let session = session_over(http).await.expect("a real libSQL session");
    let table = format!(
        "oxyn_qa_{}",
        oxyn_core::CommandId::new().to_string().replace('-', "")
    );
    let create = format!(
        "CREATE TABLE {table} (id INTEGER NOT NULL, tenant INTEGER NOT NULL, payload TEXT, amount TEXT, note TEXT, PRIMARY KEY (tenant,id))"
    );
    let mut cursor = session
        .execute(write(&create), &CancelToken::new())
        .await
        .expect("create our own table");
    drain(&mut cursor).await;
    let values = (1..=201)
        .map(|id| {
            format!(
                "({id},{},'{{\"tags\":[\"synthetic\",null,\"東京\"]}}','123456789.0001',NULL)",
                (id - 1) % 3 + 1
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut cursor = session
        .execute(
            write(&format!("INSERT INTO {table} VALUES {values}")),
            &CancelToken::new(),
        )
        .await
        .expect("seed synthetic rows");
    drain(&mut cursor).await;
    let view = format!("{table}_view");
    let mut cursor = session
        .execute(
            write(&format!("CREATE VIEW {view} AS SELECT * FROM {table}")),
            &CancelToken::new(),
        )
        .await
        .expect("own view");
    drain(&mut cursor).await;
    for name in [&table, &view] {
        let path = CatalogPath::for_relation(None, Some("main"), name).expect("own path");
        for size in [50_u32, 100, 200] {
            let shape = PreviewShape {
                continuable: true,
                sort: vec![oxyn_core::PreviewSort::ascending("id")],
                ..PreviewShape::default()
            };
            let prepared = session
                .prepare_preview(&path, size, &shape, &CancelToken::new())
                .await
                .expect("retained preparation");
            assert_eq!(prepared.mode, oxyn_driver::PreviewMode::Retained);
            let mut cursor = session
                .execute_preview(prepared.request, &CancelToken::new())
                .await
                .expect("real Hrana cursor");
            let batches = drain(&mut cursor).await;
            let ids: Vec<i64> = batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .column(0)
                        .as_primitive::<Int64Type>()
                        .values()
                        .iter()
                        .copied()
                })
                .collect();
            assert_eq!(ids, (1..=201).collect::<Vec<i64>>());
        }
    }
    let mut cursor = session
        .execute(write(&format!("DROP VIEW {view}")), &CancelToken::new())
        .await
        .expect("remove own view");
    drain(&mut cursor).await;
    let mut cursor = session
        .execute(write(&format!("DROP TABLE {table}")), &CancelToken::new())
        .await
        .expect("remove only the table this test created");
    drain(&mut cursor).await;
}

#[tokio::test]
async fn opening_proves_the_token_and_speaks_hrana_on_the_pipeline_endpoint() {
    let server = Server::database(database(|_| unreachable!("only the probe"))).await;
    let _session = session(&server).await;

    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v2/pipeline");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(
        seen[0].body,
        json!({"requests": [
            {"type": "execute", "stmt": {"sql": VERSION_PROBE, "args": [], "want_rows": true}},
            {"type": "close"},
        ]})
    );
}

#[tokio::test]
async fn a_refused_token_is_an_authentication_error_that_does_not_repeat_it() {
    let server = Server::database(database(|_| unreachable!())).await;
    let http =
        Http::new(ConnectSpec::plaintext(&server.base(), "eyJ-wrong-token")).expect("client");
    let error = session_over(http).await.err().expect("refused");
    assert!(matches!(error, OxynError::Authentication(_)), "{error:?}");
    assert!(!error.to_string().contains("eyJ-wrong-token"), "{error}");
}

#[tokio::test]
async fn a_version_that_is_not_a_version_is_not_kept_nor_logged() {
    for hostile in ["3.45.1\nforged log line".to_owned(), "9".repeat(10_000)] {
        let server = Server::database(move |_| {
            rows(
                &[("sqlite_version()", None)],
                vec![vec![value::text(&hostile)]],
            )
        })
        .await;
        let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN)).expect("client");
        let error = session_over(http).await.err().expect("refused");
        assert!(matches!(error, OxynError::Connection(_)), "{error:?}");
    }
}

#[tokio::test]
async fn the_session_and_the_client_never_show_the_token() {
    let server = Server::database(database(|_| unreachable!())).await;
    let session = session(&server).await;
    // The concrete session is not reachable through the trait object, so the
    // client that built it stands for it.
    let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN)).expect("client");
    assert!(!format!("{http:?}").contains(TOKEN));
    drop(session);
}

#[tokio::test]
async fn a_select_arrives_as_typed_arrow_columns() {
    let server = Server::database(database(|request| {
        assert_eq!(request.sql(), "SELECT id, ratio, label, raw, note FROM t");
        rows(
            &[
                ("id", Some("INTEGER")),
                ("ratio", Some("REAL")),
                ("label", Some("TEXT")),
                ("raw", Some("BLOB")),
                ("note", None),
            ],
            vec![
                vec![
                    value::int(i64::MAX),
                    value::float(0.25),
                    value::text("é"),
                    value::blob("AP8"),
                    value::null(),
                ],
                vec![
                    value::int(-1),
                    value::float(2.0),
                    value::text("b"),
                    value::blob("AQI="),
                    value::text("x"),
                ],
            ],
        )
    }))
    .await;
    let session = session(&server).await;
    let mut cursor = session
        .execute(
            read("SELECT id, ratio, label, raw, note FROM t"),
            &CancelToken::new(),
        )
        .await
        .map_err(|error| error.to_string())
        .expect("cursor");
    let schema = cursor.schema();
    let types: Vec<&DataType> = schema.fields().iter().map(|f| f.data_type()).collect();
    assert_eq!(
        types,
        [
            &DataType::Int64,
            &DataType::Float64,
            &DataType::Utf8,
            &DataType::Binary,
            &DataType::Utf8
        ]
    );
    assert_eq!(
        schema
            .field(0)
            .metadata()
            .get(META_DECLARED_TYPE)
            .map(String::as_str),
        Some("INTEGER")
    );
    let batches = drain(&mut cursor).await;
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(
        batch.column(0).as_primitive::<Int64Type>().value(0),
        i64::MAX
    );
    assert_eq!(batch.column(2).as_string::<i32>().value(0), "é");
    assert_eq!(batch.column(3).as_binary::<i32>().value(0), [0, 255]);
    assert!(batch.column(4).is_null(0));
    assert_eq!(cursor.stats().rows, 2);
    assert!(!cursor.stats().truncated);
    assert!(cursor.stats().server_time.is_some());
}

#[tokio::test]
async fn a_write_reports_the_rows_it_changed_and_returns_no_batch() {
    let server = Server::database(database(|_| affected(3))).await;
    let session = session(&server).await;
    let mut cursor = session
        .execute(write("UPDATE t SET a = 1"), &CancelToken::new())
        .await
        .map_err(|error| error.to_string())
        .expect("cursor");
    assert_eq!(cursor.schema().fields().len(), 0);
    assert!(drain(&mut cursor).await.is_empty());
    assert_eq!(cursor.stats().rows, 3);
}

#[tokio::test]
async fn rows_past_the_bound_are_dropped_and_the_result_says_so() {
    let server = Server::database(database(|_| {
        rows(
            &[("n", Some("INTEGER"))],
            (0..5).map(|n| vec![value::int(n)]).collect(),
        )
    }))
    .await;
    let session = session(&server).await;
    let mut request = read("SELECT n FROM t");
    request.limits.max_rows = Some(2);
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .map_err(|error| error.to_string())
        .expect("cursor");
    let batches = drain(&mut cursor).await;
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    assert!(cursor.stats().truncated);
}

#[tokio::test]
async fn a_large_result_comes_in_batches_closed_by_size() {
    let total = BATCH_ROWS * 2 + 10;
    let server = Server::database(database(move |_| {
        rows(
            &[("n", Some("INTEGER"))],
            (0..total)
                .map(|n| vec![value::int(i64::try_from(n).unwrap_or(0))])
                .collect(),
        )
    }))
    .await;
    let session = session(&server).await;
    let mut request = read("SELECT n FROM t");
    request.limits.max_rows = None;
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .map_err(|error| error.to_string())
        .expect("cursor");
    let sizes: Vec<usize> = drain(&mut cursor)
        .await
        .iter()
        .map(RecordBatch::num_rows)
        .collect();
    assert_eq!(sizes, [BATCH_ROWS, BATCH_ROWS, 10]);
}

#[tokio::test]
async fn bound_values_travel_as_typed_arguments_never_in_the_text() {
    let server = Server::database(database(|_| rows(&[("a", None)], vec![]))).await;
    let session = session(&server).await;
    let sql = "SELECT * FROM t WHERE a = ?1 AND b = ?2 AND c = ?3";
    let request = read(sql).with_params(vec![
        ScalarValue::Text("O'Brien'; DROP TABLE t; --".to_owned()),
        ScalarValue::Int64(3),
        ScalarValue::Null,
    ]);
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .map_err(|error| error.to_string())
        .expect("cursor");
    drain(&mut cursor).await;
    let sent = server.requests().pop().expect("a request");
    assert_eq!(sent.sql(), sql);
    assert_eq!(
        sent.args(),
        &json!([
            {"type": "text", "value": "O'Brien'; DROP TABLE t; --"},
            {"type": "integer", "value": "3"},
            {"type": "null"},
        ])
    );
}

#[tokio::test]
async fn what_a_stream_cannot_carry_is_refused_before_anything_is_sent() {
    let server = Server::database(database(|_| unreachable!())).await;
    let session = session(&server).await;
    for sql in [
        "BEGIN",
        "COMMIT",
        "PRAGMA foreign_keys = ON",
        "ATTACH 'x' AS y",
    ] {
        let error = failed(session.execute(read(sql), &CancelToken::new()).await);
        assert!(
            matches!(error, OxynError::NotSupported { .. }),
            "{sql}: {error:?}"
        );
    }
    assert_eq!(
        server.requests().len(),
        1,
        "only the opening probe went out"
    );
}

#[tokio::test]
async fn a_write_under_a_read_only_bound_is_denied_without_a_request() {
    let server = Server::database(database(|_| unreachable!())).await;
    let session = session(&server).await;
    let request = ExecRequest::new(LANGUAGE, "DELETE FROM t")
        .with_intent(StatementIntent::Write)
        .with_limits(ExecLimits::default());
    assert!(request.limits.read_only, "the default bound is read-only");
    let error = failed(session.execute(request, &CancelToken::new()).await);
    assert!(matches!(error, OxynError::PolicyDenied { .. }), "{error:?}");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_stopped_read_is_cancelled_and_its_request_is_dropped() {
    let server = Server::database(database(|_| Reply::Hang)).await;
    let session = session(&server).await;
    let token = CancelToken::new();
    let execution = session.execute(read("SELECT count(*) FROM big"), &token);
    let stop = async {
        server.requests_seen(2).await;
        token.cancel();
    };
    let (outcome, ()) = tokio::join!(execution, stop);
    assert!(matches!(failed(outcome), OxynError::Cancelled));
    // The connection was closed: that is all a stop can do against Turso.
    server.hung_up(1).await;
}

#[tokio::test]
async fn a_stopped_write_is_never_reported_as_cancelled() {
    let server = Server::database(database(|_| Reply::Hang)).await;
    let session = session(&server).await;
    let token = CancelToken::new();
    let execution = session.execute(write("INSERT INTO t VALUES (1)"), &token);
    let stop = async {
        server.requests_seen(2).await;
        token.cancel();
    };
    let (outcome, ()) = tokio::join!(execution, stop);
    let error = failed(outcome);
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    assert!(!error.is_retryable());
    server.hung_up(1).await;
}

#[tokio::test]
async fn an_execution_cancelled_before_it_starts_sends_nothing() {
    let server = Server::database(database(|_| unreachable!())).await;
    let session = session(&server).await;
    let token = CancelToken::new();
    token.cancel();
    for request in [read("SELECT 1"), write("DELETE FROM t")] {
        let error = failed(session.execute(request, &token).await);
        assert!(matches!(error, OxynError::Cancelled), "{error:?}");
    }
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_statement_past_its_deadline_times_out_and_a_write_is_not_replayable() {
    let server = Server::database(database(|_| Reply::Hang)).await;
    let session = session(&server).await;
    // A zero deadline elapses the first time the request would have to wait:
    // no clock is read.
    let mut request = write("INSERT INTO t VALUES (1)");
    request.limits.timeout = Some(Duration::ZERO);
    let error = failed(session.execute(request, &CancelToken::new()).await);
    assert!(matches!(error, OxynError::Timeout { .. }), "{error:?}");
    assert_eq!(error.class(), ErrorClass::Ambiguous);
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn a_result_past_the_budget_is_refused_and_the_stream_is_not_read_to_its_end() {
    const FLOOD: usize = 256 * 1024 * 1024;
    let server = Server::database(database(|_| Reply::Flood(FLOOD))).await;
    let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN))
        .expect("client")
        .with_max_response(64 * 1024);
    let session = open_without_probe(http);
    let error = failed(
        session
            .execute(read("SELECT * FROM huge"), &CancelToken::new())
            .await,
    );
    assert!(
        matches!(&error, OxynError::Query(text) if text == RESULT_TOO_LARGE),
        "{error:?}"
    );
    assert!(
        server.flooded() < FLOOD,
        "the client stopped reading at its budget: {} of {FLOOD} bytes written",
        server.flooded()
    );
}

/// A session on a client, without the opening probe: the flood server answers
/// every request with a flood, the probe included.
fn open_without_probe(http: Http) -> crate::session::TursoSession {
    crate::session::TursoSession::new(Arc::new(http), DriverId::turso(), "3.45.1".to_owned())
}

#[tokio::test]
async fn a_declared_length_past_the_budget_is_refused_before_the_body_is_read() {
    let server = Server::database(database(|_| {
        rows(&[("v", None)], vec![vec![value::text(&"x".repeat(4096))]])
    }))
    .await;
    let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN))
        .expect("client")
        .with_max_response(1024);
    // The probe's answer is under the budget; the statement's is not.
    let session = session_over(http).await.expect("a session");
    let error = failed(
        session
            .execute(read("SELECT v FROM t"), &CancelToken::new())
            .await,
    );
    assert!(matches!(error, OxynError::Query(_)), "{error:?}");
}

#[tokio::test]
async fn the_token_a_server_echoes_is_removed_from_the_message() {
    let server = Server::database(database(|request| {
        let echoed = request.authorization.clone().unwrap_or_default();
        Reply::Json(
            400,
            json!({"error": format!("rejected header {echoed}")}).to_string(),
        )
    }))
    .await;
    let session = session(&server).await;
    let error = failed(session.execute(read("SELECT 1"), &CancelToken::new()).await);
    assert!(error.to_string().contains("rejected header"), "{error}");
    assert!(!error.to_string().contains(TOKEN), "{error}");
}

#[tokio::test]
async fn http_failures_are_classified_by_what_the_statement_may_have_done() {
    let server = Server::database(database(|_| {
        Reply::Json(503, json!({"error": "overloaded"}).to_string())
    }))
    .await;
    let session = session(&server).await;
    let read_error = failed(session.execute(read("SELECT 1"), &CancelToken::new()).await);
    assert!(
        matches!(read_error, OxynError::Connection(_)),
        "{read_error:?}"
    );
    assert!(read_error.is_retryable());
    let write_error = failed(
        session
            .execute(write("DELETE FROM t"), &CancelToken::new())
            .await,
    );
    assert!(
        matches!(write_error, OxynError::OutcomeUnknown(_)),
        "{write_error:?}"
    );
    assert!(!write_error.is_retryable());
}

#[tokio::test]
async fn a_body_that_breaks_in_flight_is_a_failed_exchange_not_a_foreign_answer() {
    let server = Server::database(database(|_| Reply::Truncated)).await;
    let session = session(&server).await;
    let read_error = failed(session.execute(read("SELECT 1"), &CancelToken::new()).await);
    assert!(
        matches!(read_error, OxynError::Connection(_)),
        "{read_error:?}"
    );
    assert!(read_error.is_retryable());
    let write_error = failed(
        session
            .execute(write("DELETE FROM t"), &CancelToken::new())
            .await,
    );
    assert!(
        matches!(write_error, OxynError::OutcomeUnknown(_)),
        "{write_error:?}"
    );
}

#[tokio::test]
async fn an_answer_that_is_not_hrana_is_permanent_for_a_read_and_a_doubt_for_a_write() {
    let server = Server::database(database(|_| {
        Reply::Raw(200, b"<html>captive portal</html>".to_vec())
    }))
    .await;
    let session = session(&server).await;
    let read_error = failed(session.execute(read("SELECT 1"), &CancelToken::new()).await);
    assert_eq!(read_error.class(), ErrorClass::Permanent);
    let write_error = failed(
        session
            .execute(write("DELETE FROM t"), &CancelToken::new())
            .await,
    );
    assert!(
        matches!(write_error, OxynError::OutcomeUnknown(_)),
        "{write_error:?}"
    );
}

#[tokio::test]
async fn a_statement_error_is_a_permanent_query_error_and_a_lock_is_transient() {
    let server = Server::database(database(|request| {
        if request.sql().contains("busy") {
            failure("SQLITE_BUSY", "database is locked")
        } else {
            failure("SQLITE_CONSTRAINT", "UNIQUE constraint failed: t.id")
        }
    }))
    .await;
    let session = session(&server).await;
    let constraint = failed(
        session
            .execute(write("INSERT INTO t VALUES (1)"), &CancelToken::new())
            .await,
    );
    assert!(
        matches!(&constraint, OxynError::Query(text) if text.contains("t.id")),
        "{constraint:?}"
    );
    assert_eq!(constraint.class(), ErrorClass::Permanent);
    let busy = failed(
        session
            .execute(write("INSERT INTO busy VALUES (1)"), &CancelToken::new())
            .await,
    );
    // Not proven undone behind a primary: never offered for replay.
    assert_eq!(busy.class(), ErrorClass::Ambiguous);
    assert!(!busy.is_retryable());
}

#[tokio::test]
async fn ping_is_a_round_trip() {
    let server = Server::database(database(|_| {
        rows(&[("1", None)], vec![vec![value::int(1)]])
    }))
    .await;
    let session = session(&server).await;
    session.ping().await.expect("a pong");
    assert_eq!(server.requests().last().expect("ping").sql(), "SELECT 1");
}

#[tokio::test]
async fn cancelling_a_statement_that_is_over_is_not_an_error() {
    let server = Server::database(database(|_| unreachable!())).await;
    let session = session(&server).await;
    session
        .cancel(oxyn_core::StatementHandle::new())
        .await
        .expect("nothing to stop");
}

mod catalog {
    use oxyn_catalog::model::{ReferentialAction, RelationKind};

    use super::*;

    fn fixture(request: &Recorded) -> Reply {
        let sql = request.sql();
        if sql.contains("FROM sqlite_master WHERE type IN") {
            rows(
                &[("name", None), ("type", None)],
                vec![
                    vec![value::text("orders"), value::text("table")],
                    vec![value::text("recent"), value::text("view")],
                ],
            )
        } else if sql.contains("CASE WHEN sql LIKE") {
            rows(
                &[("type", None), ("virtual", None)],
                vec![vec![value::text("table"), value::int(0)]],
            )
        } else if sql.contains("pragma_table_info(?1) ORDER BY cid") {
            rows(
                &[("cid", None)],
                vec![
                    vec![
                        value::int(0),
                        value::text("id"),
                        value::text("INTEGER"),
                        value::int(1),
                        value::null(),
                        value::int(1),
                    ],
                    vec![
                        value::int(1),
                        value::text("label"),
                        value::text("VARCHAR(20)"),
                        value::int(0),
                        value::text("'none'"),
                        value::int(0),
                    ],
                ],
            )
        } else if sql.contains("AS rowid_key") {
            rows(&[("rowid_key", None)], vec![vec![value::int(1)]])
        } else if sql.contains("pragma_index_list") {
            rows(
                &[("name", None)],
                vec![
                    vec![value::text("ix_label"), value::int(0), value::text("label")],
                    vec![value::text("ux_pair"), value::int(1), value::text("a")],
                    vec![value::text("ux_pair"), value::int(1), value::text("b")],
                    vec![value::text("ix_expr"), value::int(0), value::null()],
                ],
            )
        } else if sql.contains("pragma_foreign_key_list") {
            rows(
                &[("id", None)],
                vec![vec![
                    value::int(0),
                    value::text("users"),
                    value::text("user_id"),
                    value::null(),
                    value::text("CASCADE"),
                ]],
            )
        } else if sql.contains("pragma_table_info(?1) WHERE pk > 0") {
            rows(&[("name", None)], vec![vec![value::text("id")]])
        } else {
            panic!("an unexpected catalog statement: {sql}");
        }
    }

    async fn catalog_session() -> (Server, Box<dyn Session>) {
        let server = Server::database(database(fixture)).await;
        let session = session(&server).await;
        (server, session)
    }

    fn relation(name: &str) -> CatalogPath {
        CatalogPath::for_relation(None, Some("main"), name).expect("path")
    }

    #[tokio::test]
    async fn the_tree_has_one_namespace_and_lists_tables_and_views() {
        let (_server, session) = catalog_session().await;
        let catalog = session.catalog();
        let cancel = CancelToken::new();
        assert!(
            catalog
                .list_catalogs(&cancel)
                .await
                .expect("catalogs")
                .is_empty()
        );
        let namespaces = catalog
            .list_namespaces(None, &cancel)
            .await
            .expect("namespaces");
        assert_eq!(namespaces.len(), 1);
        assert_eq!(namespaces[0].name(), "main");
        assert_eq!(
            catalog.resolution_path(&cancel).await.expect("path"),
            ["main"]
        );

        let listed = catalog
            .list_relations(&namespaces[0].path(), &cancel)
            .await
            .expect("relations");
        let summary: Vec<(&str, RelationKind)> =
            listed.iter().map(|r| (r.name(), r.kind)).collect();
        assert_eq!(
            summary,
            [
                ("orders", RelationKind::Table),
                ("recent", RelationKind::View)
            ]
        );
        let info = catalog.server_info(&cancel).await.expect("info");
        assert_eq!(info.version, "3.45.1");
    }

    #[tokio::test]
    async fn a_table_is_described_with_its_key_and_declared_types() {
        let (_server, session) = catalog_session().await;
        let described = session
            .catalog()
            .describe_relation(&relation("orders"), &CancelToken::new())
            .await
            .expect("relation");
        assert_eq!(described.fields.len(), 2);
        let id = &described.fields[0];
        assert!(id.is_primary_key && !id.nullable);
        assert_eq!(id.primary_key_rank, Some(1));
        let label = &described.fields[1];
        assert!(label.nullable);
        assert_eq!(label.default.as_deref(), Some("'none'"));
        assert_eq!(label.raw_type, "VARCHAR(20)");
        assert!(described.previews_in_key_order());
    }

    #[tokio::test]
    async fn indexes_group_their_columns_and_skip_an_expression() {
        let (_server, session) = catalog_session().await;
        let indexes = session
            .catalog()
            .list_indexes(&relation("orders"), &CancelToken::new())
            .await
            .expect("indexes");
        let summary: Vec<(&str, bool, Vec<&str>)> = indexes
            .iter()
            .map(|i| {
                (
                    i.name.as_str(),
                    i.unique,
                    i.fields.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("ix_label", false, vec!["label"]),
                ("ux_pair", true, vec!["a", "b"]),
                ("ix_expr", false, vec![]),
            ]
        );
    }

    #[tokio::test]
    async fn an_implicit_foreign_key_target_is_resolved_to_the_primary_key() {
        let (server, session) = catalog_session().await;
        let keys = session
            .catalog()
            .list_foreign_keys(&relation("orders"), &CancelToken::new())
            .await
            .expect("keys");
        assert_eq!(keys.len(), 1);
        assert!(keys[0].is_well_formed());
        assert_eq!(keys[0].references.fields, ["id"]);
        assert_eq!(keys[0].on_delete, ReferentialAction::Cascade);
        // The target's name went as an argument of the lookup.
        let lookup = server.requests().pop().expect("the lookup");
        assert_eq!(lookup.args(), &json!([{"type": "text", "value": "users"}]));
    }

    #[tokio::test]
    async fn a_hostile_relation_name_is_a_bound_value_in_every_introspection() {
        let (server, session) = catalog_session().await;
        let evil = "users\"; DROP TABLE audit; --";
        let cancel = CancelToken::new();
        let _ = session
            .catalog()
            .describe_relation(&relation(evil), &cancel)
            .await;
        let _ = session
            .catalog()
            .list_indexes(&relation(evil), &cancel)
            .await;
        let _ = session
            .catalog()
            .list_foreign_keys(&relation(evil), &cancel)
            .await;
        // Past the opening probe, and past the primary-key lookup, whose
        // argument is the foreign key's target (the fixture's `users`).
        let sent: Vec<Recorded> = server
            .requests()
            .into_iter()
            .skip(1)
            .filter(|request| request.path != "/v3" && !request.sql().contains("WHERE pk > 0"))
            .collect();
        assert!(sent.len() >= 3);
        for request in sent {
            assert!(!request.sql().contains("DROP"), "{}", request.sql());
            assert_eq!(
                request.args()[0],
                json!({"type": "text", "value": evil}),
                "{}",
                request.sql()
            );
        }
    }

    #[tokio::test]
    async fn another_database_than_main_is_refused() {
        let (_server, session) = catalog_session().await;
        let other = CatalogPath::for_relation(None, Some("other"), "t").expect("path");
        let error = session
            .catalog()
            .describe_relation(&other, &CancelToken::new())
            .await
            .expect_err("one database");
        assert!(matches!(error, OxynError::Query(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_preview_is_composed_from_what_the_catalog_says() {
        let (_server, session) = catalog_session().await;
        let request = session
            .preview_request(
                &relation("orders"),
                10,
                &PreviewShape::unordered(),
                &CancelToken::new(),
            )
            .await
            .expect("request");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"orders\" ORDER BY \"id\" ASC LIMIT 10"
        );
        assert_eq!(request.intent, StatementIntent::Read);
    }

    #[tokio::test]
    async fn preview_page_sizes_and_offsets_reach_the_hrana_server() {
        let server = Server::database(database(|request| {
            for size in [50_u32, 100, 200] {
                for offset in [0, size] {
                    let suffix = if offset == 0 {
                        String::new()
                    } else {
                        format!(" OFFSET {offset}")
                    };
                    let expected = format!(
                        "SELECT * FROM \"main\".\"orders\" ORDER BY \"id\" ASC LIMIT {size}{suffix}"
                    );
                    if request.sql() == expected {
                        return rows(
                            &[("id", Some("INTEGER"))],
                            (offset + 1..=201.min(offset + size))
                                .map(|id| vec![value::int(i64::from(id))])
                                .collect(),
                        );
                    }
                }
            }
            fixture(request)
        }))
        .await;
        let session = session(&server).await;
        for size in [50_u32, 100, 200] {
            for offset in [0, size] {
                let shape = PreviewShape {
                    offset: u64::from(offset),
                    ..PreviewShape::unordered()
                };
                let request = session
                    .preview_request(&relation("orders"), size, &shape, &CancelToken::new())
                    .await
                    .expect("a bounded page");
                let mut cursor = session
                    .execute(request, &CancelToken::new())
                    .await
                    .expect("a page is read");
                let batches = drain(&mut cursor).await;
                let ids: Vec<i64> = batches
                    .iter()
                    .flat_map(|batch| {
                        batch
                            .column(0)
                            .as_primitive::<Int64Type>()
                            .values()
                            .iter()
                            .copied()
                    })
                    .collect();
                assert_eq!(
                    ids,
                    (offset + 1..=201.min(offset + size))
                        .map(i64::from)
                        .collect::<Vec<_>>()
                );
            }
        }
        assert_eq!(
            server
                .requests()
                .iter()
                .filter(|request| request.sql().starts_with("SELECT * FROM"))
                .count(),
            6
        );
    }
}

mod connection {
    use oxyn_core::{ConnectionConfig, Environment};
    use oxyn_driver::{Credentials, Driver as _};
    use secrecy::SecretString;

    use crate::driver::TursoDriver;

    #[tokio::test]
    async fn a_url_that_is_not_https_or_libsql_never_reaches_the_network() {
        let server = super::Server::database(super::database(|_| unreachable!())).await;
        let config = ConnectionConfig::new("edge", oxyn_core::DriverId::turso())
            .with_environment(Environment::Local)
            .with_param("url", server.base());
        let credentials = Credentials::new().with_token(SecretString::from(super::TOKEN));
        let error = TursoDriver::new()
            .connect(&config, &credentials, &oxyn_core::CancelToken::new())
            .await
            .err()
            .expect("a plain http URL is refused, even for a local connection");
        assert!(
            matches!(error, oxyn_core::OxynError::Config(_)),
            "{error:?}"
        );
        assert!(server.requests().is_empty());
    }
}
