//! Tests against a real ClickHouse server.
//!
//! Every test is `#[ignore]`: run with `--ignored` and
//! `OXYN_CLICKHOUSE_TEST_URL=http://user:password@host:port` (see the crate
//! documentation). Without the variable a test returns at once and prints
//! `not set: test skipped` — `make test-serveurs` refuses a CI run where that
//! line appears. Reading the variable is the test's doing, never the driver's.
//!
//! `--test-threads=1` is not decorative: the tests create and drop objects in
//! the same databases, and the cancellation tests count statements in
//! `system.processes`.
//!
//! The account must be able to create databases and kill its own statements:
//! the `CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1` user of a disposable container.
//!
//! | Module | What it holds |
//! |---|---|
//! | here | opening, errors, read-only, session state, the row bound |
//! | [`types`] | every type the decoder knows, compared with the server's own text |
//! | [`stop`] | the two tests the checklist imposes, and the verdict of a stopped write |
//! | [`catalog`] | introspection, hostile names, preview, context |

mod catalog;
mod stop;
mod types;

use arrow::record_batch::RecordBatch;
use arrow::util::display::array_value_to_string;
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits,
    ExecRequest, OxynError, ScalarValue, StatementIntent,
};
use oxyn_driver::{Credentials, Driver, Session};
use secrecy::SecretString;
use url::Url;

use crate::{ClickhouseDriver, ClickhouseError, LANGUAGE};

/// The variable that carries the test server URL.
const VARIABLE: &str = "OXYN_CLICKHOUSE_TEST_URL";

/// The test server, or `None` when the variable is absent.
fn target() -> Option<(ConnectionConfig, Credentials)> {
    let Ok(raw) = std::env::var(VARIABLE) else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    let url = Url::parse(&raw).expect("OXYN_CLICKHOUSE_TEST_URL is a URL");
    let config = ConnectionConfig::new("clickhouse-it", DriverId::clickhouse())
        .with_environment(Environment::Local)
        .with_param("host", url.host_str().expect("the URL names a host"))
        .with_param(
            "port",
            url.port().expect("the URL names a port").to_string(),
        )
        .with_param("user", url.username())
        .with_param("sslmode", "disable");
    let credentials = Credentials::new().with_password(SecretString::from(
        url.password()
            .expect("the URL carries a password")
            .to_owned(),
    ));
    Some((config, credentials))
}

/// A session on the test server, or `None` when the variable is absent.
pub(super) async fn session() -> Option<Box<dyn Session>> {
    let (config, credentials) = target()?;
    Some(open(&config, &credentials).await)
}

async fn open(config: &ConnectionConfig, credentials: &Credentials) -> Box<dyn Session> {
    ClickhouseDriver::new()
        .connect(config, credentials, &CancelToken::new())
        .await
        .expect("the test server accepts the connection")
}

/// A read, bounded as a production console bounds it: read-only on the
/// server, which then refuses a `SETTINGS` clause too.
pub(super) fn read(sql: &str) -> ExecRequest {
    ExecRequest::new(LANGUAGE, sql).with_intent(StatementIntent::Read)
}

/// A read outside production: not bounded read-only, so it may carry its
/// own `SETTINGS` clause.
pub(super) fn unbound_read(sql: &str) -> ExecRequest {
    read(sql).with_limits(ExecLimits::default().writable())
}

/// A write or a DDL, allowed to write.
pub(super) fn write(sql: &str, intent: StatementIntent) -> ExecRequest {
    ExecRequest::new(LANGUAGE, sql)
        .with_intent(intent)
        .with_limits(ExecLimits::default().writable())
}

/// Every batch of an execution.
pub(super) async fn run(
    session: &dyn Session,
    request: ExecRequest,
) -> Result<Vec<RecordBatch>, OxynError> {
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        batches.push(batch);
    }
    Ok(batches)
}

