//! An execution whose future is dropped before it ends — `JoinSet::abort_all`,
//! a lost `select!` branch, a runtime shutting down — while preparing or while
//! draining. Nothing awaits its outcome any more, so whatever must happen has
//! to happen on drop.

use super::*;
use async_trait::async_trait;
use oxyn_core::{Capabilities, DefaultPolicy, DriverId, ExecLimits, QueryLanguage, SqlDialect};
use oxyn_driver::Session;
use oxyn_driver_sqlite::{BatchLimits, SqliteDriver};
use parking_lot::Mutex;

/// Emits its first row at once, then scans a billion rows for the second: the
/// only way to end it early is to interrupt the engine.
const LONG_SCAN: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000) \
     SELECT x FROM c WHERE x = 1 OR x = 1000000000";

/// Far below the scan, far above a free session answering `SELECT 1`.
const FREED_WITHIN: Duration = Duration::from_secs(10);

struct Bench {
    executor: Arc<Executor>,
    connection: ConnectionId,
    session: SessionId,
}

async fn bench() -> Bench {
    let connection = ConnectionConfig::new("abandon", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("abandon").expect("workspace").id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(
            // One row per batch: the cursor exists before the long scan starts,
            // so the drop lands in `drain`, not in `execute`.
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
    let Outcome::Connected { session, .. } = executor
        .dispatch(
            Actor::Human,
            Command::Connect {
                connection: connection.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("memory connection")
    else {
        panic!("expected a connected session");
    };
    Bench {
        executor,
        connection: connection.id,
        session,
    }
}

fn execute(bench: &Bench, text: &str, limits: ExecLimits) -> Command {
    Command::Execute {
        connection: bench.connection,
        session: bench.session,
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text).with_limits(limits),
        ),
    }
}

/// Waits for the first batch of `id`, so the drop happens while draining.
async fn first_batch(
    events: &mut tokio::sync::broadcast::Receiver<crate::ExecEvent>,
    id: CommandId,
) -> ResultId {
    loop {
        let event = events.recv().await.expect("event stream");
        if event.command != id {
            continue;
        }
        if let Event::BatchReady { result, .. } = event.event {
            return result;
        }
        assert!(
            !event.is_terminal(),
            "the scan ended before it could be abandoned: {:?}",
            event.event
        );
    }
}

#[tokio::test]
async fn an_execution_dropped_while_draining_releases_its_statement_and_its_session() {
    for limits in [ExecLimits::default(), ExecLimits::default().writable()] {
        let bench = bench().await;
        let mut events = bench.executor.subscribe();
        let id = CommandId::new();
        let command = execute(&bench, LONG_SCAN, limits);

        let task = tokio::spawn({
            let executor = Arc::clone(&bench.executor);
            async move {
                executor
                    .dispatch_as(id, Actor::Human, command, &CancelToken::new())
                    .await
            }
        });
        let result = tokio::time::timeout(FREED_WITHIN, first_batch(&mut events, id))
            .await
            .expect("the first row arrives at once");
        assert_eq!(bench.executor.running().len(), 1, "the scan is registered");

        task.abort();
        assert!(
            task.await.is_err_and(|error| error.is_cancelled()),
            "the task was aborted mid-drain"
        );

        // Nothing awaits the outcome any more: the registry must not keep a
        // statement nobody will ever finish.
        assert!(
            bench.executor.running().is_empty(),
            "an abandoned statement stays registered as running"
        );

        // Whoever still reads the result must not wait forever for its end,
        // nor read a partial result as a whole one.
        let buffer = bench.executor.result(result).expect("result retained");
        assert!(buffer.is_complete(), "the abandoned result is never closed");
        assert!(buffer.stats().truncated, "a partial result must say so");

        // The execution's end is announced, as for any other.
        let terminal = tokio::time::timeout(FREED_WITHIN, async {
            loop {
                let event = events.recv().await.expect("event stream");
                if event.command == id && event.is_terminal() {
                    return event.event;
                }
            }
        })
        .await
        .expect("an abandoned execution announces its end");
        assert_eq!(terminal, Event::Cancelled);

        // The audit journal holds the decision, and its outcome once written by
        // the periodic writer — never from the drop itself.
        let of_command = |executor: &Executor| {
            executor
                .store()
                .journal()
                .recent(100)
                .expect("journal")
                .into_iter()
                .filter(|entry| entry.record.command_id == Some(id))
                .map(|entry| entry.record)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            of_command(&bench.executor).len(),
            1,
            "only the decision is written before the writer runs"
        );
        assert_eq!(bench.executor.journal_abandoned(), 1);
        assert_eq!(bench.executor.journal_abandoned(), 0, "written once");
        let records = of_command(&bench.executor);
        let outcome = records
            .iter()
            .find(|record| record.error.is_some())
            .expect("the abandoned command has an audit outcome");
        assert_eq!(
            outcome.error.as_deref(),
            Some(crate::abandon::ABANDONED_OUTCOME)
        );
        assert_eq!(outcome.error_class, Some(oxyn_core::ErrorClass::Ambiguous));
        let decision = records
            .iter()
            .find(|record| record.error.is_none())
            .expect("the decision");
        assert_eq!(
            outcome.statement, decision.statement,
            "the outcome carries what the ordinary audit carries, no more"
        );

        // The proof on the engine side: a statement still scanning would hold
        // the session's thread for minutes.
        let freed = tokio::time::timeout(
            FREED_WITHIN,
            bench.executor.dispatch(
                Actor::Human,
                execute(&bench, "SELECT 1", ExecLimits::default()),
                &CancelToken::new(),
            ),
        )
        .await
        .expect("the engine was interrupted and the session answers again")
        .expect("SELECT 1");
        assert!(matches!(freed, Outcome::Executed { .. }));
        assert!(bench.executor.running().is_empty());
    }
}

/// Never answers `execute`, and keeps the token it was handed.
#[derive(Default)]
struct Preparing {
    token: Mutex<Option<CancelToken>>,
    started: tokio::sync::Notify,
    catalog: catalog_tests::Probe,
    /// A server that never answers the close.
    hangs_on_close: bool,
}

struct PreparingSession(Arc<Preparing>);

#[async_trait]
impl Session for PreparingSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }
    async fn execute(&self, _: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        *self.0.token.lock() = Some(cancel.clone());
        self.0.started.notify_one();
        std::future::pending().await
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
        if self.0.hangs_on_close {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

#[tokio::test]
async fn a_shutdown_cut_by_its_bound_still_journals_queued_abandoned_outcomes() {
    let config =
        ConnectionConfig::new("hanging", DriverId::sqlite()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("hanging").expect("workspace").id;
    let executor = Arc::new(
        Executor::builder(store, policy)
            .with_workspace(workspace)
            .build(),
    );
    executor.register_connection(&config);
    let probe = Arc::new(Preparing {
        hangs_on_close: true,
        ..Preparing::default()
    });
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(PreparingSession(Arc::clone(&probe))),
    ));
    let id = CommandId::new();
    let command = Command::Execute {
        connection: config.id,
        session: slot.id(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1")),
    };

    let started = probe.started.notified();
    let task = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move {
            executor
                .dispatch_as(id, Actor::Human, command, &CancelToken::new())
                .await
        }
    });
    tokio::time::timeout(FREED_WITHIN, started)
        .await
        .expect("the driver was called");
    task.abort();
    assert!(task.await.is_err_and(|error| error.is_cancelled()));

    // The periodic writer has not run: the outcome waits in memory, and the
    // application's bound on shutdown expires on a close that never returns.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), executor.shutdown())
            .await
            .is_err(),
        "the close never returns, so the shutdown is cut"
    );

    let outcome = executor
        .store()
        .journal()
        .recent(100)
        .expect("journal")
        .into_iter()
        .find(|entry| entry.record.command_id == Some(id) && entry.record.error.is_some())
        .expect("the queued outcome was written before the sessions were closed");
    assert_eq!(
        outcome.record.error.as_deref(),
        Some(crate::abandon::ABANDONED_OUTCOME)
    );
}

