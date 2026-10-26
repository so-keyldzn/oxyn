use super::*;

use futures::executor::block_on;
use oxyn_core::{
    AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ErrorClass, ExecLimits,
    MutationRisk, QueryLanguage, SqlDialect, StatementIntent,
};
use oxyn_store::{ActorKind, HistoryStatus, PolicyOutcome};

/// A complete bench: in-memory local state, default policy, no open
/// session. No driver is registered — the tests that follow are about what
/// happens **before** a driver is reached, and that is precisely what
/// matters.
struct Harness {
    executor: Executor,
    policy: Arc<DefaultPolicy>,
    store: Arc<Store>,
}

impl Harness {
    fn new(connection: &ConnectionConfig) -> Self {
        let store = Arc::new(Store::open_in_memory().expect("in-memory local state"));
        let setup = store
            .workspaces()
            .create("tests")
            .expect("workspace creation");
        store
            .connections()
            .save(setup.id, connection)
            .expect("saving the connection");

        let policy = Arc::new(DefaultPolicy::new());
        policy.register(connection);

        // `policy.clone()` and not `Arc::clone(&policy)`: the function
        // form resolves `T` from the expected type — hence `dyn PolicyGate` —
        // and demands a `&Arc<dyn PolicyGate>` before any coercion. In method
        // syntax, `T` comes from the receiver, and the obtained
        // `Arc<DefaultPolicy>` coerces at assignment.
        let gate: Arc<dyn PolicyGate> = policy.clone();
        let executor = Executor::builder(Arc::clone(&store), gate)
            .with_workspace(setup.id)
            .build();
        executor.register_connection(connection);

        Self {
            executor,
            policy,
            store,
        }
    }
}

fn agent() -> Actor {
    Actor::agent(AgentId::new(), AgentSessionId::new())
}

fn execution(conn: ConnectionId, text: &str, intent: StatementIntent) -> Command {
    Command::Execute {
        connection: conn,
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text)
                .with_intent(intent)
                .with_limits(
                    ExecLimits::default()
                        .writable()
                        .with_timeout(None::<Duration>),
                ),
        ),
    }
}

// ── The two tests that protect the product's promise ────────────────────

/// **An agent deletes nothing in production.**
///
/// The agent declares a read; the text says otherwise. Reclassification
/// takes place *before* the gate, so the gate sees an unbounded `DELETE` on
/// a production connection, and refuses — a refusal, not a stronger
/// confirmation (I-02, ADR-0004).
///
/// The proof that reclassification did take place is not in the refusal:
/// it is in the **log**, which recorded the `write` intent, not the one the
/// agent had declared.
#[test]
fn an_agent_cannot_delete_on_a_production_connection() {
    let connection = ConnectionConfig::new("base client", DriverId::postgres())
        .with_environment(Environment::Production);
    let bench = Harness::new(&connection);
    let mut events = bench.executor.subscribe();

    // The agent declares itself read-only.
    let cmd = execution(
        connection.id,
        "DELETE FROM clients WHERE 1=1",
        StatementIntent::Read,
    );

    let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
        .expect("a denial is not a failure");

    let Outcome::Denied { command, reason } = outcome else {
        panic!("an agent must be refused on a production connection: {outcome:?}");
    };
    assert!(
        reason.contains("production"),
        "the reason must name production: {reason}"
    );

    // Nothing awaits approval: it is a refusal, not a confirmation.
    assert!(
        bench.executor.approvals().is_empty(),
        "a denial puts nothing on hold for approval"
    );

    // The log saw the **real** intent, not the declared one.
    let trace = bench
        .store
        .journal()
        .recent(1)
        .expect("reading the log back")
        .pop()
        .expect("one entry");
    assert_eq!(trace.record.command_id, Some(command));
    assert_eq!(trace.record.actor_kind, ActorKind::Agent);
    assert_eq!(trace.record.decision, PolicyOutcome::Denied);
    assert_eq!(
        trace.record.intent,
        StatementIntent::Write,
        "the intent declared by the agent must have been overwritten"
    );
    assert_eq!(trace.record.risk, MutationRisk::UnboundedDelete);
    assert_eq!(
        trace.record.statement.as_deref(),
        Some("DELETE FROM clients WHERE 1=1")
    );

    // And the interface learns it through the channel, not by polling a state.
    let received = events.try_recv().expect("an event was emitted");
    assert_eq!(received.command, command);
    assert!(
        matches!(received.event, Event::Failed { .. }),
        "{received:?}"
    );
}

