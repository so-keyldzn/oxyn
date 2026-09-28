//! Open sessions, and the resolution of the credentials that open them.
//!
//! A driver [`Session`] is `Send + Sync` and is used through `&self`: it can
//! therefore serve several executions at once. What it does not support is
//! being closed while in use — [`Session::close`] consumes the session. Hence
//! [`SessionSlot`]: an **asynchronous** read-write lock around a possibly
//! closed session.
//!
//! The lock is `tokio`'s and not `parking_lot`'s because its guard crosses an
//! `await`: `execute` is held for reading during the whole server round trip.
//! A `parking_lot` guard is not `Send`, and an execution that blocks a runtime
//! thread for thirty seconds is exactly what the threading model forbids
//! (ARCHITECTURE §9).
//!
//! # What is not here
//!
//! **The keychain.** `oxyn-exec` does not depend on `oxyn-secrets`: credentials
//! arrive through [`CredentialResolver`], which `oxyn-desktop` wires to the
//! system keychain. It is not a speculative abstraction — it is the boundary
//! that keeps the executor from reading a password itself, and that makes
//! tests possible without a keychain.
//!
//! Catalog reads keep the session read guard until provider completion.
//! Cancellation reaches the provider token, including during disconnect.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, ConnectionId, ExecRequest, OxynError, Result,
    SessionId, StatementHandle, TransactionState,
};
use oxyn_driver::{Credentials, Cursor, Session};
use parking_lot::RwLock;
use tokio::sync::RwLock as AsyncRwLock;

/// What knows how to find a connection's credentials.
///
/// Implemented by `oxyn-desktop` on top of `oxyn-secrets`. The method is
/// **synchronous**: system keychains are, and pretending otherwise would hide
/// that it blocks.
///
/// The returned value never goes through a log or a trace:
/// [`Credentials`] masks its content in `Debug` (I-03).
pub trait CredentialResolver: Send + Sync {
    /// Resolves a connection's credentials.
    ///
    /// `config` carries only a
    /// [`secret_ref`](oxyn_core::ConnectionConfig::secret_ref); it is the one
    /// that is resolved.
    ///
    /// # Errors
    /// [`OxynError::Authentication`] if the keychain refuses or does not know
    /// the reference, [`OxynError::Config`] if the reference is unreadable.
    fn resolve(&self, config: &ConnectionConfig) -> Result<Credentials>;

    /// Name of the resolver, for traces.
    fn name(&self) -> &'static str {
        "credentials"
    }
}

/// The resolver that resolves nothing.
///
/// Returns **empty** credentials, which is the truth for SQLite and for any
/// connection without a secret. It is not a stub that pretends: a driver that
/// needs a password will receive an empty `Credentials` and return
/// [`OxynError::Authentication`], which is the right answer as long as the
/// keychain is not wired.
#[derive(Clone, Copy, Default)]
pub struct NoCredentials;

impl CredentialResolver for NoCredentials {
    fn resolve(&self, _config: &ConnectionConfig) -> Result<Credentials> {
        Ok(Credentials::new())
    }

    fn name(&self) -> &'static str {
        "no-credentials"
    }
}

impl fmt::Debug for NoCredentials {
    /// Written by hand, like everything that touches credentials: a derived
    /// `Debug` on this family of types is what leaks six months later, when
    /// someone adds a `tracing::debug!` (I-03). The type carries nothing
    /// today; the rule holds for the day it will carry something.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoCredentials")
    }
}

/// An open session, shareable and closable.
///
/// Handled through `Arc`: the executor keeps one copy in its registry while an
/// execution holds another.
pub struct SessionSlot {
    id: SessionId,
    connection: ConnectionId,
    capabilities: Capabilities,
    opened_at: Instant,
    session: AsyncRwLock<Option<Box<dyn Session>>>,
    closing: CancelToken,
}

impl SessionSlot {
    /// Stores a freshly opened session.
    ///
    /// Capabilities are **read at opening**: they are set by the server version
    /// and the account's rights, which do not change during the session's life
    /// (ADR-0003).
    #[must_use]
    pub fn new(connection: ConnectionId, session: Box<dyn Session>) -> Self {
        let capabilities = session.capabilities();
        Self {
            id: SessionId::new(),
            connection,
            capabilities,
            opened_at: Instant::now(),
            session: AsyncRwLock::new(Some(session)),
            closing: CancelToken::new(),
        }
    }

