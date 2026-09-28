//! The registry of executions in progress, and the cancellation that goes
//! **all the way to the server**.
//!
//! `Esc` must really cancel. Abandoning the future on the client side frees
//! neither the connection, nor the lock held, nor the plan being executed: at
//! the tenth closed tab, the database refuses connections and the user
//! concludes that Oxyn broke their production
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md),
//! [I-13](../../../CLAUDE.md#i-13)).
//!
//! Cancellation is therefore done **in this order**, and the order is carried
//! by [`CancelRegistry::cancel`] rather than by its callers:
//!
//! 1. the execution's [`CancelToken`] is triggered — the draining loop hands
//!    back control at the next checkpoint, without waiting for the network;
//! 2. if — and only if — the session declares
//!    [`Capabilities::SERVER_SIDE_CANCEL`], cancellation is requested from the
//!    server (`pg_cancel_backend`, `KILL QUERY`, `sqlite3_interrupt`).
//!
//! The second step is conditional because a session that does not declare it
//! would return [`OxynError::NotSupported`](oxyn_core::OxynError): calling
//! anyway would produce an error in the log at every `Esc` on SQLite, and a
//! systematic error stops being read.
//!
//! # What the registry does not do
//!
//! It does not remove the entry on cancellation. It is the draining loop that
//! calls [`finish`](CancelRegistry::finish) when it has really handed back
//! control: `Esc` pressed twice must have no effect, not an "unknown
//! execution" error while it is still running.

use std::collections::HashMap;
use std::time::Instant;

use oxyn_core::{CancelToken, Capabilities, CommandId, ConnectionId, SessionId, StatementHandle};
use parking_lot::RwLock;

use crate::sessions::SessionRegistry;

/// An execution in progress, as the registry knows it.
///
/// Carries **neither** the query text **nor** its parameters: this registry is
/// consulted on the cancellation path, not the audit path, and duplicating the
/// text here would make it a second place it can leak from (I-03).
#[derive(Debug, Clone)]
pub struct RunningStatement {
    /// The handle minted by the driver, target of the server-side cancellation.
    pub statement: StatementHandle,
    /// The command that started it, to correlate with the log.
    pub command: CommandId,
    /// The target connection.
    pub connection: ConnectionId,
    /// The session it runs on.
    pub session: SessionId,
    /// What the session can do — this is where
    /// [`Capabilities::SERVER_SIDE_CANCEL`] is read.
    pub capabilities: Capabilities,
    /// When the execution was submitted.
    pub started_at: Instant,
    /// The token specific to this execution. Child of the caller's token:
    /// cancelling it does not cancel the tab.
    token: CancelToken,
}

impl RunningStatement {
    /// Records an execution that starts.
    #[must_use]
    pub fn new(
        statement: StatementHandle,
        command: CommandId,
        connection: ConnectionId,
        session: SessionId,
        capabilities: Capabilities,
        token: CancelToken,
    ) -> Self {
        Self {
            statement,
            command,
            connection,
            session,
            capabilities,
            started_at: Instant::now(),
            token,
        }
    }

    /// The token of this execution.
    #[must_use]
    pub fn token(&self) -> &CancelToken {
        &self.token
    }

    /// Can the session cancel server-side?
    #[must_use]
    pub const fn supports_server_cancel(&self) -> bool {
        self.capabilities.contains(Capabilities::SERVER_SIDE_CANCEL)
    }

    /// Was the execution already cancelled?
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// What was attempted server-side.
///
/// Distinguishes "not tried", "not possible" and "tried without success": the
/// three look alike on screen and do not call for the same conclusion. A query
/// the server could not interrupt is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServerCancel {
    /// Nothing was attempted: the execution was no longer in progress.
    NotAttempted,
    /// The session does not declare [`Capabilities::SERVER_SIDE_CANCEL`].
    ///
    /// It is not a failure: it is a driver limit, honestly declared. The query
    /// may continue server-side until it ends.
    Unsupported,
    /// The server accepted the interruption request.
    Requested,
    /// The server refused or did not answer.
    Failed,
}

impl ServerCancel {
    /// Was the interruption really requested from the server?
    #[must_use]
    pub const fn reached_server(&self) -> bool {
        matches!(self, Self::Requested)
    }
}

/// What a cancellation actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CancelReport {
    /// The target execution.
    pub statement: StatementHandle,
    /// Was it still in progress at the time of the request?
    pub was_running: bool,
    /// Was the client token triggered?
    pub client: bool,
    /// What was attempted server-side.
    pub server: ServerCancel,
}

/// The executions in progress, indexed by their handle.
///
/// The lock is a `RwLock`: reading is frequent — every cancellation, every tab
/// closing — and writing only happens at the start and end of an execution.
#[derive(Debug, Default)]
pub struct CancelRegistry {
    running: RwLock<HashMap<StatementHandle, RunningStatement>>,
}

