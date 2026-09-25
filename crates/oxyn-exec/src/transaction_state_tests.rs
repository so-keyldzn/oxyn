//! The transaction state published at every exit of an execution
//! ([ADR-0039](../../../docs/adr/0039-etat-de-transaction-d-une-session.md) §3):
//! success, failed drain, early failure of `slot.execute`, cancellation, and
//! what an abandonment leaves for the interface to infer.

use super::*;
use async_trait::async_trait;
use oxyn_core::{
    Capabilities, DefaultPolicy, DriverId, ExecLimits, QueryLanguage, SqlDialect, TransactionState,
};
use oxyn_driver::Session;
use oxyn_driver_sqlite::{BatchLimits, SqliteDriver};

/// Far above anything these tests wait for, as a guard against a hang.
const WITHIN: Duration = Duration::from_secs(10);

struct Bench {
    executor: Arc<Executor>,
    connection: ConnectionId,
    session: SessionId,
}

/// An executor with one SQLite session on `path`, one row per batch so that a
/// read has a cursor to drain before it ends.
async fn bench(path: &str) -> Bench {
    let connection = ConnectionConfig::new("transactions", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, path);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("transactions")
        .expect("workspace")
        .id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(
            SqliteDriver::new().with_batch_limits(BatchLimits::new().with_max_rows(1)),
        ))
        .expect("SQLite registration");
    let executor = Arc::new(
        Executor::builder(store, policy)
            .with_drivers(Arc::new(drivers))
            .with_workspace(workspace)
            .build(),
    );
    executor.register_connection(&connection);
    let Outcome::Connected {
        session,
        transaction_state,
        ..
    } = executor
        .dispatch(
            Actor::Human,
            Command::Connect {
                connection: connection.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("SQLite connection")
    else {
        panic!("expected a connected session");
    };
    // Read through the bus at opening (ADR-0039 §4), never assumed.
    assert_eq!(transaction_state, TransactionState::Idle);
    Bench {
        executor,
        connection: connection.id,
        session,
    }
}

fn execute(bench: &Bench, text: &str) -> Command {
    Command::Execute {
        connection: bench.connection,
        session: bench.session,
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text)
                .with_limits(ExecLimits::default().writable()),
        ),
    }
}

/// Runs `text` to its end and returns what it announced, in order.
async fn run(bench: &Bench, text: &str) -> (Result<Outcome>, Vec<Event>) {
    let mut events = bench.executor.subscribe();
    let id = CommandId::new();
    let issue = bench
        .executor
        .dispatch_as(id, Actor::Human, execute(bench, text), &CancelToken::new())
        .await;
    (issue, announced(&mut events, id))
}

/// What `id` announced so far; every event is published before `dispatch`
/// returns, so nothing is still in flight.
fn announced(
    events: &mut tokio::sync::broadcast::Receiver<crate::ExecEvent>,
    id: CommandId,
) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.command == id {
            seen.push(event.event);
        }
    }
    seen
}

/// The event that ended the execution; `HistoryRecorded` may follow it.
fn terminal(events: &[Event]) -> Option<&Event> {
    events.iter().find(|event| event.is_terminal())
}

/// The states announced, and whether each came before any terminal event.
fn states(bench: &Bench, events: &[Event]) -> Vec<TransactionState> {
    let mut states = Vec::new();
    let mut ended = false;
    for event in events {
        match event {
            Event::TransactionState { session, state } => {
                assert_eq!(*session, bench.session, "the state names its session");
                assert!(!ended, "a state after the terminal event: {events:?}");
                states.push(*state);
            }
            event if event.is_terminal() => ended = true,
            _ => {}
        }
    }
    states
}

#[tokio::test]
async fn a_completed_execution_publishes_the_state_before_completed() {
    let bench = bench(SqliteDriver::MEMORY).await;
    let (issue, events) = run(&bench, "BEGIN").await;
    assert!(issue.is_ok(), "{issue:?}");
    assert_eq!(states(&bench, &events), [TransactionState::Open]);
    assert!(
        matches!(terminal(&events), Some(Event::Completed { .. })),
        "{events:?}"
    );

    let (issue, events) = run(&bench, "COMMIT").await;
    assert!(issue.is_ok(), "{issue:?}");
    assert_eq!(states(&bench, &events), [TransactionState::Idle]);
}

