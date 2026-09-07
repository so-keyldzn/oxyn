//! Preview contracts: real in-memory SQLite and controllable driver boundaries.

use super::*;
use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use futures::{FutureExt, executor::block_on};
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ExecLimits, QueryLanguage,
    SqlDialect, StatementIntent,
};
use oxyn_driver::Session;
use oxyn_driver_sqlite::SqliteDriver;
use oxyn_store::{ActorKind, PolicyOutcome};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const HOSTILE: &str = "t\"; DROP TABLE audit; --";

fn actors() -> [Actor; 2] {
    [
        Actor::Human,
        Actor::agent(AgentId::new(), AgentSessionId::new()),
    ]
}

fn preview(connection: ConnectionId, session: SessionId, limit: u32) -> Command {
    Command::PreviewRelation {
        connection,
        session,
        catalog: None,
        namespace: Some("main".into()),
        relation: HOSTILE.into(),
        limit,
    }
}

fn executor(policy: Arc<dyn PolicyGate>, connection: &ConnectionConfig) -> Executor {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("preview tests")
        .expect("workspace")
        .id;
    store
        .connections()
        .save(workspace, connection)
        .expect("connection");
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .expect("SQLite registration");
    let executor = Executor::builder(store, policy)
        .with_drivers(Arc::new(drivers))
        .with_workspace(workspace)
        .build();
    executor.register_connection(connection);
    executor
}

async fn execute_sql(
    executor: &Executor,
    connection: ConnectionId,
    session: SessionId,
    text: String,
    writable: bool,
) -> Outcome {
    executor
        .dispatch(
            Actor::Human,
            Command::Execute {
                connection,
                session,
                request: Box::new(
                    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text).with_limits(
                        if writable {
                            ExecLimits::default().writable()
                        } else {
                            ExecLimits::default()
                        },
                    ),
                ),
            },
            &CancelToken::new(),
        )
        .await
        .expect("fixture SQL")
}

