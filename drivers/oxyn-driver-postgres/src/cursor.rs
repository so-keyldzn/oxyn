//! The batch stream, and the cancellation that really reaches the server.
//!
//! # Why a task and a channel
//!
//! `sqlx` returns a row stream that **borrows** the connection and the
//! prepared statement. A cursor owning all three would be self-referential,
//! which in Rust requires either `unsafe` — forbidden by the workspace — or one
//! more dependency. The task owns all three and pushes its batches into a
//! bounded channel; the cursor holds only the receiving end.
//!
//! The channel has **one slot**. That is not timidity: it is back-pressure. The
//! task decodes the next batch only once the previous one has been taken, so a
//! `SELECT *` over 500 GB never grows memory beyond two batches
//! ([I-06](../../../CLAUDE.md#i-06)).
//!
//! # Abandoning a cursor cuts the query
//!
//! Closing a tab drops the cursor. Its `Drop` **cancels its token**, which wakes
//! the task, makes it issue `pg_cancel_backend` from a second connection, then
//! close its own. Without that, the four-minute aggregation would continue on
//! the server, the connection taken and the lock held — and at the tenth closed
//! tab the database would refuse connections
//! ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! That is also why `Drop` **does not abort** the task: an aborted task can no
//! longer cancel anything. It is asked to stop, it is not killed.

use std::time::Instant;

use std::collections::HashMap;
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::StreamExt as _;
use oxyn_core::{
    CancelToken, DriverId, ErrorClass, ExecLimits, ExecStats, OxynError, Result, StatementHandle,
    StatementIntent,
};
use oxyn_driver::Cursor;
use sqlx::postgres::{PgArguments, PgRow, PgStatement};
use sqlx::{Column as _, Row as _, SqlStr, TypeInfo as _};
use sqlx::{Either, Statement as _};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::cancel::{BackendCanceller, StatementRegistry, Verdict, VerdictSender};
use crate::decode::BatchAssembler;
use crate::error::{Bound, map_stream_error};
use crate::lease::Lease;
use crate::session::{SQL_RESET_AFTER_WRITE, SQL_RESET_SEARCH_PATH, SQL_ROLLBACK};
use crate::types::{META_FALLBACK, META_PG_TYPE, PgDecoding};

/// Accumulated bytes beyond which a batch is closed and emitted.
///
/// **In bytes, not in rows.** A thousand rows each carrying a one-megabyte BLOB
/// make a gigabyte; a threshold counted in rows works on demo tables and
/// triggers the OOM on real ones.
pub const BATCH_BYTE_BUDGET: usize = 1 << 20;

/// Rows beyond which a batch is closed, whatever its size.
///
/// Complements the byte threshold from below: without it, a million booleans
/// would never fill a megabyte and the first batch would never arrive. The
/// first-display budget is 100 ms ([PERFORMANCE](../../../docs/PERFORMANCE.md)).
pub const BATCH_ROW_CEILING: usize = 8_192;

/// What the stream task pushes to the cursor.
#[derive(Debug)]
enum CursorEvent {
    /// A batch ready to display.
    Batch(RecordBatch),
    /// The stream is exhausted, or bounded.
    Finished {
        /// Affected rows, for a statement that returns no columns.
        affected_rows: u64,
        /// Was the result cut by the execution bounds?
        truncated: bool,
    },
    /// The stream stopped on an already classified error.
    Failed(Box<OxynError>),
}

/// Why the stream loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Halt {
    /// The server has nothing more to send.
    Exhausted,
    /// [`ExecLimits::max_rows`] is reached by a statement that does not
    /// mutate: a row beyond it was received and dropped. A mutating statement
    /// never stops here; it is drained to its end.
    RowLimit,
    /// The cancellation token fired.
    Cancelled,
    /// [`ExecLimits::timeout`] has elapsed.
    TimedOut,
    /// The cursor was dropped: nobody waits for the batches any more.
    Abandoned,
    /// An error was encountered and already emitted.
    Failed,
}

