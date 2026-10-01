//! The session's single connection, and the rule that keeps it honest.
//!
//! # One connection per session
//!
//! ADR-0050 §1: a session holds **one** `Conn`. A MySQL session carries state
//! the user sees — the current database, an open transaction, user variables —
//! and a pool would scatter it across connections. Every operation of the
//! session — execution, introspection, `begin`, `transaction_state` — takes the
//! connection in turn through an asynchronous mutex, which is also what orders
//! `transaction_state` after everything already submitted (ADR-0039): Tokio's
//! mutex is fair, so the caller queues behind the operations before it.
//!
//! # When the connection is closed
//!
//! A connection is closed, and reopened lazily on the next operation, when the
//! driver can no longer vouch for it: a transport error, a protocol error, or a
//! cancellation whose effect the driver could not observe (see `crate::cancel`).
//! If a transaction was open, it is gone with the connection — the server
//! rolls it back — and the session says so: its transaction state becomes
//! [`TransactionState::Unknown`](oxyn_core::TransactionState::Unknown), never
//! `Idle` as if nothing had happened.

use std::sync::{Arc, Mutex, PoisonError};

use futures::future::BoxFuture;
use mysql_async::consts::StatusFlags;
use mysql_async::prelude::Queryable as _;
use mysql_async::{Conn, DriverError, Error as MyError, Opts, Row, Statement, Value};
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementIntent, TransactionState};
use tokio::sync::OwnedMutexGuard;

use crate::cancel::{DrainVerdict, Killer};
use crate::error::{breaks_connection, is_query_interrupted, map_connect_error, map_exec_error};
use crate::options::ConnectSpec;

/// A `TIMESTAMP` arrives in UTC (ADR-0050 §6). A literal.
pub(crate) const SQL_SET_TIME_ZONE: &str = "SET time_zone = '+00:00'";
/// Text arrives in `utf8mb4`, whatever the server's default. A literal.
pub(crate) const SQL_SET_NAMES: &str = "SET NAMES utf8mb4";
/// The connection's real id, which the handshake truncates to 32 bits.
const SQL_CONNECTION_ID: &str = "SELECT CONNECTION_ID()";
/// The server's identity.
pub(crate) const SQL_VERSION: &str = "SELECT VERSION()";

/// Opens a connection with the session setup applied.
///
/// # Errors
/// Those of [`connect_bare`], then any error of the setup statements.
pub(crate) async fn open(spec: &ConnectSpec, driver: &DriverId) -> Result<(Conn, u64)> {
    let mut conn = connect_bare(spec).await?;
    let setup = async {
        conn.query_drop(SQL_SET_TIME_ZONE).await?;
        conn.query_drop(SQL_SET_NAMES).await?;
        first_value(&mut conn, SQL_CONNECTION_ID).await
    }
    .await;
    match setup {
        Ok(value) => value
            .as_ref()
            .and_then(value_u64)
            .map(|id| (conn, id))
            .ok_or_else(|| {
                OxynError::Connection("the server did not return its connection id".to_owned())
            }),
        Err(error) => Err(map_exec_error(driver, StatementIntent::Read, &error)),
    }
}

/// The first value of the first row of a composed query.
///
/// Through `Row`, whose conversion cannot fail: the typed helpers of
/// `mysql_async` (`query_first::<u64, _>`) go through `FromRow`, which
/// **panics** when the server sends something else — server input, on a path
/// that must not panic ([I-09](../../../CLAUDE.md#i-09)).
pub(crate) async fn first_value(
    conn: &mut Conn,
    sql: &str,
) -> std::result::Result<Option<Value>, MyError> {
    let row = conn.query_first::<Row, _>(sql).await?;
    Ok(row.and_then(|row| row.unwrap_raw().into_iter().next().flatten()))
}

