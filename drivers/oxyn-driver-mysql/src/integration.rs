//! The tests that need a real MySQL or MariaDB server.
//!
//! All marked `#[ignore]`: `cargo test` without a server must neither fail nor
//! wait. Without `OXYN_MYSQL_TEST_URL` each test stops without failing and
//! says so. Reading the variable is the test's doing, never the driver's.
//!
//! # Running them
//!
//! ```sh
//! docker run -d --name oxyn-it-mysql -e MYSQL_ROOT_PASSWORD=oxyn -e MYSQL_DATABASE=t \
//!   -p 127.0.0.1:13306:3306 mysql:8.4
//! OXYN_MYSQL_TEST_URL='mysql://root:oxyn@127.0.0.1:13306/t?sslmode=prefer' \
//!   cargo test -p oxyn-driver-mysql -- --ignored --test-threads=1
//! docker rm -f oxyn-it-mysql
//! ```
//!
//! The same suite runs against `mysql:9.7` and `mariadb:11.8`
//! (`MARIADB_ROOT_PASSWORD`, `MARIADB_DATABASE`). The account must be able to
//! `SET GLOBAL local_infile`: the root of a disposable container.
//!
//! `--test-threads=1` is not decorative: the tests create and drop tables in
//! the same database.
//!
//! # What they hold
//!
//! The two the checklist imposes — **server-side cancellation observed in the
//! server's process list**, and **a stream whose batches stay bounded while
//! the result would not fit in memory** — and the observations ADR-0050 rests
//! on: 1064 before 1295, read-only refusals, `LOCAL INFILE` refused, zero
//! dates, `DECIMAL(65,30)`, `BIGINT UNSIGNED`, `VECTOR`, `CALL` with two
//! result sets, the transaction state, the transaction kept across a proven
//! kill (ADR-0050 §7 as amended), the verdict of a stopped write (never
//! `Cancelled` once it was sent: a MyISAM update, in a transaction or not, an
//! InnoDB update in a transaction, a `CALL`), a MariaDB
//! write returning rows past its bound drained rather than killed, the single
//! error of an execution that loses its transaction, the statements an
//! introspection closes, the columns only the execution gives, the
//! transaction kept across a failing statement, the bound on a kill the
//! server accepts and never answers, and a typed `USE` refused before anything
//! reaches the server (ADR-0019).

use std::time::Duration;

use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use arrow::util::display::array_value_to_string;
use oxyn_catalog::CatalogPath;
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits,
    ExecRequest, OxynError, PreviewShape, PreviewSort, ScalarValue, StatementIntent,
    TransactionState,
};
use oxyn_driver::{Credentials, Driver as _, ParsedDsn, Session, SessionContext};

use crate::cursor::BATCH_BYTE_BUDGET;
use crate::variant::LANGUAGE;
use crate::{META_MYSQL_TYPE, MysqlDriver, MysqlError};

/// The variable that carries the test server URL.
const VARIABLE: &str = "OXYN_MYSQL_TEST_URL";

/// Ten rows, to multiply into large results without a table or recursion.
const DIGITS: &str = "WITH d AS (SELECT 0 AS i UNION ALL SELECT 1 UNION ALL SELECT 2 \
     UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 \
     UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9)";

/// A statement that runs for minutes, marked so the process list finds it.
///
/// An aggregate: its columns come with its only row, at the end — MySQL sends
/// nothing before, so `execute` waits until then.
fn long_count() -> String {
    format!(
        "{DIGITS} SELECT COUNT(*) AS oxyn_long_marker \
         FROM d a, d b, d c, d e, d f, d g, d h, d j, d k, d l"
    )
}

/// Two gigabytes of rows that start at once: `execute` returns with their
/// columns, and the server then waits on the reader. Marked as
/// [`long_count`].
fn long_rows() -> String {
    format!(
        "{DIGITS} SELECT a.i, REPEAT('x', 200) AS oxyn_long_marker \
         FROM d a, d b, d c, d e, d f, d g, d h"
    )
}

/// Ten rows as a derived table, for a subquery that runs for minutes inside an
/// `UPDATE`: [`DIGITS`] is a `WITH`, which has to open the statement.
fn ten_rows(alias: char) -> String {
    format!(
        "(SELECT 0 AS i UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
         UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
         UNION ALL SELECT 8 UNION ALL SELECT 9) {alias}"
    )
}

/// An `UPDATE` of `table` (`id`, `n`) that changes row 1 at once and then
/// spends minutes in a subquery for any other row, marked as [`long_count`] so
/// the process list finds it. The row changed first is what a table without
/// rollback keeps when the statement is killed.
fn long_update(table: &str) -> String {
    let product = ('a'..='j').map(ten_rows).collect::<Vec<_>>().join(", ");
    format!(
        "UPDATE {table} SET n = n + 1 \
         WHERE id = 1 OR (SELECT COUNT(*) AS oxyn_long_marker FROM {product}) > 0"
    )
}

/// The batches a cursor still holds, then the error it ends on.
async fn end_of(cursor: &mut Box<dyn oxyn_driver::Cursor>) -> Result<(), OxynError> {
    while cursor.next_batch().await?.is_some() {}
    Ok(())
}

/// Runs `request` and fires its execution token once `observer` saw the
/// long statement run — while `execute` may still wait for the result's
/// columns, which is when the interface's Stop reaches it. Returns how many
/// long statements ran, and how the execution ended.
async fn cancel_through_the_token(
    session: &dyn Session,
    observer: &dyn Session,
    request: ExecRequest,
) -> (u64, Result<(), OxynError>) {
    let token = CancelToken::new();
    let executing = async {
        let mut cursor = session.execute(request, &token).await?;
        end_of(&mut cursor).await
    };
    let cancelling = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let running = running_long(observer).await;
        token.cancel();
        running
    };
    let (ended, running) = futures::future::join(executing, cancelling).await;
    (running, ended)
}

/// The test configuration, or `None` when no server is declared.
fn target() -> Option<(ConnectionConfig, Credentials)> {
    let url = std::env::var(VARIABLE).ok()?;
    let parsed = ParsedDsn::parse(&url).expect("OXYN_MYSQL_TEST_URL must be a `mysql://` URL");
    let (parts, credentials) = parsed.into_parts();
    let config = parts
        .to_config("trial", DriverId::mysql())
        .with_environment(Environment::Local);
    Some((config, credentials))
}

/// Opens a session, or returns `None` and says so.
async fn session() -> Option<Box<dyn Session>> {
    let Some((config, credentials)) = target() else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    Some(
        MysqlDriver::new()
            .connect(&config, &credentials, &CancelToken::new())
            .await
            .expect("the test server must be reachable"),
    )
}

/// The error of an execution that should have been refused. `expect_err`
/// would need `Debug` on `dyn Cursor`, which holds the connection.
fn refusal<T>(issue: Result<T, OxynError>, expected: &str) -> OxynError {
    match issue {
        Ok(_) => panic!("{expected}"),
        Err(error) => error,
    }
}

fn read_request(sql: &str) -> ExecRequest {
    ExecRequest::new(LANGUAGE, sql)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(None).with_timeout(None))
}

fn write_request(sql: &str) -> ExecRequest {
    ExecRequest::new(LANGUAGE, sql)
        .with_intent(StatementIntent::Write)
        .with_limits(
            ExecLimits::default()
                .writable()
                .with_max_rows(None)
                .with_timeout(None),
        )
}

/// Runs a request and collects its batches, or returns the first error.
async fn run(session: &dyn Session, request: ExecRequest) -> Result<Vec<RecordBatch>, OxynError> {
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        batches.push(batch);
    }
    Ok(batches)
}

/// Applies a statement that must be accepted.
async fn apply(session: &dyn Session, sql: &str) {
    run(session, write_request(sql))
        .await
        .unwrap_or_else(|error| panic!("`{sql}` must be accepted: {error}"));
}

/// The first value of a read, as text.
async fn first(session: &dyn Session, sql: &str) -> String {
    let batches = run(session, read_request(sql))
        .await
        .unwrap_or_else(|error| panic!("`{sql}`: {error}"));
    let batch = batches.first().expect("one batch");
    array_value_to_string(batch.column(0), 0).expect("printable")
}

