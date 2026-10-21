//! A Stop on a write reports how the engine ended it, never a cancellation the
//! token alone decided (issue #181, [I-13](../../../CLAUDE.md#i-13)).
//!
//! A `RETURNING` makes all its changes at its first step, before its rows are
//! read: by the time the cursor holds a batch, the write may be applied.
//!
//! The same holds for a read that ends a batch whose earlier statements already
//! applied (issue #221): `INSERT …; SELECT …` stopped is not a cancellation, the
//! insert is in the data.

use std::time::Duration;

use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits, ExecRequest,
    OxynError, QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::{BatchLimits, SqliteDriver};

/// Bounds a test that would otherwise hang on a regression.
const GIVE_UP_AFTER: Duration = Duration::from_secs(30);

/// How long a batch runs before the Stop of [`stopped_batch`]: long enough for
/// the earlier statements to be done, short against [`LONG_READ`].
const STOP_AFTER: Duration = Duration::from_millis(250);

/// A read that takes seconds, so a Stop lands inside its statement.
const LONG_READ: &str = "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s \
                         WHERE n < 100000000) SELECT count(*) FROM s";

async fn workshop(limits: BatchLimits) -> Box<dyn Session> {
    let config = ConnectionConfig::new("stopped-write", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    SqliteDriver::new()
        .with_batch_limits(limits)
        .connect(&config, &Credentials::new(), &CancelToken::new())
        .await
        .expect("an in-memory database always opens")
}

pub(super) fn writable(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
}

pub(super) async fn run(session: &dyn Session, sql: &str) {
    let mut cursor = session
        .execute(writable(sql), &CancelToken::new())
        .await
        .expect("fixture SQL");
    while cursor.next_batch().await.expect("fixture SQL").is_some() {}
}

/// The single integer a read-only query returns.
pub(super) async fn scalar(session: &dyn Session, sql: &str) -> i64 {
    let mut cursor = session
        .execute(
            ExecRequest::new(QueryLanguage::SQL, sql),
            &CancelToken::new(),
        )
        .await
        .expect("scalar query");
    let batch = cursor
        .next_batch()
        .await
        .expect("scalar query")
        .expect("one row");
    batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("the query returns an integer")
        .value(0)
}

pub(super) async fn count(session: &dyn Session) -> i64 {
    scalar(session, "SELECT COUNT(*) FROM t").await
}

/// Runs `sql` and stops it [`STOP_AFTER`] in, while its last statement is still
/// a long read. Returns what `execute` failed with.
async fn stopped_batch(session: &dyn Session, sql: &str) -> OxynError {
    let cancel = CancelToken::new();
    let stop = cancel.clone();
    let stopper = tokio::spawn(async move {
        tokio::time::sleep(STOP_AFTER).await;
        stop.cancel();
    });
    let outcome = tokio::time::timeout(GIVE_UP_AFTER, session.execute(writable(sql), &cancel))
        .await
        .expect("the interruption reaches the engine");
    stopper.await.expect("the stopper ends");
    failure(outcome, "a hundred million rows do not fit in the delay")
}

/// The failure of a call that had to fail; `expect_err` would require `Debug`
/// on a cursor.
pub(super) fn failure<T>(outcome: oxyn_core::Result<T>, why: &str) -> OxynError {
    match outcome {
        Ok(_) => panic!("{why}"),
        Err(err) => err,
    }
}

#[tokio::test]
async fn a_returning_write_finished_before_the_stop_is_completed() {
    // The audit's reproduction: every row fits in the first batch, so the
    // statement ran to its end before the Stop.
    let session = workshop(BatchLimits::default()).await;
    run(session.as_ref(), "CREATE TABLE t(id INTEGER)").await;

    let cancel = CancelToken::new();
    let mut cursor = session
        .execute(
            writable("INSERT INTO t VALUES (1), (2) RETURNING id"),
            &cancel,
        )
        .await
        .expect("execution");
    let first = cursor.next_batch().await.expect("first batch");
    assert_eq!(first.map(|batch| batch.num_rows()), Some(2));

    cancel.cancel();
    let end = cursor.next_batch().await;
    assert!(
        matches!(end, Ok(None)),
        "the write was applied, the Stop came too late: {:?}",
        end.map(|batch| batch.map(|batch| batch.num_rows()))
    );
    drop(cursor);
    assert_eq!(count(session.as_ref()).await, 2);
}

#[tokio::test]
async fn a_returning_write_stopped_between_batches_is_ambiguous() {
    // Its changes are made at the first step; the rows are still being read
    // when the Stop arrives. Nothing proves it rolled back.
    let session = workshop(BatchLimits::new().with_max_rows(10)).await;
    run(session.as_ref(), "CREATE TABLE t(id INTEGER)").await;

    let cancel = CancelToken::new();
    let mut cursor = session
        .execute(
            writable(
                "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 50) \
                 INSERT INTO t SELECT n FROM s RETURNING id",
            ),
            &cancel,
        )
        .await
        .expect("execution");
    let first = cursor.next_batch().await.expect("first batch");
    assert_eq!(first.map(|batch| batch.num_rows()), Some(10));

    cancel.cancel();
    let err = failure(cursor.next_batch().await, "a stopped write must not end");
    assert!(!err.is_cancelled(), "never a proven cancellation: {err:?}");
    assert!(matches!(err, OxynError::OutcomeUnknown(_)), "{err:?}");
    assert_eq!(err.class(), ErrorClass::Ambiguous);
    assert!(cursor.stats().truncated);

    drop(cursor);
    // The statement is atomic: all of it or none of it, which is exactly what
    // the ambiguity leaves open.
    let applied = count(session.as_ref()).await;
    assert!(applied == 0 || applied == 50, "{applied} rows");
}

#[tokio::test]
async fn a_returning_write_interrupted_at_its_first_step_reports_the_engine() {
    // The Stop lands while the engine applies the changes: it is the engine
    // that says the statement was interrupted, and an interrupted write is
    // ambiguous (`error::engine_bound`). The data shows it rolled back.
    let session = workshop(BatchLimits::default()).await;
    run(session.as_ref(), "CREATE TABLE t(id INTEGER)").await;

    let cancel = CancelToken::new();
    let stop = cancel.clone();
    let stopper = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
    });
    let outcome = tokio::time::timeout(
        GIVE_UP_AFTER,
        session.execute(
            writable(
                "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s \
                 WHERE n < 100000000) INSERT INTO t SELECT n FROM s RETURNING id",
            ),
            &cancel,
        ),
    )
    .await
    .expect("the interruption reaches the engine");
    stopper.await.expect("the stopper ends");

    let err = failure(outcome, "a hundred million rows do not fit in 100 ms");
    assert!(
        !err.is_cancelled(),
        "the token alone proves nothing: {err:?}"
    );
    assert_eq!(err.class(), ErrorClass::Ambiguous, "{err:?}");
    assert_eq!(count(session.as_ref()).await, 0);
}