impl Halt {
    /// Must the server be asked to stop the query?
    ///
    /// Anything that is not exhaustion leaves a query running on the server.
    /// Abandoning the stream on the client side releases neither the connection
    /// nor the lock.
    const fn needs_server_cancel(self) -> bool {
        !matches!(self, Self::Exhausted | Self::Failed)
    }
}

/// A `RecordBatch` stream fed by a PostgreSQL execution.
///
/// `Debug` is written by hand: the channel and the task handle teach nobody
/// anything, and the schema is what one wants to see in diagnostics.
pub struct PostgresCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    events: mpsc::Receiver<CursorEvent>,
    /// This execution's **own** token. Cancelled by `Drop`, it is what turns
    /// closing a tab into `pg_cancel_backend`.
    cancel: CancelToken,
    task: JoinHandle<()>,
    stats: ExecStats,
    started: Instant,
    finished: bool,
    /// Does the statement return columns? Otherwise, `rows` counts the
    /// **affected** rows, which is not the same measure.
    projects_columns: bool,
}

impl PostgresCursor {
    /// Is the stream task finished?
    ///
    /// Used for diagnostics and tests: after a cancellation, the task must stop
    /// on its own, without having been aborted.
    #[must_use]
    pub fn task_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl std::fmt::Debug for PostgresCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresCursor")
            .field("handle", &self.handle)
            .field("columns", &self.schema.fields().len())
            .field("finished", &self.finished)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Drop for PostgresCursor {
    /// Requests the stop; does not impose it.
    ///
    /// Aborting the task would deprive it of the right to issue
    /// `pg_cancel_backend` and to return its connection — that is, of exactly
    /// what makes the cancellation real.
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
        }
    }
}