/// An unsigned integer from either protocol's form of a value.
pub(crate) fn value_u64(value: &Value) -> Option<u64> {
    match value {
        Value::UInt(v) => Some(*v),
        Value::Int(v) => u64::try_from(*v).ok(),
        Value::Bytes(bytes) => std::str::from_utf8(bytes).ok()?.trim().parse().ok(),
        _ => None,
    }
}

/// Text from a value, or `None` when it is not valid UTF-8 text.
pub(crate) fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Bytes(bytes) => String::from_utf8(bytes.clone()).ok(),
        Value::Int(v) => Some(v.to_string()),
        Value::UInt(v) => Some(v.to_string()),
        _ => None,
    }
}

/// Opens a connection without setup: the one `KILL QUERY` goes out on.
///
/// In `prefer` mode, a server that offers no TLS gets a second, clear-text
/// attempt: that is what the mode means, and it is only accepted on a local
/// connection ([`ConnectSpec`]). It is the negotiation of one connection, not
/// the retry of an operation.
///
/// # Errors
/// [`OxynError::Connection`], [`OxynError::Authentication`] or
/// [`OxynError::Config`], as `map_connect_error` classifies them.
pub(crate) async fn connect_bare(spec: &ConnectSpec) -> Result<Conn> {
    let (primary, fallback) = spec.attempts();
    match Conn::new(Opts::clone(primary)).await {
        Ok(conn) => Ok(conn),
        Err(MyError::Driver(DriverError::NoClientSslFlagFromServer)) => match fallback {
            Some(clear_text) => Conn::new(Opts::clone(clear_text))
                .await
                .map_err(|error| map_connect_error(&error)),
            None => Err(map_connect_error(&MyError::Driver(
                DriverError::NoClientSslFlagFromServer,
            ))),
        },
        Err(error) => Err(map_connect_error(&error)),
    }
}

/// Races a future against cancellation; the token wins ties.
pub(crate) async fn race_cancel<T, F>(cancel: &CancelToken, operation: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(OxynError::Cancelled),
        outcome = operation => outcome,
    }
}

/// The connection and what the driver knows about it.
///
/// No derived `Debug`: `Conn`'s own prints its options, password included
/// ([I-03](../../../CLAUDE.md#i-03)).
pub(crate) struct Link {
    conn: Option<Conn>,
    /// The id `KILL QUERY` targets, read from the server at opening.
    id: u64,
    /// What the server last said about the transaction.
    tx: TransactionState,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("open", &self.conn.is_some())
            .field("tx", &self.tx)
            .finish_non_exhaustive()
    }
}

impl Link {
    /// A link around a connection just opened: no transaction yet.
    pub(crate) const fn new(conn: Conn, id: u64) -> Self {
        Self {
            conn: Some(conn),
            id,
            tx: TransactionState::Idle,
        }
    }

    /// The connection, if one is open and alive.
    pub(crate) fn conn(&mut self) -> Option<&mut Conn> {
        self.conn.as_mut().filter(|conn| !conn.is_disconnected())
    }

    /// The id of the server thread of the open connection.
    pub(crate) const fn id(&self) -> u64 {
        self.id
    }

    /// The transaction state as last observed.
    pub(crate) const fn transaction(&self) -> TransactionState {
        self.tx
    }

    /// Reads `SERVER_STATUS_IN_TRANS` from the last OK packet (ADR-0050 §8).
    ///
    /// After an error the library forgets the last OK packet: the state is
    /// then unknown until [`Self::refresh`].
    pub(crate) fn record_status(&mut self) {
        self.tx = match self.conn.as_ref().and_then(Conn::last_ok_packet) {
            Some(ok)
                if ok
                    .status_flags()
                    .contains(StatusFlags::SERVER_STATUS_IN_TRANS) =>
            {
                TransactionState::Open
            }
            Some(_) => TransactionState::Idle,
            None => TransactionState::Unknown,
        };
    }

    /// Puts back the state observed before an exchange whose answer the
    /// library misreads as an OK packet: see [`Shared::prepare`].
    pub(crate) const fn keep_transaction(&mut self, state: TransactionState) {
        self.tx = state;
    }

