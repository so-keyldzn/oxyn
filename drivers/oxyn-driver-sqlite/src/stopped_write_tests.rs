//! A Stop on a write reports how the engine ended it, never a cancellation the
//! token alone decided (issue #181, [I-13](../../../CLAUDE.md#i-13)).
//!
//! A `RETURNING` makes all its changes at its first step, before its rows are
//! read: by the time the cursor holds a batch, the write may be applied.

use std::time::Duration;

use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits, ExecRequest,
    OxynError, QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::{BatchLimits, SqliteDriver};

/// Bounds a test that would otherwise hang on a regression.
const GIVE_UP_AFTER: Duration = Duration::from_secs(30);

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

fn writable(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
}

async fn run(session: &dyn Session, sql: &str) {
    let mut cursor = session
        .execute(writable(sql), &CancelToken::new())
        .await
        .expect("fixture SQL");
    while cursor.next_batch().await.expect("fixture SQL").is_some() {}
}

async fn count(session: &dyn Session) -> i64 {
    let mut cursor = session
        .execute(
            ExecRequest::new(QueryLanguage::SQL, "SELECT COUNT(*) FROM t"),
            &CancelToken::new(),
        )
        .await
        .expect("count");
    let batch = cursor.next_batch().await.expect("count").expect("one row");
    batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("COUNT(*) is an integer")
        .value(0)
}

/// The failure of a call that had to fail; `expect_err` would require `Debug`
/// on a cursor.
fn failure<T>(outcome: oxyn_core::Result<T>, why: &str) -> OxynError {
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
