//! The session: an open connection, and what it can do.
//!
//! # Cancellation is not server-side, because there is no server
//!
//! SQLite runs **in Oxyn's process**. `sqlite3_interrupt` stops a running
//! statement, but it is a *local* cancellation: no remote connection is freed, no
//! server lock is released, because there is none.
//!
//! The session therefore does **not** declare
//! [`Capabilities::SERVER_SIDE_CANCEL`](oxyn_core::Capabilities::SERVER_SIDE_CANCEL),
//! and [`SqliteSession::cancel`] refuses. It is not a gap to fill: the interface
//! uses the flag to say "the Cancel button really cuts the query on the server",
//! and showing it for an embedded database would assert something meaningless.
//!
//! **Interruption exists all the same**, and it is complete: it goes through the
//! [`CancelToken`] handed to [`execute`](SqliteSession::execute), which the cursor
//! checks between two batches and watches while it waits for one. See
//! [`crate::SqliteCursor`].
//!
//! # A session does one thing at a time
//!
//! It is the semantics of an SQLite connection, not a limitation of the
//! transport: an introspection requested while a cursor streams waits for the
//! current batch to be produced. Interruption, however, does not queue.
//!
//! # What the session does not change behind its back
//!
//! Neither `PRAGMA foreign_keys`, nor `PRAGMA busy_timeout`, nor
//! `PRAGMA journal_mode`. SQLite lets the application choose, and a `PRAGMA` set
//! silently would change the meaning of the user's next queries. Accepted
//! consequence: foreign keys are **not** checked by default — it is SQLite's
//! behavior. A locked file is not reported at once either: `rusqlite` 0.37.0
//! (`Cargo.lock`) calls `sqlite3_busy_timeout(db, 5000)` on every open
//! (`src/inner_connection.rs`, read on 2026-10-20) and this driver does not
//! override it, so a lock held by another connection is waited on for up to
//! 5 seconds before `SQLITE_BUSY` comes back. That error is classified as
//! transient, which the caller may retry if it deems it replayable — unless an
//! earlier statement of the same batch already applied, in which case it never
//! is (`error::after_applied`).

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::provider::CatalogProvider;
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, OxynError, Result, StatementHandle, TransactionState,
};
use oxyn_driver::{Cursor, Session};
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};

use crate::catalog::SqliteCatalog;
use crate::cursor::SqliteCursor;
use crate::error::{self, Effect};
use crate::options::BatchLimits;
use crate::stream::StreamJob;
use crate::worker::WorkerHandle;

/// An open SQLite connection.
#[derive(Debug)]
pub struct SqliteSession {
    worker: WorkerHandle,
    /// The worker thread's handle, kept so that it is never orphaned.
    ///
    /// It is not joined on close: the thread closes the connection **before**
    /// replying, so waiting for its stack to unwind would block the runtime for
    /// nothing.
    thread: Option<JoinHandle<()>>,
    capabilities: Capabilities,
    catalog: SqliteCatalog,
    limits: BatchLimits,
}

impl SqliteSession {
    /// Assembles a session around an already open worker thread.
    pub(crate) fn new(
        worker: WorkerHandle,
        thread: JoinHandle<()>,
        capabilities: Capabilities,
        limits: BatchLimits,
    ) -> Self {
        Self {
            catalog: SqliteCatalog::new(worker.clone(), capabilities),
            worker,
            thread: Some(thread),
            capabilities,
            limits,
        }
    }

    /// The bounds of an Arrow batch, for this session.
    #[must_use]
    pub const fn batch_limits(&self) -> BatchLimits {
        self.limits
    }

    /// Issues a transaction statement.
    async fn transaction(
        &self,
        cancel: &CancelToken,
        sql: &'static str,
        effect: Effect,
    ) -> Result<()> {
        self.capabilities.require(Capabilities::TRANSACTIONS)?;
        self.worker
            .call(cancel, move |connection: &Connection| {
                connection
                    .execute_batch(sql)
                    .map_err(|err| error::engine(err, effect))
            })
            .await
    }
}