#[tokio::test]
async fn a_stopped_read_is_still_cancelled_at_once() {
    let session = workshop(BatchLimits::new().with_max_rows(10)).await;
    let cancel = CancelToken::new();
    let mut cursor = session
        .execute(
            writable(
                "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 50) \
                 SELECT n FROM s",
            ),
            &cancel,
        )
        .await
        .expect("execution");
    assert!(cursor.next_batch().await.expect("first batch").is_some());

    cancel.cancel();
    let err = failure(cursor.next_batch().await, "a stopped read must not end");
    assert!(err.is_cancelled(), "{err:?}");
}

/// What a batch whose earlier statement applied must not be told as.
fn assert_applied_and_not_retryable(err: &OxynError) {
    assert!(
        !err.is_cancelled(),
        "the batch changed data: not a cancellation: {err:?}"
    );
    assert!(
        !err.is_retryable(),
        "replaying would apply it twice: {err:?}"
    );
    assert!(
        err.to_string().contains("already applied"),
        "the message must say what applied: {err}"
    );
}

#[tokio::test]
async fn an_interrupted_read_after_an_applied_insert_is_neither_cancelled_nor_retryable() {
    // Issue #221, scenario (b): the INSERT is in the data, the final read is
    // what the Stop interrupts, and the engine reports it as a read.
    let session = workshop(BatchLimits::default()).await;
    run(session.as_ref(), "CREATE TABLE t(id INTEGER)").await;

    let err = stopped_batch(
        session.as_ref(),
        &format!("INSERT INTO t VALUES (1); {LONG_READ}"),
    )
    .await;

    assert_applied_and_not_retryable(&err);
    assert_eq!(
        count(session.as_ref()).await,
        1,
        "the insert committed before the Stop"
    );
}