    /// The session's identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// The connection it comes from.
    #[must_use]
    pub const fn connection(&self) -> ConnectionId {
        self.connection
    }

    /// What **this** session can do.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// How long it has been open.
    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.opened_at.elapsed()
    }

    /// Is the session still open?
    ///
    /// Instant answer when the lock is free; `false` out of caution if a
    /// closing is in progress — a session being closed must not be entrusted
    /// with an execution.
    #[must_use]
    pub fn is_open(&self) -> bool {
        if self.closing.is_cancelled() {
            return false;
        }
        // `tokio::sync::RwLock::try_read` returns a `Result`: failure means
        // "a writer holds the lock", that is, a closing in progress.
        self.session.try_read().is_ok_and(|g| g.is_some())
    }

    /// Executes a request on this session.
    ///
    /// # Errors
    /// [`OxynError::Connection`] if the session was closed, and any error
    /// returned by the driver.
    pub async fn execute(
        &self,
        request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        if self.closing.is_cancelled() {
            return Err(session_closed());
        }
        let guard = tokio::select! {
            biased;
            _ = self.closing.cancelled() => return Err(session_closed()),
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            guard = self.session.read() => guard,
        };
        let Some(session) = guard.as_deref() else {
            return Err(session_closed());
        };
        session.execute(request, cancel).await
    }

    /// Compose a preview while retaining the open session; lock waiting is cancellable.
    pub(crate) async fn preview_request(
        &self,
        path: &oxyn_catalog::CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        let token = cancel.child();
        let guard = tokio::select! {
            biased;
            _ = self.closing.cancelled() => return Err(session_closed()),
            _ = token.cancelled() => return Err(OxynError::Cancelled),
            guard = self.session.read() => guard,
        };
        let session = guard.as_deref().ok_or_else(session_closed)?;
        let operation = session.preview_request(path, limit, shape, &token);
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => result,
            _ = self.closing.cancelled() => { token.cancel(); operation.await }
        }
    }

    /// Declares where this session resolves unqualified names.
    ///
    /// Waits for the server to confirm, like every other operation that crosses
    /// the boundary: the interface shows what came back, never what was asked
    /// ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)).
    pub(crate) async fn set_context(
        &self,
        context: &oxyn_driver::SessionContext,
        cancel: &CancelToken,
    ) -> Result<()> {
        let token = cancel.child();
        let guard = tokio::select! {
            biased;
            _ = self.closing.cancelled() => return Err(session_closed()),
            _ = token.cancelled() => return Err(OxynError::Cancelled),
            guard = self.session.read() => guard,
        };
        let session = guard.as_deref().ok_or_else(session_closed)?;
        let operation = session.set_context(context, &token);
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => result,
            _ = self.closing.cancelled() => { token.cancel(); operation.await }
        }
    }

    /// What the session reports as its context, if it reports one.
    ///
    /// Read under the lock and cloned: the caller must not hold a borrow into
    /// the session while awaiting anything else.
    pub(crate) async fn context(
        &self,
        cancel: &CancelToken,
    ) -> Result<Option<oxyn_driver::SessionContext>> {
        let guard = tokio::select! {
            biased;
            _ = self.closing.cancelled() => return Err(session_closed()),
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            guard = self.session.read() => guard,
        };
        let session = guard.as_deref().ok_or_else(session_closed)?;
        Ok(session.context())
    }

    /// The transaction state once every operation already submitted has ended.
    ///
    /// Takes no token from the caller: an execution's own token, or its tab's,
    /// has already fired after a Stop or a timeout, and would make every read
    /// after one `Unknown`. The only bound is this session closing
    /// ([ADR-0039](../../../docs/adr/0039-etat-de-transaction-d-une-session.md)).
    /// A session closing, or closed, reports `Unknown` — never `Idle`.
    pub async fn transaction_state(&self) -> TransactionState {
        let token = self.closing.child();
        let guard = tokio::select! {
            biased;
            _ = token.cancelled() => return TransactionState::Unknown,
            guard = self.session.read() => guard,
        };
        let Some(session) = guard.as_deref() else {
            return TransactionState::Unknown;
        };
        session.transaction_state(&token).await
    }

    /// Keeps the provider borrowed until cancellation cleanup finishes.
    pub(crate) async fn read_catalog(
        &self,
        scope: &oxyn_core::CatalogRefreshScope,
        cancel: &CancelToken,
    ) -> Result<crate::catalog::CatalogPatch> {
        let token = cancel.child();
        let guard = tokio::select! {
            biased;
            _ = self.closing.cancelled() => return Err(session_closed()),
            _ = token.cancelled() => return Err(OxynError::Cancelled),
            guard = self.session.read() => guard,
        };
        let session = guard.as_deref().ok_or_else(session_closed)?;
        let operation = crate::catalog::read(session.catalog(), self.capabilities, scope, &token);
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => result,
            _ = self.closing.cancelled() => { token.cancel(); operation.await }
        }
    }

    /// Asks the server to interrupt an execution.
    ///
    /// Only makes sense if the session declares
    /// [`Capabilities::SERVER_SIDE_CANCEL`]; it is
    /// [`CancelRegistry::cancel`](crate::CancelRegistry::cancel) that does this
    /// check, so that it exists in one place only.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the driver cannot cancel server-side, and
    /// any transport error.
    pub async fn cancel_statement(&self, statement: StatementHandle) -> Result<()> {
        let guard = self.session.read().await;
        let Some(session) = guard.as_deref() else {
            // A closed session released its queries: there is nothing left to
            // interrupt, and saying so as an error would make noise at every
            // tab closing.
            return Ok(());
        };
        session.cancel(statement).await
    }

    /// Checks that the connection is alive.
    ///
    /// # Errors
    /// [`OxynError::Connection`] if the session was closed, and any transport
    /// error.
    pub async fn ping(&self) -> Result<Duration> {
        let guard = self.session.read().await;
        let Some(session) = guard.as_deref() else {
            return Err(session_closed());
        };
        session.ping().await
    }

    /// Marks the session as closing before waiting for its provider lock.
    pub(crate) fn begin_close(&self) {
        self.closing.cancel();
    }

    /// Includes preparation and cursor draining, before a statement handle exists.
    pub(crate) fn closing_token(&self) -> &CancelToken {
        &self.closing
    }

    /// Closes the session.
    ///
    /// Idempotent: closing twice is not an error. Local resources are released
    /// in every case, including if the server refuses the closing.
    ///
    /// # Errors
    /// Any transport error met while closing.
    pub async fn close(&self) -> Result<()> {
        self.begin_close();
        let session = { self.session.write().await.take() };
        match session {
            Some(session) => session.close().await,
            None => Ok(()),
        }
    }
}

