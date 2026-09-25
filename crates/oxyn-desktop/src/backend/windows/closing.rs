//! Closing a window that is not the last, and letting go of what a window
//! held ([ADR-0043](../../../../../docs/adr/0043-multi-fenetre.md),
//! « Fermeture »).
//!
//! Closing the last window quits: that is the ordered exit of
//! `backend/exit.rs` and `backend/recovery.rs`, unchanged. Closing another
//! window closes its consoles, after the webview has asked the user about
//! those that would lose work and resolved its open transactions. A webview
//! that does not answer does not keep its window open: its consoles are then
//! **not** closed — their documents stay open, claimed by no window — and
//! only its sessions go, which rolls their transactions back.

use std::time::Duration;

use oxyn_core::{Command, CommandId, ConnectionId};

use super::{Closing, WindowKey};
use crate::backend::Backend;
use crate::backend::exit::warn_unacknowledged;
use crate::ipc::recovery::{ExitScope, ShutdownSignal};
use crate::ipc::windows::WindowSignal;
use crate::ipc::{CommandOutcome, IpcError};

/// How long a webview may take to acknowledge its window's close: the grace
/// of the drafts' flush (`backend/recovery.rs`), for the same reason — a
/// frozen webview must not keep its window open.
const ACKNOWLEDGE_GRACE: Duration = Duration::from_secs(2);

/// What a close asked on a window does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseStep {
    /// The window is the last one: its close is the application's exit.
    Quit,
    /// The webview acknowledged: the user decides in the window, which comes
    /// to the front.
    Asked,
    /// Nobody answered: the window goes now, its consoles left open.
    Close,
    /// Nothing to do: a close of this window is already under way, or the
    /// window is gone.
    Held,
}

impl Backend {
    /// Reserves a new window: the tests that build none.
    #[cfg(test)]
    pub(crate) fn reserve_window(&self, initial: bool) -> Result<WindowKey, IpcError> {
        self.reserve_restored_window(initial, None)
    }

    /// Reserves a window before it is built, bounded to 16, under the key the
    /// workspace file kept for it when it is restored.
    ///
    /// # Errors
    /// Past the bound, or a restored key already open.
    pub(crate) fn reserve_restored_window(
        &self,
        initial: bool,
        restored: Option<oxyn_core::WindowLayout>,
    ) -> Result<WindowKey, IpcError> {
        let key = restored.as_ref().map(|layout| WindowKey::of(layout.window));
        let key = self.inner.windows.reserve(initial, key)?;
        self.inner.layouts.register(key, restored);
        Ok(key)
    }

    /// The first window, reserved if there is none: the tests that speak to
    /// one webview.
    #[cfg(test)]
    pub(crate) fn test_window(&self) -> WindowKey {
        let first = self.inner.windows.keys().first().copied();
        first.unwrap_or_else(|| self.reserve_window(true).expect("a first window"))
    }

    /// A close asked on `window` — its close button, or the webview asking
    /// again once its transactions are resolved.
    pub(crate) async fn window_close_step(&self, window: WindowKey) -> CloseStep {
        self.window_close_step_within(window, ACKNOWLEDGE_GRACE)
            .await
    }

    pub(crate) async fn window_close_step_within(
        &self,
        window: WindowKey,
        grace: Duration,
    ) -> CloseStep {
        let windows = &self.inner.windows;
        // Decided under the registry's lock: a window whose close is
        // confirmed no longer counts, one whose dialog is open still does.
        if windows.is_last(window) {
            return CloseStep::Quit;
        }
        if !windows.advance_close(
            window,
            &[Closing::Open, Closing::Deciding],
            Closing::Listing,
        ) {
            return CloseStep::Held;
        }
        // Its own sessions only: another window's transactions are not this
        // close's to resolve.
        let transactions = self
            .open_transactions(grace, Some(window))
            .await
            .remove(&window)
            .unwrap_or_default();
        windows.expect_answers(&[window]);
        if !windows.advance_close(window, &[Closing::Listing], Closing::Asking) {
            return CloseStep::Held;
        }
        let sent = if transactions.is_empty() {
            windows.signal(window, WindowSignal::CloseRequested)
        } else {
            windows.shutdown_signal(
                window,
                ShutdownSignal::ResolveTransactions {
                    transactions: transactions.clone(),
                    scope: ExitScope::Window,
                },
            )
        };
        let acknowledged = sent
            && windows
                .answers(&[window], super::Answer::acknowledged, grace)
                .await
                .contains(&window);
        if acknowledged {
            return if windows.advance_close(window, &[Closing::Asking], Closing::Deciding) {
                CloseStep::Asked
            } else {
                CloseStep::Held
            };
        }
        warn_unacknowledged(&transactions);
        if windows.advance_close(window, &[Closing::Asking], Closing::Closed) {
            tracing::warn!("a window closed without its webview's answer; its documents stay open");
            CloseStep::Close
        } else {
            CloseStep::Held
        }
    }