#[tokio::test]
async fn an_execution_dropped_while_preparing_cancels_the_token_the_driver_holds() {
    let config =
        ConnectionConfig::new("preparing", DriverId::sqlite()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("preparing")
        .expect("workspace")
        .id;
    let executor = Arc::new(
        Executor::builder(store, policy)
            .with_workspace(workspace)
            .build(),
    );
    executor.register_connection(&config);
    let probe = Arc::new(Preparing::default());
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(PreparingSession(Arc::clone(&probe))),
    ));
    let mut events = executor.subscribe();
    let id = CommandId::new();
    let command = Command::Execute {
        connection: config.id,
        session: slot.id(),
        request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1")),
    };

    let started = probe.started.notified();
    let task = tokio::spawn({
        let executor = Arc::clone(&executor);
        async move {
            executor
                .dispatch_as(id, Actor::Human, command, &CancelToken::new())
                .await
        }
    });
    tokio::time::timeout(FREED_WITHIN, started)
        .await
        .expect("the driver was called");
    let token = probe
        .token
        .lock()
        .clone()
        .expect("token handed to the driver");
    assert!(!token.is_cancelled());

    task.abort();
    assert!(task.await.is_err_and(|error| error.is_cancelled()));

    // A driver that handed this token to background work learns it is over.
    assert!(
        token.is_cancelled(),
        "the abandoned execution's token stays live"
    );
    assert!(executor.running().is_empty());
    let terminal = tokio::time::timeout(FREED_WITHIN, async {
        loop {
            let event = events.recv().await.expect("event stream");
            if event.command == id && event.is_terminal() {
                return event.event;
            }
        }
    })
    .await
    .expect("an abandoned execution announces its end");
    assert_eq!(terminal, Event::Cancelled);
}
