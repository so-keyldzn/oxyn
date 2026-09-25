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
use tauri::ipc::Channel;
use tokio::sync::Notify;

use super::windows::{Answer, WindowKey};
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
    unresolved_write: bool,
    pending: Arc<PendingWrites>,
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
        // Unreadable counts as unresolved: the warning only asks to inspect,
        // while its absence would vouch for writes nobody could read (I-13).
        let unresolved_write = store
            .history()
            .any_requires_reconciliation()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "history unreadable; assuming a write needs inspection");
                true
            });
        Ok(Self {
            session,
            previous,
            unresolved_write,
            pending: Arc::new(PendingWrites::default()),
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
            | Command::ReconcileHistoryEntry { .. }
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
    ///
    /// Only the window built at launch may offer it: it speaks of how the
    /// previous launch ended, which a window opened later by `New window`
    /// did not witness, and one screen per window would offer the same
    /// copies twice (ADR-0043).
    #[must_use]
    pub fn recovery_status(&self, window: WindowKey) -> RecoveryStatus {
        let local = &self.inner.workbench.local;
        let initial = self.inner.windows.is_initial(window);
        RecoveryStatus {
            abnormal: initial && local.previous.needs_recovery(),
            unresolved_write: initial && local.unresolved_write,
        }
    }

    /// Where the backend asks this window to flush its drafts before closing.
    /// A reload replaces the channel.
    pub fn subscribe_shutdown(&self, window: WindowKey, channel: Channel<ShutdownSignal>) {
        self.inner.windows.subscribe_shutdown(window, channel);
    }

    /// This window has submitted every draft it held.
    pub fn shutdown_flushed(&self, window: WindowKey) {
        self.inner.windows.flushed(window);
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
                tracing::info!(session = %local.session, "session close recorded");
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

    /// Records the close of an exit nothing could hold, waiting at most `grace`.
    ///
    /// macOS ends the loop on `terminate:` — the Dock's Quit, a logout —
    /// without an `ExitRequested`: the webview can no longer be asked to flush,
    /// its replies needing the main thread this runs on. Its drafts are written
    /// at the latest 250 ms after the last key (ADR-0024), which is what the
    /// ordered shutdown already accepts from a webview that does not answer
    /// (ADR-0040). Local writes are still waited for: the close is never
    /// written over them.
    ///
    /// Blocks the calling thread for at most `grace`. Returns whether the
    /// close was recorded by then. Past `grace` the task is not cancelled: it
    /// may still record the close, always after the writes, if the process
    /// lives long enough. An ordered shutdown already under way is not waited
    /// for: the process ends with it, and whatever it recorded stands.
    ///
    /// A transaction still open on a console is not resolved: the end of the
    /// process rolls it back, and the journal names its session (ADR-0043).
    pub(crate) fn close_on_forced_exit(&self, grace: Duration) -> bool {
        if !self.begin_shutdown() {
            return self.inner.workbench.local.closed.load(Ordering::SeqCst);
        }
        self.warn_forced_exit_transactions();
        let (done, finished) = std::sync::mpsc::sync_channel(1);
        let backend = self.clone();
        tauri::async_runtime::spawn(async move {
            backend.close_session_after_local_writes().await;
            let _ = done.send(());
        });
        if finished.recv_timeout(grace).is_err() {
            tracing::warn!(
                "local writes still pending at a forced exit; the close is not recorded yet"
            );
        }
        self.inner.workbench.local.closed.load(Ordering::SeqCst)
    }

    /// Whether the close has been handled, recorded or given up.
    pub(crate) fn shutdown_finished(&self) -> bool {
        self.inner.workbench.local.closed.load(Ordering::SeqCst)
    }

    /// Asks the webview to flush, waits for it and for local writes, then
    /// records the close. Each wait is bounded.
    ///
    /// A webview that does not confirm its flush — silent, unreachable, late —
    /// does not keep the close unwritten: its drafts are at most 250 ms old
    /// (ADR-0024), the loss ADR-0040 accepts. Only a local write still running
    /// past its grace leaves the close unwritten, never recorded over it
    /// (ADR-0021).
    pub(crate) async fn shutdown(&self) {
        self.shutdown_within(FLUSH_GRACE).await;
    }

    async fn shutdown_within(&self, flush_grace: Duration) {
        let local = &self.inner.workbench.local;
        // Every window at once, `flush_grace` bounding them all rather than
        // each: a close is recorded once every webview has answered, or the
        // grace has passed (ADR-0043). Recorded all the same, see above: the
        // warning says which drafts may be missing.
        let windows = &self.inner.windows;
        let all = windows.keys();
        windows.expect_answers(&all);
        let asked: Vec<WindowKey> = all
            .iter()
            .copied()
            .filter(|window| windows.shutdown_signal(*window, ShutdownSignal::FlushDrafts))
            .collect();
        if asked.is_empty() {
            tracing::warn!("no webview subscribed to the shutdown; drafts not flushed");
        } else if asked.len() < all.len() {
            tracing::warn!(
                windows = all.len() - asked.len(),
                "some windows could not be asked to flush their drafts"
            );
        }
        let flushed = windows.answers(&asked, Answer::flushed, flush_grace).await;
        if flushed.len() < asked.len() {
            tracing::warn!(
                windows = asked.len() - flushed.len(),
                "some webviews did not confirm their drafts were flushed"
            );
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
            !backend.recovery_status(backend.test_window()).abnormal,
            "a first launch speaks of no crash"
        );
        assert!(
            !backend
                .recovery_status(backend.test_window())
                .unresolved_write,
            "nor of a write it never saw"
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

    /// The reported defect: an ordinary close, then recovery at every launch.
    #[test]
    fn an_ordinary_shutdown_is_not_offered_recovery_at_the_next_launch() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        assert!(first.begin_shutdown());
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(30), first.shutdown()).await
            })
            .expect("the shutdown is bounded");
        assert!(first.shutdown_finished());
        // Long enough for an unrecorded close to read as a crash.
        exit_and_age(&store, first);

        let next = Backend::assemble(store, secrets).expect("relaunch");
        assert!(!next.recovery_status(next.test_window()).abnormal);
    }

    /// A webview subscribed to the shutdown. `confirms` says whether it answers
    /// `FlushDrafts`; `reachable` whether the signal reaches it at all. Holds
    /// the backend weakly: the backend owns the channel.
    fn webview(backend: &Backend, confirms: bool, reachable: bool) -> Arc<AtomicUsize> {
        let asked = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&asked);
        let inner = Arc::downgrade(&backend.inner);
        let window = backend.test_window();
        backend.subscribe_shutdown(
            window,
            Channel::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                if !reachable {
                    return Err(tauri::Error::FailedToReceiveMessage);
                }
                if let (true, Some(inner)) = (confirms, inner.upgrade()) {
                    Backend { inner }.shutdown_flushed(window);
                }
                Ok(())
            }),
        );
        asked
    }

    /// Runs the shutdown of `launch`, bounded well under `flush_grace` when the
    /// webview confirms, then ends the launch.
    fn shut_down(
        runtime: &tokio::runtime::Runtime,
        store: &Store,
        launch: Backend,
        flush_grace: Duration,
    ) {
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(30), launch.shutdown_within(flush_grace))
                    .await
            })
            .expect("the shutdown is bounded");
        assert!(launch.shutdown_finished());
        exit_and_age(store, launch);
    }

    /// The webview confirms its flush: the shutdown goes on at once, not at
    /// the end of the grace, and the next launch speaks of no crash.
    #[test]
    fn a_confirmed_flush_records_the_close_without_waiting_for_the_grace() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        let asked = webview(&first, true, true);
        // Longer than the outer bound: waiting for it would fail the test.
        shut_down(&runtime, &store, first, Duration::from_secs(3600));
        assert_eq!(
            asked.load(Ordering::SeqCst),
            1,
            "the webview was asked once"
        );

        let next = Backend::assemble(store, secrets).expect("relaunch");
        assert!(!next.recovery_status(next.test_window()).abnormal);
    }

    /// A webview that never confirms, or cannot be reached: the close is
    /// recorded all the same (ADR-0040), after the grace, and the running
    /// sessions are still closed. Recovery is not offered for drafts at most
    /// 250 ms old.
    #[test]
    fn an_unconfirmed_flush_still_records_the_close_after_the_grace() {
        for reachable in [true, false] {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("a test runtime starts");
            let _guard = runtime.enter();
            let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
            let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

            let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
            let asked = webview(&first, false, reachable);
            let sessions = Arc::clone(&first.inner);
            shut_down(&runtime, &store, first, Duration::from_millis(100));
            assert_eq!(asked.load(Ordering::SeqCst), 1);
            assert!(sessions.executor.sessions().is_empty());
            assert!(sessions.executor.running().is_empty());
            drop(sessions);

            let next = Backend::assemble(store, secrets).expect("relaunch");
            assert!(
                !next.recovery_status(next.test_window()).abnormal,
                "reachable: {reachable}; an unconfirmed flush is not a crash"
            );
        }
    }

    /// The confirmation arrives once the close is recorded: it changes
    /// nothing, and the next launch speaks of no crash.
    #[test]
    fn a_late_flush_confirmation_changes_nothing() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        webview(&first, false, true);
        runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(30),
                    first.shutdown_within(Duration::from_millis(100)),
                )
                .await
            })
            .expect("the shutdown is bounded");
        first.shutdown_flushed(first.test_window());
        assert!(first.shutdown_finished());
        exit_and_age(&store, first);

        let next = Backend::assemble(store, secrets).expect("relaunch");
        assert!(!next.recovery_status(next.test_window()).abnormal);
    }

    /// The Dock's Quit: macOS ends the loop without `ExitRequested`, and the
    /// close is recorded from `RunEvent::Exit` (ADR-0040).
    #[test]
    fn a_forced_exit_records_the_close_and_is_not_offered_recovery() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        assert!(first.close_on_forced_exit(Duration::from_secs(10)));
        exit_and_age(&store, first);

        let next = Backend::assemble(store, secrets).expect("relaunch");
        assert!(!next.recovery_status(next.test_window()).abnormal);
    }

    /// A write still running when macOS ends the process: the close is never
    /// written over it, and the wait does not outlast its grace.
    #[test]
    fn a_forced_exit_over_a_pending_write_is_bounded_and_leaves_the_close_unwritten() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        let write = first
            .local_write(&Command::WriteDocument {
                workspace: first.inner.executor.workspace(),
                document: oxyn_core::DocumentId::new(),
                text: String::new(),
            })
            .expect("a document write is a local write");
        let grace = Duration::from_millis(100);
        let started = std::time::Instant::now();
        assert!(!first.close_on_forced_exit(grace));
        assert!(
            started.elapsed() < grace + Duration::from_secs(5),
            "the main thread waits for the grace, not for the write"
        );
        // The process ends with the write still running.
        exit_and_age(&store, first);

        let next = Backend::assemble(store, secrets).expect("relaunch");
        assert!(
            next.recovery_status(next.test_window()).abnormal,
            "a close nobody could confirm offers recovery"
        );
        drop(write);
    }

    /// The case recovery exists for: a process killed without its close.
    #[test]
    fn a_killed_launch_is_offered_recovery_once() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());

        // `kill -9`: no shutdown, and the heartbeat stops with the process.
        let killed = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("launch");
        exit_and_age(&store, killed);

        let relaunch = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("relaunch");
        assert!(relaunch.recovery_status(relaunch.test_window()).abnormal);
        runtime.block_on(relaunch.shutdown());
        exit_and_age(&store, relaunch);

        let next = Backend::assemble(store, secrets).expect("the launch after");
        assert!(
            !next.recovery_status(next.test_window()).abnormal,
            "a crash already announced does not come back after an ordinary close"
        );
    }

    /// Ends a launch, then ages its heartbeat past the abandonment threshold.
    fn exit_and_age(store: &Store, launch: Backend) {
        let session = launch.inner.workbench.local.session;
        drop(launch);
        store
            .mark_session_stale_for_tests(session)
            .expect("aged heartbeat");
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
        assert!(backend.recovery_status(backend.test_window()).abnormal);
        assert!(
            !backend
                .recovery_status(backend.test_window())
                .unresolved_write,
            "a crash alone is not a write with an unknown outcome"
        );
    }

    #[test]
    fn an_expired_write_in_history_is_reported_as_unresolved() {
        use oxyn_core::{Actor, OxynError, QueryLanguage, StatementIntent};
        use oxyn_store::history::HistoryRecord;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let expired = HistoryRecord::new(
            &Actor::Human,
            QueryLanguage::SQL,
            "INSERT INTO t VALUES (1)",
        )
        .with_intent(StatementIntent::Write)
        .failed(&OxynError::Timeout {
            after: Duration::from_secs(30),
        });
        store.history().record(&expired).expect("recorded");
        let backend = Backend::assemble(store, Arc::new(oxyn_secrets::MemorySecretStore::new()))
            .expect("backend");
        let status = backend.recovery_status(backend.test_window());
        assert!(status.unresolved_write);
        assert!(!status.abnormal, "and says nothing of a crash");
    }

    #[test]
    fn a_reconciled_write_no_longer_warns_at_the_next_launch() {
        use oxyn_core::{Actor, OxynError, QueryLanguage, StatementIntent};
        use oxyn_store::history::HistoryRecord;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        let entry = store
            .history()
            .record(
                &HistoryRecord::new(
                    &Actor::Human,
                    QueryLanguage::SQL,
                    "INSERT INTO t VALUES (1)",
                )
                .with_intent(StatementIntent::Write)
                .failed(&OxynError::Timeout {
                    after: Duration::from_secs(30),
                }),
            )
            .expect("recorded");
        let secrets = Arc::new(oxyn_secrets::MemorySecretStore::new());
        let first = Backend::assemble(Arc::clone(&store), secrets.clone()).expect("backend");
        assert!(first.recovery_status(first.test_window()).unresolved_write);

        runtime
            .block_on(first.reconcile_history_entry(entry))
            .expect("the user inspected the server");
        assert!(
            first.recovery_status(first.test_window()).unresolved_write,
            "this launch keeps what it observed at startup"
        );

        let next = Backend::assemble(store, secrets).expect("next launch");
        assert!(!next.recovery_status(next.test_window()).unresolved_write);
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