/// The server's error number carried by a driver error.
fn server_code(error: &OxynError) -> Option<u16> {
    match error {
        OxynError::Driver { source, .. } => source.downcast_ref::<MysqlError>()?.code(),
        _ => None,
    }
}

async fn is_mariadb(session: &dyn Session) -> bool {
    session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .is_ok_and(|info| info.product == "MariaDB")
}

async fn server_version(session: &dyn Session) -> String {
    session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("server identity")
        .version
}

/// How many statements carrying the long marker run, from another connection.
async fn running_long(observer: &dyn Session) -> u64 {
    first(
        observer,
        "SELECT COUNT(*) FROM information_schema.PROCESSLIST \
         WHERE INFO LIKE '%oxyn\\_long\\_marker%' AND ID <> CONNECTION_ID() \
         AND INFO NOT LIKE '%PROCESSLIST%'",
    )
    .await
    .parse()
    .expect("a count")
}

/// [`running_long`] once the server had two seconds to end what was killed:
/// a killed thread leaves the process list when its statement notices.
async fn settled_long(observer: &dyn Session) -> u64 {
    let mut remaining = running_long(observer).await;
    for _ in 0..20 {
        if remaining == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        remaining = running_long(observer).await;
    }
    remaining
}

/// Is the server thread `id` executing a statement? From another connection.
///
/// The process list says whether the thread is busy, not which text it runs:
/// a procedure shows the statement it is in, not the `CALL`, so the marker of
/// [`running_long`] cannot find one.
async fn thread_is_busy(observer: &dyn Session, id: u64) -> bool {
    first(
        observer,
        &format!(
            "SELECT COUNT(*) FROM information_schema.PROCESSLIST \
             WHERE ID = {id} AND COMMAND <> 'Sleep'"
        ),
    )
    .await
        != "0"
}