/// **A denied command leaves a trace.**
///
/// A log that records only what worked says nothing about what was
/// attempted — and that is exactly what an audit looks for.
#[test]
fn the_log_holds_an_entry_even_when_the_command_is_denied() {
    // Read-only connection: the refusal applies to a human too.
    let connection = ConnectionConfig::new("replica", DriverId::postgres())
        .with_environment(Environment::Local)
        .read_only();
    let bench = Harness::new(&connection);

    assert_eq!(
        bench.store.journal().count().expect("count"),
        0,
        "the log starts empty"
    );

    let command = execution(
        connection.id,
        "DROP TABLE clients",
        StatementIntent::Unknown,
    );
    let outcome = block_on(
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .expect("a denial is not a failure");
    assert!(outcome.is_denied(), "{outcome:?}");
    assert!(!outcome.took_effect());

    assert_eq!(
        bench.store.journal().count().expect("count"),
        1,
        "a denied command is logged like the others"
    );
    let trace = bench
        .store
        .journal()
        .recent(1)
        .expect("read back")
        .pop()
        .expect("one entry");
    assert_eq!(trace.record.decision, PolicyOutcome::Denied);
    assert_eq!(trace.record.actor_kind, ActorKind::Human);
    assert_eq!(trace.record.command_kind, "Execute");
    assert_eq!(
        trace.record.statement.as_deref(),
        Some("DROP TABLE clients")
    );
    assert!(
        trace
            .record
            .decision_reason
            .as_deref()
            .is_some_and(|pattern| pattern.contains("read-only")),
        "{:?}",
        trace.record.decision_reason
    );

    // And the trace survives what the user can erase.
    bench.store.history().clear().expect("purging the history");
    assert_eq!(bench.store.journal().count().expect("count"), 1);
}

// ── The rest of the sequence ────────────────────────────────────────────

#[test]
fn an_agent_write_outside_production_waits_for_approval_and_executes_nothing() {
    let conn = ConnectionConfig::new("atelier", DriverId::postgres())
        .with_environment(Environment::Development);
    let bench = Harness::new(&conn);

    let cmd = execution(
        conn.id,
        "UPDATE clients SET active = true WHERE id = 1",
        StatementIntent::Read,
    );
    let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
        .expect("an approval request is not a failure");

    let Outcome::NeedsApproval {
        command, preview, ..
    } = outcome
    else {
        panic!("an agent write must request an approval: {outcome:?}");
    };

    // The preview names the connection, never its identifier.
    let preview = preview.expect("a preview");
    assert_eq!(preview.connection, "atelier");
    assert!(!preview.connection.contains(&conn.id.to_string()));

    // Nothing was executed: no session was even looked for.
    assert_eq!(bench.executor.approvals().len(), 1);
    assert!(bench.executor.running().is_empty());

    // And the trace of the request is already in the log.
    let trace = bench
        .store
        .journal()
        .recent(1)
        .expect("read back")
        .pop()
        .expect("one entry");
    assert_eq!(trace.record.decision, PolicyOutcome::ApprovalRequired);
    assert_eq!(trace.record.command_id, Some(command));
}

#[rstest::rstest]
#[case::agent_production(true, Environment::Production)]
#[case::agent_development(true, Environment::Development)]
#[case::human_production(false, Environment::Production)]
fn copy_server_side_effects_reach_the_policy_as_writes(
    #[case] is_agent: bool,
    #[case] environment: Environment,
) {
    for sql in ["COPY t TO PROGRAM 'x'", "COPY t TO '/tmp/x.csv'"] {
        let connection = ConnectionConfig::new("customer database", DriverId::postgres())
            .with_environment(environment);
        let bench = Harness::new(&connection);
        let actor = if is_agent { agent() } else { Actor::Human };
        let command = execution(connection.id, sql, StatementIntent::Read);
        let outcome = block_on(bench.executor.dispatch(actor, command, &CancelToken::new()))
            .expect("the gate decides before looking for a session");
        if is_agent && environment.is_production() {
            assert!(matches!(outcome, Outcome::Denied { .. }), "{outcome:?}");
            assert!(bench.executor.approvals().is_empty());
        } else {
            let Outcome::NeedsApproval {
                preview, reason, ..
            } = outcome
            else {
                panic!("server-side COPY requires approval: {outcome:?}");
            };
            assert_eq!(
                preview.expect("a named connection preview").connection,
                connection.name
            );
            assert!(reason.contains("database server"), "{reason}");
            assert_eq!(bench.executor.approvals().len(), 1);
        }
        assert!(bench.executor.running().is_empty());
    }
}

/// Issue #246: `ANALYSE` is PostgreSQL's other spelling of `ANALYZE`. An
/// agent announcing a read is held for approval, and the command set aside
/// carries the true intent, which the history will record.
#[test]
fn an_agents_explain_analyse_of_a_delete_needs_approval() {
    let connection = ConnectionConfig::new("customer database", DriverId::postgres())
        .with_environment(Environment::Development);
    let bench = Harness::new(&connection);
    let command = execution(
        connection.id,
        "EXPLAIN (ANALYSE) DELETE FROM orders",
        StatementIntent::Read,
    );

    let outcome = block_on(
        bench
            .executor
            .dispatch(agent(), command, &CancelToken::new()),
    )
    .expect("the gate decides before looking for a session");

    let Outcome::NeedsApproval { reason, .. } = outcome else {
        panic!("EXPLAIN (ANALYSE) DELETE runs the DELETE: {outcome:?}");
    };
    assert!(reason.contains("DELETE without a WHERE clause"), "{reason}");
    let pending = bench.executor.approvals().pending();
    let [held] = pending.as_slice() else {
        panic!("exactly one command waits: {pending:?}");
    };
    assert_eq!(held.command.intent(), StatementIntent::Write);
    assert_eq!(held.command.mutation_risk(), MutationRisk::UnboundedDelete);
    assert!(bench.executor.running().is_empty());
}

#[test]
fn an_unknown_or_stale_approval_executes_nothing() {
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&connection);

    let outcome = block_on(bench.executor.approve(
        "nicolas",
        CommandId::new(),
        &CancelToken::new(),
    ));
    let error = outcome.expect_err("a moot approval is refused");
    assert!(matches!(error, OxynError::PolicyDenied { .. }), "{error:?}");
    assert_eq!(
        bench.store.journal().count().expect("count"),
        0,
        "nothing went through the gate: there is nothing to log"
    );
}