#[tokio::test]
async fn an_interrupted_read_after_a_create_table_is_not_cancelled() {
    // `total_changes` never moves for DDL: counting rows alone would leave this
    // batch looking untouched.
    let session = workshop(BatchLimits::default()).await;

    let err = stopped_batch(session.as_ref(), &format!("CREATE TABLE d(x); {LONG_READ}")).await;

    assert_applied_and_not_retryable(&err);
    assert_eq!(
        scalar(
            session.as_ref(),
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'd'"
        )
        .await,
        1,
        "the table exists"
    );
}

#[tokio::test]
async fn a_read_stopped_at_rest_after_an_applied_insert_is_not_cancelled() {
    // The cursor's own refusal, with no request in flight: `abort` used to
    // answer a bare cancellation for any read.
    let session = workshop(BatchLimits::new().with_max_rows(10)).await;
    run(session.as_ref(), "CREATE TABLE t(id INTEGER)").await;

    let cancel = CancelToken::new();
    let mut cursor = session
        .execute(
            writable(
                "INSERT INTO t VALUES (1); \
                 WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 50) \
                 SELECT n FROM s",
            ),
            &cancel,
        )
        .await
        .expect("execution");
    assert!(cursor.next_batch().await.expect("first batch").is_some());

    cancel.cancel();
    let err = failure(cursor.next_batch().await, "a stopped read must not end");
    assert_applied_and_not_retryable(&err);
    drop(cursor);
    assert_eq!(count(session.as_ref()).await, 1);
}

#[tokio::test]
async fn a_read_stopped_while_a_batch_is_pulled_after_an_applied_insert_is_not_cancelled() {
    // The first batch is cheap; the second scans a hundred million rows for the
    // next match, so the Stop lands while the cursor waits for the worker thread.
    // `await_reply` would let go at once with a bare cancellation.
    let session = workshop(BatchLimits::new().with_max_rows(5)).await;
    run(session.as_ref(), "CREATE TABLE t(id INTEGER)").await;

    let cancel = CancelToken::new();
    let mut cursor = session
        .execute(
            writable(
                "INSERT INTO t VALUES (1); \
                 WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s \
                 WHERE n < 100000000) SELECT n FROM s WHERE n <= 5 OR n > 99999990",
            ),
            &cancel,
        )
        .await
        .expect("execution");
    let first = cursor.next_batch().await.expect("first batch");
    assert_eq!(first.map(|batch| batch.num_rows()), Some(5));

    let stop = cancel.clone();
    let stopper = tokio::spawn(async move {
        tokio::time::sleep(STOP_AFTER).await;
        stop.cancel();
    });
    let outcome = tokio::time::timeout(GIVE_UP_AFTER, cursor.next_batch())
        .await
        .expect("the interruption reaches the engine");
    stopper.await.expect("the stopper ends");

    let err = failure(outcome, "the second batch needs a hundred million steps");
    assert_applied_and_not_retryable(&err);
    drop(cursor);
    assert_eq!(count(session.as_ref()).await, 1);
}
