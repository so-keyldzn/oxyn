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
//! behavior — and a locked file immediately returns `SQLITE_BUSY`, classified as
//! transient, which the caller may retry if it deems it replayable.

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
    /// only if a sort or a page requires an order, or if a projection names columns
    /// to check: a preview without request does not pay a `PRAGMA table_info` for a
    /// clause it does not compose. This read goes through the worker thread and is
    /// interrupted like the others.
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
        let facts = if shape.needs_total_order() || shape.columns.is_some() {
            crate::preview::RelationFacts::of(&self.catalog.describe_relation(path, cancel).await?)
        } else {
            crate::preview::RelationFacts::default()
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
mod tests {
    use std::pin::pin;

    use oxyn_core::{ConnectionId, QueryLanguage, SqlDialect};

    use super::*;
    use crate::worker::{self, OpenSpec, OpenTarget};

    /// A `count(*)` over an endless series: it never returns on its own.
    const ENDLESS: &str =
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";

    #[tokio::test]
    async fn abandoning_execute_during_the_first_batch_frees_the_engine() {
        // The scenario: the tab closes while SQLite computes the first batch of an
        // aggregation. The cursor does not exist yet, so nobody else can interrupt.
        // Without interruption, the session stays busy for the whole duration of the
        // computation — here, forever.
        let cancel_token = CancelToken::new();
        let spec = OpenSpec {
            target: OpenTarget::Memory(ConnectionId::new()),
            read_only: false,
        };
        let (handle, thread) = worker::spawn(spec, &cancel_token).await.expect("open");
        let session = SqliteSession::new(handle, thread, Capabilities::SQL, BatchLimits::new());

        {
            let exec_request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), ENDLESS);
            let mut execution = pin!(session.execute(exec_request, &cancel_token));
            assert!(futures::poll!(execution.as_mut()).is_pending());
            // The worker thread has picked up the execution: we do not abandon a
            // task still queued, which would prove something else.
            tokio::time::timeout(Duration::from_secs(10), async {
                while session.worker.running().is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the worker thread picks up the execution");
        }

        // The proof on the engine side: it answers a `SELECT 1`.
        let response = tokio::time::timeout(Duration::from_secs(10), session.ping()).await;
        let round_trip = response
            .expect("the engine must be freed by the abandonment")
            .expect("ping");
        assert!(
            round_trip < Duration::from_secs(1),
            "the engine must answer quickly: {round_trip:?}"
        );

        Box::new(session).close().await.expect("close");
    }

    /// A session opened through the driver, as the executor gets it.
    async fn open_session(db_file: &str) -> Box<dyn Session> {
        use oxyn_core::{ConnectionConfig, DriverId, Environment};
        use oxyn_driver::{Credentials, Driver};

        let conn_config = ConnectionConfig::new("transactions", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(crate::SqliteDriver::PATH, db_file);
        crate::SqliteDriver::new()
            .connect(&conn_config, &Credentials::new(), &CancelToken::new())
            .await
            .unwrap_or_else(|err| panic!("open: {err}"))
    }

    /// Runs a statement to its end, cursor released.
    async fn run_to_end(session: &dyn Session, sql: &str) {
        let exec_request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), sql)
            .with_limits(oxyn_core::ExecLimits::unbounded());
        let mut batch_cursor = match session.execute(exec_request, &CancelToken::new()).await {
            Ok(batch_cursor) => batch_cursor,
            Err(err) => panic!("execution of `{sql}`: {err}"),
        };
        while batch_cursor
            .next_batch()
            .await
            .expect("next batch")
            .is_some()
        {}
    }

    async fn state_of(session: &dyn Session) -> TransactionState {
        session.transaction_state(&CancelToken::new()).await
    }

    #[tokio::test]
    async fn the_state_follows_written_transactions_and_those_of_the_trait() {
        // ADR-0039 §2, the contract of a session that declares TRANSACTIONS.
        let session = open_session(crate::SqliteDriver::MEMORY).await;
        assert!(session.capabilities().contains(Capabilities::TRANSACTIONS));
        assert_eq!(
            state_of(session.as_ref()).await,
            TransactionState::Idle,
            "a fresh connection is in autocommit, and says so"
        );
        run_to_end(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        for terminator in ["COMMIT", "ROLLBACK", "END"] {
            run_to_end(session.as_ref(), "BEGIN").await;
            assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
            run_to_end(session.as_ref(), "INSERT INTO t(v) VALUES (1)").await;
            assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
            run_to_end(session.as_ref(), terminator).await;
            assert_eq!(
                state_of(session.as_ref()).await,
                TransactionState::Idle,
                "{terminator}"
            );
        }

        let cancel_token = CancelToken::new();
        session.begin(&cancel_token).await.expect("begin");
        assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
        session.commit(&cancel_token).await.expect("commit");
        assert_eq!(state_of(session.as_ref()).await, TransactionState::Idle);

        session.begin(&cancel_token).await.expect("begin");
        assert_eq!(state_of(session.as_ref()).await, TransactionState::Open);
        session.rollback(&cancel_token).await.expect("rollback");
        assert_eq!(state_of(session.as_ref()).await, TransactionState::Idle);

        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn an_already_fired_token_returns_unknown_never_idle() {
        let session = open_session(crate::SqliteDriver::MEMORY).await;
        let cancel_token = CancelToken::new();
        cancel_token.cancel();
        assert_eq!(
            session.transaction_state(&cancel_token).await,
            TransactionState::Unknown
        );
        session.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_automatic_rollback_after_a_stop_is_observed() {
        // The scenario: `BEGIN`, a write, then Stop on an endless `INSERT`.
        // SQLite rolls the whole transaction back on `SQLITE_INTERRUPT`, the
        // earlier write included: `execute` reports the engine's interruption,
        // ambiguous for a write, not a cancellation (issue #181). A state read
        // then must come after that step, not before.
        let dir = tempfile::tempdir().expect("temporary directory");
        let db_path = dir.path().join("transactions.sqlite");
        let journal = dir.path().join("transactions.sqlite-journal");
        let session = open_session(db_path.to_str().expect("UTF-8 path")).await;

        run_to_end(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;
        run_to_end(session.as_ref(), "BEGIN").await;
        assert!(
            !journal.exists(),
            "a deferred BEGIN writes nothing: the journal marks the first write"
        );

        let cancel_token = CancelToken::new();
        let exec_request = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            "INSERT INTO t(v) SELECT x FROM \
             (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)",
        )
        .with_limits(oxyn_core::ExecLimits::unbounded());
        let mut endless_write = Box::pin(session.execute(exec_request, &cancel_token));
        assert!(futures::poll!(endless_write.as_mut()).is_pending());
        // The rollback journal appears on the first page written: the INSERT
        // is then inside `sqlite3_step`, past the point where an interrupt
        // could be lost. A condition, not a delay.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !journal.exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the endless INSERT starts writing");

        cancel_token.cancel();
        match endless_write.await {
            Ok(_) => panic!("an endless INSERT only ends interrupted"),
            Err(err) => assert_eq!(err.class(), oxyn_core::ErrorClass::Ambiguous, "{err:?}"),
        }

        assert_eq!(
            state_of(session.as_ref()).await,
            TransactionState::Idle,
            "the interrupted write rolled the transaction back"
        );
        session.close().await.expect("close");
    }
}