#[test]
fn an_approval_does_not_survive_a_stricter_marking() {
    // The connection is open when the approval is requested, marked
    // read-only when it is given. The late refusal wins over the approval.
    let connection = ConnectionConfig::new("atelier", DriverId::postgres())
        .with_environment(Environment::Development);
    let bench = Harness::new(&connection);

    let cmd = execution(
        connection.id,
        "UPDATE clients SET active = true WHERE id = 1",
        StatementIntent::Write,
    );
    let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
        .expect("approval request");
    let Outcome::NeedsApproval { command, .. } = outcome else {
        panic!("{outcome:?}");
    };

    // Meanwhile, the user marks the connection read-only.
    let strict = connection.clone().read_only();
    bench.policy.register(&strict);
    bench.executor.register_connection(&strict);

    let outcome = block_on(
        bench
            .executor
            .approve("nicolas", command, &CancelToken::new()),
    )
    .expect("a late denial is not a failure");
    assert!(outcome.is_denied(), "{outcome:?}");
}

#[test]
fn a_write_under_read_only_limits_is_refused() {
    // `ExecLimits::default()` is read-only: writing is always an explicit
    // request. It is the last barrier before the driver.
    let conn =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&conn);

    let command = Command::Execute {
        connection: conn.id,
        session: SessionId::new(),
        request: Box::new(ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            "INSERT INTO clients (name) VALUES ('x')",
        )),
    };

    let error = block_on(
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .expect_err("the inconsistency is settled on the cautious side");
    assert!(matches!(error, OxynError::PolicyDenied { .. }), "{error:?}");

    // Two entries: the policy decision, then the outcome.
    assert_eq!(bench.store.journal().count().expect("count"), 2);
}