    /// Asks the server for its status with `COM_PING`, whose OK packet carries
    /// the transaction flag. A connection that does not answer is closed.
    pub(crate) async fn refresh(&mut self) {
        let Some(conn) = self.conn() else {
            return;
        };
        match conn.ping().await {
            Ok(()) => self.record_status(),
            Err(_) => self.discard(),
        }
    }

    /// Forgets the connection: the library drains and closes it in the
    /// background. An open transaction is lost with it, and the state says so.
    pub(crate) fn discard(&mut self) {
        if self.conn.take().is_some() && self.tx != TransactionState::Idle {
            self.tx = TransactionState::Unknown;
        }
    }

    /// Gives up the connection **without draining it**.
    ///
    /// For a result the library can only drain by panicking — a binary result
    /// set announcing a type internal to the server (see
    /// [`crate::types::panics_in_binary_protocol`]): `disconnect` and the
    /// cleanup `Drop` spawns both read the pending rows first. The connection
    /// is leaked instead; the server closes its end after
    /// `net_write_timeout`. It only happens against a server that breaks the
    /// protocol.
    pub(crate) fn abandon(&mut self) {
        if let Some(conn) = self.conn.take() {
            std::mem::forget(conn);
        }
        if self.tx != TransactionState::Idle {
            self.tx = TransactionState::Unknown;
        }
    }

    /// Closes a prepared statement, or the connection when that fails.
    ///
    /// With the statement cache disabled, closing is the driver's job
    /// (ADR-0050 §2): a statement left open counts against the server's
    /// `max_prepared_stmt_count` for the life of the connection. A connection
    /// that cannot close it is not trusted with the next statement, and
    /// discarding it frees the statement with it.
    pub(crate) async fn close_statement(&mut self, statement: Statement) {
        let Some(conn) = self.conn() else {
            return;
        };
        if conn.close(statement).await.is_err() {
            self.discard();
        }
    }

    /// Says goodbye to the server (`COM_QUIT`) and releases the connection.
    ///
    /// # Errors
    /// [`OxynError::Connection`] with the transport's reason; the connection is
    /// released in every case.
    pub(crate) async fn close(&mut self) -> Result<()> {
        let Some(conn) = self.conn.take() else {
            return Ok(());
        };
        conn.disconnect()
            .await
            .map_err(|error| OxynError::Connection(error.to_string()))
    }

    /// After an error on this connection: close it if the error broke it,
    /// otherwise learn what the error did to the transaction.
    pub(crate) async fn after_error(&mut self, error: &MyError) {
        if breaks_connection(error) {
            self.discard();
        } else {
            self.refresh().await;
        }
    }

    /// Puts a freshly opened connection in place of a closed one.
    ///
    /// The transaction state is kept: if it says a transaction was lost, the
    /// user learns it from the next statement's status, not from a silent
    /// `Idle`.
    fn replace(&mut self, conn: Conn, id: u64) {
        self.conn = Some(conn);
        self.id = id;
    }
}

/// What the session and its catalog share: the connection and how to reopen
/// it.
pub(crate) struct Shared {
    pub(crate) driver: DriverId,
    pub(crate) spec: ConnectSpec,
    link: Arc<tokio::sync::Mutex<Link>>,
    /// The database a context declared, applied again to every reopened
    /// connection. Under a synchronous lock, never held across an `await`.
    database: Mutex<Option<String>>,
    pub(crate) killer: Arc<Killer>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("driver", &self.driver)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

/// The connection, taken for one operation.
pub(crate) type Lease = OwnedMutexGuard<Link>;

impl Shared {
    pub(crate) fn new(
        driver: DriverId,
        spec: ConnectSpec,
        link: Link,
        killer: Arc<Killer>,
    ) -> Self {
        Self {
            driver,
            spec,
            link: Arc::new(tokio::sync::Mutex::new(link)),
            database: Mutex::new(None),
            killer,
        }
    }