#[async_trait]
impl Cursor for PostgresCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        SchemaRef::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.finished {
            return Ok(None);
        }
        match self.events.recv().await {
            Some(CursorEvent::Batch(batch)) => {
                self.stats.record_batch(
                    u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                Ok(Some(batch))
            }
            Some(CursorEvent::Finished {
                affected_rows,
                truncated,
            }) => {
                self.seal(truncated);
                if !self.projects_columns {
                    self.stats.rows = affected_rows;
                }
                Ok(None)
            }
            Some(CursorEvent::Failed(error)) => {
                self.seal(true);
                Err(*error)
            }
            // The channel closes without `Finished` only if the task disappeared
            // without concluding: it is a driver bug, not faulty data.
            None => {
                self.seal(true);
                Err(OxynError::Internal(
                    "the PostgreSQL streaming task stopped without concluding".to_owned(),
                ))
            }
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

impl PostgresCursor {
    /// Closes the cursor: nothing more will come.
    fn seal(&mut self, truncated: bool) {
        self.finished = true;
        self.stats.total_time = self.started.elapsed();
        if truncated {
            self.stats.mark_truncated();
        }
    }
}

/// What the stream task needs to live its life.
///
/// A type rather than eleven parameters: the function that took them all
/// exceeded the `clippy::too_many_arguments` limit and, above all, nobody could
/// review the order of the arguments.
pub(crate) struct StreamRequest {
    /// The driver, to classify errors.
    pub(crate) driver: DriverId,
    /// The connection that will carry the execution, borrowed from the pool and
    /// already marked dirty: it goes back only if the task resets it to the
    /// default.
    pub(crate) connection: Lease,
    /// Where the rows come from.
    pub(crate) source: Source,
    /// Were there caller values among them?
    ///
    /// Recorded by the session before `arguments` is moved into `query_with`:
    /// the server's message can quote a bound value, and `PgArguments` no
    /// longer says, here, where its bytes come from
    /// ([I-03](../../../CLAUDE.md#i-03)).
    pub(crate) bound: Bound,
    /// The batches' schema, known before the first row — except for
    /// [`Source::Simple`], whose cursor learns it at the first row.
    pub(crate) schema: SchemaRef,
    /// The decoding plan, aligned with the schema.
    pub(crate) decodings: Vec<PgDecoding>,
    /// Was a `search_path` set for this execution?
    ///
    /// It must then be undone before the connection goes back to the pool.
    /// Otherwise it carries a console's context to everything that borrows it
    /// next — and introspection depends on it: `pg_get_indexdef`,
    /// `pg_get_constraintdef`, `pg_get_expr` and `format_type` qualify their
    /// text **relative to the `search_path`**. The same object would then be
    /// described differently from one read to the next, depending on the
    /// connection drawn ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)).
    pub(crate) restore_context: bool,
    /// Does the user's text open a transaction (`BEGIN`, `START TRANSACTION`)?
    /// The connection is then closed instead of being returned.
    pub(crate) opens_transaction: bool,
    /// The execution's bounds.
    pub(crate) limits: ExecLimits,
    /// Does the statement not mutate, so that it may stop at its bound?
    ///
    /// A read reads row N + 1 to tell exactly N rows from more than N, then
    /// stops and cancels. A write is never cut there: `Execute` goes out
    /// without a row limit, a `RETURNING` runs whole before its first row, and
    /// an autocommit statement only commits at `Sync`, once its last row is
    /// written. Cutting it would roll back a write whose rows the user is
    /// shown. Its rows past the bound are drained instead, neither decoded nor
    /// kept, under the same token and deadline, until `CommandComplete`.
    pub(crate) confirm_end: bool,
    /// The intent, which decides the class of transport errors.
    pub(crate) intent: StatementIntent,
    /// The pid of the server process that executes, for cancellation.
    pub(crate) backend_pid: i32,
    /// What opens the second connection that will cancel.
    pub(crate) canceller: std::sync::Arc<BackendCanceller>,
    /// The registry of running executions, from which the task removes itself
    /// on leaving.
    pub(crate) statements: std::sync::Arc<StatementRegistry>,
    /// Where to return to `Session::cancel` what the task did with the
    /// cancellation.
    pub(crate) verdict: VerdictSender,
    /// The handle [`Cursor::handle`] exposes.
    pub(crate) handle: StatementHandle,
}

/// Where an execution's rows come from.
pub(crate) enum Source {
    /// The extended protocol: the prepared statement, which gave the schema,
    /// and the bound parameters, already encoded.
    Prepared {
        statement: PgStatement,
        arguments: PgArguments,
    },
    /// The simple protocol, for a statement `sqlx` cannot prepare or whose
    /// result has no binary form (ADR-0048). No parameter, every value as
    /// text, and the schema learnt at the first row.
    Simple { sql: SqlStr },
}

/// Starts the stream and returns the cursor that drains it.
///
/// For a prepared statement the call returns at once: the schema is already
/// known, so the grid draws its columns while the first row is still
/// travelling. The simple protocol gives the columns with the first row: the
/// call waits for it, or for the end of the stream.
///
/// `cancel` is the execution's **own** token, already recorded in the registry:
/// dropping this cursor fires it, and so does `Session::cancel`. While this
/// call waits for the schema, the caller does not hold the handle yet, so
/// `Session::cancel` cannot reach it; two things still do: the caller's token,
/// whose child `cancel` is, and dropping this future, which drops the cursor
/// already built here.
pub(crate) async fn spawn(request: StreamRequest, cancel: CancelToken) -> PostgresCursor {
    let (send, reception) = mpsc::channel(1);

    let schema = SchemaRef::clone(&request.schema);
    let handle = request.handle;
    let token = cancel.clone();
    let (schema_sender, schema_reception) = match request.source {
        Source::Prepared { .. } => (None, None),
        Source::Simple { .. } => {
            let (sender, reception) = oneshot::channel();
            (Some(sender), Some(reception))
        }
    };

    let task = tokio::spawn(async move {
        run(request, token, send, schema_sender).await;
    });

    let mut cursor = PostgresCursor {
        handle,
        projects_columns: !schema.fields().is_empty(),
        schema,
        events: reception,
        cancel,
        task,
        stats: ExecStats::default(),
        started: Instant::now(),
        finished: false,
    };
    if let Some(reception) = schema_reception {
        // A task that ends without a row — an error, a cancellation, an empty
        // result — drops the sender: the cursor then has no columns, and its
        // events say why.
        if let Ok(learnt) = reception.await {
            cursor.projects_columns = !learnt.fields().is_empty();
            cursor.schema = learnt;
        }
    }
    cursor
}

/// The schema of a simple-protocol result: every column is the server's text.
///
/// Built from names and OIDs only. The typed decoding would call
/// `PgTypeInfo::kind`, which **panics** on the unresolved types this protocol
/// yields (ADR-0048).
fn text_schema(row: &PgRow) -> (SchemaRef, Vec<PgDecoding>) {
    let mut fields = Vec::with_capacity(row.columns().len());
    for column in row.columns() {
        let type_info = column.type_info();
        let pg_type = match (type_info.name(), type_info.oid()) {
            ("?", Some(raw_type)) => format!("oid {}", raw_type.0),
            (name, _) => name.to_owned(),
        };
        let metadata = HashMap::from([
            (META_PG_TYPE.to_owned(), pg_type),
            (META_FALLBACK.to_owned(), "text".to_owned()),
        ]);
        fields.push(Field::new(column.name(), DataType::Utf8, true).with_metadata(metadata));
    }
    let decodings = vec![PgDecoding::Text; fields.len()];
    (Arc::new(Schema::new(fields)), decodings)
}

/// The body of the stream task.
#[allow(clippy::too_many_lines)]
async fn run(
    request: StreamRequest,
    cancel: CancelToken,
    events: mpsc::Sender<CursorEvent>,
    mut schema_sender: Option<oneshot::Sender<SchemaRef>>,
) {
    let StreamRequest {
        driver,
        mut connection,
        source,
        bound,
        restore_context,
        opens_transaction,
        schema,
        decodings,
        limits,
        confirm_end,
        intent,
        backend_pid,
        canceller,
        statements,
        verdict,
        handle,
    } = request;

    let mut assembler = BatchAssembler::new(schema, &decodings);
    let mut affected: u64 = 0;
    let mut produced: usize = 0;
    // Did a write's drain pass rows the user will not see?
    let mut rows_dropped = false;
    let deadline = limits
        .timeout
        .map(|duration| tokio::time::Instant::now() + duration);

    let stop = {
        // `Statement::query_with` rather than `sqlx::query_statement_with`: the
        // database type there is the statement's, with no inference to bubble
        // up.
        // `fetch_many` is deprecated because multi-statement only ever worked
        // in SQLite. That is not what it is used for here: it is the only stream
        // that also returns the final `QueryResult`, hence the only one that
        // gives `rows_affected()` — the `Either::Left` arm below. The
        // `raw_sql()` the deprecation suggests would give up the prepared
        // statement, hence the bound values, hence I-10: it is a safety
        // regression, not a replacement.
        //
        // TODO(2026-12-01, oxyn-driver-postgres): go back to a non-deprecated
        // API when sqlx exposes the affected row count on `fetch()`. Tracking:
        // https://github.com/launchbadge/sqlx/issues/3108
        // Declared before the stream, which borrows it.
        let statement: PgStatement;
        let mut stream = match source {
            Source::Prepared {
                statement: prepared,
                arguments,
            } => {
                statement = prepared;
                #[expect(deprecated, reason = "only stream exposing rows_affected(); see above")]
                let stream = statement.query_with(arguments).fetch_many(&mut *connection);
                stream
            }
            Source::Simple { sql } => sqlx::raw_sql(sql).fetch_many(&mut *connection),
        };

        loop {
            let step = tokio::select! {
                // `biased`: an already requested cancellation always wins over
                // a ready batch. Without it, a fast stream can make the
                // cancellation wait indefinitely.
                biased;
                () = cancel.cancelled() => Step::Interrupted(Halt::Cancelled),
                () = sleep_until_deadline(deadline) => Step::Interrupted(Halt::TimedOut),
                received = stream.next() => match received {
                    Some(result) => Step::Received(result),
                    None => Step::End,
                },
            };

            let result = match step {
                Step::End => break Halt::Exhausted,
                Step::Interrupted(reason) => break reason,
                Step::Received(result) => result,
            };

            let element = match result {
                Ok(element) => element,
                Err(error) => {
                    let oxyn = map_stream_error(&driver, intent, limits.read_only, bound, error);
                    let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
                    break Halt::Failed;
                }
            };

            let row = match element {
                // End of a statement: the affected row count is the most useful
                // thing a write has to say.
                Either::Left(resume) => {
                    affected = affected.saturating_add(resume.rows_affected());
                    continue;
                }
                Either::Right(row) => row,
            };

            // The simple protocol's first row brings the columns: the cursor
            // waiting in `spawn` gets them, and the assembler starts on them.
            if let Some(sender) = schema_sender.take() {
                let (learnt, plan) = text_schema(&row);
                assembler = BatchAssembler::new(SchemaRef::clone(&learnt), &plan);
                let _ = sender.send(learnt);
            }

            // A row beyond the bound: the result is really longer. It is
            // neither decoded nor kept; reading it, under the same token and
            // deadline as every other row, is what tells exactly N rows from
            // more than N. A read stops there; a write is drained on to its
            // `CommandComplete` (see `StreamRequest::confirm_end`).
            if limits.max_rows.is_some_and(|max| produced >= max) {
                if confirm_end {
                    break Halt::RowLimit;
                }
                rows_dropped = true;
                continue;
            }

            if let Err(error) = assembler.push(&row) {
                let oxyn = OxynError::driver(driver.clone(), ErrorClass::Permanent, error);
                let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
                break Halt::Failed;
            }
            produced = produced.saturating_add(1);

            let limit_reached = limits.max_rows.is_some_and(|max| produced >= max);
            let batch_full =
                assembler.bytes() >= BATCH_BYTE_BUDGET || assembler.rows() >= BATCH_ROW_CEILING;
            // A write's batch that reaches the bound is held until the drain
            // ends. The executor stops reading at N and drops the cursor, which
            // cancels the token: emitted now, row N would cut the very drain
            // that keeps the write from being rolled back.
            let held = limit_reached && !confirm_end;

            if (batch_full || limit_reached) && !held {
                // The token is watched **also** during back-pressure: a grid
                // that no longer reads would otherwise let `Session::cancel`
                // wait forever, the query still running on the server.
                let emission = tokio::select! {
                    biased;
                    () = cancel.cancelled() => Emission::Cancelled,
                    emission = emit(&mut assembler, &driver, &events) => emission,
                };
                match emission {
                    Emission::Proceed => {}
                    Emission::Cancelled => break Halt::Cancelled,
                    Emission::Abandoned => break Halt::Abandoned,
                    Emission::Failed => break Halt::Failed,
                }
            }
        }
    };
    // Was a write stopped once its rows had started to arrive? A `RETURNING`
    // runs whole before its first row: only the commit, at `Sync`, is left,
    // and the server may make it before the cancellation sent below lands —
    // during the drain past the bound as well as before it. The deadline needs
    // no such care: `Timeout` is already ambiguous.
    let drain_cut = !confirm_end && (produced > 0 || rows_dropped);

    // The stop is decided: a token fired after this point changes nothing any
    // more. The task removes itself from the registry, and it — not
    // `Session::cancel` — sends the cancellation if one is needed (see the
    // `cancel` module).
    statements.forget(handle);

    // The stream is dropped: the borrow on the connection is lifted.
    if stop.needs_server_cancel() {
        // A connection whose stream was abandoned may have kept unread bytes:
        // returning it to the pool would desynchronize the next borrower. It is
        // held **until after** the cancellation is sent: as long as it is, no
        // other query can run on `backend_pid`.
        connection.discard();
        match canceller.cancel_backend(backend_pid).await {
            Ok(()) => verdict.settle(Verdict::Cancelled),
            Err(error) => {
                // Reported here, and returned to `Session::cancel` if it is
                // waiting: a dropped cursor, for its part, has nobody to tell.
                tracing::warn!(
                    target: "oxyn::driver::postgres",
                    error = %error,
                    "server-side cancellation did not succeed"
                );
                verdict.settle(Verdict::CancelFailed(std::sync::Arc::new(error)));
            }
        }
    } else {
        // Nothing to cut: `Session::cancel` need not wait for the cleanup.
        verdict.settle(Verdict::Finished);
        // Each reset to the default must be **confirmed** by the server for the
        // connection to go back to the pool. The first one that fails leaves
        // the borrow dirty, hence the connection closed: better open a fresh one
        // than let introspection or another console inherit a state it did not
        // ask for. A task interrupted between two `await`s — runtime shutdown —
        // leaves the borrow dirty too.
        let mut pendings: Vec<(&'static str, &'static str)> = Vec::with_capacity(2);
        if limits.read_only {
            // The transaction opened by `BEGIN READ ONLY` must be closed before
            // returning to the pool: a connection left `idle in transaction`
            // keeps locks and blocks the `VACUUM` of the whole database. The
            // `ROLLBACK` also undoes any `SET` the user's statement may have set
            // inside the transaction — not the context `SET`, set before it.
            pendings.push((
                SQL_ROLLBACK,
                "the read-only transaction could not be closed",
            ));
            if restore_context {
                pendings.push((
                    SQL_RESET_SEARCH_PATH,
                    "the session context could not be reset",
                ));
            }
        } else if opens_transaction {
            // The user opened a transaction. Returning it to the pool would
            // have the next borrower inherit it; undoing it with a `ROLLBACK`
            // would silently cancel what they wanted to keep. Nothing is reset:
            // the borrow stays dirty, the connection is closed. See
            // `transaction_text::opens_transaction`.
        } else {
            // Outside a transaction, nothing undoes a `SET` typed in a console:
            // neither `standard_conforming_strings = off`, which the splitter
            // does not assume, nor a `search_path`, which the pool makes
            // unreliable for the user anyway — the next query may go out on
            // another connection. A console's schema goes through its session
            // context (ADR-0019), which this reset undoes too.
            pendings.push((
                SQL_RESET_AFTER_WRITE,
                "the session state could not be reset after a write",
            ));
        }

        let mut pending = !pendings.is_empty();
        for (instruction, failure) in pendings {
            if let Err(error) = sqlx::raw_sql(instruction).execute(&mut *connection).await {
                // Translated before being logged, as everywhere else: a raw
                // `sqlx::Error` can carry the connection URL.
                tracing::warn!(
                    target: "oxyn::driver::postgres",
                    error = %crate::error::map_exec_error(
                        &driver,
                        oxyn_core::StatementIntent::Read,
                        error,
                    ),
                    "{failure}"
                );
                pending = false;
                break;
            }
        }
        if pending {
            connection.restored();
        }
    }
    drop(connection);

    match stop {
        Halt::Failed => {}
        // Not `Cancelled`: that would read as "nothing happened" and invite
        // running the write again (I-13).
        Halt::Cancelled | Halt::Abandoned if drain_cut => {
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::OutcomeUnknown(
                    "the write was interrupted while its returned rows were being read: the \
                     server either committed it or rolled it back; check the data before \
                     running it again"
                        .to_owned(),
                ))))
                .await;
        }
        Halt::Cancelled | Halt::Abandoned => {
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::Cancelled)))
                .await;
        }
        Halt::TimedOut => {
            let delay = limits.timeout.unwrap_or_default();
            // `Timeout` is ambiguous by construction: the server may have
            // applied the write (I-13). `OxynError::class` says so, not this
            // message.
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::Timeout {
                    after: delay,
                })))
                .await;
        }
        Halt::Exhausted | Halt::RowLimit => {
            if !assembler.is_empty()
                && let Emission::Failed | Emission::Abandoned =
                    emit(&mut assembler, &driver, &events).await
            {
                return;
            }
            let _ = events
                .send(CursorEvent::Finished {
                    affected_rows: affected,
                    truncated: stop == Halt::RowLimit || rows_dropped,
                })
                .await;
        }
    }
}