#[test]
fn a_local_read_asks_nothing_and_fails_for_lack_of_a_session() {
    // The gate allows; execution fails because no session is open. What
    // this test checks is that the failure comes **after** the gate, and
    // that both log entries are written.
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&connection);

    let command = execution(connection.id, "SELECT 1", StatementIntent::Unknown);
    let err = block_on(
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .expect_err("no session is open");
    assert!(matches!(err, OxynError::Connection(_)), "{err:?}");

    let entries = bench.store.journal().recent(2).expect("read back");
    assert_eq!(entries.len(), 2, "decision before, outcome after");
    assert!(
        entries
            .iter()
            .all(|e| e.record.decision == PolicyOutcome::Allowed),
        "the read was indeed authorized"
    );
    assert!(
        entries.iter().any(|e| e.record.error.is_some()),
        "the execution failure is in the log"
    );
}

#[test]
fn a_connection_unknown_to_the_executor_counts_as_production() {
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&connection);
    bench.executor.forget_connection(connection.id);

    let command = execution(connection.id, "SELECT 1", StatementIntent::Read);
    assert_eq!(
        bench.executor.environment_of(&command),
        Environment::Production
    );
}

#[test]
fn the_executor_debug_shows_no_content() {
    let connection = ConnectionConfig::new("base client", DriverId::postgres())
        .with_param("host", "internal.example");
    let bench = Harness::new(&connection);
    let rendered = format!("{:?}", bench.executor);
    assert!(!rendered.contains("internal.example"), "{rendered}");
    assert!(rendered.contains("DefaultPolicy"), "{rendered}");
}

#[test]
fn a_cancellation_is_always_allowed() {
    // Refusing a cancellation protects nothing and leaves a query running.
    let conn = ConnectionConfig::new("base client", DriverId::postgres())
        .with_environment(Environment::Production);
    let bench = Harness::new(&conn);

    let outcome = block_on(bench.executor.dispatch(
        agent(),
        Command::Cancel {
            connection: conn.id,
            statement: StatementHandle::new(),
        },
        &CancelToken::new(),
    ))
    .expect("cancelling is not refused");

    let Outcome::Cancelled { report } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(!report.was_running, "the target execution does not exist");
}

// ── The query history ───────────────────────────────────────────────────

/// A session that returns a single two-row batch, without a server.
///
/// It only serves to prove the history's nominal path: without an
/// execution that succeeds, the `succeeded` row exists in no test, and it
/// is exactly the one that was missing until now.
struct StubSession;