#[tokio::test]
async fn a_failed_drain_publishes_the_state_before_failed() {
    // The second row overflows: the first batch is out, the drain fails. The
    // error does not end the transaction, and the state says so.
    let bench = bench(SqliteDriver::MEMORY).await;
    run(&bench, "BEGIN").await.0.expect("BEGIN");
    let (issue, events) = run(
        &bench,
        "WITH c(x) AS (VALUES (1), (-9223372036854775808)) SELECT abs(x) FROM c",
    )
    .await;
    assert!(issue.is_err(), "the overflow fails the drain");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::SchemaReady { .. })),
        "the cursor existed: this is the drain failing, not execute: {events:?}"
    );
    assert_eq!(states(&bench, &events), [TransactionState::Open]);
    assert!(
        matches!(terminal(&events), Some(Event::Failed { .. })),
        "{events:?}"
    );
}

#[tokio::test]
async fn an_early_failure_of_execute_still_publishes_the_state() {
    // No cursor, no terminal event: the error goes back to the caller. The
    // state is published all the same.
    let bench = bench(SqliteDriver::MEMORY).await;
    let (issue, events) = run(&bench, "INSERT INTO missing VALUES (1)").await;
    assert!(issue.is_err());
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(states(&bench, &events), [TransactionState::Idle]);
}

#[tokio::test]
async fn a_stop_during_a_read_reads_the_state_with_its_own_token() {
    // The execution's token has fired: handed to the read, it would make it
    // `Unknown`. An interrupted read does not end the transaction.
    let bench = bench(SqliteDriver::MEMORY).await;
    run(&bench, "BEGIN").await.0.expect("BEGIN");

    let mut events = bench.executor.subscribe();
    let id = CommandId::new();
    let stop = CancelToken::new();
    let task = tokio::spawn({
        let executor = Arc::clone(&bench.executor);
        let command = execute(
            &bench,
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) \
             SELECT x FROM c WHERE x = 1 OR x < 0",
        );
        let stop = stop.clone();
        async move { executor.dispatch_as(id, Actor::Human, command, &stop).await }
    });
    tokio::time::timeout(WITHIN, async {
        loop {
            let event = events.recv().await.expect("event stream");
            if event.command == id && matches!(event.event, Event::BatchReady { .. }) {
                return;
            }
        }
    })
    .await
    .expect("the first row arrives at once");
    stop.cancel();
    let issue = task.await.expect("the task ends");
    assert!(
        matches!(
            issue,
            Ok(Outcome::Executed {
                sink: SinkOutcome::Cancelled,
                ..
            })
        ),
        "{issue:?}"
    );
    let events = announced(&mut events, id);
    assert_eq!(states(&bench, &events), [TransactionState::Open]);
    assert_eq!(terminal(&events), Some(&Event::Cancelled));
}

#[tokio::test]
async fn a_stop_during_a_write_publishes_the_rollback_sqlite_did() {
    // Stop lands in `slot.execute`, since a write runs whole there: an early
    // failure, `Cancelled`, returned while the engine is still unwinding the
    // transaction. The state read after it sees the rollback.
    let folder = tempfile::tempdir().expect("temporary folder");
    let base = folder.path().join("transactions.sqlite");
    let journal = folder.path().join("transactions.sqlite-journal");
    let bench = bench(base.to_str().expect("UTF-8 path")).await;
    run(&bench, "CREATE TABLE t(v INTEGER)")
        .await
        .0
        .expect("DDL");
    let (_, events) = run(&bench, "BEGIN").await;
    assert_eq!(states(&bench, &events), [TransactionState::Open]);

    let mut events = bench.executor.subscribe();
    let id = CommandId::new();
    let stop = CancelToken::new();
    let task = tokio::spawn({
        let executor = Arc::clone(&bench.executor);
        let command = execute(
            &bench,
            "INSERT INTO t(v) SELECT x FROM \
             (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)",
        );
        let stop = stop.clone();
        async move { executor.dispatch_as(id, Actor::Human, command, &stop).await }
    });
    // The rollback journal appears on the first page written: the INSERT is
    // inside `sqlite3_step`, where the interrupt cannot be lost.
    tokio::time::timeout(WITHIN, async {
        while !journal.exists() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the endless INSERT starts writing");
    stop.cancel();

    let issue = task.await.expect("the task ends");
    assert!(
        issue.as_ref().is_err_and(OxynError::is_cancelled),
        "{issue:?}"
    );
    let events = announced(&mut events, id);
    assert_eq!(states(&bench, &events), [TransactionState::Idle]);
}

/// Answers `execute` with an error at once; then waits in
/// `transaction_state` until its token fires.
#[derive(Default)]
struct Waiting {
    asked: tokio::sync::Notify,
    catalog: catalog_tests::Probe,
}

struct WaitingSession(Arc<Waiting>);

#[async_trait]
impl Session for WaitingSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL | Capabilities::TRANSACTIONS
    }
    async fn execute(&self, _: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        Err(OxynError::Connection("refused".to_owned()))
    }
    async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
        self.0.asked.notify_one();
        cancel.cancelled().await;
        TransactionState::Unknown
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
        &self.0.catalog
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

fn waiting_bench() -> (Arc<Executor>, ConnectionId, Arc<SessionSlot>, Arc<Waiting>) {
    let config =
        ConnectionConfig::new("waiting", DriverId::sqlite()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("waiting").expect("workspace").id;
    let executor = Arc::new(
        Executor::builder(store, policy)
            .with_workspace(workspace)
            .build(),
    );
    executor.register_connection(&config);
    let probe = Arc::new(Waiting::default());
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(WaitingSession(Arc::clone(&probe))),
    ));
    (executor, config.id, slot, probe)
}

