//! The shutdown marker and the local writes it waits for
//! ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).
//!
//! A launch begins a session in the store, beats while it runs and writes its
//! close **after** every local write already submitted. Written before them,
//! the close would mark a clean shutdown over unwritten work — exactly the case
//! recovery exists for.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use oxyn_core::{AppSessionId, Command, WorkspaceId};
use oxyn_store::Store;
use oxyn_store::sessions::PreviousShutdown;
use parking_lot::Mutex;
use tauri::ipc::Channel;
use tokio::sync::Notify;

use super::{Backend, Inner};
use crate::ipc::recovery::{RecoveryStatus, ShutdownSignal};

/// How long the webview may take to flush the drafts it holds before closing.
///
/// A draft is written at the latest 250 ms after the last key (ADR-0024); the
/// margin covers a busy webview. A webview that never answers — crashed,
/// reloading — must not keep the window open.
const FLUSH_GRACE: Duration = Duration::from_secs(2);

/// How long local writes may take before the close is given up.
///
/// Given up means **not written**: the next launch then offers recovery, the
/// cautious reading of a close nobody could confirm.
const WRITES_GRACE: Duration = Duration::from_secs(10);

/// How long cancelling running statements and closing sessions may take.
///
/// Past it the exit goes on: a server that does not answer a cancel must not
/// keep the application open. The statements were still told to stop.
const SESSIONS_GRACE: Duration = Duration::from_secs(5);

/// This launch, what the previous one left, and the local writes in flight.
pub(crate) struct LocalWork {
    session: AppSessionId,
    previous: PreviousShutdown,
    pending: Arc<PendingWrites>,
    shutdown: Mutex<Option<Channel<ShutdownSignal>>>,
    flushed: Notify,
    closing: AtomicBool,
    closed: AtomicBool,
}

#[derive(Default)]
pub(crate) struct PendingWrites {
    count: AtomicUsize,
    changed: Notify,
}

/// Counts one local write until dropped, however the write ends.
pub(crate) struct LocalWriteGuard(Arc<PendingWrites>);

impl Drop for LocalWriteGuard {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

impl LocalWork {
    /// Reads how the previous launch ended, then begins this one.
    ///
    /// Before anything else writes: a session begun later would count itself.
    pub(crate) fn begin(store: &Store, workspace: WorkspaceId) -> Result<Self> {
        let (session, previous) = store
            .sessions()
            .begin(workspace)
            .context("recording the application session")?;
        Ok(Self {
            session,
            previous,
            pending: Arc::new(PendingWrites::default()),
            shutdown: Mutex::new(None),
            flushed: Notify::new(),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn guard(&self) -> LocalWriteGuard {
        self.pending.count.fetch_add(1, Ordering::SeqCst);
        LocalWriteGuard(Arc::clone(&self.pending))
    }
}

/// Whether a command writes local state the shutdown must wait for.
///
/// Without a provider declaration here, a ⌘Q in the second after « Save »
/// writes a clean close over lost work.
pub(crate) const fn is_local_write(command: &Command) -> bool {
    matches!(
        command,
        Command::WriteWorkspacePreferences { .. }
            | Command::SaveQueryDocument { .. }
            | Command::CloseQueryDocument { .. }
            | Command::DeleteQueryDocument { .. }
            | Command::WriteDocument { .. }
            | Command::SaveAiProvider { .. }
            | Command::RemoveAiProvider { .. }
    )
}

impl Backend {
    /// Starts the heartbeat. Called once, by `assemble`.
    ///
    /// On the blocking pool: a beat is a SQLite write. A weak handle, so the
    /// task ends with the backend.
    pub(crate) fn start_heartbeat(&self) {
        let weak = Arc::downgrade(&self.inner);
        tauri::async_runtime::spawn(async move {
            let period = oxyn_store::sessions::HEARTBEAT_INTERVAL
                .to_std()
                .unwrap_or(Duration::from_secs(30));
            let mut interval = tokio::time::interval(period);
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                if inner.workbench.local.closed.load(Ordering::SeqCst) {
                    break;
                }
                let _ = tokio::task::spawn_blocking(move || {
                    // A missed beat is not worth telling anyone: the next one
                    // comes in thirty seconds, and abandonment needs four.
                    let _ = inner
                        .executor
                        .store()
                        .sessions()
                        .heartbeat(inner.workbench.local.session);
                })
                .await;
            }
        });
    }

    /// Counts a local write for the shutdown, when `command` is one.
    pub(crate) fn local_write(&self, command: &Command) -> Option<LocalWriteGuard> {
        is_local_write(command).then(|| self.inner.workbench.local.guard())
    }

    /// What the recovery screen may assert: observed at startup, never re-read.
    #[must_use]
    pub fn recovery_status(&self) -> RecoveryStatus {
        RecoveryStatus {
            abnormal: self.inner.workbench.local.previous.needs_recovery(),
        }
    }

    /// Where the backend asks the webview to flush its drafts before closing.
    /// A reload replaces the channel.
    pub fn subscribe_shutdown(&self, channel: Channel<ShutdownSignal>) {
        *self.inner.workbench.local.shutdown.lock() = Some(channel);
    }

    /// The webview has submitted every draft it held.
    pub fn shutdown_flushed(&self) {
        self.inner.workbench.local.flushed.notify_waiters();
    }

    /// Waits for already submitted local writes, even those whose view is gone.
    pub async fn wait_for_local_writes(&self) {
        let pending = &self.inner.workbench.local.pending;
        loop {
            let mut notified = std::pin::pin!(pending.changed.notified());
            notified.as_mut().enable();
            if pending.count.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }

    /// Records this session's ordinary close, **after** local writes.
    ///
    /// A failure is not propagated: nobody is left to read it, and a session
    /// without a close is treated as abandoned — the cautious direction.
    pub async fn close_session_after_local_writes(&self) {
        self.wait_for_local_writes().await;
        let inner: Arc<Inner> = Arc::clone(&self.inner);
        let _ = tokio::task::spawn_blocking(move || {
            let local = &inner.workbench.local;
            if let Err(error) = inner.executor.store().sessions().close(local.session) {
                tracing::warn!(%error, "the session close could not be recorded");
            } else {
                local.closed.store(true, Ordering::SeqCst);
            }
        })
        .await;
    }

    /// Begins the shutdown once. Returns `false` if it had already begun, in
    /// which case the caller lets the exit proceed only when it has finished.
    pub(crate) fn begin_shutdown(&self) -> bool {
        !self
            .inner
            .workbench
            .local
            .closing
            .swap(true, Ordering::SeqCst)
    }

    /// Whether the close has been handled, recorded or given up.
    pub(crate) fn shutdown_finished(&self) -> bool {
        self.inner.workbench.local.closed.load(Ordering::SeqCst)
    }

    /// Asks the webview to flush, waits for it and for local writes, then
    /// records the close. Each wait is bounded; an unconfirmed close is left
    /// unwritten on purpose.
    pub(crate) async fn shutdown(&self) {
        let local = &self.inner.workbench.local;
        let channel = local.shutdown.lock().clone();
        if let Some(channel) = channel {
            let flushed = local.flushed.notified();
            if channel.send(ShutdownSignal::FlushDrafts).is_ok() {
                let _ = tokio::time::timeout(FLUSH_GRACE, flushed).await;
            }
        }
        match tokio::time::timeout(WRITES_GRACE, self.close_session_after_local_writes()).await {
            Ok(()) => {}
            Err(_) => {
                tracing::warn!("local writes still pending at exit; the close is not recorded");
            }
        }
        // `app.exit` ends the process without dropping a future: whatever runs
        // on a server keeps running unless it is cancelled here.
        if tokio::time::timeout(SESSIONS_GRACE, self.inner.executor.shutdown())
            .await
            .is_err()
        {
            tracing::warn!("sessions still closing at exit; their statements were told to stop");
        }
        // Marked finished whatever happened: the exit must not be held twice.
        local.closed.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_close_waits_for_local_writes_and_is_not_offered_recovery() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let backend = Backend::assemble(
            Arc::clone(&store),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
        )
        .expect("backend");
        assert!(
            !backend.recovery_status().abnormal,
            "a first launch speaks of no crash"
        );

        let write = backend
            .local_write(&Command::WriteDocument {
                workspace: backend.inner.executor.workspace(),
                document: oxyn_core::DocumentId::new(),
                text: String::new(),
            })
            .expect("a document write is a local write");
        let closing = runtime.spawn({
            let backend = backend.clone();
            async move { backend.close_session_after_local_writes().await }
        });
        runtime.block_on(tokio::time::sleep(Duration::from_millis(50)));
        assert!(
            !closing.is_finished(),
            "the close waits for the pending write"
        );
        drop(write);
        runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), closing).await })
            .expect("closes once the write ends")
            .expect("task joins");

