//! The execution deadline covers the whole execution — the wait for the
//! session, the preparation, the first batch and the drain — and interrupts
//! the driver when it expires, instead of dropping its future.

use super::*;
use async_trait::async_trait;
use oxyn_core::{Capabilities, DefaultPolicy, DriverId, ExecLimits, QueryLanguage, SqlDialect};
use oxyn_driver::Session;
use oxyn_driver_sqlite::SqliteDriver;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A single aggregate row, computed entirely inside SQLite's `execute`: the
/// cursor only comes back once every one of the billion rows is summed.
const LONG_AGGREGATE: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000) \
     SELECT sum(x) FROM c";

/// Scheduling slack: far above an interrupted engine or a free session
/// answering `SELECT 1`, far below the aggregate run to its end.
const TOLERANCE: Duration = Duration::from_secs(2);

/// Bounds a test that would otherwise wait for the whole aggregate.
const GIVE_UP_AFTER: Duration = Duration::from_secs(20);

struct Bench {
    executor: Arc<Executor>,
    connection: ConnectionId,
    session: SessionId,
}

async fn sqlite_bench() -> Bench {
    let connection = ConnectionConfig::new("deadline", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("deadline").expect("workspace").id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
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

async fn run(bench: &Bench, text: &str, limits: ExecLimits) -> Result<Outcome> {
    let command = Command::Execute {
        connection: bench.connection,
        session: bench.session,
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text).with_limits(limits),
        ),
    };
    tokio::time::timeout(
        GIVE_UP_AFTER,
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .await
    .expect("the deadline must end the execution long before the aggregate does")
}

#[tokio::test]
async fn a_sqlite_aggregate_stops_at_the_deadline_not_at_its_first_batch() {
    let bench = sqlite_bench().await;
    let limit = Duration::from_millis(1);

    let started = Instant::now();
    let outcome = run(
        &bench,
        LONG_AGGREGATE,
        ExecLimits::default().with_timeout(limit),
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        matches!(outcome, Err(OxynError::Timeout { after }) if after == limit),
        "expected a timeout, got {outcome:?}"
    );
    assert!(
        elapsed < TOLERANCE,
        "the aggregate ran {elapsed:?} past a {limit:?} deadline"
    );

    // The engine was interrupted, not left computing: the same session is
    // free at once.
    let started = Instant::now();
    let answer = run(&bench, "SELECT 1", ExecLimits::default()).await;
    assert!(
        matches!(answer, Ok(Outcome::Executed { .. })),
        "SELECT 1 after the timeout: {answer:?}"
    );
    assert!(
        started.elapsed() < TOLERANCE,
        "the session stayed busy {:?}",
        started.elapsed()
    );
}

/// A driver whose `execute` never returns a cursor on its own: it waits for
/// its token, as the contract asks of any call that may last.
#[derive(Default)]
struct Stuck {
    calls: AtomicUsize,
    interrupted: AtomicBool,
    catalog: catalog_tests::Probe,
}

struct StuckSession(Arc<Stuck>);

#[async_trait]
impl Session for StuckSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }
    async fn execute(&self, _: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        cancel.cancelled().await;
        self.0.interrupted.store(true, Ordering::SeqCst);
        Err(OxynError::Cancelled)
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

#[tokio::test]
async fn an_execution_stuck_before_its_cursor_is_interrupted_once_and_never_replayed() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("stuck").expect("workspace").id;
    let config =
        ConnectionConfig::new("stuck", DriverId::sqlite()).with_environment(Environment::Local);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&config);
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&config);
    let probe = Arc::new(Stuck::default());
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(StuckSession(probe.clone())),
    ));

    // A write: the server may have applied it when the deadline strikes.
    let limit = Duration::from_millis(50);
    let request = ExecRequest::new(QueryLanguage::SQL, "INSERT INTO t VALUES (1)")
        .with_limits(ExecLimits::default().writable().with_timeout(limit));
    let outcome = tokio::time::timeout(
        GIVE_UP_AFTER,
        executor.dispatch(
            Actor::Human,
            Command::Execute {
                connection: config.id,
                session: slot.id(),
                request: Box::new(request),
            },
            &CancelToken::new(),
        ),
    )
    .await
    .expect("the deadline must reach an execution that has no cursor yet");

    let Err(error) = outcome else {
        panic!("expected a timeout, got {outcome:?}");
    };
    assert!(matches!(error, OxynError::Timeout { after } if after == limit));
    // Interrupted through its token, not dropped mid-call.
    assert!(probe.interrupted.load(Ordering::SeqCst));
    // Ambiguous, so never retried: the write reached the driver once (I-13).
    assert_eq!(error.class(), oxyn_core::ErrorClass::Ambiguous);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
}
