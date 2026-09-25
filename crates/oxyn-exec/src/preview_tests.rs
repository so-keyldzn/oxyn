//! Preview contracts: real in-memory SQLite and controllable driver boundaries.

use super::*;
use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use futures::executor::block_on;
use oxyn_catalog::{CatalogPath, CatalogProvider, CatalogScope, Freshness};
use oxyn_core::{
    AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ExecLimits, QueryLanguage,
    SqlDialect, StatementIntent,
};
use oxyn_driver::Session;
use oxyn_driver_sqlite::SqliteDriver;
use oxyn_store::{ActorKind, PolicyOutcome};
use parking_lot::Mutex;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

const HOSTILE: &str = "t\"; DROP TABLE audit; --";

/// Drives `future` until `ready` holds, without ever seeing it resolve.
///
/// Every condition awaited here is state the command itself sets while it
/// runs, so the future's own wake-ups are the only signal worth waiting on:
/// the decision and history writes that `dispatch` hands to the blocking pool
/// (ADR-0035) wake this task when they land, however long a loaded machine
/// takes to run them. Counting `yield_now` rounds instead was a hidden delay —
/// ten thousand yields last well under a millisecond, and a blocking-pool
/// thread under load routinely takes longer to be scheduled. A condition that
/// never holds hangs, and nextest's `slow-timeout` names the test.
async fn poll_until_ready(
    future: &mut (impl Future<Output = Result<Outcome>> + Unpin),
    mut ready: impl FnMut() -> bool,
) {
    futures::future::poll_fn(|context| {
        assert!(
            Pin::new(&mut *future).poll(context).is_pending(),
            "the command resolved before the awaited condition held"
        );
        if ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

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
        shape: oxyn_core::PreviewShape::unordered(),
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
        assert!(
            !stats.truncated,
            "the limited preview SQL was fully received"
        );
        assert!(!buffer.stats().truncated);
        assert!(buffer.is_complete());
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
                // A preview runs on the session like any execution: the state
                // it leaves is announced before `Completed` (ADR-0039).
                Event::TransactionState { state, .. } => {
                    assert!(!completed, "the state comes before the terminal event");
                    assert_eq!(state, oxyn_core::TransactionState::Idle);
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
                    shape: oxyn_core::PreviewShape::unordered(),
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

/// **Un DDL réussi marque le catalogue de la connexion à relire.**
///
/// C'est ce qui permet à l'interface de se rafraîchir sans clic Refresh
/// (I-10 : la connexion entière est visée, jamais la seule table nommée dans
/// le texte, puisque cet identifiant n'est jamais reconstruit depuis le SQL).
#[tokio::test]
async fn a_successful_ddl_invalidates_the_catalog_and_broadcasts_it() {
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

    // A node that has never been read stays `Never`, not `Invalidated`
    // (module docs of `CatalogCache`): read it once first, like a workspace
    // that already has its catalog tree open would have.
    executor
        .dispatch(
            Actor::Human,
            Command::RefreshCatalog {
                connection: connection.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("initial catalog read");
    let cache = executor
        .catalog(connection.id)
        .expect("catalog exists once connected");
    assert!(
        matches!(
            cache.read().freshness(&CatalogScope::Server),
            Freshness::Fetched(_)
        ),
        "the initial refresh must have left the scope fresh"
    );

    let mut events = executor.subscribe();
    execute_sql(
        &executor,
        connection.id,
        session,
        "CREATE TABLE audit (id INTEGER)".into(),
        true,
    )
    .await;

    assert!(
        matches!(
            cache.read().freshness(&CatalogScope::Server),
            Freshness::Invalidated
        ),
        "a DDL must mark the whole connection stale, without naming the table it touched"
    );
    let mut saw_catalog_updated = false;
    while let Ok(received) = events.try_recv() {
        if matches!(received.event, Event::CatalogUpdated) {
            assert_eq!(received.connection, Some(connection.id));
            saw_catalog_updated = true;
        }
    }
    assert!(
        saw_catalog_updated,
        "the interface learns to re-read the catalog through the event bus, not by polling"
    );
}

#[derive(Default)]
struct Probe {
    prepared: Mutex<Vec<(CatalogPath, u32)>>,
    executed: Mutex<Vec<ExecRequest>>,
    cancelled: AtomicBool,
    mutating: bool,
    /// La session déclare-t-elle savoir trier et filtrer un aperçu ?
    ///
    /// Faux par défaut : c'est l'état d'un moteur qui ne sait pas le faire, et
    /// c'est lui que le refus de capacité doit exercer.
    shapes_previews: bool,
    waiting: bool,
    waiting_for_metadata: bool,
    catalog: catalog_tests::Probe,
}

struct PreviewSession(Arc<Probe>);

#[async_trait]
impl Session for PreviewSession {
    fn capabilities(&self) -> Capabilities {
        let mut capacites = Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL;
        if self.0.shapes_previews {
            capacites |= Capabilities::PREVIEW_SORT | Capabilities::PREVIEW_FILTER;
        }
        capacites
    }
    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        self.0.prepared.lock().push((path.clone(), limit));
        if self.0.waiting_for_metadata {
            cancel.cancelled().await;
            return Err(OxynError::Cancelled);
        }
        // Le prédicat est inséré comme le fait un vrai driver : c'est ce qui
        // permet d'exercer la reclassification sur le texte final.
        let texte = if self.0.mutating {
            "DELETE FROM audit".to_owned()
        } else if let Some(predicat) = shape.predicate() {
            format!("SELECT * FROM t WHERE ({predicat}\n)")
        } else {
            "SELECT 1".to_owned()
        };
        // Deliberately wrong limits and intent test executor defenses.
        Ok(
            ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), texte)
                .with_intent(StatementIntent::Read)
                .with_limits(ExecLimits::unbounded()),
        )
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
    assert_eq!(
        request.limits.max_rows,
        Some(3),
        "one receive slot confirms the bounded SQL ended"
    );
    assert!(!request.is_mutating());
    let prepared = probe.prepared.lock();
    assert_eq!(
        prepared.first().expect("prepared path").0.relation(),
        Some(HOSTILE)
    );
}

#[tokio::test]
async fn preview_metadata_cancellation_never_executes_the_preview() {
    let probe = Arc::new(Probe {
        waiting_for_metadata: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    let token = CancelToken::new();
    let mut run =
        Box::pin(executor.dispatch(Actor::Human, preview(connection, session, 200), &token));
    poll_until_ready(&mut run, || probe.prepared.lock().len() == 1).await;
    token.cancel();
    assert!(matches!(run.await, Err(OxynError::Cancelled)));
    assert!(probe.executed.lock().is_empty());
    assert!(executor.running.is_empty());
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
        poll_until_ready(&mut run, || {
            !executor.running.for_connection(connection).is_empty()
        })
        .await;
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

/// Un tri ou un prédicat qu'une session ne déclare pas est refusé **avant** que
/// le driver ne compose quoi que ce soit.
///
/// La session factice de ce module ne déclare ni `PREVIEW_SORT` ni
/// `PREVIEW_FILTER`. C'est le cas d'un moteur qui ne sait pas ordonner une
/// lecture — le produit vise aussi les familles clé-valeur — et le refus doit
/// arriver là, pas dans la composition du SQL, où la tentation serait
/// d'abandonner la demande en silence : l'utilisateur croirait alors avoir
/// exclu des lignes qui sont pourtant à l'écran (ADR-0003, ADR-0020).
#[test]
fn preview_refuses_a_sort_or_predicate_the_session_does_not_declare() {
    let probe = Arc::new(Probe::default());
    let (executor, connection, session) = fake(probe.clone(), None);

    let mut trie = preview(connection, session, 200);
    if let Command::PreviewRelation { shape, .. } = &mut trie {
        shape.sort = vec![oxyn_core::PreviewSort::ascending("id")];
    }
    let mut filtre = preview(connection, session, 200);
    if let Command::PreviewRelation { shape, .. } = &mut filtre {
        shape.predicate = Some("id > 10".into());
    }

    for command in [trie, filtre] {
        assert!(matches!(
            block_on(executor.dispatch(Actor::Human, command, &CancelToken::new())),
            Err(OxynError::NotSupported { .. })
        ));
    }
    assert!(
        probe.prepared.lock().is_empty(),
        "le driver n'a même pas été sollicité"
    );
    assert!(probe.executed.lock().is_empty());

    // Le test négatif, sans lequel le précédent passerait même si l'aperçu était
    // refusé en toutes circonstances : un prédicat vide ne demande rien, donc
    // rien n'est refusé.
    let mut vide = preview(connection, session, 200);
    if let Command::PreviewRelation { shape, .. } = &mut vide {
        shape.predicate = Some("   ".into());
    }
    assert!(block_on(executor.dispatch(Actor::Human, vide, &CancelToken::new())).is_ok());
    assert_eq!(probe.prepared.lock().len(), 1);
}

/// Un prédicat illisible et une écriture sont deux refus différents.
///
/// Le classificateur compte pour mutant ce qu'il ne comprend pas, et c'est la
/// bonne prudence. Mais le message doit dire ce qui s'est passé : sur un
/// aperçu, la seule part écrite à la main est le prédicat, et « pas en lecture
/// seule » enverrait l'utilisateur chercher un droit manquant alors qu'il a une
/// faute de frappe.
#[test]
fn preview_tells_an_unreadable_filter_apart_from_a_write() {
    let probe = Arc::new(Probe {
        shapes_previews: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(probe.clone(), None);
    let mut casse = preview(connection, session, 200);
    if let Command::PreviewRelation { shape, .. } = &mut casse {
        shape.predicate = Some("id >< 3".into());
    }
    let Err(OxynError::PolicyDenied { reason }) =
        block_on(executor.dispatch(Actor::Human, casse, &CancelToken::new()))
    else {
        panic!("un prédicat illisible est refusé");
    };
    assert!(
        reason.contains("syntax"),
        "le refus parle du prédicat, pas d'un droit : {reason}"
    );
    assert!(
        !reason.contains("read-only"),
        "et ne renvoie pas vers la lecture seule : {reason}"
    );

    // Le test négatif : un driver qui compose réellement une écriture garde son
    // refus d'origine, celui qui dit la vérité pour ce cas-là.
    let mutant = Arc::new(Probe {
        mutating: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(mutant, None);
    let Err(OxynError::PolicyDenied { reason }) = block_on(executor.dispatch(
        Actor::Human,
        preview(connection, session, 200),
        &CancelToken::new(),
    )) else {
        panic!("une écriture composée par le driver est refusée");
    };
    assert!(reason.contains("read-only"), "{reason}");
}
