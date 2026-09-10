//! Session close fences include preparation and draining without touching siblings.

use super::*;
use async_trait::async_trait;
use oxyn_core::{AgentId, AgentSessionId, Capabilities, DefaultPolicy, DriverId, QueryLanguage};
use oxyn_driver::Session;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Probe {
    preparing: bool,
    started: tokio::sync::Notify,
    done: tokio::sync::Notify,
    active: AtomicBool,
    finished: AtomicBool,
    cancelled: AtomicBool,
    closed: AtomicBool,
    catalog: catalog_tests::Probe,
}
impl Probe {
    fn finish(&self, cancelled: bool) {
        self.cancelled.store(cancelled, Ordering::SeqCst);
        self.finished.store(true, Ordering::SeqCst);
        self.done.notify_one();
    }
}
struct ClosingSession(Arc<Probe>);
#[async_trait]
impl Session for ClosingSession {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }
    async fn execute(&self, _: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.0.active.store(true, Ordering::SeqCst);
        self.0.started.notify_one();
        if self.0.preparing {
            cancel.cancelled().await;
            self.0.finish(true);
            Err(OxynError::Cancelled)
        } else {
            Ok(Box::new(ClosingCursor {
                probe: self.0.clone(),
                cancel: cancel.clone(),
                handle: StatementHandle::new(),
            }))
        }
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
        if self.0.active.load(Ordering::SeqCst) && !self.0.finished.load(Ordering::SeqCst) {
            self.0.done.notified().await;
        }
        self.0.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
struct ClosingCursor {
    probe: Arc<Probe>,
    cancel: CancelToken,
    handle: StatementHandle,
}
#[async_trait]
impl Cursor for ClosingCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> arrow::datatypes::SchemaRef {
        Arc::new(arrow::datatypes::Schema::empty())
    }
    async fn next_batch(&mut self) -> Result<Option<arrow::array::RecordBatch>> {
        self.cancel.cancelled().await;
        Err(OxynError::Cancelled)
    }
    fn stats(&self) -> ExecStats {
        ExecStats::default()
    }
}
impl Drop for ClosingCursor {
    fn drop(&mut self) {
        self.probe.finish(self.cancel.is_cancelled());
    }
}

#[tokio::test]
async fn closing_only_the_owned_session_interrupts_preparation_and_draining_for_both_actors() {
    for preparing in [true, false] {
        for actor in [
            Actor::Human,
            Actor::agent(AgentId::new(), AgentSessionId::new()),
        ] {
            let store = Arc::new(Store::open_in_memory().expect("store"));
            let workspace = store.workspaces().create("sessions").expect("workspace").id;
            let config = ConnectionConfig::new("sessions", DriverId::sqlite())
                .with_environment(Environment::Local);
            let policy = Arc::new(DefaultPolicy::new());
            policy.register(&config);
            let executor = Arc::new(
                Executor::builder(store, policy)
                    .with_workspace(workspace)
                    .build(),
            );
            executor.register_connection(&config);
            let probe = Arc::new(Probe {
                preparing,
                ..Default::default()
            });
            let target = executor.sessions.insert(SessionSlot::new(
                config.id,
                Box::new(ClosingSession(probe.clone())),
            ));
            let sibling = executor.sessions.insert(SessionSlot::new(
                config.id,
                Box::new(ClosingSession(Arc::new(Probe::default()))),
            ));
            let catalog = Arc::new(crate::catalog::ConnectionCatalog::new());
            executor.catalogs.write().insert(config.id, catalog.clone());
            let task_executor = executor.clone();
            let session = target.id();
            let connection = config.id;
            let task = tokio::spawn(async move {
                task_executor
                    .dispatch(
                        actor,
                        Command::Execute {
                            connection,
                            session,
                            request: Box::new(ExecRequest::new(QueryLanguage::SQL, "SELECT 1")),
                        },
                        &CancelToken::new(),
                    )
                    .await
            });
            probe.started.notified().await;
            assert!(
                executor
                    .dispatch(
                        actor,
                        Command::CloseSession {
                            connection: ConnectionId::new(),
                            session
                        },
                        &CancelToken::new()
                    )
                    .await
                    .is_err()
            );
            assert!(!probe.closed.load(Ordering::SeqCst));
            let outcome = tokio::time::timeout(
                Duration::from_secs(2),
                executor.dispatch(
                    actor,
                    Command::CloseSession {
                        connection,
                        session,
                    },
                    &CancelToken::new(),
                ),
            )
            .await
            .expect("close must not hang")
            .expect("close");
            assert!(
                matches!(outcome, Outcome::SessionClosed { session: closed } if closed == session)
            );
            assert!(probe.cancelled.load(Ordering::SeqCst));
            assert!(probe.closed.load(Ordering::SeqCst));
            assert!(sibling.is_open());
            assert!(executor.sessions.get(session).is_none());
            assert!(!catalog.closed.is_cancelled());
            let execution = task.await.expect("task");
            assert!(matches!(
                execution,
                Err(OxynError::Cancelled)
                    | Ok(Outcome::Executed {
                        sink: SinkOutcome::Cancelled,
                        ..
                    })
            ));
            executor
                .dispatch(
                    actor,
                    Command::CloseSession {
                        connection,
                        session,
                    },
                    &CancelToken::new(),
                )
                .await
                .expect("idempotent close");
        }
    }
}