    /// `Cancel` in the dialog of this window's close. Returns whether a close
    /// of this window was under way.
    pub(crate) fn cancel_window_close(&self, window: WindowKey) -> bool {
        self.inner.windows.advance_close(
            window,
            &[Closing::Listing, Closing::Asking, Closing::Deciding],
            Closing::Open,
        )
    }

    /// The webview has closed its consoles and confirms its window's close.
    /// Returns whether the window may now go: only a close the user is
    /// deciding can be confirmed.
    pub(crate) fn confirm_window_close(&self, window: WindowKey) -> bool {
        self.inner
            .windows
            .advance_close(window, &[Closing::Deciding], Closing::Closed)
    }

    /// Lets go of everything a closed window held, once: its commands are
    /// cancelled and its pending approvals rejected, its sessions closed, its
    /// assistants forgotten, its results released, and the connections no
    /// other window holds disconnected. A later call finds nothing.
    ///
    /// An operation still running is cancelled as `⌘W` would: a write whose
    /// outcome becomes unknown stays flagged in history, never replayed
    /// ([I-13](../../../../../CLAUDE.md#i-13)).
    pub(crate) async fn release_window(&self, window: WindowKey) {
        let owned = self.inner.windows.forget(window);
        for command in owned.commands {
            self.drop_command(command).await;
        }
        for (connection, session) in owned.sessions {
            if let Err(error) = self.close_console(connection, session).await {
                tracing::warn!(
                    session = %session,
                    error = %error.message,
                    "a closed window's session could not be closed"
                );
            }
        }
        for connection in owned.assistants {
            self.inner.ai.release_agents(connection);
            self.close_ai_conversation(connection);
        }
        // Forgetting may delete spill files: off the async workers.
        let _ = self
            .on_blocking_pool(move |backend| {
                for (result, views) in owned.results {
                    for _ in 0..views {
                        backend.forget_result(result);
                    }
                }
                Ok(())
            })
            .await;
        for connection in owned.released {
            self.disconnect_released(connection).await;
        }
    }

    /// Cancels a closed window's command, and rejects it if it waits for a
    /// decision: an approval pending in a closed window is rejected, like a
    /// closed tab's (ADR-0042). A native dialog still on screen then approves
    /// nothing: the decision it showed is no longer pending.
    async fn drop_command(&self, command: CommandId) {
        let inner = &self.inner;
        let _ = self.cancel(command);
        if inner.pending_connections.lock().contains_key(&command) {
            let _ = self.decide_connection(command, false).await;
        } else if inner.settings.pending_changes.lock().contains_key(&command) {
            let _ = self.decide_connection_change(command, false).await;
        } else if inner.executor.approvals().peek(command).is_some() {
            let _ = self.decide(command, false).await;
        }
    }

    /// Closes a window's workspace on `connection`: its own sessions, its
    /// assistant there, then — if no other window holds the connection — the
    /// connection itself. A window never disconnects what another shows
    /// (ADR-0043).
    ///
    /// # Errors
    /// The disconnection, when this window was the last to hold it.
    pub async fn release_connection(
        &self,
        window: WindowKey,
        connection: ConnectionId,
    ) -> Result<CommandOutcome, IpcError> {
        let windows = &self.inner.windows;
        for session in windows.sessions_on(window, connection) {
            windows.release_session(session);
            if let Err(error) = self.close_console(connection, session).await {
                tracing::warn!(
                    session = %session,
                    error = %error.message,
                    "a released workspace's session could not be closed"
                );
            }
        }
        if windows.has_assistant(window, connection) {
            // An agent's tools act on a session of this connection: none
            // outlives the sessions it was given.
            self.inner.ai.release_agents(connection);
            // Its sample grants, queued questions and live threads go too:
            // the front's `ai_forget` that follows finds nothing left to
            // forget once the registry let the assistant go.
            self.close_ai_conversation(connection);
            windows.release_assistant(window, connection);
        }
        if windows.release_connection(window, connection) {
            self.inner.ai.release_agents(connection);
            return self
                .run(CommandId::new(), Command::Disconnect { connection })
                .await;
        }
        Ok(CommandOutcome::Done)
    }

    /// Disconnects a connection a closed window was the last to hold.
    async fn disconnect_released(&self, connection: ConnectionId) {
        self.inner.ai.release_agents(connection);
        if let Err(error) = self
            .run(CommandId::new(), Command::Disconnect { connection })
            .await
        {
            tracing::warn!(error = %error.message, "a closed window's connection could not be closed");
        }
    }
}

#[cfg(test)]
mod tests;