/// Runs a statement that must succeed.
pub(super) async fn apply(session: &dyn Session, sql: &str, intent: StatementIntent) {
    run(session, write(sql, intent))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

/// The first value of a one-column read, as Arrow displays it.
pub(super) async fn first(session: &dyn Session, sql: &str) -> String {
    let batches = run(session, read(sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let batch = batches
        .iter()
        .find(|batch| batch.num_rows() > 0)
        .unwrap_or_else(|| panic!("{sql}: no row"));
    array_value_to_string(batch.column(0), 0).expect("displayable")
}

/// The server's code of a driver error.
pub(super) fn code(error: &OxynError) -> Option<u32> {
    match error {
        OxynError::Driver { source, .. } => source
            .downcast_ref::<ClickhouseError>()
            .and_then(ClickhouseError::code),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_session_opens_and_says_what_it_can_do() {
    let Some(session) = session().await else {
        return;
    };
    let capabilities = session.capabilities();
    assert!(capabilities.supports_language(LANGUAGE));
    assert!(
        capabilities.contains(Capabilities::SERVER_SIDE_CANCEL),
        "the container's account may kill its own statements"
    );
    assert!(!capabilities.contains(Capabilities::TRANSACTIONS));
    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("server info");
    assert_eq!(info.product, "ClickHouse");
    assert!(!info.version.is_empty());
    session.ping().await.expect("ping");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_wrong_password_is_an_authentication_error() {
    let Some((config, _)) = target() else {
        return;
    };
    let wrong = Credentials::new().with_password(SecretString::from("not-the-password"));
    let error = ClickhouseDriver::new()
        .connect(&config, &wrong, &CancelToken::new())
        .await
        .err()
        .expect("refused");
    assert!(matches!(error, OxynError::Authentication(_)), "{error:?}");
    assert!(!error.to_string().contains("not-the-password"), "{error}");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn an_unreachable_server_is_a_connection_error() {
    let Some((config, credentials)) = target() else {
        return;
    };
    // Port 1 on the loopback: nothing listens there.
    let config = config.with_param("port", "1");
    let error = ClickhouseDriver::new()
        .connect(&config, &credentials, &CancelToken::new())
        .await
        .err()
        .expect("refused");
    assert!(matches!(error, OxynError::Connection(_)), "{error:?}");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn server_errors_carry_their_code_and_their_class() {
    let Some(session) = session().await else {
        return;
    };
    let syntax = run(&*session, read("SELEC 1")).await.expect_err("syntax");
    assert_eq!(code(&syntax), Some(62), "{syntax:?}");
    assert_eq!(syntax.class(), ErrorClass::Permanent);

    let missing = run(&*session, read("SELECT * FROM oxyn_no_such_table"))
        .await
        .expect_err("unknown table");
    assert_eq!(code(&missing), Some(60), "{missing:?}");
    assert_eq!(missing.class(), ErrorClass::Permanent);

    // Refused before it ran, and the summary says nothing was written.
    let insert = run(
        &*session,
        write(
            "INSERT INTO oxyn_no_such_table VALUES (1)",
            StatementIntent::Write,
        ),
    )
    .await
    .expect_err("unknown table");
    assert_eq!(code(&insert), Some(60), "{insert:?}");
    assert_eq!(insert.class(), ErrorClass::Permanent);

    // The server's own deadline on a read: passing.
    let slow = run(
        &*session,
        unbound_read(
            "SELECT sleepEachRow(0.1) FROM numbers(100) \
             SETTINGS max_block_size = 1, max_execution_time = 1",
        ),
    )
    .await
    .expect_err("timeout");
    assert_eq!(code(&slow), Some(159), "{slow:?}");
    assert_eq!(slow.class(), ErrorClass::Transient);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn an_exception_after_the_first_rows_ends_the_stream_classified() {
    let Some(session) = session().await else {
        return;
    };
    // Small blocks: the first rows leave before the exception is raised, so
    // the status is already 200 and the exception travels in the body.
    let sql = "SELECT throwIf(number = 50000, 'oxyn mid-stream failure') AS x \
               FROM numbers(100000) SETTINGS max_block_size = 1000";
    let mut cursor = session
        .execute(
            unbound_read(sql).with_limits(ExecLimits::default().writable().with_max_rows(None)),
            &CancelToken::new(),
        )
        .await
        .expect("the header arrives before the failure");
    let mut rows = 0;
    let error = loop {
        match cursor.next_batch().await {
            Ok(Some(batch)) => rows += batch.num_rows(),
            Ok(None) => panic!("the stream ended cleanly after {rows} rows"),
            Err(error) => break error,
        }
    };
    assert!(rows > 0, "rows came before the exception");
    assert!(rows < 100_000, "the exception was decoded as rows: {rows}");
    assert_eq!(code(&error), Some(395), "{error:?}");
    assert!(
        error.to_string().contains("oxyn mid-stream failure"),
        "{error}"
    );
    drop(cursor);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn read_only_is_enforced_by_the_server() {
    let Some(session) = session().await else {
        return;
    };
    apply(
        &*session,
        "CREATE TABLE IF NOT EXISTS default.oxyn_ro (a UInt8) ENGINE = Memory",
        StatementIntent::Ddl,
    )
    .await;
    // A request marked read-only carries `readonly = 1`: the server refuses
    // the write, whatever intent the caller gave.
    let request = ExecRequest::new(LANGUAGE, "INSERT INTO default.oxyn_ro VALUES (1)")
        .with_intent(StatementIntent::Read);
    let error = run(&*session, request).await.expect_err("refused");
    assert_eq!(code(&error), Some(164), "{error:?}");
    let ddl = run(&*session, read("DROP TABLE default.oxyn_ro"))
        .await
        .expect_err("refused");
    assert_eq!(code(&ddl), Some(164), "{ddl:?}");
    // A table function that reaches out is refused before any connection.
    let reach = run(
        &*session,
        read("SELECT * FROM url('http://127.0.0.1:1/x', 'CSV', 'a UInt8')"),
    )
    .await
    .expect_err("refused");
    assert_eq!(code(&reach), Some(164), "{reach:?}");
    // The price of `readonly = 1`, documented in ADR-0060: a bounded read
    // cannot carry its own settings either.
    let settings = run(
        &*session,
        read("SELECT 1 FROM system.one SETTINGS max_threads = 2"),
    )
    .await
    .expect_err("refused");
    assert_eq!(code(&settings), Some(164), "{settings:?}");
    assert_eq!(settings.class(), ErrorClass::Permanent);
    assert_eq!(
        first(&*session, "SELECT count() FROM default.oxyn_ro").await,
        "0"
    );
    apply(
        &*session,
        "DROP TABLE default.oxyn_ro",
        StatementIntent::Ddl,
    )
    .await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn session_state_is_refused_before_it_reaches_the_server() {
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "USE system",
        "SET max_threads = 1",
        "CREATE TEMPORARY TABLE t (a UInt8)",
    ] {
        let error = run(&*session, write(sql, StatementIntent::Unknown))
            .await
            .expect_err(sql);
        assert!(
            matches!(error, OxynError::NotSupported { .. }),
            "{sql}: {error:?}"
        );
    }
    // A setting for one statement is the statement's own clause.
    let batches = run(
        &*session,
        unbound_read("SELECT getSetting('max_threads') FROM system.one SETTINGS max_threads = 3"),
    )
    .await
    .expect("a statement's own setting");
    assert_eq!(
        array_value_to_string(batches[0].column(0), 0).expect("displayable"),
        "3"
    );
    // Two statements in one request: the server refuses them itself.
    let error = run(&*session, read("SELECT 1; SELECT 2"))
        .await
        .expect_err("refused");
    assert_eq!(error.class(), ErrorClass::Permanent, "{error:?}");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_write_reports_its_rows_and_a_bounded_read_says_it_is_truncated() {
    let Some(session) = session().await else {
        return;
    };
    apply(
        &*session,
        "CREATE OR REPLACE TABLE default.oxyn_rows (n UInt64) ENGINE = MergeTree ORDER BY n",
        StatementIntent::Ddl,
    )
    .await;
    let mut cursor = session
        .execute(
            write(
                "INSERT INTO default.oxyn_rows SELECT number FROM numbers(1234)",
                StatementIntent::Write,
            ),
            &CancelToken::new(),
        )
        .await
        .expect("insert");
    assert!(cursor.schema().fields().is_empty());
    assert!(cursor.next_batch().await.expect("end").is_none());
    assert_eq!(cursor.stats().rows, 1234, "the summary's written rows");
    drop(cursor);

    let bounded = read("SELECT n FROM default.oxyn_rows ORDER BY n")
        .with_limits(ExecLimits::default().with_max_rows(100));
    let mut cursor = session
        .execute(bounded, &CancelToken::new())
        .await
        .expect("read");
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 100);
    assert!(cursor.stats().truncated, "a cut result says so");
    drop(cursor);

    let whole = read("SELECT n FROM default.oxyn_rows WHERE n < 100")
        .with_limits(ExecLimits::default().with_max_rows(100));
    let mut cursor = session
        .execute(whole, &CancelToken::new())
        .await
        .expect("read");
    while cursor.next_batch().await.expect("stream").is_some() {}
    assert!(!cursor.stats().truncated, "exactly the bound is whole");
    drop(cursor);
    // `TRUNCATE` is declared: the server runs it on a MergeTree table.
    apply(
        &*session,
        "TRUNCATE TABLE default.oxyn_rows",
        StatementIntent::Ddl,
    )
    .await;
    assert_eq!(
        first(&*session, "SELECT count() FROM default.oxyn_rows").await,
        "0"
    );
    apply(
        &*session,
        "DROP TABLE default.oxyn_rows",
        StatementIntent::Ddl,
    )
    .await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn bound_values_reach_their_placeholders_unchanged() {
    let Some(session) = session().await else {
        return;
    };
    let hostile = "a\\b\tc'; DROP TABLE audit; -- \\N";
    let instant = chrono::DateTime::from_timestamp(1_700_000_000, 123_456_789).expect("valid");
    let request = read(
        "SELECT {s:String} = {s:String} AS same, hex({s:String}) AS raw, \
                {n:Nullable(UInt8)} IS NULL AS null, {i:Int64} + 1 AS next, \
                toUnixTimestamp64Nano({t:DateTime64(9, 'UTC')}) AS nanos",
    )
    .with_params(vec![
        ScalarValue::Text(hostile.to_owned()),
        ScalarValue::Null,
        ScalarValue::Int64(41),
        ScalarValue::Timestamp(instant),
    ]);
    let batches = run(&*session, request).await.expect("bound");
    let batch = batches.first().expect("one batch");
    let cell = |name: &str| {
        array_value_to_string(batch.column_by_name(name).expect("column"), 0).expect("text")
    };
    // ClickHouse comparisons return `UInt8`.
    assert_eq!(cell("same"), "1");
    let expected_hex: String = hostile.bytes().map(|byte| format!("{byte:02X}")).collect();
    assert_eq!(cell("raw"), expected_hex, "the server binds the exact text");
    assert_eq!(cell("null"), "1");
    assert_eq!(cell("next"), "42");
    assert_eq!(cell("nanos"), "1700000000123456789");

    let mismatch = run(
        &*session,
        read("SELECT {a:UInt8}").with_params(vec![ScalarValue::Int64(1), ScalarValue::Int64(2)]),
    )
    .await
    .expect_err("two values for one placeholder");
    assert!(matches!(mismatch, OxynError::Config(_)), "{mismatch:?}");
    session.close().await.expect("close");
}