#[async_trait]
impl Session for SqliteSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Executes a request and returns a cursor over its batches.
    ///
    /// Returns **as soon as the schema is known** — that is, as soon as the first
    /// batch is ready, since in SQLite a column's type is read from the values as
    /// much as from the declaration ([`convert`](crate::convert)). This first batch
    /// travels with the cursor: it is not read again.
    ///
    /// The SQL **dialect** is not checked: it is "a nuance of grammar, not a
    /// distinct capability" (ADR-0003). A query written for PostgreSQL is therefore
    /// sent as is, and SQLite's parser rejects it — with its own message, which the
    /// user recognizes.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the language is not SQL,
    /// [`OxynError::PolicyDenied`] if the request declares itself read-only and a
    /// statement writes, [`OxynError::Cancelled`] if the token fires, or the engine
    /// error.
    async fn execute(
        &self,
        mut request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if self.capabilities.contains(Capabilities::READ_ONLY_SESSION) {
            request.limits.read_only = true;
        }

        let handle = StatementHandle::new();
        let started = Instant::now();
        let (pulls, orders) = mpsc::unbounded_channel();
        let (start, opened) = oneshot::channel();
        let read_only = request.limits.read_only;

        let work = self.worker.start_stream(Box::new(StreamJob {
            request,
            limits: self.limits,
            start,
            pulls: orders,
        }))?;

        // It is during this wait that SQLite computes the first batch — a whole
        // `count(*)`, or every change of a `RETURNING`. Abandoning this future
        // interrupts the task: see `WorkerHandle::await_reply`. A request that
        // may write waits, past a Stop, for the engine to say how it ended: the
        // token firing proves nothing about it (I-13).
        let begun = if read_only {
            self.worker.await_reply(opened, cancel, work).await?
        } else {
            self.worker.await_verdict(opened, cancel, work).await?
        };
        Ok(Box::new(SqliteCursor::new(
            handle,
            begun,
            pulls,
            self.worker.clone(),
            work,
            cancel.clone(),
            started,
        )))
    }

    /// Refuses: SQLite has no server to ask for a cancellation.
    ///
    /// See the module note. Cancellation exists, it goes through the
    /// [`CancelToken`] handed to [`execute`](Self::execute).
    ///
    /// # Errors
    /// Always [`OxynError::NotSupported`].
    async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "SERVER_SIDE_CANCEL".to_owned(),
        })
    }

    /// Composes the preview of a relation, and reads what its shape requires.
    ///
    /// The description of the relation — its columns and its primary key — is read
    /// on every preview, since one with no sort asked for still comes in key order
    /// ([ADR-0068](../../../docs/adr/0068-a-preview-comes-in-key-order.md)). A
    /// `PRAGMA table_info` is a read of the schema the connection already holds; it
    /// goes through the worker thread and is interrupted like the others.
    ///
    /// # Errors
    /// [`OxynError::Cancelled`] if the token fires, those of the composition —
    /// unknown sort or projection column, empty or oversized projection, page
    /// without unique key —, and those of the engine during introspection.
    async fn preview_request(
        &self,
        path: &oxyn_catalog::CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        // A projection is checked against the same description: a name the
        // relation does not declare is refused here, not by the engine.
        // Described where the preview reads: the catalog looks at the namespace
        // alone, and a preview path may carry the database as a catalog.
        let described = oxyn_catalog::CatalogPath::for_relation(
            None,
            Some(crate::preview::database_of(path)?),
            path.relation().ok_or_else(|| {
                OxynError::CatalogUnavailable("preview path must name a relation".into())
            })?,
        )?;
        let facts = match self.catalog.describe_relation(&described, cancel).await {
            Ok(relation) => crate::preview::RelationFacts::of(&relation),
            // A table whose module this connection does not load cannot be
            // described, and nothing here needs it: the read that follows fails
            // with the same module as data, which is what the Data tab explains.
            // Only a shape that depends on the description fails here.
            Err(error)
                if error.missing_module().is_some()
                    && !shape.needs_total_order()
                    && shape.columns.is_none() =>
            {
                crate::preview::RelationFacts::default()
            }
            Err(error) => return Err(error),
        };
        crate::preview::request(path, limit, shape, &facts)
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Checks that the connection is alive.
    ///
    /// The measured duration is that of a round trip to the worker thread, not
    /// that of a network: it mostly tells whether the thread is free or busy with
    /// a running stream.
    ///
    /// # Errors
    /// The engine error, or a driver error if the thread has disappeared.
    async fn ping(&self) -> Result<Duration> {
        let started = Instant::now();
        // `ping` has no token in its contract: a fresh token, never cancelled,
        // lets the call finish.
        let cancel = CancelToken::new();
        self.worker
            .call(&cancel, |connection: &Connection| {
                connection
                    .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .map(|_| ())
                    .map_err(|err| error::engine(err, Effect::ReadOnly))
            })
            .await?;
        Ok(started.elapsed())
    }

    /// Closes the connection and releases the file.
    ///
    /// # Errors
    /// The engine error on close. An already closed session is not one: local
    /// resources are released in every case.
    async fn close(self: Box<Self>) -> Result<()> {
        let Self { worker, thread, .. } = *self;
        let outcome = worker.close().await;
        // The channel closes here: the thread leaves its loop even if the close
        // above failed.
        drop(worker);
        drop(thread);
        outcome
    }

    async fn begin(&self, cancel: &CancelToken) -> Result<()> {
        // `BEGIN` alone modifies nothing: interrupted, it left nothing behind.
        self.transaction(cancel, "BEGIN", Effect::ReadOnly).await
    }

    async fn commit(&self, cancel: &CancelToken) -> Result<()> {
        // An interrupted `COMMIT` leaves the effect unknown: ambiguous, hence never
        // replayed (I-13).
        self.transaction(cancel, "COMMIT", Effect::Mutating).await
    }

    async fn rollback(&self, cancel: &CancelToken) -> Result<()> {
        self.transaction(cancel, "ROLLBACK", Effect::Mutating).await
    }

    /// Reads `sqlite3_get_autocommit` on the carrier thread.
    ///
    /// Submitted as a task, not read from a stored value: the thread runs its
    /// tasks in submission order, so this one runs only once the previous one
    /// has returned from `sqlite3_step` — including the automatic rollback an
    /// interruption triggers, which "the only way to find out" about is this
    /// call (ADR-0039).
    async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
        let autocommit = self
            .worker
            .call(cancel, |connection: &Connection| {
                Ok(connection.is_autocommit())
            })
            .await;
        match autocommit {
            Ok(true) => TransactionState::Idle,
            Ok(false) => TransactionState::Open,
            Err(_) => TransactionState::Unknown,
        }
    }
}

#[cfg(test)]
mod tests;