/// [`thread_is_busy`], given two seconds to turn false: a killed thread
/// leaves its statement when it notices.
async fn thread_goes_idle(observer: &dyn Session, id: u64) -> bool {
    for _ in 0..20 {
        if !thread_is_busy(observer, id).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn the_connection_identifies_the_server_and_declares_cancellation() {
    let Some(session) = session().await else {
        return;
    };
    let capabilities = session.capabilities();
    assert!(capabilities.contains(Capabilities::SERVER_SIDE_CANCEL));
    assert!(capabilities.contains(Capabilities::TRANSACTIONS));
    assert!(!capabilities.contains(Capabilities::MULTIPLE_STATEMENTS));
    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("identity");
    assert!(["MySQL", "MariaDB"].contains(&info.product.as_str()));
    assert!(!info.version.is_empty());
    // ADR-0050 §6: UTC and utf8mb4 on every connection.
    assert_eq!(
        first(&*session, "SELECT @@session.time_zone").await,
        "+00:00"
    );
    assert_eq!(
        first(&*session, "SELECT @@session.character_set_results").await,
        "utf8mb4"
    );
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn the_schema_is_known_before_the_first_row() {
    let Some(session) = session().await else {
        return;
    };
    let mut cursor = session
        .execute(
            read_request("SELECT 1 AS one, 'two' AS two"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    assert_eq!(cursor.schema().fields().len(), 2);
    assert_eq!(cursor.schema().field(0).name(), "one");
    while cursor.next_batch().await.expect("stream").is_some() {}
    drop(cursor);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_volume_that_would_not_fit_in_memory_arrives_in_bounded_batches() {
    // The checklist's streaming test. A million rows of ~110 bytes is about a
    // hundred megabytes on the wire; the bound held is the one that protects
    // any volume: no batch grows past the byte budget and a row's worth.
    let Some(session) = session().await else {
        return;
    };
    let sql = format!(
        "{DIGITS} SELECT a.i + 10 * b.i + 100 * c.i + 1000 * e.i + 10000 * f.i \
         + 100000 * g.i AS n, REPEAT('x', 100) AS pad FROM d a, d b, d c, d e, d f, d g"
    );
    let mut cursor = session
        .execute(read_request(&sql), &CancelToken::new())
        .await
        .expect("execution");
    let mut rows = 0;
    let mut batches = 0;
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        rows += batch.num_rows();
        batches += 1;
        assert!(
            batch.get_array_memory_size() <= 4 * BATCH_BYTE_BUDGET,
            "a batch of {} bytes: the stream is not bounded",
            batch.get_array_memory_size()
        );
    }
    assert_eq!(rows, 1_000_000);
    assert!(batches > 10, "the result must arrive in batches: {batches}");
    assert_eq!(cursor.stats().rows, 1_000_000);
    assert!(!cursor.stats().truncated);
    drop(cursor);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn cancellation_stops_the_statement_in_the_server_process_list() {
    // The checklist's cancellation test, checked where it counts: the server's
    // process list, from another connection.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let before = first(&*session, "SELECT CONNECTION_ID()").await;
    // Through the handle, once the rows flow.
    let mut cursor = session
        .execute(read_request(&long_rows()), &CancelToken::new())
        .await
        .expect("execution");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(running_long(&*observer).await, 1, "the statement runs");

    session.cancel(cursor.handle()).await.expect("cancellation");
    let issue = end_of(&mut cursor).await;
    assert!(
        matches!(issue, Err(ref error) if error.is_cancelled()),
        "the cursor reports the cancellation: {issue:?}"
    );
    drop(cursor);
    assert_eq!(
        settled_long(&*observer).await,
        0,
        "the server still runs the cancelled statement"
    );
    // Through the token, before the result gave its columns.
    let (running, issue) =
        cancel_through_the_token(&*session, &*observer, read_request(&long_count())).await;
    assert_eq!(running, 1, "the aggregate runs");
    assert!(
        matches!(issue, Err(ref error) if error.is_cancelled()),
        "the execution reports the cancellation: {issue:?}"
    );
    assert_eq!(
        settled_long(&*observer).await,
        0,
        "the server still runs the cancelled aggregate"
    );
    // Both kills were proven spent (1317): the connection was kept.
    assert_eq!(first(&*session, "SELECT CONNECTION_ID()").await, before);
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn dropping_cursors_stops_their_statements() {
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    for _ in 0..5 {
        let cursor = session
            .execute(read_request(&long_rows()), &CancelToken::new())
            .await
            .expect("execution");
        drop(cursor);
    }
    assert_eq!(first(&*session, "SELECT 1").await, "1");
    assert_eq!(settled_long(&*observer).await, 0);
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_cancelled_statement_keeps_the_open_transaction() {
    // ADR-0050 §7 as amended: the kill proven spent, the transaction lives on.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_tx_cancel").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_tx_cancel (id INT PRIMARY KEY)",
    )
    .await;
    apply(&*session, "START TRANSACTION").await;
    apply(&*session, "INSERT INTO oxyn_tx_cancel VALUES (1)").await;
    // Not bounded to read-only: that bound is refused inside a transaction.
    let mut cursor = session
        .execute(write_request(&long_rows()), &CancelToken::new())
        .await
        .expect("execution");
    tokio::time::sleep(Duration::from_millis(300)).await;
    session.cancel(cursor.handle()).await.expect("cancellation");
    assert!(end_of(&mut cursor).await.is_err());
    drop(cursor);
    assert_eq!(
        session.transaction_state(&CancelToken::new()).await,
        TransactionState::Open,
        "the transaction survives the kill"
    );
    apply(&*session, "COMMIT").await;
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_tx_cancel").await,
        "1",
        "the row written before the cancellation is committed"
    );
    apply(&*session, "DROP TABLE oxyn_tx_cancel").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_myisam_update_interrupted_by_stop_is_not_cancelled() {
    // A 1317 proves the kill was spent, not that the statement kept nothing:
    // a MyISAM table has no rollback, so what the update changed before the
    // kill stays. `Cancelled` would invite running it again (I-13).
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_myisam_stop").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_myisam_stop (id INT PRIMARY KEY, n INT NOT NULL) ENGINE=MyISAM",
    )
    .await;
    apply(
        &*session,
        "INSERT INTO oxyn_myisam_stop VALUES (1, 0), (2, 0)",
    )
    .await;
    let (running, issue) = cancel_through_the_token(
        &*session,
        &*observer,
        write_request(&long_update("oxyn_myisam_stop")),
    )
    .await;
    assert_eq!(running, 1, "the update runs");
    let Err(error) = issue else {
        panic!("a stopped update is an error");
    };
    assert!(
        !error.is_cancelled(),
        "a MyISAM update stopped mid-way kept its first rows: {error:?}"
    );
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    assert_eq!(error.class(), ErrorClass::Ambiguous);
    assert!(!error.is_retryable(), "never replayed (I-13)");
    // The server stopped it all the same: the verdict is about the data.
    assert_eq!(
        settled_long(&*observer).await,
        0,
        "the server still runs the stopped update"
    );
    // The session goes on, with its connection kept or reopened.
    assert_eq!(
        first(&*session, "SELECT COUNT(*) FROM oxyn_myisam_stop").await,
        "2"
    );
    apply(&*session, "DROP TABLE oxyn_myisam_stop").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_call_killed_after_its_procedure_committed_an_insert_is_outcome_unknown() {
    // Whether the server answers 1317 on the CALL or lets it end after the
    // interrupted SLEEP, the procedure keeps the INSERT it committed: a CALL
    // is never `Cancelled`, in a transaction or not.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    apply(
        &*session,
        "DROP PROCEDURE IF EXISTS oxyn_commits_then_waits",
    )
    .await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_call_log").await;
    apply(&*session, "CREATE TABLE oxyn_call_log (id INT PRIMARY KEY)").await;
    apply(
        &*session,
        "CREATE PROCEDURE oxyn_commits_then_waits() \
         BEGIN INSERT INTO oxyn_call_log VALUES (1); DO SLEEP(30); END",
    )
    .await;
    let id: u64 = first(&*session, "SELECT CONNECTION_ID()")
        .await
        .parse()
        .expect("a connection id");

    let token = CancelToken::new();
    let executing = async {
        let mut cursor = session
            .execute(write_request("CALL oxyn_commits_then_waits()"), &token)
            .await?;
        end_of(&mut cursor).await
    };
    let stopping = async {
        // The INSERT is committed once the observer sees its row: the
        // procedure is then in its SLEEP, with something to keep.
        let mut committed = false;
        for _ in 0..100 {
            if first(&*observer, "SELECT COUNT(*) FROM oxyn_call_log").await == "1" {
                committed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(committed, "the procedure committed its INSERT");
        assert!(
            thread_is_busy(&*observer, id).await,
            "the CALL is still running when the stop arrives"
        );
        token.cancel();
    };
    // Shorter than the procedure's SLEEP: a stop that does not cut fails here.
    let (ended, ()) = tokio::time::timeout(
        Duration::from_secs(25),
        futures::future::join(executing, stopping),
    )
    .await
    .expect("the stop cuts the CALL instead of waiting for its SLEEP");

    let Err(error) = ended else {
        panic!("a CALL stopped before its end is an error");
    };
    assert!(
        matches!(error, OxynError::OutcomeUnknown(_)),
        "a procedure keeps what it already committed: {error:?}"
    );
    assert_eq!(error.class(), ErrorClass::Ambiguous);
    assert!(!error.is_retryable(), "never replayed (I-13)");
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_call_log").await,
        "1",
        "the row the procedure committed before the stop is there"
    );
    assert!(
        thread_goes_idle(&*observer, id).await,
        "the server still runs the stopped CALL"
    );
    apply(&*session, "DROP PROCEDURE oxyn_commits_then_waits").await;
    apply(&*session, "DROP TABLE oxyn_call_log").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

/// Opens a transaction on a fresh two-row table of `engine`, writes a third
/// row, then stops a long `UPDATE` of the table through the token. Returns how
/// many long statements ran and how the execution ended; the transaction is
/// still open when it returns.
async fn stop_an_update_inside_a_transaction(
    session: &dyn Session,
    observer: &dyn Session,
    table: &str,
    engine: &str,
) -> (u64, Result<(), OxynError>) {
    apply(session, &format!("DROP TABLE IF EXISTS {table}")).await;
    apply(
        session,
        &format!("CREATE TABLE {table} (id INT PRIMARY KEY, n INT NOT NULL) ENGINE={engine}"),
    )
    .await;
    apply(
        session,
        &format!("INSERT INTO {table} VALUES (1, 0), (2, 0)"),
    )
    .await;
    apply(session, "START TRANSACTION").await;
    apply(session, &format!("INSERT INTO {table} VALUES (3, 0)")).await;
    // Not bounded to read-only: that bound is refused inside a transaction.
    cancel_through_the_token(session, observer, write_request(&long_update(table))).await
}

/// What a stopped write must say whatever the engine: not `Cancelled`.
fn assert_unknown_outcome(issue: &Result<(), OxynError>) {
    let Err(error) = issue else {
        panic!("a stopped update is an error: {issue:?}");
    };
    assert!(
        matches!(error, OxynError::OutcomeUnknown(_)),
        "a transaction proves nothing of the tables' engine: {error:?}"
    );
    assert_eq!(error.class(), ErrorClass::Ambiguous);
    assert!(!error.is_retryable(), "never replayed (I-13)");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn an_innodb_update_in_a_transaction_stopped_by_the_kill_is_an_unknown_outcome() {
    // The server undoes an interrupted InnoDB statement alone and keeps the
    // transaction. The driver cannot see the engine without composing schema
    // lookups from the statement's identifiers (I-10), and an open transaction
    // proves nothing about it: it reports `OutcomeUnknown`, which criterion 3
    // of #222 allows ("can stay Cancelled"). What the server did is checked
    // here from another connection, not read off the verdict.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let (running, issue) =
        stop_an_update_inside_a_transaction(&*session, &*observer, "oxyn_tx_stop", "InnoDB").await;
    assert_eq!(running, 1, "the update runs");
    assert_unknown_outcome(&issue);
    assert_eq!(
        settled_long(&*observer).await,
        0,
        "the server still runs the stopped update"
    );
    assert_eq!(
        session.transaction_state(&CancelToken::new()).await,
        TransactionState::Open,
        "the transaction survives the kill"
    );
    // Committed, not rolled back: the data shows what InnoDB did.
    apply(&*session, "COMMIT").await;
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_tx_stop").await,
        "3",
        "the row written before the stopped statement is committed"
    );
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_tx_stop WHERE n <> 0").await,
        "0",
        "the interrupted update changed nothing"
    );
    apply(&*session, "DROP TABLE oxyn_tx_stop").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_myisam_update_in_a_transaction_stopped_by_the_kill_is_not_cancelled() {
    // The case an open transaction used to turn into `Cancelled`: the kill is
    // confirmed and the transaction survives, but a MyISAM table has no
    // rollback, so the rows the update changed before the kill stay. Replaying
    // would apply them twice (I-13).
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let (running, issue) =
        stop_an_update_inside_a_transaction(&*session, &*observer, "oxyn_tx_myisam", "MyISAM")
            .await;
    assert_eq!(running, 1, "the update runs");
    assert!(
        !matches!(issue, Err(ref error) if error.is_cancelled()),
        "a MyISAM table written inside a transaction keeps its rows: {issue:?}"
    );
    assert_unknown_outcome(&issue);
    assert_eq!(
        settled_long(&*observer).await,
        0,
        "the server still runs the stopped update"
    );
    assert_eq!(
        session.transaction_state(&CancelToken::new()).await,
        TransactionState::Open,
        "the transaction survives the kill"
    );
    apply(&*session, "ROLLBACK").await;
    apply(&*session, "DROP TABLE oxyn_tx_myisam").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_row_bound_inside_a_transaction_truncates_and_keeps_it() {
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_tx_bound").await;
    apply(&*session, "CREATE TABLE oxyn_tx_bound (id INT PRIMARY KEY)").await;
    apply(&*session, "START TRANSACTION").await;
    apply(&*session, "INSERT INTO oxyn_tx_bound VALUES (1)").await;
    let sql =
        format!("{DIGITS} SELECT a.i, REPEAT('x', 200) FROM d a, d b, d c, d e, d f, d g, d h");
    let request = ExecRequest::new(LANGUAGE, sql)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().writable().with_max_rows(100));
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("execution");
    let mut rows = 0;
    while let Some(batch) = cursor
        .next_batch()
        .await
        .expect("a bounded read is no error")
    {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 100);
    assert!(cursor.stats().truncated, "the bound says so");
    drop(cursor);
    assert_eq!(
        session.transaction_state(&CancelToken::new()).await,
        TransactionState::Open
    );
    apply(&*session, "COMMIT").await;
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_tx_bound").await,
        "1"
    );
    // A result just past the bound needs no kill at all.
    let small = ExecRequest::new(LANGUAGE, format!("{DIGITS} SELECT i FROM d"))
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(5));
    let mut cursor = session
        .execute(small, &CancelToken::new())
        .await
        .expect("execution");
    while cursor.next_batch().await.expect("stream").is_some() {}
    assert!(cursor.stats().truncated);
    drop(cursor);
    apply(&*session, "DROP TABLE oxyn_tx_bound").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_write_returning_past_its_bound_is_drained_not_killed() {
    let Some(session) = session().await else {
        return;
    };
    if !is_mariadb(&*session).await {
        eprintln!("only MariaDB returns rows from a write: test skipped");
        return;
    }
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_write_bound").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_write_bound (id INT PRIMARY KEY)",
    )
    .await;
    // Far more rows than the read-ahead a read gets past its bound: a kill
    // would roll this autocommit statement back while 100 of its rows are
    // shown.
    let request = ExecRequest::new(
        LANGUAGE,
        "INSERT INTO oxyn_write_bound (id) SELECT seq FROM seq_1_to_100000 RETURNING id",
    )
    .with_intent(StatementIntent::Write)
    .with_limits(
        ExecLimits::default()
            .writable()
            .with_max_rows(100)
            .with_timeout(None),
    );
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("execution");
    let mut rows = 0;
    while let Some(batch) = cursor
        .next_batch()
        .await
        .expect("a drained write is no error")
    {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 100);
    assert!(cursor.stats().truncated, "the bound says so");
    drop(cursor);
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_write_bound").await,
        "100000",
        "the write ran whole"
    );
    apply(&*session, "DROP TABLE oxyn_write_bound").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

/// Runs `sql` in an open transaction, kills the connection from `observer`
/// while the statement waits on the server — before it gave any schema — and
/// returns every error the execution reported.
///
/// The execution must conclude within a bound: it used to wait on itself,
/// the stream task blocked on a second terminal event while `execute` waited
/// for the schema the task still held.
async fn errors_after_a_kill_mid_transaction(
    session: &dyn Session,
    observer: &dyn Session,
    sql: &str,
) -> Vec<OxynError> {
    apply(session, "DROP TABLE IF EXISTS oxyn_tx_lost").await;
    apply(session, "CREATE TABLE oxyn_tx_lost (id INT PRIMARY KEY)").await;
    // Read before the transaction: a read-only bound is refused inside one.
    let id = first(session, "SELECT CONNECTION_ID()").await;
    apply(session, "START TRANSACTION").await;
    apply(session, "INSERT INTO oxyn_tx_lost VALUES (1)").await;
    let killing = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        apply(observer, &format!("KILL {id}")).await;
    };
    let executing = async {
        let mut errors = Vec::new();
        match session
            .execute(write_request(sql), &CancelToken::new())
            .await
        {
            Err(error) => errors.push(error),
            Ok(mut cursor) => loop {
                match cursor.next_batch().await {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(error) => errors.push(error),
                }
            },
        }
        errors
    };
    let (errors, ()) = tokio::time::timeout(
        Duration::from_secs(20),
        futures::future::join(executing, killing),
    )
    .await
    .expect("the execution concludes instead of waiting on itself");
    assert_eq!(
        session.transaction_state(&CancelToken::new()).await,
        TransactionState::Unknown,
        "the transaction went with the connection"
    );
    apply(session, "DROP TABLE oxyn_tx_lost").await;
    errors
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_call_failing_before_its_schema_in_a_transaction_ends_on_one_error() {
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*session, "DROP PROCEDURE IF EXISTS oxyn_sleepy").await;
    apply(
        &*session,
        "CREATE PROCEDURE oxyn_sleepy() BEGIN DO SLEEP(30); SELECT 1 AS a; END",
    )
    .await;
    let errors =
        errors_after_a_kill_mid_transaction(&*session, &*observer, "CALL oxyn_sleepy()").await;
    let [error] = errors.as_slice() else {
        panic!("exactly one error expected: {errors:?}");
    };
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    apply(&*session, "DROP PROCEDURE oxyn_sleepy").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_text_statement_failing_in_a_transaction_ends_on_one_error() {
    // MySQL cannot prepare `LOCK TABLES` (1295): it runs through the text
    // protocol, whose schema comes with the result. It commits the open
    // transaction, then waits on the observer's lock until the kill — the
    // case the transaction's loss is reported as an unknown outcome for.
    // Should a server prepare it, the same path runs in the binary protocol.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*observer, "DROP TABLE IF EXISTS oxyn_lock_held").await;
    apply(&*observer, "CREATE TABLE oxyn_lock_held (id INT)").await;
    apply(&*observer, "LOCK TABLES oxyn_lock_held WRITE").await;
    let errors = errors_after_a_kill_mid_transaction(
        &*session,
        &*observer,
        "LOCK TABLES oxyn_lock_held READ",
    )
    .await;
    apply(&*observer, "UNLOCK TABLES").await;
    apply(&*observer, "DROP TABLE oxyn_lock_held").await;
    let [error] = errors.as_slice() else {
        panic!("exactly one error expected: {errors:?}");
    };
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_multi_statement_text_fails_the_prepare_with_1064_before_1295() {
    // ADR-0050 §3: the observation the fallback rests on, kept per server.
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_order").await;
    apply(&*session, "CREATE TABLE oxyn_order (id INT)").await;
    for sql in [
        "CREATE TRIGGER oxyn_x BEFORE INSERT ON oxyn_order FOR EACH ROW SET @a = 1; DROP TABLE oxyn_order",
        "SELECT 1; DROP TABLE oxyn_order",
        "START TRANSACTION; DROP TABLE oxyn_order",
        "SELECT 1 /*! ; DROP TABLE oxyn_order */",
    ] {
        let error = refusal(run(&*session, write_request(sql)).await, sql);
        assert_eq!(server_code(&error), Some(1064), "`{sql}`: {error}");
        assert_eq!(error.class(), ErrorClass::Permanent);
    }
    assert_eq!(
        first(&*session, "SELECT COUNT(*) FROM oxyn_order").await,
        "0"
    );
    apply(&*session, "DROP TABLE oxyn_order").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn what_mysql_cannot_prepare_runs_through_the_text_protocol() {
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP PROCEDURE IF EXISTS oxyn_two").await;
    apply(
        &*session,
        "CREATE PROCEDURE oxyn_two() BEGIN SELECT 1 AS a; SELECT 2 AS b, 3 AS c; END",
    )
    .await;
    // A CALL: the first result set is the result, the second is counted.
    let mut cursor = session
        .execute(read_request("CALL oxyn_two()"), &CancelToken::new())
        .await
        .expect("CALL");
    assert_eq!(cursor.schema().fields().len(), 1);
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 1);
    assert!(
        cursor.stats().truncated,
        "the second result set is not dropped in silence"
    );
    drop(cursor);
    // `START TRANSACTION` is what MySQL refuses to prepare (1295) and the text
    // protocol runs; MariaDB prepares it. Either way the server opened the
    // transaction, and says so in its status flags.
    apply(&*session, "START TRANSACTION").await;
    assert_eq!(
        session.transaction_state(&CancelToken::new()).await,
        TransactionState::Open
    );
    apply(&*session, "ROLLBACK").await;
    // `USE` no longer takes this path, on MySQL or on MariaDB (which prepares
    // it): a typed `USE` is refused before the prepare. The refusal and what
    // it leaves untouched are the subject of
    // `a_typed_use_is_refused_and_points_to_the_context_selector`.
    let error = refusal(
        run(&*session, write_request("USE t")).await,
        "a typed USE is refused",
    );
    assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    // A bound parameter on an unpreparable statement.
    let bound =
        write_request("DROP PROCEDURE IF EXISTS oxyn_two").with_params(vec![ScalarValue::Int64(1)]);
    if !is_mariadb(&*session).await {
        let error = refusal(run(&*session, bound).await, "bound unpreparable");
        assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    }
    apply(&*session, "DROP PROCEDURE oxyn_two").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn read_only_is_enforced_by_the_server_in_both_protocols() {
    // ADR-0050 §9: the observation behind READ_ONLY_SESSION.
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_ro").await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_ro2").await;
    apply(&*session, "CREATE TABLE oxyn_ro (id INT)").await;
    let bounded = |sql: &str| {
        ExecRequest::new(LANGUAGE, sql)
            .with_intent(StatementIntent::Read)
            .with_limits(ExecLimits::default())
    };
    for sql in [
        "INSERT INTO oxyn_ro VALUES (1)",
        "UPDATE oxyn_ro SET id = 2",
        "DELETE FROM oxyn_ro",
        "CREATE TABLE oxyn_ro2 (id INT)",
        "DROP TABLE oxyn_ro",
        "TRUNCATE TABLE oxyn_ro",
        "ALTER TABLE oxyn_ro ADD COLUMN x INT",
        "RENAME TABLE oxyn_ro TO oxyn_ro2",
        "CREATE INDEX oxyn_ro_i ON oxyn_ro (id)",
        "CREATE TEMPORARY TABLE oxyn_ro2 (id INT)",
    ] {
        let error = refusal(run(&*session, bounded(sql)).await, sql);
        assert!(
            matches!(error, OxynError::Query(ref text) if text.contains("read-only")),
            "`{sql}`: {error:?}"
        );
    }
    assert_eq!(
        first(&*session, "SELECT COUNT(*) FROM oxyn_ro").await,
        "0",
        "a read passes"
    );
    // The session is back in read-write mode after a bounded execution.
    apply(&*session, "INSERT INTO oxyn_ro VALUES (1)").await;
    // Inside an open transaction, the bound cannot be honored: refused.
    apply(&*session, "START TRANSACTION").await;
    let error = refusal(
        run(&*session, bounded("SELECT 1")).await,
        "read-only in a transaction",
    );
    assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    apply(&*session, "ROLLBACK").await;
    apply(&*session, "DROP TABLE oxyn_ro").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_local_infile_request_is_refused_without_reading_a_file() {
    // ADR-0050 §1: no handler, nothing loaded, the connection closed.
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "SET GLOBAL local_infile = 1").await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_li").await;
    apply(&*session, "CREATE TABLE oxyn_li (line TEXT)").await;
    let error = refusal(
        run(
            &*session,
            write_request("LOAD DATA LOCAL INFILE '/etc/hosts' INTO TABLE oxyn_li"),
        )
        .await,
        "LOAD DATA LOCAL must be refused",
    );
    assert!(
        matches!(error, OxynError::Connection(ref text) if text.contains("LOAD DATA LOCAL")),
        "{error:?}"
    );
    // The session opens a new connection and the table is empty.
    assert_eq!(first(&*session, "SELECT COUNT(*) FROM oxyn_li").await, "0");
    apply(&*session, "DROP TABLE oxyn_li").await;
    apply(&*session, "SET GLOBAL local_infile = 0").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn the_type_table_crosses_the_wire() {
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_types").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_types (t1 TINYINT(1), u BIGINT UNSIGNED, y YEAR, b BIT(12), \
         tm TIME(6), dt DATETIME(6), ts TIMESTAMP(6) NULL, j JSON, p POINT, e ENUM('a','b'), \
         bl BLOB, vc VARCHAR(10), dec65 DECIMAL(65,30), f FLOAT, dbl DOUBLE)",
    )
    .await;
    apply(
        &*session,
        "INSERT INTO oxyn_types VALUES (100, 18446744073709551615, 2024, b'101', \
         '-838:59:59', '2026-09-30 12:34:56.5', '2026-09-30 12:34:56', '{\"a\": 1}', \
         POINT(1, 2), 'b', X'00FF', 'é', \
         '-12345678901234567890123456789012345.123456789012345678901234567890', 1.5, 2.5)",
    )
    .await;
    let batches = run(&*session, read_request("SELECT * FROM oxyn_types"))
        .await
        .expect("SELECT *");
    let batch = batches.first().expect("one batch");
    let schema = batch.schema();
    let expected = [
        ("t1", DataType::Int8, "100"),
        ("u", DataType::UInt64, "18446744073709551615"),
        ("y", DataType::UInt16, "2024"),
        ("b", DataType::UInt64, "5"),
        (
            "dec65",
            DataType::Decimal256(65, 30),
            "-12345678901234567890123456789012345.123456789012345678901234567890",
        ),
        ("vc", DataType::Utf8, "é"),
        ("e", DataType::Utf8, "b"),
        ("f", DataType::Float32, "1.5"),
        ("dbl", DataType::Float64, "2.5"),
    ];
    for (name, data_type, value) in expected {
        let index = schema.index_of(name).expect(name);
        assert_eq!(schema.field(index).data_type(), &data_type, "{name}");
        assert_eq!(
            array_value_to_string(batch.column(index), 0).expect("printable"),
            value,
            "{name}"
        );
    }
    for (name, data_type) in [
        (
            "tm",
            DataType::Duration(arrow::datatypes::TimeUnit::Microsecond),
        ),
        (
            "dt",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        (
            "ts",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, Some("UTC".into())),
        ),
        ("j", DataType::Utf8),
        ("p", DataType::Binary),
        ("bl", DataType::Binary),
    ] {
        let index = schema.index_of(name).expect(name);
        assert_eq!(schema.field(index).data_type(), &data_type, "{name}");
    }
    let time = batch
        .column(schema.index_of("tm").expect("tm"))
        .as_any()
        .downcast_ref::<arrow::array::DurationMicrosecondArray>()
        .expect("Duration");
    assert_eq!(time.value(0), -(838 * 3_600 + 59 * 60 + 59) * 1_000_000);
    let point = schema.field(schema.index_of("p").expect("p"));
    assert_eq!(
        point.metadata().get(META_MYSQL_TYPE).map(String::as_str),
        Some("GEOMETRY")
    );
    // The literals, through the same table.
    assert_eq!(
        first(&*session, "SELECT CAST(18446744073709551615 AS UNSIGNED)").await,
        "18446744073709551615"
    );
    apply(&*session, "DROP TABLE oxyn_types").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_vector_is_bytes_and_marked_when_the_server_says_so() {
    let Some(session) = session().await else {
        return;
    };
    let version = server_version(&*session).await;
    let (sql, marked) = if is_mariadb(&*session).await {
        ("SELECT VEC_FromText('[1,2,3]') AS v", false)
    } else if version.starts_with("9.") {
        ("SELECT STRING_TO_VECTOR('[1,2,3]') AS v", true)
    } else {
        eprintln!("MySQL {version} has no VECTOR type: test skipped");
        session.close().await.expect("close");
        return;
    };
    let batches = run(&*session, read_request(sql)).await.expect("VECTOR");
    let batch = batches.first().expect("one batch");
    let field = batch.schema().field(0).clone();
    assert_eq!(field.data_type(), &DataType::Binary);
    let bytes = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::BinaryArray>()
        .expect("Binary")
        .value(0)
        .to_vec();
    assert_eq!(bytes.len(), 12, "three packed f32");
    assert_eq!(bytes.get(4..8), Some(&2.0_f32.to_le_bytes()[..]));
    if marked {
        assert_eq!(
            field.metadata().get(META_MYSQL_TYPE).map(String::as_str),
            Some("VECTOR")
        );
    }
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_zero_date_is_refused_naming_the_column_and_the_cast() {
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "SET SESSION sql_mode = ''").await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_zero").await;
    apply(&*session, "CREATE TABLE oxyn_zero (d DATE, dt DATETIME)").await;
    apply(
        &*session,
        "INSERT INTO oxyn_zero VALUES ('0000-00-00', '2024-00-15 10:00:00')",
    )
    .await;
    for column in ["d", "dt"] {
        let error = refusal(
            run(
                &*session,
                read_request(&format!("SELECT {column} FROM oxyn_zero")),
            )
            .await,
            "zero date",
        );
        let text = error.to_string();
        assert!(text.contains(&format!("`{column}`")), "{text}");
        assert!(text.contains("CAST("), "{text}");
        assert_eq!(error.class(), ErrorClass::Permanent);
    }
    assert_eq!(
        first(&*session, "SELECT CAST(d AS CHAR) FROM oxyn_zero").await,
        "0000-00-00"
    );
    apply(&*session, "DROP TABLE oxyn_zero").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn the_transaction_state_is_the_servers() {
    // ADR-0039 through ADR-0050 §8: read from SERVER_STATUS_IN_TRANS.
    let Some(session) = session().await else {
        return;
    };
    let state = || async { session.transaction_state(&CancelToken::new()).await };
    assert_eq!(state().await, TransactionState::Idle, "at opening");
    apply(&*session, "START TRANSACTION").await;
    assert_eq!(
        state().await,
        TransactionState::Open,
        "after START TRANSACTION"
    );
    apply(&*session, "COMMIT").await;
    assert_eq!(state().await, TransactionState::Idle, "after COMMIT");
    apply(&*session, "BEGIN").await;
    apply(&*session, "ROLLBACK").await;
    assert_eq!(state().await, TransactionState::Idle, "after ROLLBACK");
    session.begin(&CancelToken::new()).await.expect("begin");
    assert_eq!(state().await, TransactionState::Open, "after begin");
    session.commit(&CancelToken::new()).await.expect("commit");
    assert_eq!(state().await, TransactionState::Idle, "after commit");
    session.begin(&CancelToken::new()).await.expect("begin");
    session
        .rollback(&CancelToken::new())
        .await
        .expect("rollback");
    assert_eq!(state().await, TransactionState::Idle, "after rollback");

    // An interrupted autocommit write: the server rolled the statement back.
    // A write answers only once done: its token stops it.
    let Some(observer) = self::session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_tx_write").await;
    apply(&*session, "CREATE TABLE oxyn_tx_write (n BIGINT)").await;
    let long_write = format!(
        "INSERT INTO oxyn_tx_write {DIGITS} SELECT a.i AS oxyn_long_marker \
         FROM d a, d b, d c, d e, d f, d g, d h, d j"
    );
    let (running, issue) =
        cancel_through_the_token(&*session, &*observer, write_request(&long_write)).await;
    assert_eq!(running, 1, "the write runs");
    assert!(issue.is_err(), "the write was interrupted");
    observer.close().await.expect("close");
    assert_eq!(
        state().await,
        TransactionState::Idle,
        "after an interrupted write"
    );
    assert_eq!(
        first(&*session, "SELECT COUNT(*) FROM oxyn_tx_write").await,
        "0"
    );
    apply(&*session, "DROP TABLE oxyn_tx_write").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_bound_value_never_comes_out_of_the_server_message() {
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "SET SESSION sql_mode = 'STRICT_ALL_TABLES'").await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_bound").await;
    apply(&*session, "CREATE TABLE oxyn_bound (n INT)").await;
    let secret = "secret-4111111111111111";
    let request = write_request("INSERT INTO oxyn_bound VALUES (?)")
        .with_params(vec![ScalarValue::Text(secret.to_owned())]);
    let error = refusal(
        run(&*session, request).await,
        "an integer column refuses text",
    );
    assert!(!error.to_string().contains(secret), "leak: {error}");
    assert_eq!(server_code(&error), Some(1366), "{error}");
    apply(&*session, "DROP TABLE oxyn_bound").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn introspection_walks_databases_relations_and_keys() {
    let Some(session) = session().await else {
        return;
    };
    let hostile = "x`; DROP TABLE oxyn_guard; --";
    let quoted = "`x``; DROP TABLE oxyn_guard; --`";
    apply(&*session, "DROP TABLE IF EXISTS oxyn_child").await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_parent").await;
    apply(&*session, &format!("DROP TABLE IF EXISTS {quoted}")).await;
    apply(&*session, "DROP TABLE IF EXISTS oxyn_guard").await;
    apply(&*session, "CREATE TABLE oxyn_guard (id INT)").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_parent (a INT, b VARCHAR(10), PRIMARY KEY (b, a)) COMMENT 'parents'",
    )
    .await;
    apply(
        &*session,
        "CREATE TABLE oxyn_child (id INT PRIMARY KEY, x VARCHAR(10), y INT, \
         KEY oxyn_child_xy (x, y), \
         CONSTRAINT oxyn_fk FOREIGN KEY (x, y) REFERENCES oxyn_parent (b, a) ON DELETE CASCADE)",
    )
    .await;
    apply(
        &*session,
        &format!("CREATE TABLE {quoted} (id INT PRIMARY KEY)"),
    )
    .await;
    let cancel = CancelToken::new();
    let catalog = session.catalog();

    let databases = catalog
        .list_namespaces(None, &cancel)
        .await
        .expect("databases");
    assert!(databases.iter().any(|db| db.name() == "t"));
    assert!(
        databases
            .iter()
            .any(|db| db.name() == "mysql" && db.is_system)
    );

    let database = CatalogPath::for_namespace(None, "t").expect("path");
    let relations = catalog
        .list_relations(&database, &cancel)
        .await
        .expect("relations");
    let parent = relations
        .iter()
        .find(|relation| relation.name() == "oxyn_parent")
        .expect("parent listed");
    assert_eq!(parent.comment.as_deref(), Some("parents"));

    let child_path = CatalogPath::for_relation(None, Some("t"), "oxyn_child").expect("path");
    let described = catalog
        .describe_relation(&child_path, &cancel)
        .await
        .expect("description");
    assert_eq!(described.fields.len(), 3);
    assert_eq!(described.primary_key().len(), 1);

    let indexes = catalog
        .list_indexes(&child_path, &cancel)
        .await
        .expect("indexes");
    let composite = indexes
        .iter()
        .find(|index| index.name == "oxyn_child_xy")
        .expect("composite index");
    assert_eq!(composite.fields, ["x", "y"]);

    let keys = catalog
        .list_foreign_keys(&child_path, &cancel)
        .await
        .expect("keys");
    let key = keys.first().expect("one key");
    assert_eq!(key.fields, ["x", "y"]);
    assert_eq!(key.references.fields, ["b", "a"]);
    assert_eq!(
        key.on_delete,
        oxyn_catalog::model::ReferentialAction::Cascade
    );

    let parent_path = CatalogPath::for_relation(None, Some("t"), "oxyn_parent").expect("path");
    let incoming = catalog
        .list_incoming_foreign_keys(&parent_path, &cancel)
        .await
        .expect("incoming");
    assert_eq!(incoming.len(), 1);
    assert_eq!(
        incoming.first().and_then(|k| k.source.relation()),
        Some("oxyn_child")
    );

    let definition = catalog
        .relation_definition(&parent_path, &cancel)
        .await
        .expect("definition");
    assert!(
        definition.sql.starts_with("CREATE TABLE `oxyn_parent`"),
        "{}",
        definition.sql
    );

    // A hostile name is quoted everywhere Oxyn composes it.
    let hostile_path = CatalogPath::for_relation(None, Some("t"), hostile).expect("legal name");
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("id")],
        offset: 1,
        ..PreviewShape::default()
    };
    let preview = session
        .preview_request(&hostile_path, 10, &shape, &cancel)
        .await
        .expect("preview composed");
    run(&*session, preview).await.expect("preview runs");
    catalog
        .relation_definition(&hostile_path, &cancel)
        .await
        .expect("definition of a hostile name");
    assert_eq!(
        first(&*session, "SELECT COUNT(*) FROM oxyn_guard").await,
        "0"
    );

    for sql in [
        "DROP TABLE oxyn_child",
        "DROP TABLE oxyn_parent",
        "DROP TABLE oxyn_guard",
    ] {
        apply(&*session, sql).await;
    }
    apply(&*session, &format!("DROP TABLE {quoted}")).await;
    session.close().await.expect("close");
}

/// The statements prepared on the whole server, read from `observer` — whose
/// own read is one of them, every time.
async fn prepared_statements(observer: &dyn Session) -> u64 {
    let table = if is_mariadb(observer).await {
        "information_schema.GLOBAL_STATUS"
    } else {
        "performance_schema.global_status"
    };
    first(
        observer,
        &format!("SELECT VARIABLE_VALUE FROM {table} WHERE VARIABLE_NAME = 'Prepared_stmt_count'"),
    )
    .await
    .parse()
    .expect("a count")
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_failing_introspection_leaves_no_statement_open() {
    // The execution fails after the prepare, on a connection that is kept:
    // the statement must be closed all the same, or each failure holds one
    // against `max_prepared_stmt_count` until the session ends. Counted on
    // the whole server: run on a server of its own.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let connection = first(&*session, "SELECT CONNECTION_ID()").await;
    let before = prepared_statements(&*observer).await;
    // Accepted at prepare, refused at execute by both servers (1104).
    apply(&*session, "SET SESSION max_join_size = 1").await;
    let database = CatalogPath::for_namespace(None, "t").expect("path");
    for _ in 0..5 {
        let refused = session
            .catalog()
            .list_relations(&database, &CancelToken::new())
            .await;
        assert!(refused.is_err(), "the server refuses the execution");
    }
    apply(&*session, "SET SESSION max_join_size = DEFAULT").await;
    assert_eq!(
        first(&*session, "SELECT CONNECTION_ID()").await,
        connection,
        "the connection was kept through the failures"
    );
    assert_eq!(prepared_statements(&*observer).await, before);
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_context_selects_the_database_and_is_quoted() {
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP DATABASE IF EXISTS `oxyn ctx``db`").await;
    apply(&*session, "CREATE DATABASE `oxyn ctx``db`").await;
    let context = SessionContext::new(None, Some("oxyn ctx`db".to_owned()));
    session
        .set_context(&context, &CancelToken::new())
        .await
        .expect("context");
    assert_eq!(first(&*session, "SELECT DATABASE()").await, "oxyn ctx`db");
    assert_eq!(session.context(), Some(context));
    let missing = SessionContext::new(None, Some("oxyn_missing_db".to_owned()));
    assert!(
        session
            .set_context(&missing, &CancelToken::new())
            .await
            .is_err()
    );
    apply(&*session, "DROP DATABASE `oxyn ctx``db`").await;
    session.close().await.expect("close");
}

/// A per-session status counter of the server, from `SHOW SESSION STATUS`.
async fn session_counter(session: &dyn Session, name: &str) -> u64 {
    let batches = run(
        session,
        read_request(&format!("SHOW SESSION STATUS LIKE '{name}'")),
    )
    .await
    .unwrap_or_else(|error| panic!("`{name}`: {error}"));
    let batch = batches.first().expect("one batch");
    array_value_to_string(batch.column(1), 0)
        .expect("printable")
        .parse()
        .expect("a count")
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn the_resolution_path_is_the_database_the_connection_selected() {
    // ADR-0057: MySQL resolves an unqualified name in the selected database
    // only, and the catalog session selects the one the form names. The test
    // URL names `t`.
    let Some(session) = session().await else {
        return;
    };
    let path = session
        .catalog()
        .resolution_path(&CancelToken::new())
        .await
        .expect("the selected database");
    assert_eq!(path, ["t"]);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_typed_use_is_refused_and_points_to_the_context_selector() {
    // Issue #223. A `USE` typed in a statement would select a database the
    // session never declared: `Session::context` would go on naming the
    // previous one, and the next reconnect would select that one again. It is
    // refused for every actor, before the prepare — MariaDB prepares `USE`, so
    // the server would not refuse it —, and the declared context stays the
    // only one.
    let Some(session) = session().await else {
        return;
    };
    for database in ["oxyn_use_declared", "oxyn_use_other"] {
        apply(&*session, &format!("DROP DATABASE IF EXISTS {database}")).await;
        apply(&*session, &format!("CREATE DATABASE {database}")).await;
    }
    let declared = SessionContext::new(None, Some("oxyn_use_declared".to_owned()));
    session
        .set_context(&declared, &CancelToken::new())
        .await
        .expect("context");
    let connection = first(&*session, "SELECT CONNECTION_ID()").await;

    // What a refusal that reached the server would move is measured, not
    // assumed: two reads with nothing between them give the counter's step.
    let first_read = session_counter(&*session, "Com_stmt_prepare").await;
    let second_read = session_counter(&*session, "Com_stmt_prepare").await;
    let step = second_read - first_read;
    assert!(step >= 1, "a read is prepared, the counter must move");

    for text in [
        "USE oxyn_use_other",
        "use `oxyn_use_other`;",
        "/* x */ USE oxyn_use_other",
        "SELECT 1; USE oxyn_use_other",
        // The server runs the body of an executable comment, and MariaDB
        // prepares it: nothing else would stop these two.
        "/*!50000 USE oxyn_use_other */",
        "/*M!100000 USE oxyn_use_other */",
    ] {
        // With the read-only bound or without: the refusal looks at the text
        // only, and the driver never sees who typed it.
        for request in [write_request(text), read_request(text)] {
            let error = refusal(run(&*session, request).await, text);
            let OxynError::NotSupported { capability } = &error else {
                panic!("`{text}` must be refused as unsupported: {error:?}");
            };
            assert!(
                capability.contains("context selector"),
                "`{text}`: the message points to the selector: {capability}"
            );
            assert!(
                !capability.contains("oxyn_use_other"),
                "`{text}`: the message echoes the name: {capability}"
            );
        }
    }

    // Nothing was sent: no `COM_STMT_PREPARE` reached the server for any of
    // the twelve refusals, and the same connection is still the session's.
    assert_eq!(
        session_counter(&*session, "Com_stmt_prepare").await - second_read,
        step,
        "a refused USE must not even be prepared"
    );
    assert_eq!(
        first(&*session, "SELECT DATABASE()").await,
        "oxyn_use_declared"
    );
    assert_eq!(session.context(), Some(declared.clone()));
    assert_eq!(
        first(&*session, "SELECT CONNECTION_ID()").await,
        connection,
        "a refusal costs no reconnection"
    );

    // The way out the message points to does work.
    let other = SessionContext::new(None, Some("oxyn_use_other".to_owned()));
    session
        .set_context(&other, &CancelToken::new())
        .await
        .expect("the selector");
    assert_eq!(
        first(&*session, "SELECT DATABASE()").await,
        "oxyn_use_other"
    );
    assert_eq!(session.context(), Some(other));
    for database in ["oxyn_use_declared", "oxyn_use_other"] {
        apply(&*session, &format!("DROP DATABASE {database}")).await;
    }
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn truncate_is_accepted_as_declared() {
    // ADR-0042: a declared flag is held by a test against the engine.
    let Some(session) = session().await else {
        return;
    };
    assert!(session.capabilities().contains(Capabilities::TRUNCATE));
    apply(&*session, "DROP TABLE IF EXISTS oxyn_trunc").await;
    apply(&*session, "CREATE TABLE oxyn_trunc (id INT)").await;
    apply(&*session, "INSERT INTO oxyn_trunc VALUES (1), (2)").await;
    apply(&*session, "TRUNCATE TABLE oxyn_trunc").await;
    assert_eq!(
        first(&*session, "SELECT COUNT(*) FROM oxyn_trunc").await,
        "0"
    );
    apply(&*session, "DROP TABLE oxyn_trunc").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn affected_rows_are_the_servers_count() {
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_affected").await;
    apply(&*session, "CREATE TABLE oxyn_affected (id INT)").await;
    let mut cursor = session
        .execute(
            write_request("INSERT INTO oxyn_affected VALUES (1), (2), (3)"),
            &CancelToken::new(),
        )
        .await
        .expect("insert");
    while cursor.next_batch().await.expect("stream").is_some() {}
    assert_eq!(cursor.stats().rows, 3);
    drop(cursor);
    apply(&*session, "DROP TABLE oxyn_affected").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn statements_whose_columns_come_with_the_execution_return_their_rows() {
    // MySQL prepares these without columns, or with other types than the
    // rows carry (`SHOW INDEX`, `SHOW TABLE STATUS`); MariaDB some of them.
    // The execution's columns are the schema (RESEARCH-NOTES, 2026-10-01).
    let Some(session) = session().await else {
        return;
    };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_meta").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_meta (id INT PRIMARY KEY, n INT)",
    )
    .await;
    apply(&*session, "INSERT INTO oxyn_meta VALUES (1, 2)").await;
    for sql in [
        "SHOW PROCESSLIST",
        "SHOW FULL PROCESSLIST",
        "SHOW STATUS LIKE 'Uptime'",
        "SHOW VARIABLES LIKE 'version'",
        "SHOW GRANTS",
        "SHOW ENGINES",
        "SHOW CREATE TABLE oxyn_meta",
        "SHOW INDEX FROM oxyn_meta",
        "SHOW TABLE STATUS",
        "EXPLAIN SELECT * FROM oxyn_meta",
        "CHECK TABLE oxyn_meta",
        "ANALYZE TABLE oxyn_meta",
        "REPAIR TABLE oxyn_meta",
        "OPTIMIZE TABLE oxyn_meta",
        "CHECKSUM TABLE oxyn_meta",
    ] {
        let mut cursor = session
            .execute(write_request(sql), &CancelToken::new())
            .await
            .unwrap_or_else(|error| panic!("`{sql}`: {error}"));
        let schema = cursor.schema();
        assert!(
            !schema.fields().is_empty(),
            "`{sql}`: a schema before the rows"
        );
        let mut rows = 0;
        while let Some(batch) = cursor
            .next_batch()
            .await
            .unwrap_or_else(|error| panic!("`{sql}`: {error}"))
        {
            assert_eq!(batch.schema(), schema, "`{sql}`: the announced schema");
            rows += batch.num_rows();
        }
        assert!(rows > 0, "`{sql}`: rows");
    }
    let batches = run(&*session, read_request("SHOW CREATE TABLE oxyn_meta"))
        .await
        .expect("SHOW CREATE TABLE");
    let ddl = batches
        .first()
        .map(|batch| array_value_to_string(batch.column(1), 0).expect("printable"))
        .expect("one batch");
    assert!(ddl.contains("CREATE TABLE"), "{ddl}");
    apply(&*session, "DROP TABLE oxyn_meta").await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_bound_value_types_its_column_as_the_execution_does() {
    // Prepared, `?` is a string (MySQL) or bytes (MariaDB); executed with a
    // number, the column is a `BIGINT`.
    let Some(session) = session().await else {
        return;
    };
    let batches = run(
        &*session,
        read_request("SELECT ? AS x").with_params(vec![ScalarValue::Int64(5)]),
    )
    .await
    .expect("execution");
    let batch = batches.first().expect("one batch");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(
        array_value_to_string(batch.column(0), 0).expect("printable"),
        "5"
    );
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_failing_statement_keeps_the_open_transaction() {
    // A server error that keeps the connection leaves the transaction open:
    // the state is asked of the server, not left unknown.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let state = || async { session.transaction_state(&CancelToken::new()).await };
    apply(&*session, "DROP TABLE IF EXISTS oxyn_tx_failing").await;
    apply(
        &*session,
        "CREATE TABLE oxyn_tx_failing (id INT PRIMARY KEY)",
    )
    .await;
    apply(&*session, "START TRANSACTION").await;
    apply(&*session, "INSERT INTO oxyn_tx_failing VALUES (1)").await;
    // Fails at execution, in the cursor.
    let duplicate = refusal(
        run(
            &*session,
            write_request("INSERT INTO oxyn_tx_failing VALUES (1)"),
        )
        .await,
        "a duplicate key is refused",
    );
    assert_eq!(server_code(&duplicate), Some(1062), "{duplicate}");
    assert_eq!(
        state().await,
        TransactionState::Open,
        "after a duplicate key"
    );
    // Fails at prepare.
    let missing = refusal(
        run(&*session, write_request("SELECT * FROM oxyn_no_such_table")).await,
        "a missing table is refused",
    );
    assert_eq!(server_code(&missing), Some(1146), "{missing}");
    assert_eq!(
        state().await,
        TransactionState::Open,
        "after a missing table"
    );
    apply(&*session, "COMMIT").await;
    assert_eq!(state().await, TransactionState::Idle, "after COMMIT");
    assert_eq!(
        first(&*observer, "SELECT COUNT(*) FROM oxyn_tx_failing").await,
        "1",
        "the row written before the failures is committed"
    );
    apply(&*session, "DROP TABLE oxyn_tx_failing").await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

/// Relays one connection to `upstream`, until the client sends a `KILL`: from
/// then on nothing more is forwarded, either way, and the sockets stay open.
/// The server accepted the connection and the login, then answers nothing —
/// what no real server can be asked to do on cue. `stalled` says the `KILL`
/// was reached. Plain text only: the relay reads the packets.
async fn relay_until_kill(
    client: tokio::net::TcpStream,
    upstream: tokio::net::TcpStream,
    stalled: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let (mut from_client, mut to_client) = client.into_split();
    let (mut from_server, mut to_server) = upstream.into_split();
    let answers = tokio::io::copy(&mut from_server, &mut to_client);
    let requests = async {
        let mut pending = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = from_client.read(&mut chunk).await?;
            if read == 0 {
                return Ok(());
            }
            pending.extend_from_slice(chunk.get(..read).unwrap_or_default());
            while let Some(length) = whole_packet(&pending) {
                let packet: Vec<u8> = pending.drain(..length).collect();
                // Four header bytes, then `COM_QUERY` (3) and its text.
                if packet.get(4..9) == Some(b"\x03KILL".as_slice()) {
                    stalled.store(true, std::sync::atomic::Ordering::SeqCst);
                    std::future::pending::<()>().await;
                }
                to_server.write_all(&packet).await?;
            }
        }
    };
    let (answered, requested) = tokio::join!(answers, requests);
    answered.and(requested)
}

/// The length of the first complete packet of `bytes`, header included.
fn whole_packet(bytes: &[u8]) -> Option<usize> {
    let [low, middle, high, _sequence, ..] = *bytes else {
        return None;
    };
    let length = 4 + (usize::from(low) | usize::from(middle) << 8 | usize::from(high) << 16);
    (bytes.len() >= length).then_some(length)
}

#[tokio::test]
#[ignore = "needs a MySQL or MariaDB server: see the module documentation"]
async fn a_kill_the_server_never_answers_ends_within_its_bound() {
    let Some((mut config, credentials)) = target() else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return;
    };
    let host = config.params.get("host").cloned().unwrap_or_default();
    let port = config.params.get("port").cloned().unwrap_or_default();
    let upstream = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local port");
    let relay_port = listener.local_addr().expect("a bound listener").port();
    let stalled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay = tokio::spawn({
        let stalled = std::sync::Arc::clone(&stalled);
        async move {
            let (client, _) = listener.accept().await?;
            let upstream = tokio::net::TcpStream::connect(upstream).await?;
            relay_until_kill(client, upstream, &stalled).await
        }
    });
    config
        .params
        .insert("host".to_owned(), "127.0.0.1".to_owned());
    config
        .params
        .insert("port".to_owned(), relay_port.to_string());
    config
        .params
        .insert("sslmode".to_owned(), "disable".to_owned());
    let spec = crate::options::ConnectSpec::from_config(
        &crate::driver::mysql_metadata(),
        &config,
        &credentials,
    )
    .expect("the test configuration, moved to the relay");
    let bound = Duration::from_secs(2);
    let killer = crate::cancel::Killer::new(spec, DriverId::mysql()).within(bound);

    let started = std::time::Instant::now();
    // The id is never read: the relay keeps the `KILL` from the server.
    let issue = killer.kill_query(1).await;
    let elapsed = started.elapsed();

    assert!(
        stalled.load(std::sync::atomic::Ordering::SeqCst),
        "the killing connection logged in and sent its KILL through the relay"
    );
    assert!(
        elapsed < bound + Duration::from_secs(1),
        "the kill ends with its bound, not with the server: {elapsed:?}"
    );
    let error = refusal(issue, "a KILL nobody answers is not a success");
    assert!(
        matches!(error, OxynError::Timeout { .. }),
        "an unanswered KILL may have been applied: {error}"
    );
    assert_eq!(error.class(), ErrorClass::Ambiguous);
    relay.abort();
}