    /// Takes the connection, reopening it if it was closed.
    ///
    /// # Errors
    /// [`OxynError::Cancelled`] if `cancel` fires while waiting or opening,
    /// and the errors of [`open`] or of the context's `USE`.
    pub(crate) async fn lease(&self, cancel: &CancelToken) -> Result<Lease> {
        let mut lease = race_cancel(cancel, async {
            Ok(Arc::clone(&self.link).lock_owned().await)
        })
        .await?;
        if lease.conn().is_none() {
            let (conn, id) = race_cancel(cancel, open(&self.spec, &self.driver)).await?;
            lease.replace(conn, id);
            let database = self.database();
            if let Some(database) = database {
                let statement = use_statement(&database);
                let outcome = race_cancel(cancel, async {
                    match lease.conn() {
                        Some(conn) => conn.query_drop(statement).await.map_err(|error| {
                            map_exec_error(&self.driver, StatementIntent::Read, &error)
                        }),
                        None => Err(OxynError::Connection("the connection closed".to_owned())),
                    }
                })
                .await;
                if outcome.is_err() {
                    lease.discard();
                }
                outcome?;
            }
        }
        Ok(lease)
    }

    /// Takes the connection as it is, without reopening it: for what must not
    /// cost a round trip — the transaction state — or must not open anything —
    /// closing.
    pub(crate) async fn lock(&self) -> Lease {
        Arc::clone(&self.link).lock_owned().await
    }

    /// The database a context declared.
    pub(crate) fn database(&self) -> Option<String> {
        self.database
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Records the database the server confirmed.
    pub(crate) fn set_database(&self, database: Option<String>) {
        *self.database.lock().unwrap_or_else(PoisonError::into_inner) = database;
    }

    /// Runs statements the driver composed, cancellable on the server.
    ///
    /// When `cancel` fires, the operation is **not** dropped: `KILL QUERY` goes
    /// out from a second connection, and the operation is awaited until the
    /// server confirms the interruption (1317). Only then is the connection
    /// kept — with the user's transaction. If the operation ended otherwise,
    /// or not within the drain bound, the kill cannot be proven consumed and
    /// the connection is closed (ADR-0050 §7).
    ///
    /// # Errors
    /// [`OxynError::Cancelled`] when cancelled, [`OxynError::Connection`] if
    /// the connection is closed, or the server's error, classified with
    /// `intent`.
    pub(crate) async fn run<T: Send>(
        &self,
        lease: &mut Lease,
        cancel: &CancelToken,
        intent: StatementIntent,
        operation: impl for<'c> FnOnce(&'c mut Conn) -> BoxFuture<'c, std::result::Result<T, MyError>>,
    ) -> Result<T> {
        self.run_raw(lease, cancel, operation)
            .await?
            .map_err(|error| map_exec_error(&self.driver, intent, &error))
    }

    /// [`Self::run`], handing the server's error back untranslated: for the
    /// caller that decides on it — the prepare, whose 1295 opens the fallback.
    ///
    /// # Errors
    /// The outer error is the cancellation or a closed connection; the inner
    /// one is the server's, already acted upon for the connection.
    pub(crate) async fn run_raw<T: Send>(
        &self,
        lease: &mut Lease,
        cancel: &CancelToken,
        operation: impl for<'c> FnOnce(&'c mut Conn) -> BoxFuture<'c, std::result::Result<T, MyError>>,
    ) -> Result<std::result::Result<T, MyError>> {
        let id = lease.id();
        let outcome = {
            let Some(conn) = lease.conn() else {
                return Err(OxynError::Connection(
                    "the connection to the server is closed".to_owned(),
                ));
            };
            let mut pending = operation(conn);
            // The operation is polled first: its first poll sends the command,
            // so a kill decided now reaches it rather than an idle connection.
            tokio::select! {
                biased;
                outcome = &mut pending => Interrupted::Finished(outcome),
                () = cancel.cancelled() => Interrupted::Cancelled,
            }
            .settle(&self.killer, id, pending)
            .await
        };
        match outcome {
            Settled::Done(Ok(value)) => {
                lease.record_status();
                Ok(Ok(value))
            }
            Settled::Done(Err(error)) => {
                lease.after_error(&error).await;
                Ok(Err(error))
            }
            Settled::Cancelled(DrainVerdict::Consumed) => {
                lease.refresh().await;
                Err(OxynError::Cancelled)
            }
            Settled::Cancelled(DrainVerdict::Unproven) => {
                // The kill may have been lost: end the connection on the
                // server, which ends its statement too.
                let _ = self.killer.kill_connection(id).await;
                lease.discard();
                Err(OxynError::Cancelled)
            }
        }
    }