#[tokio::test]
async fn preview_sqlite_is_bounded_preserves_hostile_table_and_correlates_events_and_audit() {
    let connection = ConnectionConfig::new("memory", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let executor = executor(policy, &connection);
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
        panic!("expected connected session");
    };
    execute_sql(
        &executor,
        connection.id,
        session,
        "CREATE TABLE audit (id INTEGER)".into(),
        true,
    )
    .await;
    let qualified = CatalogPath::for_relation(None, Some("main"), HOSTILE)
        .expect("hostile name")
        .qualify_sql(SqlDialect::Sqlite);
    execute_sql(
        &executor,
        connection.id,
        session,
        format!("CREATE TABLE {qualified} (id INTEGER)"),
        true,
    )
    .await;
    execute_sql(
        &executor,
        connection.id,
        session,
        format!("INSERT INTO {qualified} VALUES (1), (2), (3), (4)"),
        true,
    )
    .await;
    for actor in actors() {
        let id = CommandId::new();
        let mut events = executor.subscribe();
        let outcome = executor
            .dispatch_as(
                id,
                actor,
                preview(connection.id, session, 2),
                &CancelToken::new(),
            )
            .await
            .expect("preview");
        let Outcome::Executed {
            result,
            buffer,
            stats,
            ..
        } = outcome
        else {
            panic!("expected results");
        };
        assert_eq!(buffer.row_count(), 2);
        assert_eq!(stats.rows, 2);
        let mut schema = false;
        let mut batch = false;
        let mut completed = false;
        while let Ok(event) = events.try_recv() {
            assert_eq!(event.command, id);
            assert_eq!(event.connection, Some(connection.id));
            match event.event {
                Event::SchemaReady { result: found } => {
                    assert_eq!(found, result);
                    schema = true;
                }
                Event::BatchReady { result: found, .. } => {
                    assert_eq!(found, result);
                    batch = true;
                }
                Event::Completed { result: found, .. } => {
                    assert_eq!(found, result);
                    completed = true;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert!(schema && batch && completed);
        let records = executor.store.journal().recent(2).expect("audit");
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .all(|entry| entry.record.command_id == Some(id)
                    && entry.record.command_kind == "PreviewRelation"
                    && entry.record.intent == StatementIntent::Read
                    && entry.record.decision == PolicyOutcome::Allowed)
        );
        assert!(records.iter().all(|entry| entry.record.statement.is_none()));
    }
    let Outcome::Executed { buffer, .. } = execute_sql(
        &executor,
        connection.id,
        session,
        format!("SELECT * FROM {qualified}"),
        false,
    )
    .await
    else {
        panic!("table intact");
    };
    assert_eq!(buffer.row_count(), 4);
    execute_sql(
        &executor,
        connection.id,
        session,
        "SELECT * FROM audit".into(),
        false,
    )
    .await;

    // An attached database is a SQLite namespace, never an extra schema.
    execute_sql(
        &executor,
        connection.id,
        session,
        "ATTACH DATABASE ':memory:' AS \"aux\"\"; --\"".into(),
        true,
    )
    .await;
    execute_sql(
        &executor,
        connection.id,
        session,
        "CREATE TABLE \"aux\"\"; --\".items (id INTEGER)".into(),
        true,
    )
    .await;
    execute_sql(
        &executor,
        connection.id,
        session,
        "INSERT INTO \"aux\"\"; --\".items VALUES (10), (11)".into(),
        true,
    )
    .await;
    for (catalog, namespace) in [
        (None, Some("aux\"; --".into())),
        (Some("aux\"; --".into()), None),
    ] {
        let outcome = executor
            .dispatch(
                Actor::Human,
                Command::PreviewRelation {
                    connection: connection.id,
                    session,
                    catalog,
                    namespace,
                    relation: "items".into(),
                    limit: 1,
                },
                &CancelToken::new(),
            )
            .await
            .expect("attached preview");
        let Outcome::Executed { buffer, .. } = outcome else {
            panic!("attached result");
        };
        assert_eq!(buffer.row_count(), 1);
        let (batch, _) = buffer.row(0).expect("row read").expect("row exists");
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("integer column");
        assert_eq!(values.value(0), 10);
    }
    executor
        .dispatch(
            Actor::Human,
            Command::Disconnect {
                connection: connection.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("close");
}

#[derive(Default)]
struct Probe {
    prepared: Mutex<Vec<(CatalogPath, u32)>>,
    executed: Mutex<Vec<ExecRequest>>,
    cancelled: AtomicBool,
    mutating: bool,
    waiting: bool,
    catalog: catalog_tests::Probe,
}

struct PreviewSession(Arc<Probe>);

#[async_trait]
impl Session for PreviewSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL
    }
    fn preview_request(&self, path: &CatalogPath, limit: u32) -> Result<ExecRequest> {
        self.0.prepared.lock().push((path.clone(), limit));
        // Deliberately wrong limits and intent test executor defenses.
        Ok(ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            if self.0.mutating {
                "DELETE FROM audit"
            } else {
                "SELECT 1"
            },
        )
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::unbounded()))
    }
    async fn execute(&self, request: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.0.executed.lock().push(request);
        Ok(Box::new(PreviewCursor {
            handle: StatementHandle::new(),
            waiting: self.0.waiting,
            sent: false,
        }))
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        self.0.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn catalog(&self) -> &dyn CatalogProvider {
        &self.0.catalog
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

struct PreviewCursor {
    handle: StatementHandle,
    waiting: bool,
    sent: bool,
}

#[async_trait]
impl Cursor for PreviewCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]))
    }
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.waiting {
            return futures::future::pending().await;
        }
        if self.sent {
            return Ok(None);
        }
        self.sent = true;
        Ok(Some(
            RecordBatch::try_new(
                self.schema(),
                vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
            )
            .expect("valid batch"),
        ))
    }
    fn stats(&self) -> ExecStats {
        ExecStats::default()
    }
}

fn fake(
    probe: Arc<Probe>,
    policy: Option<Arc<dyn PolicyGate>>,
) -> (Executor, ConnectionId, SessionId) {
    let connection = ConnectionConfig::new("fixture", DriverId::sqlite());
    let executor = executor(
        policy.unwrap_or_else(|| Arc::new(DefaultPolicy::new())),
        &connection,
    );
    let slot = executor.sessions.insert(SessionSlot::new(
        connection.id,
        Box::new(PreviewSession(probe)),
    ));
    (executor, connection.id, slot.id())
}

struct DenyPreview;
impl PolicyGate for DenyPreview {
    fn authorize(&self, _: &Actor, command: &Command, _: Environment) -> Decision {
        assert_eq!(command.name(), "PreviewRelation");
        assert_eq!(command.intent(), StatementIntent::Read);
        assert!(command.touches_database());
        assert!(!command.is_mutating());
        Decision::deny("fixture policy refuses previews")
    }
}

#[test]
fn preview_policy_refuses_human_and_agent_before_any_preparation() {
    for actor in actors() {
        let expected_actor = if actor.is_agent() {
            ActorKind::Agent
        } else {
            ActorKind::Human
        };
        let probe = Arc::new(Probe::default());
        let (executor, connection, session) = fake(probe.clone(), Some(Arc::new(DenyPreview)));
        let outcome = block_on(executor.dispatch(
            actor,
            preview(connection, session, 200),
            &CancelToken::new(),
        ))
        .expect("denial outcome");
        assert!(outcome.is_denied());
        assert!(probe.prepared.lock().is_empty());
        assert!(probe.executed.lock().is_empty());
        let audit = executor
            .store
            .journal()
            .recent(1)
            .expect("journal")
            .pop()
            .expect("denied entry");
        assert_eq!(audit.record.actor_kind, expected_actor);
        assert_eq!(audit.record.decision, PolicyOutcome::Denied);
        assert_eq!(audit.record.command_kind, "PreviewRelation");
    }
}

