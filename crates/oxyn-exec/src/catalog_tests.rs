//! Contract tests without a database or runtime thread.

use super::*;
use async_trait::async_trait;
use futures::{FutureExt, executor::block_on};
use oxyn_catalog::{
    CatalogPath, CatalogProvider, CatalogRef, NamespaceRef, Relation, RelationKind, RelationRef,
    ServerInfo,
};
use oxyn_core::{AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, ErrorClass};
use oxyn_driver::Session;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const HOSTILE: &str = "users\"; DROP TABLE audit; --";

#[derive(Default)]
pub(crate) struct Probe {
    calls: Mutex<Vec<String>>,
    schemas: bool,
    catalogs: bool,
    wait: AtomicBool,
    observed_cancel: AtomicBool,
    failure: Mutex<Option<ErrorClass>>,
}

#[async_trait]
impl CatalogProvider for Probe {
    async fn server_info(&self, _: &CancelToken) -> Result<ServerInfo> {
        self.calls.lock().push("info".into());
        Ok(ServerInfo::new("fixture", "", Capabilities::empty()))
    }
    async fn list_catalogs(&self, _: &CancelToken) -> Result<Vec<CatalogRef>> {
        self.calls.lock().push("catalogs".into());
        Ok(if self.catalogs {
            vec![CatalogRef::new("db").expect("valid fixture")]
        } else {
            vec![]
        })
    }
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        _: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        self.calls.lock().push("namespaces".into());
        let parent = CatalogPath::from_levels(catalog.map(str::to_owned), None, None)?;
        Ok(vec![NamespaceRef::new(parent, "public")?])
    }
    async fn list_relations(
        &self,
        parent: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        self.calls.lock().push(format!("relations:{parent}"));
        if self.wait.load(Ordering::SeqCst) {
            cancel.cancelled().await;
            self.observed_cancel.store(true, Ordering::SeqCst);
            return Err(OxynError::Cancelled);
        }
        if let Some(class) = *self.failure.lock() {
            return Err(OxynError::Driver {
                driver: DriverId::sqlite(),
                class,
                source: Box::new(std::io::Error::other("fixture failure")),
            });
        }
        Ok(vec![RelationRef::new(
            parent.clone(),
            HOSTILE,
            RelationKind::Table,
        )?])
    }
    async fn describe_relation(&self, path: &CatalogPath, _: &CancelToken) -> Result<Relation> {
        self.calls.lock().push("describe".into());
        Ok(Relation::new(
            path.relation().expect("validated relation"),
            RelationKind::Table,
        ))
    }
}

pub(crate) struct FakeSession(pub Arc<Probe>);

#[async_trait]
impl Session for FakeSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::TABLES
            | if self.0.schemas {
                Capabilities::SCHEMAS
            } else {
                Capabilities::empty()
            }
    }
    async fn execute(&self, _: ExecRequest, _: &CancelToken) -> Result<Box<dyn Cursor>> {
        panic!("catalog commands must never execute SQL")
    }
    async fn cancel(&self, _: StatementHandle) -> Result<()> {
        panic!("catalog provider owns its cancellation")
    }
    fn catalog(&self) -> &dyn CatalogProvider {
        self.0.as_ref()
    }
    async fn ping(&self) -> Result<Duration> {
        Ok(Duration::ZERO)
    }
    async fn close(self: Box<Self>) -> Result<()> {
        self.0.calls.lock().push("close".into());
        Ok(())
    }
}

fn bench(probe: Arc<Probe>) -> (Executor, ConnectionId) {
    let connection = ConnectionConfig::new("fixture", DriverId::sqlite());
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store.workspaces().create("fixture").expect("workspace").id;
    let policy = Arc::new(DefaultPolicy::new());
    policy.register(&connection);
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    executor.register_connection(&connection);
    executor.sessions.insert(SessionSlot::new(
        connection.id,
        Box::new(FakeSession(probe)),
    ));
    executor.catalogs.write().insert(
        connection.id,
        Arc::new(crate::catalog::ConnectionCatalog::new()),
    );
    (executor, connection.id)
}

fn refresh(connection: ConnectionId, scope: CatalogRefreshScope) -> Command {
    Command::RefreshCatalogScope { connection, scope }
}

