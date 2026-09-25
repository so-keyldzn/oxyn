//! An agent sharing a session with the user cannot settle the user's
//! transaction (issue #19).
//!
//! `COMMIT`, `ROLLBACK`, `SAVEPOINT` and `RELEASE` are classified `Read`: they
//! read and write nothing of their own. On a session the user holds a
//! transaction on, they commit or throw away the user's writes. The scenario
//! is the audit's, through the real bus: SQLite marked production, one
//! session, a human `BEGIN` and an approved `INSERT`, then the agent.

use super::*;
use oxyn_core::{
    AgentId, AgentSessionId, DefaultPolicy, DriverId, ExecLimits, QueryLanguage, SqlDialect,
};
use oxyn_driver_sqlite::SqliteDriver;

struct Bench {
    executor: Executor,
    connection: ConnectionId,
    session: SessionId,
    /// Keeps the database file alive for the test's length.
    _directory: tempfile::TempDir,
}

async fn bench() -> Bench {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("shared.sqlite");
    let connection = ConnectionConfig::new("customers", DriverId::sqlite())
        .with_environment(Environment::Production)
        .with_param(SqliteDriver::PATH, path.to_string_lossy());
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("agent transactions")
        .expect("workspace")
        .id;
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .expect("SQLite registration");
    let executor = Executor::builder(store, policy)
        .with_drivers(Arc::new(drivers))
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);
    let session = connect(&executor, connection.id).await;
    Bench {
        executor,
        connection: connection.id,
        session,
        _directory: directory,
    }
}

async fn connect(executor: &Executor, connection: ConnectionId) -> SessionId {
    let Outcome::Connected { session, .. } = executor
        .dispatch(
            Actor::Human,
            Command::Connect { connection },
            &CancelToken::new(),
        )
        .await
        .expect("SQLite connection")
    else {
        panic!("expected a connected session");
    };
    session
}

fn agent() -> Actor {
    Actor::agent(AgentId::new(), AgentSessionId::new())
}

/// `text` on `session`, as `writable` asks. The agent's requests are left
/// bounded to read-only: the refusal must not rest on that bound.
fn execute(bench: &Bench, session: SessionId, text: &str, writable: bool) -> Command {
    let limits = if writable {
        ExecLimits::default().writable()
    } else {
        ExecLimits::default()
    };
    Command::Execute {
        connection: bench.connection,
        session,
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text).with_limits(limits),
        ),
    }
}

/// Runs a human statement, approving it when production asks to.
async fn human(bench: &Bench, session: SessionId, text: &str) -> Outcome {
    let outcome = bench
        .executor
        .dispatch(
            Actor::Human,
            execute(bench, session, text, true),
            &CancelToken::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("`{text}`: {error}"));
    match outcome {
        Outcome::NeedsApproval { command, .. } => bench
            .executor
            .approve("tester", command, &CancelToken::new())
            .await
            .unwrap_or_else(|error| panic!("`{text}` approved: {error}")),
        Outcome::Denied { reason, .. } => panic!("`{text}` denied to the human: {reason}"),
        executed => executed,
    }
}

/// The rows `session` sees in the table.
async fn rows(bench: &Bench, session: SessionId) -> usize {
    match human(bench, session, "SELECT id FROM shared").await {
        Outcome::Executed { buffer, .. } => buffer.row_count(),
        _ => panic!("a read returns a result"),
    }
}

#[tokio::test]
async fn an_agent_cannot_commit_roll_back_or_mark_the_users_transaction() {
    let bench = bench().await;
    human(&bench, bench.session, "CREATE TABLE shared (id INTEGER)").await;
    human(&bench, bench.session, "BEGIN").await;
    human(&bench, bench.session, "INSERT INTO shared VALUES (1)").await;
    assert_eq!(rows(&bench, bench.session).await, 1);

    for text in [
        "ROLLBACK",
        "COMMIT",
        "END",
        "SAVEPOINT agent",
        "RELEASE SAVEPOINT agent",
        "ROLLBACK TO SAVEPOINT agent",
        "BEGIN",
        "SELECT 1; ROLLBACK",
    ] {
        let outcome = bench
            .executor
            .dispatch(
                agent(),
                execute(&bench, bench.session, text, false),
                &CancelToken::new(),
            )
            .await
            .expect("a decision");
        assert!(
            matches!(outcome, Outcome::Denied { .. }),
            "`{text}` must be refused to an agent"
        );
    }

    // The human's write is still pending in the human's transaction: nothing
    // was rolled back, and nothing was committed — a second session does not
    // see it yet.
    assert_eq!(rows(&bench, bench.session).await, 1);
    let other = connect(&bench.executor, bench.connection).await;
    assert_eq!(rows(&bench, other).await, 0);
}

#[tokio::test]
async fn the_human_still_controls_the_transaction() {
    let bench = bench().await;
    human(&bench, bench.session, "CREATE TABLE shared (id INTEGER)").await;
    human(&bench, bench.session, "BEGIN").await;
    human(&bench, bench.session, "INSERT INTO shared VALUES (1)").await;
    human(&bench, bench.session, "SAVEPOINT before_second").await;
    human(&bench, bench.session, "INSERT INTO shared VALUES (2)").await;
    human(&bench, bench.session, "ROLLBACK TO SAVEPOINT before_second").await;
    human(&bench, bench.session, "RELEASE SAVEPOINT before_second").await;
    human(&bench, bench.session, "COMMIT").await;

    let other = connect(&bench.executor, bench.connection).await;
    assert_eq!(
        rows(&bench, other).await,
        1,
        "the first insert only, committed"
    );

    human(&bench, bench.session, "BEGIN").await;
    human(&bench, bench.session, "INSERT INTO shared VALUES (3)").await;
    human(&bench, bench.session, "ROLLBACK").await;
    assert_eq!(
        rows(&bench, other).await,
        1,
        "the rolled back insert is gone"
    );
}
