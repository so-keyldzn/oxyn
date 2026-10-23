//! An open DuckDB session, and what it can do.
//!
//! # Cancellation is the engine's, and it is not server-side
//!
//! DuckDB runs in Oxyn's process. `duckdb_interrupt` stops a running statement
//! — an aggregate in the middle of a chunk included — but nothing remote is
//! freed because nothing is remote. The session therefore does not declare
//! [`Capabilities::SERVER_SIDE_CANCEL`], which the interface reads as "the
//! Cancel button reaches a server", and [`Session::cancel`] refuses. The
//! interruption goes through the [`CancelToken`] handed to `execute`, watched
//! by the cursor ([`crate::cursor`]).
//!
//! # What the session changes behind the user's back
//!
//! Nothing a statement can see, except what the access policy sets once, at
//! opening, for every session of the file ([`crate::open`]):
//! `enable_external_access = false`, the optional `allowed_directories`, and
//! extension autoloading off. [`Session::set_context`] runs a quoted `USE` and
//! reports what the engine then says; a `USE` typed in a console is refused,
//! so the context shown is always the engine's.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use duckdb::Connection;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, OxynError, PreviewShape, Result, StatementHandle,
    TransactionState,
};
use oxyn_driver::{Cursor, Session, SessionContext};
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use crate::catalog::DuckDbCatalog;
use crate::cursor::{DuckDbCursor, Opened};
use crate::error::{self, Effect};
use crate::options::BatchLimits;
use crate::query;
use crate::stream::StreamJob;
use crate::transaction;
use crate::worker::WorkerHandle;

/// An open DuckDB session.
#[derive(Debug)]
pub struct DuckDbSession {
    worker: WorkerHandle,
    /// Kept so the thread is never orphaned; not joined on close, the thread
    /// releases the connection before it replies.
    thread: Option<JoinHandle<()>>,
    capabilities: Capabilities,
    catalog: DuckDbCatalog,
    limits: BatchLimits,
    context: Mutex<Option<SessionContext>>,
    /// Where the session started, for a return to the server default.
    origin: SessionContext,
}

impl DuckDbSession {
    pub(crate) fn new(
        worker: WorkerHandle,
        thread: JoinHandle<()>,
        capabilities: Capabilities,
        limits: BatchLimits,
        origin: SessionContext,
        version: String,
    ) -> Self {
        Self {
            catalog: DuckDbCatalog::new(worker.clone(), capabilities, version, origin.clone()),
            worker,
            thread: Some(thread),
            capabilities,
            limits,
            context: Mutex::new(None),
            origin,
        }
    }

    /// The bounds of an Arrow batch, for this session.
    #[must_use]
    pub const fn batch_limits(&self) -> BatchLimits {
        self.limits
    }

    /// Runs a transaction statement.
    async fn transaction(
        &self,
        cancel: &CancelToken,
        sql: &'static str,
        effect: Effect,
    ) -> Result<()> {
        self.capabilities.require(Capabilities::TRANSACTIONS)?;
        let job = move |connection: &Connection| {
            connection
                .execute_batch(sql)
                .map_err(|err| error::engine(&err, effect))
        };
        match effect {
            Effect::ReadOnly => self.worker.call(cancel, job).await,
            // A `COMMIT` the token caught may have committed: the engine's
            // verdict is awaited, never assumed (I-13).
            Effect::Mutating => self.worker.call_write(cancel, job).await,
        }
    }
}

/// The database and schema the engine resolves names in, as it says.
pub(crate) fn current_context(connection: &Connection) -> Result<SessionContext> {
    let table = query::read(
        connection,
        "SELECT current_database(), current_schema()",
        &[],
    )?;
    let row = table.rows().next();
    Ok(SessionContext::new(
        row.and_then(|row| row.text(0)),
        row.and_then(|row| row.text(1)),
    ))
}

