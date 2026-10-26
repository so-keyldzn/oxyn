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

/// How long the ordered shutdown spends removing external agents' directories.
///
/// Every process group is signalled before this wait starts. Two seconds keeps
/// cleanup imperceptible after a normal quit without making a stalled temporary
/// filesystem hold the application open.
const AGENT_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// This launch, what the previous one left, and the local writes in flight.
pub(crate) struct LocalWork {
    pub(crate) session: AppSessionId,
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
            | Command::WriteWindowLayout { .. }
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
    /// Only the windows built at launch may offer it, each for its own
    /// consoles: it speaks of how the previous launch ended, which a window
    /// opened later by `New window` did not witness. The interrupted write
    /// is the history's, not a window's: only the first says it (ADR-0043).
    #[must_use]
    pub fn recovery_status(&self, window: WindowKey) -> RecoveryStatus {
        let local = &self.inner.workbench.local;
        let initial = self.inner.windows.is_initial(window);
        RecoveryStatus {
            abnormal: initial && local.previous.needs_recovery(),
            unresolved_write: self.inner.windows.is_first(window) && local.unresolved_write,
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
        // `app.exit` bypasses task and state drops. Signal every process group
        // first, then give the private directories one bounded cleanup pass
        // while this ordered path is still off Tauri's event-loop thread.
        if tokio::time::timeout(AGENT_SHUTDOWN_GRACE, self.inner.ai.shutdown_agents())
            .await
            .is_err()
        {
            tracing::warn!("some external-agent directories were not removed before exit");
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
mod tests;