#[async_trait::async_trait]
impl oxyn_driver::Session for StubSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL | Capabilities::TABLES
    }
    async fn execute(&self, _: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        Ok(Box::new(FakeCursor {
            handle: StatementHandle::new(),
            rendered: false,
            stats: ExecStats::default(),
        }))
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
        unreachable!("history tests introspect nothing")
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

struct RecordingSession {
    executed: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl oxyn_driver::Session for RecordingSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL | Capabilities::TABLES
    }
    async fn execute(&self, request: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.executed.lock().push(request.text);
        Ok(Box::new(FakeCursor {
            handle: StatementHandle::new(),
            rendered: false,
            stats: ExecStats::default(),
        }))
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        Ok(())
    }
    fn catalog(&self) -> &dyn oxyn_catalog::CatalogProvider {
        unreachable!("password-redaction tests introspect nothing")
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

struct FakeCursor {
    handle: StatementHandle,
    rendered: bool,
    stats: ExecStats,
}

fn fake_schema() -> arrow::datatypes::SchemaRef {
    Arc::new(arrow::datatypes::Schema::new(vec![
        arrow::datatypes::Field::new("id", arrow::datatypes::DataType::Int32, false),
    ]))
}

#[async_trait::async_trait]
impl Cursor for FakeCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> arrow::datatypes::SchemaRef {
        fake_schema()
    }
    async fn next_batch(&mut self) -> Result<Option<arrow::record_batch::RecordBatch>> {
        if self.rendered {
            return Ok(None);
        }
        self.rendered = true;
        let batch = arrow::record_batch::RecordBatch::try_new(
            fake_schema(),
            vec![Arc::new(arrow::array::Int32Array::from(vec![1, 2]))],
        )
        .expect("the column matches the schema built just above");
        self.stats.record_batch(2, 0);
        Ok(Some(batch))
    }
    fn stats(&self) -> ExecStats {
        self.stats
    }
}

/// **An execution that succeeds leaves a complete trace.**
///
/// Duration and rows included: a history that does not say how many rows a
/// query returned does not answer the question it is asked.
#[test]
fn a_successful_execution_is_recorded_with_its_duration_and_rows() {
    let conn =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&conn);
    let session = bench
        .executor
        .sessions
        .insert(SessionSlot::new(conn.id, Box::new(StubSession)));

    let command = Command::Execute {
        connection: conn.id,
        session: session.id(),
        request: Box::new(
            ExecRequest::new(
                QueryLanguage::Sql(SqlDialect::Sqlite),
                "SELECT id FROM clients",
            )
            .with_intent(StatementIntent::Read)
            // No timeout: `block_on` has no tokio clock, and the default
            // timeout would require one.
            .with_limits(ExecLimits::default().with_timeout(None::<Duration>)),
        ),
    };
    block_on(
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .expect("the execution succeeds");

    let entry = bench
        .store
        .history()
        .recent(10)
        .expect("reading the history back")
        .pop()
        .expect("an execution leaves an entry");
    assert_eq!(entry.record.status, HistoryStatus::Succeeded);
    assert_eq!(entry.record.rows, Some(2));
    assert!(entry.record.duration.is_some(), "a duration is measured");
    assert_eq!(entry.record.statement, "SELECT id FROM clients");
    // The name is copied to survive the connection's deletion.
    assert_eq!(entry.record.connection_name.as_deref(), Some("atelier"));
    assert!(entry.record.error.is_none());
}

#[test]
fn password_literals_execute_verbatim_but_are_redacted_from_disk() {
    let cases = [
        (
            DriverId::postgres(),
            SqlDialect::Postgres,
            "ALTER ROLE app PASSWORD 'witness-secret'",
        ),
        (
            DriverId::mysql(),
            SqlDialect::MySql,
            "CREATE USER u IDENTIFIED BY 'witness-secret'",
        ),
    ];

    for (driver, dialect, sql) in cases {
        let connection =
            ConnectionConfig::new("credential test", driver).with_environment(Environment::Local);
        let bench = Harness::new(&connection);
        let executed = Arc::new(Mutex::new(Vec::new()));
        let session = bench.executor.sessions.insert(SessionSlot::new(
            connection.id,
            Box::new(RecordingSession {
                executed: Arc::clone(&executed),
            }),
        ));
        let command = Command::Execute {
            connection: connection.id,
            session: session.id(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::Sql(dialect), sql).with_limits(
                    ExecLimits::default()
                        .writable()
                        .with_timeout(None::<Duration>),
                ),
            ),
        };

        block_on(
            bench
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new()),
        )
        .expect("the server accepts the statement");

        assert_eq!(executed.lock().as_slice(), [sql]);
        let history = bench.store.history().recent(1).expect("history");
        let journal = bench.store.journal().recent(2).expect("journal");
        assert_eq!(history.len(), 1);
        assert_eq!(
            journal.len(),
            2,
            "the decision and outcome are both retained"
        );
        assert_eq!(history[0].record.status, HistoryStatus::Succeeded);
        assert!(!history[0].record.statement.contains("witness-secret"));
        assert!(
            journal.iter().all(|entry| {
                entry
                    .record
                    .statement
                    .as_deref()
                    .is_none_or(|statement| !statement.contains("witness-secret"))
            }),
            "the audit journal must not retain the password literal"
        );
    }
}

/// The `HistoryRecorded` received so far, without waiting.
fn announced_registrations(
    received_events: &mut tokio::sync::broadcast::Receiver<crate::events::ExecEvent>,
) -> Vec<Option<ConnectionId>> {
    std::iter::from_fn(|| received_events.try_recv().ok())
        .filter(|received| received.event == Event::HistoryRecorded)
        .map(|received| received.connection)
        .collect()
}

/// **A successful read announces its record, whether it comes from the
/// human or the agent** (ADR-0022): an open library has no other way to
/// learn that a row was added. The announcement follows the writing of the
/// outcome: read again at that moment, the row is no longer "in
/// progress".
#[test]
fn a_successful_read_announces_its_history_record() {
    let conn =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&conn);
    let session = bench
        .executor
        .sessions
        .insert(SessionSlot::new(conn.id, Box::new(StubSession)));
    let mut events = bench.executor.subscribe();

    for actor in [Actor::Human, agent()] {
        let command = Command::Execute {
            connection: conn.id,
            session: session.id(),
            request: Box::new(
                ExecRequest::new(
                    QueryLanguage::Sql(SqlDialect::Sqlite),
                    "SELECT id FROM clients",
                )
                .with_intent(StatementIntent::Read)
                .with_limits(ExecLimits::default().with_timeout(None::<Duration>)),
            ),
        };
        block_on(bench.executor.dispatch(actor, command, &CancelToken::new()))
            .expect("the execution succeeds");

        assert_eq!(
            announced_registrations(&mut events),
            vec![Some(conn.id)],
            "one announcement per successful execution, attached to its connection"
        );
        let entry = bench
            .store
            .history()
            .recent(1)
            .expect("read back")
            .pop()
            .expect("an execution leaves an entry");
        assert_eq!(entry.record.status, HistoryStatus::Succeeded);
    }
}