impl fmt::Debug for SessionSlot {
    /// Returns neither the session nor what it carries: a driver is not bound
    /// to have a secret-free `Debug`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionSlot")
            .field("id", &self.id)
            .field("connection", &self.connection)
            .field("capabilities", &self.capabilities)
            .field("open", &self.is_open())
            .finish_non_exhaustive()
    }
}

/// The error of a session used after being closed.
///
/// Classified [`Connection`](OxynError::Connection), hence **transient**: the
/// right follow-up is to reopen, which the interface knows how to do.
fn session_closed() -> OxynError {
    OxynError::Connection("the session has been closed".to_owned())
}

/// The executor's open sessions.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: RwLock<HashMap<SessionId, Arc<SessionSlot>>>,
}

impl SessionRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores an open session and returns its shared handle.
    pub fn insert(&self, slot: SessionSlot) -> Arc<SessionSlot> {
        let slot = Arc::new(slot);
        self.sessions.write().insert(slot.id(), Arc::clone(&slot));
        slot
    }

    /// Finds a session.
    #[must_use]
    pub fn get(&self, id: SessionId) -> Option<Arc<SessionSlot>> {
        self.sessions.read().get(&id).map(Arc::clone)
    }

    /// Removes a session from the registry **without closing it**.
    pub fn remove(&self, id: SessionId) -> Option<Arc<SessionSlot>> {
        self.sessions.write().remove(&id)
    }

    /// The sessions open on a connection.
    #[must_use]
    pub fn for_connection(&self, connection: ConnectionId) -> Vec<Arc<SessionSlot>> {
        self.sessions
            .read()
            .values()
            .filter(|s| s.connection() == connection)
            .map(Arc::clone)
            .collect()
    }

    /// Removes and returns every session of a connection.
    ///
    /// Remove **before** closing: a session being closed must no longer be
    /// entrusted with an execution.
    pub fn drain_connection(&self, connection: ConnectionId) -> Vec<Arc<SessionSlot>> {
        let mut guard = self.sessions.write();
        let targeted: Vec<SessionId> = guard
            .values()
            .filter(|s| s.connection() == connection)
            .map(|s| s.id())
            .collect();
        targeted
            .into_iter()
            .filter_map(|id| guard.remove(&id))
            .collect()
    }

    /// Removes and returns every session.
    pub fn drain_all(&self) -> Vec<Arc<SessionSlot>> {
        self.sessions.write().drain().map(|(_, s)| s).collect()
    }

    /// Number of open sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.read().len()
    }

    /// No open session?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::DriverId;

    #[test]
    fn the_empty_resolver_claims_nothing() {
        let config = ConnectionConfig::new("atelier", DriverId::sqlite());
        let credentials = NoCredentials
            .resolve(&config)
            .expect("the empty resolver never fails");
        assert!(credentials.is_empty());
        assert_eq!(NoCredentials.name(), "no-credentials");
    }

    #[test]
    fn an_empty_registry_finds_nothing() {
        let registry = SessionRegistry::new();
        assert!(registry.is_empty());
        assert!(registry.get(SessionId::new()).is_none());
        assert!(registry.drain_connection(ConnectionId::new()).is_empty());
    }

    #[test]
    fn catalog_waiting_for_session_lock_is_cancellable() {
        use crate::executor::catalog_tests::{FakeSession, Probe};
        use futures::{FutureExt, executor::block_on};
        let slot = SessionSlot::new(
            ConnectionId::new(),
            Box::new(FakeSession(Arc::new(Probe::default()))),
        );
        let guard = block_on(slot.session.write());
        let token = CancelToken::new();
        let read = slot.read_catalog(&oxyn_core::CatalogRefreshScope::Root, &token);
        futures::pin_mut!(read);
        assert!(read.as_mut().now_or_never().is_none());
        token.cancel();
        assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
        drop(guard);
    }

    #[test]
    fn preview_waiting_for_session_lock_is_cancellable() {
        use crate::executor::catalog_tests::{FakeSession, Probe};
        use futures::{FutureExt, executor::block_on};
        let slot = SessionSlot::new(
            ConnectionId::new(),
            Box::new(FakeSession(Arc::new(Probe::default()))),
        );
        let guard = block_on(slot.session.write());
        let token = CancelToken::new();
        let path = oxyn_catalog::CatalogPath::for_relation(None, None, "t").expect("valid path");
        let simple = oxyn_core::PreviewShape::unordered();
        let read = slot.preview_request(&path, 200, &simple, &token);
        futures::pin_mut!(read);
        assert!(read.as_mut().now_or_never().is_none());
        token.cancel();
        assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
        drop(guard);
        assert!(matches!(
            block_on(slot.preview_request(
                &path,
                200,
                &oxyn_core::PreviewShape::unordered(),
                &CancelToken::new()
            )),
            Err(OxynError::NotSupported { .. })
        ));
        block_on(slot.close()).expect("closed session");
        assert!(matches!(
            block_on(slot.preview_request(
                &path,
                200,
                &oxyn_core::PreviewShape::unordered(),
                &CancelToken::new()
            )),
            Err(OxynError::Connection(_))
        ));
    }

    #[test]
    fn a_closed_session_is_a_transient_error() {
        // The right follow-up is to reopen: the interface must be able to offer
        // it without parsing the message.
        let error = session_closed();
        assert!(error.is_retryable(), "{error:?}");
    }
}