impl CancelRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an execution that starts.
    pub fn register(&self, entry: RunningStatement) {
        self.running.write().insert(entry.statement, entry);
    }

    /// Removes a finished execution and returns what the registry knew of it.
    ///
    /// To be called when the draining loop has **really** handed back control,
    /// not when cancellation is requested.
    pub fn finish(&self, statement: StatementHandle) -> Option<RunningStatement> {
        self.running.write().remove(&statement)
    }

    /// What the registry knows about an execution.
    #[must_use]
    pub fn get(&self, statement: StatementHandle) -> Option<RunningStatement> {
        self.running.read().get(&statement).cloned()
    }

    /// The executions in progress on a connection.
    #[must_use]
    pub fn for_connection(&self, connection: ConnectionId) -> Vec<RunningStatement> {
        self.running
            .read()
            .values()
            .filter(|e| e.connection == connection)
            .cloned()
            .collect()
    }

    /// The executions in progress on a session.
    #[must_use]
    pub fn for_session(&self, session: SessionId) -> Vec<RunningStatement> {
        self.running
            .read()
            .values()
            .filter(|e| e.session == session)
            .cloned()
            .collect()
    }

    /// Number of executions in progress.
    #[must_use]
    pub fn len(&self) -> usize {
        self.running.read().len()
    }

    /// No execution in progress?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.running.read().is_empty()
    }

    /// Triggers an execution's token, **without** touching the server.
    ///
    /// Useful when the caller already knows the session is lost — disconnection,
    /// closing — and an interruption request would have nowhere to go. In every
    /// other case, [`cancel`](Self::cancel) is what to call: signalling only the
    /// client leaves the query running.
    pub fn cancel_client(&self, statement: StatementHandle) -> Option<RunningStatement> {
        let entry = self.running.read().get(&statement).cloned()?;
        entry.token.cancel();
        Some(entry)
    }

    /// Cancels an execution: client token first, server next.
    ///
    /// The order matters. The token hands back control at the next checkpoint,
    /// hence immediately from the user's point of view; the request to the
    /// server is a network round trip that can take time. Reversing them would
    /// make the interface wait for nothing.
    ///
    /// Cancelling an already finished execution is **not** an error: the report
    /// says so with `was_running: false`.
    pub async fn cancel(
        &self,
        sessions: &SessionRegistry,
        statement: StatementHandle,
    ) -> CancelReport {
        let Some(entry) = self.cancel_client(statement) else {
            return CancelReport {
                statement,
                was_running: false,
                client: false,
                server: ServerCancel::NotAttempted,
            };
        };

        let server = if entry.supports_server_cancel() {
            match sessions.get(entry.session) {
                Some(slot) => match slot.cancel_statement(statement).await {
                    Ok(()) => ServerCancel::Requested,
                    Err(err) => {
                        // Logged at `warn` level and not propagated: the caller
                        // just asked for a cancellation, returning an error would
                        // leave it nothing more to do.
                        tracing::warn!(
                            error = %err,
                            "server-side cancellation refused; the statement may still run"
                        );
                        ServerCancel::Failed
                    }
                },
                None => ServerCancel::Failed,
            }
        } else {
            ServerCancel::Unsupported
        };

        CancelReport {
            statement,
            was_running: true,
            client: true,
            server,
        }
    }

    /// Cancels every execution of a connection.
    ///
    /// It is what closing a connection does: without that, the queries started
    /// from its tabs would keep running server-side.
    pub async fn cancel_connection(
        &self,
        sessions: &SessionRegistry,
        connection: ConnectionId,
    ) -> Vec<CancelReport> {
        let mut reports = Vec::new();
        for entry in self.for_connection(connection) {
            reports.push(self.cancel(sessions, entry.statement).await);
        }
        reports
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_entry(capabilities: Capabilities) -> RunningStatement {
        RunningStatement::new(
            StatementHandle::new(),
            CommandId::new(),
            ConnectionId::new(),
            SessionId::new(),
            capabilities,
            CancelToken::new(),
        )
    }

    #[test]
    fn a_cancellation_triggers_the_execution_token() {
        let registry = CancelRegistry::new();
        let entry = running_entry(Capabilities::empty());
        let ct = entry.token().clone();
        let handle = entry.statement;
        registry.register(entry);

        assert!(!ct.is_cancelled());
        let cancelled = registry
            .cancel_client(handle)
            .expect("the execution is registered");
        assert!(ct.is_cancelled());
        assert!(cancelled.is_cancelled());
    }

    #[test]
    fn the_entry_survives_cancellation() {
        // `Esc` pressed twice must not produce "unknown execution" while it is
        // still running: it is the draining loop that removes.
        let registry = CancelRegistry::new();
        let entry = running_entry(Capabilities::empty());
        let handle = entry.statement;
        registry.register(entry);

        registry.cancel_client(handle);
        assert_eq!(registry.len(), 1);
        assert!(registry.cancel_client(handle).is_some());

        assert!(registry.finish(handle).is_some());
        assert!(registry.is_empty());
        assert!(registry.finish(handle).is_none());
    }

    #[test]
    fn cancelling_an_unknown_execution_is_not_an_error() {
        let registry = CancelRegistry::new();
        assert!(registry.cancel_client(StatementHandle::new()).is_none());
    }

    #[test]
    fn the_server_cancel_capability_is_read_on_the_session() {
        assert!(!running_entry(Capabilities::SQL).supports_server_cancel());
        assert!(
            running_entry(Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL)
                .supports_server_cancel()
        );
    }

    #[test]
    fn the_registry_finds_the_executions_of_a_connection() {
        let registry = CancelRegistry::new();
        let conn = ConnectionId::new();

        for _ in 0..3 {
            let mut e = running_entry(Capabilities::empty());
            e.connection = conn;
            registry.register(e);
        }
        registry.register(running_entry(Capabilities::empty()));

        assert_eq!(registry.for_connection(conn).len(), 3);
        assert_eq!(registry.len(), 4);
    }

    #[test]
    fn a_report_tells_the_three_server_outcomes_apart() {
        assert!(ServerCancel::Requested.reached_server());
        assert!(!ServerCancel::Unsupported.reached_server());
        assert!(!ServerCancel::Failed.reached_server());
        assert!(!ServerCancel::NotAttempted.reached_server());
    }
}