/// What one iteration of the loop produced.
enum Step {
    /// The stream returned an element.
    Received(
        std::result::Result<
            Either<sqlx::postgres::PgQueryResult, sqlx::postgres::PgRow>,
            sqlx::Error,
        >,
    ),
    /// The stream is exhausted.
    End,
    /// An interruption won the race.
    Interrupted(Halt),
}

/// What emitting a batch gave.
enum Emission {
    /// The batch went out, carry on.
    Proceed,
    /// The token fired while the channel was full.
    Cancelled,
    /// Nobody listens any more: the cursor was dropped.
    Abandoned,
    /// The batch could not be built; the error is already emitted.
    Failed,
}

/// Closes the current batch and sends it, honoring the channel's
/// back-pressure.
async fn emit(
    assembler: &mut BatchAssembler,
    driver: &DriverId,
    events: &mpsc::Sender<CursorEvent>,
) -> Emission {
    let batch = match assembler.finish() {
        Ok(batch) => batch,
        Err(error) => {
            let oxyn = OxynError::driver(driver.clone(), ErrorClass::Permanent, error);
            let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
            return Emission::Failed;
        }
    };
    // `send` on a full channel **waits**: that is where back-pressure applies,
    // and it is what keeps memory from inflating.
    if events.send(CursorEvent::Batch(batch)).await.is_err() {
        return Emission::Abandoned;
    }
    Emission::Proceed
}

