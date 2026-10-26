use std::pin::pin;

use oxyn_core::{ConnectionId, QueryLanguage, SqlDialect};

use super::*;
use crate::worker::{self, OpenSpec, OpenTarget};

/// A `count(*)` over an endless series: it never returns on its own.
const ENDLESS: &str =
    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";

#[tokio::test]
async fn abandoning_execute_during_the_first_batch_frees_the_engine() {
    // The scenario: the tab closes while SQLite computes the first batch of an
    // aggregation. The cursor does not exist yet, so nobody else can interrupt.
    // Without interruption, the session stays busy for the whole duration of the
    // computation — here, forever.
    let cancel_token = CancelToken::new();
    let spec = OpenSpec {
        target: OpenTarget::Memory(ConnectionId::new()),
        read_only: false,
        vector_extension: false,
    };
    let (handle, thread) = worker::spawn(spec, &cancel_token).await.expect("open");
    let session = SqliteSession::new(handle, thread, Capabilities::SQL, BatchLimits::new());

    {
        let exec_request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), ENDLESS);
        let mut execution = pin!(session.execute(exec_request, &cancel_token));
        assert!(futures::poll!(execution.as_mut()).is_pending());
        // The worker thread has picked up the execution: we do not abandon a
        // task still queued, which would prove something else.
        tokio::time::timeout(Duration::from_secs(10), async {
            while session.worker.running().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the worker thread picks up the execution");
    }

    // The proof on the engine side: it answers a `SELECT 1`.
    let response = tokio::time::timeout(Duration::from_secs(10), session.ping()).await;
    let round_trip = response
        .expect("the engine must be freed by the abandonment")
        .expect("ping");
    assert!(
        round_trip < Duration::from_secs(1),
        "the engine must answer quickly: {round_trip:?}"
    );

    Box::new(session).close().await.expect("close");
}

/// A session opened through the driver, as the executor gets it.
async fn open_session(db_file: &str) -> Box<dyn Session> {
    use oxyn_core::{ConnectionConfig, DriverId, Environment};
    use oxyn_driver::{Credentials, Driver};

    let conn_config = ConnectionConfig::new("transactions", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(crate::SqliteDriver::PATH, db_file);
    crate::SqliteDriver::new()
        .connect(&conn_config, &Credentials::new(), &CancelToken::new())
        .await
        .unwrap_or_else(|err| panic!("open: {err}"))
}

/// Runs a statement to its end, cursor released.
async fn run_to_end(session: &dyn Session, sql: &str) {
    let exec_request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), sql)
        .with_limits(oxyn_core::ExecLimits::unbounded());
    let mut batch_cursor = match session.execute(exec_request, &CancelToken::new()).await {
        Ok(batch_cursor) => batch_cursor,
        Err(err) => panic!("execution of `{sql}`: {err}"),
    };
    while batch_cursor
        .next_batch()
        .await
        .expect("next batch")
        .is_some()
    {}
}

async fn state_of(session: &dyn Session) -> TransactionState {
    session.transaction_state(&CancelToken::new()).await
}

#[tokio::test]
async fn the_state_follows_written_transactions_and_those_of_the_trait() {
    // ADR-0039 §2, the contract of a session that declares TRANSACTIONS.
    let session = open_session(crate::SqliteDriver::MEMORY).await;
    assert!(session.capabilities().contains(Capabilities::TRANSACTIONS));
    assert_eq!(
        state_of(session.as_ref()).await,
        TransactionState::Idle,
        "a fresh connection is in autocommit, and says so"
    );
    run_to_end(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

    for terminator in ["COMMIT", "ROLLBACK", "END"] {
        run_to_end(session.as_ref(), "BEGIN").await;
        assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
        run_to_end(session.as_ref(), "INSERT INTO t(v) VALUES (1)").await;
        assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
        run_to_end(session.as_ref(), terminator).await;
        assert_eq!(
            state_of(session.as_ref()).await,
            TransactionState::Idle,
            "{terminator}"
        );
    }

    let cancel_token = CancelToken::new();
    session.begin(&cancel_token).await.expect("begin");
    assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
    session.commit(&cancel_token).await.expect("commit");
    assert_eq!(state_of(session.as_ref()).await, TransactionState::Idle);

    session.begin(&cancel_token).await.expect("begin");
    assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
    session.rollback(&cancel_token).await.expect("rollback");
    assert_eq!(state_of(session.as_ref()).await, TransactionState::Idle);

    session.close().await.expect("close");
}

#[tokio::test]
async fn an_already_fired_token_returns_unknown_never_idle() {
    let session = open_session(crate::SqliteDriver::MEMORY).await;
    let cancel_token = CancelToken::new();
    cancel_token.cancel();
    assert_eq!(
        session.transaction_state(&cancel_token).await,
        TransactionState::Unknown
    );
    session.close().await.expect("close");
}

#[tokio::test]
async fn the_automatic_rollback_after_a_stop_is_observed() {
    // The scenario: `BEGIN`, a write, then Stop on an endless `INSERT`.
    // SQLite rolls the whole transaction back on `SQLITE_INTERRUPT`, the
    // earlier write included: `execute` reports the engine's interruption,
    // ambiguous for a write, not a cancellation (issue #181). A state read
    // then must come after that step, not before.
    let dir = tempfile::tempdir().expect("temporary directory");
    let db_path = dir.path().join("transactions.sqlite");
    let journal = dir.path().join("transactions.sqlite-journal");
    let session = open_session(db_path.to_str().expect("UTF-8 path")).await;

    run_to_end(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;
    run_to_end(session.as_ref(), "BEGIN").await;
    assert!(
        !journal.exists(),
        "a deferred BEGIN writes nothing: the journal marks the first write"
    );

    let cancel_token = CancelToken::new();
    let exec_request = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Sqlite),
        "INSERT INTO t(v) SELECT x FROM \
             (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)",
    )
    .with_limits(oxyn_core::ExecLimits::unbounded());
    let mut endless_write = Box::pin(session.execute(exec_request, &cancel_token));
    assert!(futures::poll!(endless_write.as_mut()).is_pending());
    // The rollback journal appears on the first page written: the INSERT
    // is then inside `sqlite3_step`, past the point where an interrupt
    // could be lost. A condition, not a delay.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !journal.exists() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the endless INSERT starts writing");

    cancel_token.cancel();
    match endless_write.await {
        Ok(_) => panic!("an endless INSERT only ends interrupted"),
        Err(err) => assert_eq!(err.class(), oxyn_core::ErrorClass::Ambiguous, "{err:?}"),
    }

    assert_eq!(
        state_of(session.as_ref()).await,
        TransactionState::Idle,
        "the interrupted write rolled the transaction back"
    );
    session.close().await.expect("close");
}
