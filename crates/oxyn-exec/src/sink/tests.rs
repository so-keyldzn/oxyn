use std::time::Duration;

use futures::executor::block_on;
use oxyn_core::{
    ConnectionConfig, ConnectionId, DefaultPolicy, DriverId, Environment, ExecLimits, ExecRequest,
    PolicyGate, QueryLanguage, SessionId, SqlDialect, StatementIntent,
};
use oxyn_store::Store;

use super::*;

fn harness(connection: &ConnectionConfig) -> Arc<Executor> {
    let store = Arc::new(Store::open_in_memory().expect("in-memory local state"));
    let setup = store.workspaces().create("tests").expect("workspace");
    store
        .connections()
        .save(setup.id, connection)
        .expect("connection");

    let policy = Arc::new(DefaultPolicy::new());
    policy.register(connection);
    let policy: Arc<dyn PolicyGate> = policy;

    let exec = Executor::builder(store, policy)
        .with_workspace(setup.id)
        .build();
    exec.register_connection(connection);
    Arc::new(exec)
}

fn execution(conn: ConnectionId, text: &str) -> Command {
    Command::Execute {
        connection: conn,
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text).with_limits(
                ExecLimits::default()
                    .writable()
                    .with_timeout(None::<Duration>),
            ),
        ),
    }
}

#[test]
fn an_agent_cannot_impersonate_the_user() {
    let connection = ConnectionConfig::new("base client", DriverId::postgres())
        .with_environment(Environment::Production);
    let sink = ExecutorSink::for_agent(harness(&connection), AgentId::new(), AgentSessionId::new());

    let report = block_on(sink.dispatch(
        Actor::Human,
        execution(connection.id, "DELETE FROM clients"),
        &CancelToken::new(),
    ));

    assert!(
        matches!(report, DispatchReport::Denied { .. }),
        "{report:?}"
    );
    assert!(!report.took_effect());
    // And nothing reached the executor: the log is empty.
    assert_eq!(sink.executor().store().journal().count().expect("count"), 0);
}

#[test]
fn the_sink_goes_through_the_same_dispatch_as_the_interface() {
    // The proof: an agent asking for a DELETE in production receives a
    // denial written by the `PolicyGate`, and the log keeps its trace —
    // exactly as if the interface had submitted the command.
    let connection = ConnectionConfig::new("base client", DriverId::postgres())
        .with_environment(Environment::Production);
    let exec = harness(&connection);
    let agent = AgentId::new();
    let session = AgentSessionId::new();
    let sink = ExecutorSink::for_agent(Arc::clone(&exec), agent, session);

    let report = block_on(sink.dispatch(
        Actor::agent(agent, session),
        execution(connection.id, "DELETE FROM clients"),
        &CancelToken::new(),
    ));

    let DispatchReport::Denied { reason, .. } = &report else {
        panic!("{report:?}");
    };
    assert!(reason.contains("production"), "{reason}");
    assert_eq!(exec.store().journal().count().expect("count"), 1);

    let trace = exec
        .store()
        .journal()
        .for_agent(agent, 1)
        .expect("read back")
        .pop()
        .expect("an entry attributed to the agent");
    assert_eq!(trace.record.intent, StatementIntent::Write);
}

#[test]
fn an_agent_write_outside_production_is_returned_as_pending() {
    let connection = ConnectionConfig::new("atelier", DriverId::postgres())
        .with_environment(Environment::Development);
    let agent = AgentId::new();
    let session = AgentSessionId::new();
    let sink = ExecutorSink::for_agent(harness(&connection), agent, session);

    let report = block_on(sink.dispatch(
        Actor::agent(agent, session),
        execution(
            connection.id,
            "UPDATE clients SET active = true WHERE id = 1",
        ),
        &CancelToken::new(),
    ));

    let DispatchReport::AwaitingApproval { command, .. } = report else {
        panic!("{report:?}");
    };
    // The returned identifier is the one under which the approval will be given.
    assert!(
        sink.executor()
            .approvals()
            .pending()
            .iter()
            .any(|p| p.id == command)
    );
}

#[test]
fn a_failure_is_an_answer_to_the_model_not_an_incident() {
    // No session is open: execution fails after the gate.
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let agent = AgentId::new();
    let session = AgentSessionId::new();
    let sink = ExecutorSink::for_agent(harness(&connection), agent, session);

    let report = block_on(sink.dispatch(
        Actor::agent(agent, session),
        execution(connection.id, "SELECT 1"),
        &CancelToken::new(),
    ));

    let DispatchReport::Failed { class, .. } = report else {
        panic!("{report:?}");
    };
    assert!(
        class.is_retryable(),
        "a closed session is reopened: {class:?}"
    );
}

#[test]
fn an_open_sink_passes_the_actor_as_is() {
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let sink = ExecutorSink::new(harness(&connection));

    let report = block_on(sink.dispatch(
        Actor::Human,
        execution(connection.id, "SELECT 1"),
        &CancelToken::new(),
    ));
    // The gate allowed it (read, local connection): the failure comes from
    // the absence of a session, not from a denial.
    assert!(
        matches!(report, DispatchReport::Failed { .. }),
        "{report:?}"
    );
}