#[test]
fn human_and_agent_read_lazily_through_policy_and_audit() {
    for actor in [
        Actor::Human,
        Actor::agent(AgentId::new(), AgentSessionId::new()),
    ] {
        let probe = Arc::new(Probe {
            schemas: true,
            catalogs: true,
            ..Probe::default()
        });
        let (executor, connection) = bench(probe.clone());
        let token = CancelToken::new();
        let mut events = executor.subscribe();
        let outcome =
            block_on(executor.dispatch(actor, Command::RefreshCatalog { connection }, &token))
                .expect("root read");
        assert!(matches!(
            outcome,
            Outcome::CatalogRefreshed {
                scope: CatalogRefreshScope::Root,
                ..
            }
        ));
        assert_eq!(*probe.calls.lock(), ["info", "catalogs"]);
        let event = events.try_recv().expect("catalog event");
        assert_eq!(event.connection, Some(connection));
        assert!(matches!(event.event, Event::CatalogUpdated));
        let cache = executor.catalog(connection).expect("connected cache");
        assert_eq!(cache.read().catalogs().count(), 1);
        assert!(
            cache
                .read()
                .server_info()
                .expect("info")
                .capabilities
                .contains(Capabilities::SCHEMAS)
        );
        for scope in [
            CatalogRefreshScope::Namespaces {
                catalog: Some("db".into()),
            },
            CatalogRefreshScope::Relations {
                catalog: Some("db".into()),
                namespace: Some("public".into()),
            },
        ] {
            block_on(executor.dispatch(actor, refresh(connection, scope), &token))
                .expect("lazy level");
        }
        let path = CatalogPath::for_relation(Some("db"), Some("public"), HOSTILE)
            .expect("hostile valid name");
        assert!(cache.read().relation_summary(&path).is_some());
        assert!(cache.read().relation(&path).is_none());
        block_on(executor.dispatch(
            actor,
            refresh(
                connection,
                CatalogRefreshScope::Relation {
                    catalog: Some("db".into()),
                    namespace: Some("public".into()),
                    relation: HOSTILE.into(),
                },
            ),
            &token,
        ))
        .expect("explicit detail");
        assert_eq!(cache.read().relation(&path).expect("detail").name, HOSTILE);
        let journal = executor.store.journal().recent(20).expect("audit");
        assert_eq!(
            journal.len(),
            8,
            "decision and result for all four commands"
        );
        assert!(!format!("{journal:?}").contains(HOSTILE));
    }
}

#[test]
fn root_skips_absent_levels_and_never_describes_relations() {
    for schemas in [false, true] {
        let probe = Arc::new(Probe {
            schemas,
            ..Probe::default()
        });
        let (executor, connection) = bench(probe.clone());
        block_on(executor.dispatch(
            Actor::Human,
            Command::RefreshCatalog { connection },
            &CancelToken::new(),
        ))
        .expect("root");
        assert_eq!(
            *probe.calls.lock(),
            [
                "info",
                "catalogs",
                if schemas { "namespaces" } else { "relations:" }
            ]
        );
    }
}

#[test]
fn failed_refresh_preserves_cache_and_driver_error_class() {
    let probe = Arc::new(Probe::default());
    let (executor, connection) = bench(probe.clone());
    let command = refresh(
        connection,
        CatalogRefreshScope::Relations {
            catalog: None,
            namespace: None,
        },
    );
    block_on(executor.dispatch(Actor::Human, command.clone(), &CancelToken::new()))
        .expect("initial read");
    for class in [
        ErrorClass::Transient,
        ErrorClass::Permanent,
        ErrorClass::Ambiguous,
    ] {
        *probe.failure.lock() = Some(class);
        let before = probe.calls.lock().len();
        let error = block_on(executor.dispatch(Actor::Human, command.clone(), &CancelToken::new()))
            .expect_err("provider failure");
        assert!(matches!(error, OxynError::Driver { class: actual, .. } if actual == class));
        assert_eq!(error.is_retryable(), class.is_retryable());
        assert_eq!(
            executor
                .catalog(connection)
                .expect("cache survives")
                .read()
                .relation_count(),
            1
        );
        assert_eq!(probe.calls.lock().len(), before + 1, "no automatic retry");
    }
}