/// **A failure announces nothing**: reading the library again on an error
/// teaches nothing the displayed error does not already say.
#[test]
fn a_failure_announces_no_record() {
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&connection);
    let mut events = bench.executor.subscribe();

    let command = execution(connection.id, "SELECT 1", StatementIntent::Read);
    block_on(
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .expect_err("no session under this identifier");

    assert!(announced_registrations(&mut events).is_empty());
}

/// **An execution that fails leaves the error, not a silence.**
///
/// With its **family**, which is the data the right to replay depends on:
/// it is what will forbid offering "rerun" on an `INSERT` whose server-side
/// effect is unknown (I-13).
#[test]
fn a_failure_is_recorded_with_its_error_and_family() {
    let connection =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&connection);

    // No session is open: execution fails before the driver.
    let command = execution(connection.id, "SELECT 1", StatementIntent::Read);
    block_on(
        bench
            .executor
            .dispatch(Actor::Human, command, &CancelToken::new()),
    )
    .expect_err("no session under this identifier");

    let entry = bench
        .store
        .history()
        .recent(10)
        .expect("read back")
        .pop()
        .expect("a failure leaves an entry");
    assert_eq!(entry.record.status, HistoryStatus::Failed);
    assert!(entry.record.error.is_some(), "the error is kept");
    assert!(entry.record.duration.is_some(), "a failure has a duration");
    // The family is kept as data, never inferred from the message.
    assert_eq!(
        entry.record.error_class,
        Some(ErrorClass::Transient),
        "a missing session is reopened: it is transient"
    );
}

/// **A denial appears in the history, with its reason.**
///
/// A history that shows only what worked leaves the user looking for a
/// query they did run.
#[test]
fn a_denial_is_recorded_with_its_reason_and_no_bound_value() {
    let conn = ConnectionConfig::new("base client", DriverId::postgres())
        .with_environment(Environment::Production);
    let bench = Harness::new(&conn);

    // The secret travels as a bound value, never in the text (I-03).
    let command = Command::Execute {
        connection: conn.id,
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(
                QueryLanguage::Sql(SqlDialect::Postgres),
                "DELETE FROM clients WHERE token = $1",
            )
            .with_intent(StatementIntent::Read)
            .with_params(vec![oxyn_core::ScalarValue::Text(
                "hunter2-the-secret".to_owned(),
            )])
            .with_limits(ExecLimits::default().writable()),
        ),
    };
    let outcome = block_on(
        bench
            .executor
            .dispatch(agent(), command, &CancelToken::new()),
    )
    .expect("a denial is not a failure");
    assert!(outcome.is_denied(), "{outcome:?}");

    let entries = bench.store.history().recent(10).expect("read back");
    assert_eq!(entries.len(), 1, "one denial, one row — not two");
    let record = &entries[0].record;
    assert_eq!(record.status, HistoryStatus::Denied);
    assert!(
        record
            .error
            .as_deref()
            .is_some_and(|pattern| pattern.contains("production")),
        "{:?}",
        record.error
    );
    // The recorded intent is the one the text carries, not the one the
    // agent declared.
    assert_eq!(record.intent, StatementIntent::Write);
    assert!(
        !format!("{record:?}").contains("hunter2"),
        "no bound value reaches the history (I-03)"
    );
}

// ── A typed context change (issue #223) ─────────────────────────────────

/// A typed `USE other`, as a caller that wants it to pass would send it: a
/// declared read, with `context_change` left down. `execution` builds a
/// PostgreSQL request, and `USE` is a MySQL statement.
fn typed_use(conn: ConnectionId) -> Command {
    typed_context_change(conn, "USE other")
}