        let workspace = backend.inner.executor.workspace();
        let (_, previous) = store.sessions().begin(workspace).expect("next launch");
        assert_eq!(previous, PreviousShutdown::Clean);
    }

    #[test]
    fn an_abandoned_session_is_reported_as_abnormal() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let workspace = super::super::current_workspace(&store).expect("workspace");
        let (crashed, _) = store.sessions().begin(workspace).expect("first launch");
        store
            .mark_session_stale_for_tests(crashed)
            .expect("aged heartbeat");
        let backend = Backend::assemble(store, Arc::new(oxyn_secrets::MemorySecretStore::new()))
            .expect("backend");
        assert!(backend.recovery_status().abnormal);
    }

    #[test]
    fn the_shutdown_stops_running_statements_and_closes_their_sessions() {
        use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft};
        use oxyn_core::{CommandId, ConnectionId, Environment, SessionId};

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: "shutdown".into(),
            environment: Environment::Local,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: std::collections::BTreeMap::new(),
        };
        let Ok(ConnectResponse::Open(open)) =
            runtime.block_on(backend.connect(CommandId::new(), draft))
        else {
            panic!("a local in-memory connection opens without approval");
        };
        let connection: ConnectionId = open.connection.parse().expect("connection id");
        let session: SessionId = open.session.parse().expect("session id");
        let id = CommandId::new();

        // Its only row arrives after a billion steps: without a cancel reaching
        // the engine, it runs for minutes.
        let running = runtime.spawn({
            let backend = backend.clone();
            async move {
                backend
                    .execute(
                        id,
                        connection,
                        session,
                        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n \
                         WHERE x < 1000000000) SELECT count(*) FROM n"
                            .into(),
                    )
                    .await
            }
        });
        runtime.block_on(tokio::time::sleep(Duration::from_millis(200)));
        assert!(!running.is_finished(), "the statement is still running");

        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(30), backend.shutdown()).await
            })
            .expect("the shutdown is bounded");

        let outcome = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(10), running).await })
            .expect("the statement stops with the application")
            .expect("task joins");
        assert!(
            !matches!(outcome, Ok(CommandOutcome::Executed { complete: true, .. })),
            "the statement was stopped, not run to its end"
        );
        assert!(backend.inner.executor.sessions().is_empty());
        assert!(backend.inner.executor.running().is_empty());
    }

    #[test]
    fn reads_are_not_counted_as_local_writes() {
        assert!(!is_local_write(&Command::ReadHistoryEntry { entry: 1 }));
    }
}
