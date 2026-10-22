//! What the registry knows of a connection besides the windows holding it:
//! the openings in flight, and a disconnection or a deletion under way
//! ([ADR-0043](../../../../../docs/adr/0043-multi-fenetre.md)).
//!
//! An opening creates its sessions before its window adopts them. Recorded
//! from before its first session until its adoption, it counts as holding the
//! connection: another window's cleanup does not disconnect it, and a
//! deletion is refused. The two decisions that end a connection — its last
//! holder letting it go, its deletion — mark it under the same lock as their
//! check, until their command has run: an opening waits for the
//! disconnection, and is refused by the deletion. Neither mark is held across
//! a human dialog: both are taken after it.

use oxyn_core::{CancelToken, CommandId, ConnectionId, SessionId};

use super::{State, WindowKey, WindowRegistry, closed};
use crate::ipc::IpcError;

/// A connection opening recorded for the window that asked, from before its
/// first session until the window adopts what it opened. Dropped unadopted —
/// the opening failed, or opened nothing —, it is forgotten.
#[must_use]
pub(crate) struct Opening<'a> {
    registry: &'a WindowRegistry,
    command: CommandId,
}

impl Opening<'_> {
    /// Records what the opening gave its window — the connection held and
    /// its sessions — and forgets the opening, under one lock: no decision
    /// sees the sessions neither opening nor held.
    ///
    /// # Errors
    /// The window closed while the connection opened: nothing is recorded,
    /// and the caller closes the sessions.
    pub(crate) fn adopt(
        self,
        connection: ConnectionId,
        sessions: &[SessionId],
    ) -> Result<(), IpcError> {
        let mut state = self.registry.state.lock();
        let window = state
            .openings
            .remove(&self.command)
            .map(|(window, _)| window);
        let Some(window) = window.filter(|window| state.window(*window).is_some()) else {
            return Err(closed());
        };
        state
            .connections
            .entry(window)
            .or_default()
            .insert(connection);
        for session in sessions {
            state.sessions.insert(*session, (window, connection));
        }
        Ok(())
    }
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        self.registry.state.lock().openings.remove(&self.command);
    }
}

/// A connection the last window holding it let go, marked until its
/// `Disconnect` has run: an opening started meanwhile would lose its
/// sessions to it.
#[must_use]
pub(crate) struct Disconnecting<'a> {
    registry: &'a WindowRegistry,
    connection: ConnectionId,
}

impl Disconnecting<'_> {
    /// The connection to disconnect.
    pub(crate) const fn connection(&self) -> ConnectionId {
        self.connection
    }
}

impl Drop for Disconnecting<'_> {
    fn drop(&mut self) {
        self.registry
            .state
            .lock()
            .disconnecting
            .remove(&self.connection);
        self.registry.disconnected.notify_waiters();
    }
}

/// A deletion being dispatched or approved, from its last check until its
/// command has run: an opening started meanwhile is refused.
#[must_use]
pub(crate) struct Deletion<'a> {
    registry: &'a WindowRegistry,
    connection: ConnectionId,
}

impl Drop for Deletion<'_> {
    fn drop(&mut self) {
        self.registry.state.lock().deleting.remove(&self.connection);
    }
}

/// The refusal of a deletion while another window holds the connection, or
/// opens it.
pub(crate) fn open_elsewhere() -> IpcError {
    IpcError::invalid("This connection is open in another window. Close it there first.")
}

impl WindowRegistry {
    /// Records that `window` opens a connection under `command`, before
    /// anything is opened. The connection is named by
    /// [`Self::bind_opening`], once the opening knows it.
    ///
    /// # Errors
    /// The window is gone, or `command` already opens.
    pub(crate) fn begin_opening(
        &self,
        window: WindowKey,
        command: CommandId,
    ) -> Result<Opening<'_>, IpcError> {
        let mut state = self.state.lock();
        if state.window(window).is_none() {
            return Err(closed());
        }
        if state.openings.contains_key(&command) {
            return Err(IpcError::invalid(
                "A command with this id is already running",
            ));
        }
        state.openings.insert(command, (window, None));
        Ok(Opening {
            registry: self,
            command,
        })
    }

    /// Names the connection the opening of `command` opens, before its first
    /// session: from here, it counts as holding it. Waits while the
    /// connection is being disconnected, until it is or `cancel` fires.
    ///
    /// An opening no window recorded — the backend's own tests', which no
    /// window adopts — is let through unrecorded.
    ///
    /// # Errors
    /// The opening's window closed, the connection is being deleted, or the
    /// opening was cancelled while it waited.
    pub(crate) async fn bind_opening(
        &self,
        command: CommandId,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<(), IpcError> {
        loop {
            let notified = self.disconnected.notified();
            tokio::pin!(notified);
            // Registered before the state is read: a `Disconnect` that ends in
            // between still wakes this opening.
            notified.as_mut().enable();
            {
                let mut state = self.state.lock();
                let Some((window, _)) = state.openings.get(&command).copied() else {
                    return Ok(());
                };
                if state.window(window).is_none() {
                    return Err(closed());
                }
                if state.deleting.contains(&connection) {
                    return Err(IpcError::invalid("This connection is being deleted."));
                }
                if !state.disconnecting.contains(&connection) {
                    state.openings.insert(command, (window, Some(connection)));
                    return Ok(());
                }
            }
            tokio::select! {
                () = &mut notified => {}
                () = cancel.cancelled() => {
                    return Err(IpcError::invalid("Opening the connection was cancelled."));
                }
            }
        }
    }

    /// Marks `connection` as being disconnected if no window holds it and no
    /// live window is opening it. `None` when one does, or when another
    /// release already disconnects it.
    #[must_use]
    pub(crate) fn disconnect_if_unused(
        &self,
        connection: ConnectionId,
    ) -> Option<Disconnecting<'_>> {
        self.mark_unused(&mut self.state.lock(), connection)
    }

    /// See [`Self::disconnect_if_unused`]: the caller holds the lock.
    pub(super) fn mark_unused(
        &self,
        state: &mut State,
        connection: ConnectionId,
    ) -> Option<Disconnecting<'_>> {
        if state.in_use(connection) || !state.disconnecting.insert(connection) {
            return None;
        }
        Some(Disconnecting {
            registry: self,
            connection,
        })
    }

    /// Checks that no other window holds `connection` and that no window is
    /// opening it, then marks its deletion under the same lock, until the
    /// guard drops: an opening cannot start between the check and the
    /// deletion. Taken under `settings.decisions`, so one deletion at a time.
    ///
    /// # Errors
    /// [`WindowRegistry::held_elsewhere`].
    pub(crate) fn begin_deletion(
        &self,
        command: CommandId,
        connection: ConnectionId,
    ) -> Result<Deletion<'_>, IpcError> {
        let mut state = self.state.lock();
        if state.held_elsewhere(command, connection) {
            return Err(open_elsewhere());
        }
        state.deleting.insert(connection);
        Ok(Deletion {
            registry: self,
            connection,
        })
    }
}

#[cfg(test)]
mod tests;