#[test]
fn preview_rejects_invalid_limits_names_and_session_ownership_before_preparation() {
    let probe = Arc::new(Probe::default());
    let (executor, connection, session) = fake(probe.clone(), None);
    for limit in [0, 1001, u32::MAX] {
        assert!(matches!(
            block_on(executor.dispatch(
                Actor::Human,
                preview(connection, session, limit),
                &CancelToken::new()
            )),
            Err(OxynError::Config(_))
        ));
    }
    let mut invalid_name = preview(connection, session, 200);
    if let Command::PreviewRelation { relation, .. } = &mut invalid_name {
        *relation = "bad\0name".into();
    }
    for command in [
        invalid_name,
        preview(ConnectionId::new(), session, 200),
        preview(connection, SessionId::new(), 200),
    ] {
        assert!(block_on(executor.dispatch(Actor::Human, command, &CancelToken::new())).is_err());
    }
    assert!(probe.prepared.lock().is_empty());
    assert!(probe.executed.lock().is_empty());
}

#[test]
fn preview_reclassifies_driver_sql_and_refuses_writes_for_both_actors() {
    for actor in actors() {
        let probe = Arc::new(Probe {
            mutating: true,
            ..Probe::default()
        });
        let (executor, connection, session) = fake(probe.clone(), None);
        assert!(matches!(
            block_on(executor.dispatch(
                actor,
                preview(connection, session, 200),
                &CancelToken::new()
            )),
            Err(OxynError::PolicyDenied { .. })
        ));
        assert_eq!(probe.prepared.lock().len(), 1);
        assert!(probe.executed.lock().is_empty());
    }
}

#[tokio::test]
async fn preview_enforces_read_only_and_row_limit_even_for_an_incorrect_driver() {
    let probe = Arc::new(Probe::default());
    let (executor, connection, session) = fake(probe.clone(), None);
    let outcome = executor
        .dispatch(
            Actor::Human,
            preview(connection, session, 2),
            &CancelToken::new(),
        )
        .await
        .expect("preview");
    let Outcome::Executed { buffer, .. } = outcome else {
        panic!("expected results");
    };
    assert_eq!(buffer.row_count(), 2);
    let requests = probe.executed.lock();
    let request = requests.first().expect("executed request");
    assert!(request.limits.read_only);
    assert_eq!(request.limits.max_rows, Some(2));
    assert!(!request.is_mutating());
    let prepared = probe.prepared.lock();
    assert_eq!(
        prepared.first().expect("prepared path").0.relation(),
        Some(HOSTILE)
    );
}

#[tokio::test]
async fn preview_cancellation_uses_the_existing_statement_and_command_identity() {
    for cancel_by_command in [false, true] {
        let probe = Arc::new(Probe {
            waiting: true,
            ..Probe::default()
        });
        let (executor, connection, session) = fake(probe.clone(), None);
        let token = CancelToken::new();
        let id = CommandId::new();
        let mut events = executor.subscribe();
        let run = executor.dispatch_as(id, Actor::Human, preview(connection, session, 200), &token);
        futures::pin_mut!(run);
        assert!(run.as_mut().now_or_never().is_none());
        let schema = events.try_recv().expect("schema event");
        assert_eq!(schema.command, id);
        assert!(matches!(schema.event, Event::SchemaReady { .. }));
        let running = executor.running.for_connection(connection);
        let statement = running.first().expect("registered statement").statement;
        if cancel_by_command {
            let outcome = executor
                .dispatch(
                    Actor::Human,
                    Command::Cancel {
                        connection,
                        statement,
                    },
                    &CancelToken::new(),
                )
                .await
                .expect("cancel command");
            assert!(matches!(outcome, Outcome::Cancelled { .. }));
        } else {
            token.cancel();
        }
        let outcome = run.await.expect("cancelled result");
        assert!(matches!(
            outcome,
            Outcome::Executed {
                sink: SinkOutcome::Cancelled,
                ..
            }
        ));
        assert!(probe.cancelled.load(Ordering::SeqCst));
        assert!(executor.running.is_empty());
        let cancelled = events.try_recv().expect("cancelled event");
        assert_eq!(cancelled.command, id);
        assert!(matches!(cancelled.event, Event::Cancelled));
    }
}