#[tokio::test]
async fn an_execution_abandoned_while_reading_the_state_still_announces_cancelled() {
    // The guard is still armed during the read: dropped there, the execution
    // announces `Cancelled`, with no state before it — which is what tells
    // the interface to fall back to `Unknown`.
    let (executor, connection, slot, probe) = waiting_bench();
    let mut events = executor.subscribe();
    let id = CommandId::new();
    let command = Command::Execute {
        connection,
        session: slot.id(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1")),
    };
    let asked = probe.asked.notified();
    let task = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move {
            executor
                .dispatch_as(id, Actor::Human, command, &CancelToken::new())
                .await
        }
    });
    tokio::time::timeout(WITHIN, asked)
        .await
        .expect("the state is read");
    task.abort();
    assert!(task.await.is_err_and(|error| error.is_cancelled()));

    let events = announced(&mut events, id);
    assert_eq!(events, [Event::Cancelled]);
}

#[tokio::test]
async fn a_session_closing_during_the_read_reports_unknown() {
    let (executor, connection, slot, probe) = waiting_bench();
    let mut events = executor.subscribe();
    let id = CommandId::new();
    let command = Command::Execute {
        connection,
        session: slot.id(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1")),
    };
    let asked = probe.asked.notified();
    let task = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move {
            executor
                .dispatch_as(id, Actor::Human, command, &CancelToken::new())
                .await
        }
    });
    tokio::time::timeout(WITHIN, asked)
        .await
        .expect("the state is read");
    tokio::time::timeout(
        WITHIN,
        executor.dispatch(
            Actor::Human,
            Command::CloseSession {
                connection,
                session: slot.id(),
            },
            &CancelToken::new(),
        ),
    )
    .await
    .expect("closing does not wait on the read")
    .expect("the session closes");
    let issue = task.await.expect("the task ends");
    assert!(issue.is_err());

    let events = announced(&mut events, id);
    assert_eq!(
        events,
        [Event::TransactionState {
            session: slot.id(),
            state: TransactionState::Unknown,
        }]
    );
}

#[tokio::test]
async fn a_session_without_transactions_publishes_no_state() {
    // PostgreSQL today: nothing to show, so nothing announced.
    let config =
        ConnectionConfig::new("pooled", DriverId::sqlite()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("pooled").expect("workspace").id;
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&config);
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(PlainSession(catalog_tests::Probe::default())),
    ));
    let mut events = executor.subscribe();
    let id = CommandId::new();
    let issue = executor
        .dispatch_as(
            id,
            Actor::Human,
            Command::Execute {
                connection: config.id,
                session: slot.id(),
                request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1")),
            },
            &CancelToken::new(),
        )
        .await;
    assert!(issue.is_err());
    assert!(announced(&mut events, id).is_empty());
}

/// A session without `TRANSACTIONS`, whose `execute` fails at once.
struct PlainSession(catalog_tests::Probe);

#[async_trait]
impl Session for PlainSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }
    async fn execute(&self, _: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        Err(OxynError::Connection("refused".to_owned()))
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
        &self.0
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}