/// Waits for the deadline, or forever when there is none.
///
/// The instant is **absolute**: recreating this future on every loop turn
/// therefore does not push the delay back, which a relative duration would.
async fn sleep_until_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(instant) => tokio::time::sleep_until(instant).await,
        None => std::future::pending().await,
    }
}

// Batch sizing, checked at **compile time**.
//
// A runtime `assert!` on constants cannot fail other than by refusing to
// compile later: might as well say it here. The byte threshold protects from
// OOM on BLOBs; the row ceiling guarantees a first batch arrives quickly on
// narrow columns ([drivers.md](../../../.claude/rules/drivers.md) — "the batch
// is sized in bytes, not in rows").
const _: () = {
    assert!(BATCH_BYTE_BUDGET == 1 << 20);
    assert!(BATCH_ROW_CEILING > 0);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stop_that_is_not_exhaustion_requests_server_cancel() {
        // It is DRIVER-CONTRACT §2's rule reduced to one line: abandoning the
        // stream on the client side releases neither the connection nor the
        // lock.
        assert!(!Halt::Exhausted.needs_server_cancel());
        assert!(
            !Halt::Failed.needs_server_cancel(),
            "the server has already finished"
        );
        for reason in [
            Halt::Cancelled,
            Halt::TimedOut,
            Halt::RowLimit,
            Halt::Abandoned,
        ] {
            assert!(
                reason.needs_server_cancel(),
                "{reason:?} leaves a query running"
            );
        }
    }
}