/// A typed context change in `text`, declared as a read.
fn typed_context_change(conn: ConnectionId, text: &str) -> Command {
    Command::Execute {
        connection: conn,
        session: SessionId::new(),
        request: Box::new(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::MySql), text)
                .with_intent(StatementIntent::Read)
                .with_limits(
                    ExecLimits::default()
                        .writable()
                        .with_timeout(None::<Duration>),
                ),
        ),
    }
}

/// **An agent does not move the session's database by typing it.**
///
/// `USE other` was classified a read: an agent's `Execute` of it passed as
/// one, and neither the refusal of `SetSessionContext` (ADR-0019), which only
/// matches that variant, nor any confirmation applied. The agent declares a
/// read with no context change; the text says otherwise.
/// Reclassification happens before the gate, so the gate refuses it in every
/// environment — development included, where an `Unknown` statement would
/// only ask for an approval, and an approval ends up being clicked (I-02,
/// I-07).
///
/// On production too, the reason is the context one and not the production
/// one: the user is sent to the selector either way. No driver is
/// registered, so the refusal is proven to precede any dispatch.
///
/// The server runs the body of an executable comment, so the same refusal
/// holds when the `USE` hides in one: the batch used to read as empty,
/// `Unknown` with no flag, and development only asked for an approval.
#[test]
fn an_agent_cannot_type_a_context_change_on_mysql() {
    for (env, text) in [Environment::Production, Environment::Development]
        .into_iter()
        .flat_map(|env| {
            ["USE other", "/*!50000 USE other */"]
                .into_iter()
                .map(move |text| (env, text))
        })
    {
        let conn = ConnectionConfig::new("shop", DriverId::mysql()).with_environment(env);
        let bench = Harness::new(&conn);
        let mut events = bench.executor.subscribe();

        let cmd = typed_context_change(conn.id, text);
        // Only the text can raise the flag by the time the gate decides.
        assert!(
            !cmd.changes_context(),
            "{env}: the caller declared no context change"
        );

        let outcome = block_on(bench.executor.dispatch(agent(), cmd, &CancelToken::new()))
            .expect("a denial is not a failure");
        let Outcome::Denied { command, reason } = outcome else {
            panic!("an agent's typed USE must be refused on {env}: {outcome:?}");
        };
        assert!(
            reason.contains("context selector"),
            "{env}: the reason must send the user to the selector: {reason}"
        );
        assert!(
            !reason.contains("production") && !reason.contains("read-only"),
            "{env}: the production or read-only reason won over the context one: {reason}"
        );

        // A refusal, not a confirmation; and nothing started.
        assert!(
            bench.executor.approvals().is_empty(),
            "{env}: a denial puts nothing on hold for approval"
        );
        assert!(bench.executor.running().is_empty(), "{env}");

        // The log holds it, with the intent the text carries and not the
        // one the agent declared.
        assert_eq!(
            bench.store.journal().count().expect("count"),
            1,
            "{env}: a denied command is logged like the others"
        );
        let trace = bench
            .store
            .journal()
            .recent(1)
            .expect("reading the log back")
            .pop()
            .expect("one entry");
        assert_eq!(trace.record.command_id, Some(command), "{env}");
        assert_eq!(trace.record.actor_kind, ActorKind::Agent, "{env}");
        assert_eq!(trace.record.connection, Some(conn.id), "{env}");
        assert_eq!(trace.record.command_kind, "Execute", "{env}");
        assert_eq!(trace.record.decision, PolicyOutcome::Denied, "{env}");
        assert_eq!(
            trace.record.intent,
            StatementIntent::Unknown,
            "{env}: the declared read must have been overwritten"
        );
        assert_eq!(trace.record.statement.as_deref(), Some(text), "{env}");
        assert!(
            trace
                .record
                .decision_reason
                .as_deref()
                .is_some_and(|recorded| recorded.contains("context selector")),
            "{env}: the journal keeps the reason: {:?}",
            trace.record.decision_reason
        );

        // The history shows the denial too, once, with the same reason.
        let entries = bench.store.history().recent(10).expect("read back");
        assert_eq!(entries.len(), 1, "{env}: one denial, one row — not two");
        let record = &entries[0].record;
        assert_eq!(record.status, HistoryStatus::Denied, "{env}");
        assert_eq!(record.statement, text, "{env}");
        assert!(
            record
                .error
                .as_deref()
                .is_some_and(|recorded| recorded.contains("context selector")),
            "{env}: {:?}",
            record.error
        );

        // And the interface learns it through the channel.
        let received = events.try_recv().expect("an event was emitted");
        assert_eq!(received.command, command, "{env}");
        assert!(
            matches!(received.event, Event::Failed { .. }),
            "{env}: {received:?}"
        );
    }
}