#[test]
fn cancellation_reaches_provider_and_disconnect_waits_for_cleanup() {
    for disconnect in [false, true] {
        let probe = Arc::new(Probe::default());
        probe.wait.store(true, Ordering::SeqCst);
        let (executor, connection) = bench(probe.clone());
        let token = CancelToken::new();
        let read = executor.dispatch(
            Actor::Human,
            refresh(
                connection,
                CatalogRefreshScope::Relations {
                    catalog: None,
                    namespace: None,
                },
            ),
            &token,
        );
        futures::pin_mut!(read);
        assert!(read.as_mut().now_or_never().is_none());
        if disconnect {
            let close_token = CancelToken::new();
            let close = executor.dispatch(
                Actor::Human,
                Command::Disconnect { connection },
                &close_token,
            );
            futures::pin_mut!(close);
            assert!(close.as_mut().now_or_never().is_none());
            assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
            assert!(matches!(
                block_on(close),
                Ok(Outcome::Disconnected { closed: 1, .. })
            ));
            assert!(executor.catalog(connection).is_none());
        } else {
            token.cancel();
            assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
            assert!(
                executor
                    .catalog(connection)
                    .expect("cache")
                    .read()
                    .is_empty()
            );
        }
        assert!(
            probe.observed_cancel.load(Ordering::SeqCst),
            "provider acknowledged cancellation"
        );
    }
}

#[test]
fn cancellation_while_waiting_for_refresh_lock_does_not_call_provider() {
    let probe = Arc::new(Probe::default());
    let (executor, connection) = bench(probe.clone());
    let state = executor
        .catalogs
        .read()
        .get(&connection)
        .cloned()
        .expect("state");
    let guard = block_on(state.refresh.lock());
    let token = CancelToken::new();
    let read = executor.dispatch(Actor::Human, Command::RefreshCatalog { connection }, &token);
    futures::pin_mut!(read);
    assert!(read.as_mut().now_or_never().is_none());
    token.cancel();
    assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
    assert!(probe.calls.lock().is_empty());
    drop(guard);
}

#[test]
fn invalid_scope_and_missing_capability_never_call_provider() {
    let probe = Arc::new(Probe::default());
    let (executor, connection) = bench(probe.clone());
    for scope in [
        CatalogRefreshScope::Namespaces { catalog: None },
        CatalogRefreshScope::Relation {
            catalog: None,
            namespace: None,
            relation: "bad\nname".into(),
        },
    ] {
        assert!(
            block_on(executor.dispatch(
                Actor::Human,
                refresh(connection, scope),
                &CancelToken::new()
            ))
            .is_err()
        );
    }
    assert!(probe.calls.lock().is_empty());
    assert!(executor.catalog(ConnectionId::new()).is_none());
    assert!(matches!(
        block_on(executor.dispatch(
            Actor::Human,
            Command::RefreshCatalog {
                connection: ConnectionId::new()
            },
            &CancelToken::new()
        )),
        Err(OxynError::Connection(_))
    ));
}

#[test]
fn shutdown_cancels_catalog_provider_before_closing_session() {
    let probe = Arc::new(Probe::default());
    probe.wait.store(true, Ordering::SeqCst);
    let (executor, connection) = bench(probe.clone());
    let token = CancelToken::new();
    let read = executor.dispatch(
        Actor::Human,
        refresh(
            connection,
            CatalogRefreshScope::Relations {
                catalog: None,
                namespace: None,
            },
        ),
        &token,
    );
    futures::pin_mut!(read);
    assert!(read.as_mut().now_or_never().is_none());
    let shutdown = executor.shutdown();
    futures::pin_mut!(shutdown);
    assert!(shutdown.as_mut().now_or_never().is_none());
    assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
    assert_eq!(block_on(shutdown), 1);
    assert!(probe.observed_cancel.load(Ordering::SeqCst));
    assert!(executor.catalog(connection).is_none());
}

#[test]
fn refused_cache_merge_preserves_prior_data_and_emits_no_success() {
    let probe = Arc::new(Probe::default());
    let (executor, connection) = bench(probe);
    block_on(executor.dispatch(
        Actor::Human,
        Command::RefreshCatalog { connection },
        &CancelToken::new(),
    ))
    .expect("initial cache");
    let state = executor
        .catalogs
        .read()
        .get(&connection)
        .cloned()
        .expect("cache state");
    {
        let mut budget = block_on(state.refresh.lock());
        for i in 0..1023 {
            budget
                .reserve(
                    &CatalogRefreshScope::Namespaces {
                        catalog: Some(i.to_string()),
                    },
                    &crate::catalog::CatalogPatch::default(),
                )
                .expect("fill remaining scope slots");
        }
    }
    let mut events = executor.subscribe();
    let result = block_on(executor.dispatch(
        Actor::Human,
        refresh(
            connection,
            CatalogRefreshScope::Relations {
                catalog: None,
                namespace: None,
            },
        ),
        &CancelToken::new(),
    ));
    assert!(matches!(result, Err(OxynError::Config(_))));
    assert!(events.try_recv().is_err());
    assert_eq!(state.cache.read().relation_count(), 1);
}