    /// Prepares `sql` as [`Self::run_raw`] runs any operation, without the
    /// transaction state learning anything from the answer.
    ///
    /// The library records the answer to `COM_STMT_PREPARE` as an OK packet
    /// and reads status flags from the statement's own fields: inside an open
    /// transaction they say none is open (observed against MySQL 8.4 on
    /// 2026-10-01). A prepare neither opens nor ends a transaction, so the
    /// state observed before it stands — otherwise the loss of the
    /// connection in the statement that follows would be reported as if no
    /// transaction had been open.
    ///
    /// # Errors
    /// Those of [`Self::run_raw`].
    pub(crate) async fn prepare(
        &self,
        lease: &mut Lease,
        cancel: &CancelToken,
        sql: String,
    ) -> Result<std::result::Result<Statement, MyError>> {
        let before = lease.transaction();
        let prepared = self
            .run_raw(lease, cancel, move |conn| Box::pin(conn.prep(sql)))
            .await;
        if matches!(prepared, Ok(Ok(_))) {
            lease.keep_transaction(before);
        }
        prepared
    }
}

/// How the race between an operation and its token ended.
enum Interrupted<T> {
    Finished(std::result::Result<T, MyError>),
    Cancelled,
}

/// What became of the operation once a cancellation, if any, was handled.
enum Settled<T> {
    Done(std::result::Result<T, MyError>),
    Cancelled(DrainVerdict),
}

impl<T> Interrupted<T> {
    async fn settle(
        self,
        killer: &Killer,
        id: u64,
        pending: BoxFuture<'_, std::result::Result<T, MyError>>,
    ) -> Settled<T> {
        match self {
            Self::Finished(outcome) => Settled::Done(outcome),
            Self::Cancelled => {
                if killer.kill_query(id).await.is_err() {
                    return Settled::Cancelled(DrainVerdict::Unproven);
                }
                let verdict = match tokio::time::timeout(crate::cancel::DRAIN_BOUND, pending).await
                {
                    Ok(Err(error)) if is_query_interrupted(&error) => DrainVerdict::Consumed,
                    _ => DrainVerdict::Unproven,
                };
                Settled::Cancelled(verdict)
            }
        }
    }
}

/// `USE` a database, the name quoted by the driver ([I-10](../../../CLAUDE.md#i-10)).
pub(crate) fn use_statement(database: &str) -> String {
    format!("USE {}", quote_identifier(database, QuoteStyle::Backtick))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_name_is_quoted_never_concatenated() {
        assert_eq!(use_statement("shop"), "USE `shop`");
        assert_eq!(
            use_statement("x`; DROP TABLE audit; --"),
            "USE `x``; DROP TABLE audit; --`"
        );
    }

    #[test]
    fn composed_setup_statements_are_literals() {
        for sql in [
            SQL_SET_TIME_ZONE,
            SQL_SET_NAMES,
            SQL_CONNECTION_ID,
            SQL_VERSION,
        ] {
            assert!(!sql.contains('{'), "{sql}");
        }
    }
}