/// **The refusal is the agent's.**
///
/// The user types `USE other` in their own console. It is `Unknown`, so a
/// write for the gate: nothing to confirm off production, and on production
/// the confirmation that names the connection (I-02). Without this
/// counterpart, a refusal applied to everyone would pass the test above.
#[test]
fn a_user_typed_use_is_not_refused_by_the_agent_rule() {
    // Development: the gate lets it through. This bench has no session to
    // run it on, so what comes back is that failure — the proof is that the
    // journal says `Allowed` and no denial exists anywhere.
    let development =
        ConnectionConfig::new("shop", DriverId::mysql()).with_environment(Environment::Development);
    let bench = Harness::new(&development);
    let error = block_on(bench.executor.dispatch(
        Actor::Human,
        typed_use(development.id),
        &CancelToken::new(),
    ))
    .expect_err("the gate lets a user's USE through, and this bench has no session to run it");
    assert!(
        !matches!(error, OxynError::PolicyDenied { .. }),
        "the agent's refusal reached the user: {error}"
    );
    let trace = bench
        .store
        .journal()
        .recent(1)
        .expect("read back")
        .pop()
        .expect("one entry");
    assert_eq!(trace.record.actor_kind, ActorKind::Human);
    assert_eq!(trace.record.decision, PolicyOutcome::Allowed);
    assert!(
        trace
            .record
            .decision_reason
            .as_deref()
            .is_none_or(|recorded| !recorded.contains("context selector")),
        "{:?}",
        trace.record.decision_reason
    );
    assert!(
        bench
            .store
            .history()
            .recent(10)
            .expect("read back")
            .iter()
            .all(|entry| entry.record.status != HistoryStatus::Denied),
        "no denial is recorded for the user"
    );

    // Production: the same statement waits for the confirmation that names
    // the connection, and runs nothing until it comes.
    let production =
        ConnectionConfig::new("shop", DriverId::mysql()).with_environment(Environment::Production);
    let bench = Harness::new(&production);
    let outcome = block_on(bench.executor.dispatch(
        Actor::Human,
        typed_use(production.id),
        &CancelToken::new(),
    ))
    .expect("an approval request is not a failure");
    let Outcome::NeedsApproval {
        command,
        reason,
        preview,
    } = outcome
    else {
        panic!("a user's USE on production must ask for a confirmation: {outcome:?}");
    };
    assert!(
        reason.contains("shop"),
        "the confirmation must name the connection: {reason}"
    );
    assert_eq!(
        preview.expect("a preview").connection,
        "shop",
        "the preview names the connection, never its identifier"
    );
    assert_eq!(bench.executor.approvals().len(), 1);
    assert!(bench.executor.running().is_empty());
    let trace = bench
        .store
        .journal()
        .recent(1)
        .expect("read back")
        .pop()
        .expect("one entry");
    assert_eq!(trace.record.command_id, Some(command));
    assert_eq!(trace.record.decision, PolicyOutcome::ApprovalRequired);
    assert_eq!(
        trace.record.intent,
        StatementIntent::Unknown,
        "the user's USE is no longer a read: that is what asks for the confirmation"
    );
    assert_eq!(trace.record.statement.as_deref(), Some("USE other"));
}

/// **The history records only executions.**
///
/// A `Connect` or a `Cancel` among the queries would make the list
/// unreadable; they stay in the log, which records them all.
#[test]
fn a_command_that_is_not_an_execution_does_not_touch_history() {
    let conn =
        ConnectionConfig::new("atelier", DriverId::sqlite()).with_environment(Environment::Local);
    let bench = Harness::new(&conn);

    block_on(bench.executor.dispatch(
        Actor::Human,
        Command::Cancel {
            connection: conn.id,
            statement: StatementHandle::new(),
        },
        &CancelToken::new(),
    ))
    .expect("cancelling is not refused");

    assert_eq!(bench.store.history().count().expect("count"), 0);
    assert!(
        bench.store.journal().count().expect("count") > 0,
        "the log, for its part, records them all"
    );
}