#[async_trait]
impl Session for DuckDbSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Executes a request and returns a cursor once the schema is known.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] for a language other than SQL or a typed
    /// `USE`, [`OxynError::PolicyDenied`] when a read-only request holds a
    /// write or runs inside an open transaction, [`OxynError::Cancelled`], or
    /// the engine's error.
    async fn execute(
        &self,
        mut request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if self.capabilities.contains(Capabilities::READ_ONLY_SESSION) {
            // A connection opened read-only: the engine refuses data writes,
            // and this bound keeps `COPY … TO` from writing a file.
            request.limits.read_only = true;
        }
        let handle = StatementHandle::new();
        let started = Instant::now();
        let (start, opened) = oneshot::channel();
        let (batches, receiver) = mpsc::channel(1);
        let reads = request.limits.read_only;
        let work = self.worker.start_stream(Box::new(StreamJob {
            request,
            limits: self.limits,
            start,
            batches,
        }))?;
        // A request that may write waits, past a Stop, for the engine's
        // verdict: the token firing proves nothing about it (I-13).
        let start = if reads {
            self.worker.await_reply(opened, cancel, work).await?
        } else {
            self.worker.await_verdict(opened, cancel, work).await?
        };
        Ok(Box::new(DuckDbCursor::new(Opened {
            handle,
            start,
            batches: receiver,
            worker: self.worker.clone(),
            work,
            cancel: cancel.clone(),
            started,
        })))
    }

    /// Refuses: DuckDB has no server to ask; the interruption goes through the
    /// token handed to `execute`.
    ///
    /// # Errors
    /// Always [`OxynError::NotSupported`].
    async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "SERVER_SIDE_CANCEL".to_owned(),
        })
    }

    /// Composes a read-only preview, reading the relation's description when
    /// the shape needs it.
    ///
    /// # Errors
    /// [`OxynError::Cancelled`], those of the composition, and the engine's.
    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let facts = if shape.needs_total_order() || shape.columns.is_some() {
            crate::preview::RelationFacts::of(&self.catalog.describe_relation(path, cancel).await?)
        } else {
            crate::preview::RelationFacts::default()
        };
        crate::preview::request(path, limit, shape, &facts)
    }

    /// Runs `USE` on the quoted database and schema, then reads back what the
    /// engine resolves names in.
    ///
    /// # Errors
    /// [`OxynError::Cancelled`], or the engine's error for a database or
    /// schema that does not exist.
    async fn set_context(&self, context: &SessionContext, cancel: &CancelToken) -> Result<()> {
        let target = if context.is_server_default() {
            self.origin.clone()
        } else {
            context.clone()
        };
        let mut place = Vec::new();
        if let Some(catalog) = target.catalog() {
            place.push(quote_identifier(catalog, QuoteStyle::Double));
        }
        if let Some(namespace) = target.namespace() {
            place.push(quote_identifier(namespace, QuoteStyle::Double));
        }
        if place.is_empty() {
            return Ok(());
        }
        let statement = format!("USE {}", place.join("."));
        let confirmed = self
            .worker
            .call(cancel, move |connection: &Connection| {
                connection
                    .execute_batch(&statement)
                    .map_err(|err| error::engine(&err, Effect::ReadOnly))?;
                current_context(connection)
            })
            .await?;
        *self.context.lock() = (!context.is_server_default()).then_some(confirmed);
        Ok(())
    }

    fn context(&self) -> Option<SessionContext> {
        self.context.lock().clone()
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// A round trip to the carrier thread and the engine; mostly tells whether
    /// the session is free or busy with a stream.
    ///
    /// # Errors
    /// The engine's, or a closed-session error.
    async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        let cancel = CancelToken::new();
        self.worker
            .call(&cancel, |connection: &Connection| {
                query::read(connection, "SELECT 1", &[]).map(|_| ())
            })
            .await?;
        Ok(started.elapsed())
    }

    /// Closes the connection; the last session of a file releases the file.
    ///
    /// # Errors
    /// None in practice: DuckDB's disconnect does not fail.
    async fn close(self: Box<Self>) -> Result<()> {
        let Self { worker, thread, .. } = *self;
        let outcome = worker.close().await;
        drop(worker);
        drop(thread);
        outcome
    }

    async fn begin(&self, cancel: &CancelToken) -> Result<()> {
        self.transaction(cancel, "BEGIN TRANSACTION", Effect::ReadOnly)
            .await
    }

    async fn commit(&self, cancel: &CancelToken) -> Result<()> {
        // Interrupted, a COMMIT leaves its outcome unknown: ambiguous (I-13).
        self.transaction(cancel, "COMMIT", Effect::Mutating).await
    }

    async fn rollback(&self, cancel: &CancelToken) -> Result<()> {
        self.transaction(cancel, "ROLLBACK", Effect::Mutating).await
    }

    /// Asks the engine, on the carrier thread, after every task submitted
    /// before ([`crate::transaction`]).
    async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
        self.worker
            .call(cancel, |connection: &Connection| {
                Ok(transaction::state(connection))
            })
            .await
            .unwrap_or(TransactionState::Unknown)
    }
}
