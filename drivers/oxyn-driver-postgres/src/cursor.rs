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
//!
//! # A write that was sent is read to its answer
//!
//! Cutting the connection of a write that has left says nothing of what the
//! server did with it. A `CommandComplete` may already be buffered when Stop is
//! pressed, and an autocommit statement only commits at `Sync`: reporting
//! `Cancelled` there invites running the write again, and applying it twice
//! ([I-13](../../../CLAUDE.md#i-13)). A stop therefore settles a write that was
//! sent from **its own connection**: the task sends `pg_cancel_backend` from the
//! canceller while it keeps reading the same stream, within
//! `STOPPED_WRITE_SETTLE`, and `stop_event` decides from what comes back:
//!
//! * `57014` — the server says it interrupted the statement. That proves the
//!   rollback of a plain data write (`StatementIntent::Write`) and of nothing
//!   else: `Cancelled` for it, `OutcomeUnknown` for a `CALL`, a `DO` block, a
//!   structure or privilege change or a statement of unknown form, since a
//!   procedure may already have committed part of its work before the one
//!   statement that was interrupted;
//! * the end of the stream — `ReadyForQuery`, not the `CommandComplete` before
//!   it — the write ran: `Finished`, with its affected rows;
//! * any other error, or silence until the bound: `OutcomeUnknown`.
//!
//! A write whose cursor was dropped is never `Cancelled` either. A read keeps
//! `Cancelled`: nothing can have changed. A statement the task never polled has
//! not left: that is the only write a stop may still call cancelled without
//! asking the server.

use std::time::{Duration, Instant};

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::{Stream, StreamExt as _};
use oxyn_core::{
    CancelToken, DriverId, ErrorClass, ExecLimits, ExecStats, OxynError, Result, StatementHandle,
    StatementIntent,
};
use oxyn_driver::Cursor;
use sqlx::postgres::{PgArguments, PgQueryResult, PgRow, PgStatement};
use sqlx::{Column as _, Row as _, SqlStr, TypeInfo as _};
use sqlx::{Either, Statement as _};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::cancel::{BackendCanceller, StatementRegistry, Verdict, VerdictSender};
use crate::decode::BatchAssembler;
use crate::error::{Bound, INTERRUPTED_BUT_KEPT_MESSAGE, is_query_canceled, map_stream_error};
use crate::lease::Lease;
use crate::session::{SQL_RESET_SEARCH_PATH, SQL_ROLLBACK, reset_after_write};
use crate::types::{META_FALLBACK, META_PG_TYPE, PgDecoding};
use crate::variant::PostgresFlavor;

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

/// How long a stopped write is read for the server's answer, and how long its
/// cancellation request may take.
///
/// Strictly below the grace `oxyn-exec` gives a stopped write whose driver has
/// not ruled (10 s): past that, the executor reports its own `OutcomeUnknown`
/// and the verdict computed here is never seen. The same order as the MySQL
/// driver's drain bound. A cancellation that lands on a healthy server answers
/// in milliseconds; this bound only matters when it does not.
const STOPPED_WRITE_SETTLE: Duration = Duration::from_secs(5);

/// What a write interrupted while its returned rows were being read says.
const DRAIN_CUT_MESSAGE: &str = "the write was interrupted while its returned rows were being \
     read: the server either committed it or rolled it back; check the data before running it \
     again";

/// What a stopped write says when the server did not prove it was cancelled.
///
/// Fixed text: nothing the server sent, no bound value, no host (I-03).
const UNCONFIRMED_STOP_MESSAGE: &str = "the write was stopped and the server did not confirm the \
     cancellation: it either committed it or rolled it back; check the data before running it \
     again";

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

/// What reading a stopped write's own connection brought back, once the server
/// had been asked to cancel it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterCancel {
    /// The stream ran to its end — `ReadyForQuery`. The statement completed: a
    /// `CommandComplete` alone would not say so, since an autocommit statement
    /// commits at `Sync`, after it.
    Ended {
        /// Rows affected by the `CommandComplete` messages read since the stop.
        affected: u64,
        /// Did rows come after the stop? They were read and dropped, neither
        /// decoded nor kept ([I-06](../../../CLAUDE.md#i-06)).
        rows: bool,
    },
    /// The server answered `57014`: it interrupted the statement.
    QueryCanceled,
    /// The stream failed with anything else. Said nothing of the write's fate:
    /// a transport error, or the server's own, whose rollback this cursor
    /// cannot tell from a late commit failure.
    Failed,
    /// The bound elapsed with the stream silent.
    TimedOut,
}

/// What `stop_event` needs to rule on a stop.
#[derive(Debug, Clone, Copy)]
struct Stopped {
    /// Why the loop stopped.
    stop: Halt,
    /// Did the statement leave, and does it mutate? Not set for a statement
    /// that was never polled: nothing went out.
    write_sent: bool,
    /// What the statement is. It decides what a `57014` proves: only a plain
    /// data write ([`StatementIntent::Write`]) is rolled back by it, a `CALL`
    /// (`Unknown`) keeps what its procedure already committed.
    intent: StatementIntent,
    /// Was the write stopped once its rows had started to arrive?
    drain_cut: bool,
    /// What reading the connection after the cancellation brought back, when
    /// that read took place.
    settle: Option<AfterCancel>,
    /// Rows affected by the `CommandComplete` messages read before the stop.
    affected: u64,
    /// Did a write's drain pass rows the user will not see?
    rows_dropped: bool,
    /// The execution's time bound, reported by [`Halt::TimedOut`].
    timeout: Duration,
}

/// The terminal event of a stream that stopped for a reason other than its end
/// or an error it already emitted.
///
/// A pure function of what the task observed, so that every combination —
/// including the hostile ones — is tested without a server. Returns `None` for
/// the halts the task itself finishes, because they need the batch emission:
/// [`Halt::Failed`], [`Halt::Exhausted`] and [`Halt::RowLimit`].
///
/// `Cancelled` means nothing was applied ([I-13](../../../CLAUDE.md#i-13)), so
/// it leaves only for a statement that did not mutate, one that never left, or
/// a plain data write the server itself reported as interrupted.
fn stop_event(stopped: Stopped) -> Option<CursorEvent> {
    let Stopped {
        stop,
        write_sent,
        intent,
        drain_cut,
        settle,
        affected,
        rows_dropped,
        timeout,
    } = stopped;
    let failed = |error: OxynError| Some(CursorEvent::Failed(Box::new(error)));
    let unknown = |message: &str| failed(OxynError::OutcomeUnknown(message.to_owned()));

    match stop {
        Halt::Failed | Halt::Exhausted | Halt::RowLimit => None,
        // `Timeout` is ambiguous by construction: the server may have applied
        // the write (I-13). `OxynError::class` says so, not this message.
        Halt::TimedOut => failed(OxynError::Timeout { after: timeout }),
        // A read, or a write that was never polled: nothing can have changed.
        // A cut drain had necessarily left, whatever `write_sent` says.
        Halt::Cancelled | Halt::Abandoned if !(write_sent || drain_cut) => {
            failed(OxynError::Cancelled)
        }
        // Not `Cancelled`: that would read as "nothing happened" and invite
        // running the write again (I-13).
        Halt::Cancelled | Halt::Abandoned if drain_cut => unknown(DRAIN_CUT_MESSAGE),
        // Nobody reads the answer of an abandoned cursor, and the stop was
        // not settled: a write that was sent is never `Cancelled` on that path.
        Halt::Abandoned => unknown(UNCONFIRMED_STOP_MESSAGE),
        Halt::Cancelled => match settle {
            Some(AfterCancel::Ended {
                affected: settled,
                rows,
            }) => Some(CursorEvent::Finished {
                affected_rows: affected.saturating_add(settled),
                truncated: rows || rows_dropped,
            }),
            // The server rolled the statement back: for a plain data write,
            // that is all there was. A `CALL` is not undone by it, see the
            // message.
            Some(AfterCancel::QueryCanceled) if intent == StatementIntent::Write => {
                failed(OxynError::Cancelled)
            }
            Some(AfterCancel::QueryCanceled) => unknown(INTERRUPTED_BUT_KEPT_MESSAGE),
            Some(AfterCancel::Failed | AfterCancel::TimedOut) | None => {
                unknown(UNCONFIRMED_STOP_MESSAGE)
            }
        },
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
    /// The server flavor, which limits the session reset syntax.
    pub(crate) flavor: PostgresFlavor,
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
        flavor,
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

    let stopped = {
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

        // Has the statement left? `fetch_many` is lazy: nothing is written to
        // the connection before the stream's first poll, which `next_step`
        // records. Until then a stop is truthfully a cancellation; from then
        // on the server may have run the write.
        let mut sent = false;
        let stop = loop {
            let step = next_step(&mut stream, &cancel, deadline, &mut sent).await;

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
            // ends. A consumer that stops reading at N and drops the cursor
            // cancels the token: emitted now, row N would let it cut the very
            // drain that keeps the write from being rolled back. The executor
            // reads a write to its end, but the cursor does not rely on it.
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
        };

        // Was a write stopped once its rows had started to arrive? A
        // `RETURNING` runs whole before its first row: only the commit, at
        // `Sync`, is left, and the server may make it before the cancellation
        // lands — during the drain past the bound as well as before it. The
        // deadline needs no such care: `Timeout` is already ambiguous.
        let drain_cut = !confirm_end && (produced > 0 || rows_dropped);
        // `confirm_end` already folds the intent and the reported risk: a
        // statement that may mutate counts, whatever its text looks like.
        let write_sent = !confirm_end && sent;

        // A write that left, stopped by the user, is read to its answer while
        // this task still holds the connection: reading it is the only way to
        // tell a rolled-back statement from one whose commit was already made.
        // The returned-rows case (`drain_cut`) keeps its own verdict, and so do
        // the stops that nobody waits for (a dropped cursor) or that are
        // ambiguous by construction (the deadline).
        let settle = if stop == Halt::Cancelled && write_sent && !drain_cut {
            // The stop is decided: a token fired after this point changes
            // nothing any more.
            statements.forget(handle);
            Some(
                settle_stopped_write(
                    &mut stream,
                    canceller.cancel_backend(backend_pid),
                    &verdict,
                    STOPPED_WRITE_SETTLE,
                )
                .await,
            )
        } else {
            None
        };

        Stopped {
            stop,
            write_sent,
            intent,
            drain_cut,
            settle,
            affected,
            rows_dropped,
            timeout: limits.timeout.unwrap_or_default(),
        }
    };
    let stop = stopped.stop;

    // The stop is decided: a token fired after this point changes nothing any
    // more. The task removes itself from the registry, and it — not
    // `Session::cancel` — sends the cancellation if one is needed (see the
    // `cancel` module).
    statements.forget(handle);

    // The stream is dropped: the borrow on the connection is lifted.
    if stopped.settle.is_some() {
        // The cancellation went out while the stream was still being read, and
        // its verdict is settled. The connection is closed all the same: the
        // signal may still land after the server answered, and a connection
        // that goes back to the pool would hand it to the next query.
        connection.discard();
    } else if stop.needs_server_cancel() {
        // A connection whose stream was abandoned may have kept unread bytes:
        // returning it to the pool would desynchronize the next borrower. It is
        // held **until after** the cancellation is sent: as long as it is, no
        // other query can run on `backend_pid`.
        connection.discard();
        report_cancel(&verdict, canceller.cancel_backend(backend_pid).await);
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
                reset_after_write(flavor),
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

    // The stops that end on an error or on a verdict are ruled by one pure
    // function, tested without a server. What it leaves are the halts that
    // finish with a result, or that already emitted their error.
    if let Some(event) = stop_event(stopped) {
        let _ = events.send(event).await;
        return;
    }
    match stop {
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
        // Ruled above, or already emitted.
        Halt::Failed | Halt::Cancelled | Halt::TimedOut | Halt::Abandoned => {}
    }
}

/// One element of the sqlx stream: the end of a statement, a row, or an error.
type StreamItem = std::result::Result<Either<PgQueryResult, PgRow>, sqlx::Error>;

/// What one iteration of the loop produced.
enum Step {
    /// The stream returned an element.
    Received(StreamItem),
    /// The stream is exhausted.
    End,
    /// An interruption won the race.
    Interrupted(Halt),
}

/// Waits for the stream's next element, the cancellation or the deadline.
///
/// `sent` is set when the stream is first **polled**, which is when `sqlx`
/// writes the statement. A token that already fired is chosen without polling
/// it: `sent` then stays `false`, and the statement did not leave.
///
/// The stream branch is raced, so its future may be dropped when another
/// branch wins. That drops one `poll_next` call and nothing else: the `sqlx`
/// generator keeps its own state and resumes where it was. It is a designed
/// resumption point — the stopped-write settling polls this same stream again —
/// and not a hope that no byte was consumed.
async fn next_step<S>(
    stream: &mut S,
    cancel: &CancelToken,
    deadline: Option<tokio::time::Instant>,
    sent: &mut bool,
) -> Step
where
    S: Stream<Item = StreamItem> + Unpin,
{
    tokio::select! {
        // `biased`: an already requested cancellation always wins over a ready
        // batch. Without it, a fast stream can make the cancellation wait
        // indefinitely. For a write, what it hides — a `CommandComplete`
        // already buffered — is read afterwards, by `settle_stopped_write`.
        biased;
        () = cancel.cancelled() => Step::Interrupted(Halt::Cancelled),
        () = sleep_until_deadline(deadline) => Step::Interrupted(Halt::TimedOut),
        received = async {
            *sent = true;
            stream.next().await
        } => match received {
            Some(result) => Step::Received(result),
            None => Step::End,
        },
    }
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

/// Settles the verdict of the cancellation `Session::cancel` may be waiting for.
///
/// Reported here, and returned to `Session::cancel` if it is waiting: a dropped
/// cursor, for its part, has nobody to tell.
fn report_cancel(verdict: &VerdictSender, issue: Result<()>) {
    match issue {
        Ok(()) => verdict.settle(Verdict::Cancelled),
        Err(error) => {
            tracing::warn!(
                target: "oxyn::driver::postgres",
                error = %error,
                "server-side cancellation did not succeed"
            );
            verdict.settle(Verdict::CancelFailed(Arc::new(error)));
        }
    }
}

/// Sends the cancellation of a write that left, and reads its answer from the
/// same connection, both within `window`.
///
/// The cancellation and the read run side by side: the server only answers
/// once it has been asked to stop, and the answer only means something if
/// nothing else was sent on this connection meanwhile. The caller holds the
/// connection throughout, which is also what keeps another query from starting
/// on the targeted process (see the `cancel` module).
///
/// The verdict is settled when the **request** returns, not when the read ends:
/// `Session::cancel` answers whether the cancellation went out, while what
/// became of the write is the cursor's terminal event. A request that does not
/// return within `window` settles as a failure.
async fn settle_stopped_write<S>(
    stream: &mut S,
    cancel_request: impl Future<Output = Result<()>>,
    verdict: &VerdictSender,
    window: Duration,
) -> AfterCancel
where
    S: Stream<Item = StreamItem> + Unpin,
{
    let deadline = tokio::time::Instant::now() + window;
    let request = async {
        let issue = tokio::time::timeout_at(deadline, cancel_request)
            .await
            .unwrap_or_else(|_elapsed| Err(OxynError::Timeout { after: window }));
        report_cancel(verdict, issue);
    };
    let (answer, ()) = tokio::join!(read_after_cancel(stream, deadline), request);
    answer
}

/// Reads a stopped write's stream to its end, or to `deadline`.
///
/// Nothing is decoded, nothing is kept: a row that arrives is dropped as the
/// stream yields it, so a consumer that stopped reading cannot stall this read
/// ([I-06](../../../CLAUDE.md#i-06)). The stream is only taken to be over when
/// it ends — `ReadyForQuery` — and not at the first `CommandComplete`: the
/// implicit transaction of the extended protocol commits at `Sync`, after it,
/// and a failure of that commit comes back as an error behind it.
///
/// The error is read for its SQLSTATE only and never formatted: `sqlx` errors
/// can carry the connection URL ([I-03](../../../CLAUDE.md#i-03)).
async fn read_after_cancel<S>(stream: &mut S, deadline: tokio::time::Instant) -> AfterCancel
where
    S: Stream<Item = StreamItem> + Unpin,
{
    let mut affected: u64 = 0;
    let mut rows = false;
    loop {
        // Dropping the timed-out `next()` is sound here for the reason given at
        // `next_step`, and anyway nothing reads this stream afterwards.
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Err(_elapsed) => return AfterCancel::TimedOut,
            Ok(None) => return AfterCancel::Ended { affected, rows },
            Ok(Some(Ok(Either::Left(done)))) => {
                affected = affected.saturating_add(done.rows_affected());
            }
            Ok(Some(Ok(Either::Right(_row)))) => rows = true,
            Ok(Some(Err(error))) => {
                return if is_query_canceled(&error) {
                    AfterCancel::QueryCanceled
                } else {
                    AfterCancel::Failed
                };
            }
        }
    }
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
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    use super::*;
    use crate::cancel::StatementRegistry;
    use crate::error::fakes::{cut, server_error};

    /// The SQLSTATE of "canceling statement due to user request".
    const QUERY_CANCELED: &str = "57014";

    /// The `CommandComplete` of a statement that affected no row — the only
    /// one a test can build: `PgQueryResult` has no public constructor, so the
    /// affected-rows arithmetic is tested on `stop_event` instead.
    fn command_complete() -> StreamItem {
        Ok(Either::Left(PgQueryResult::default()))
    }

    /// A stream that answers nothing, and counts how often it is asked.
    struct Silent {
        polls: usize,
    }

    impl Stream for Silent {
        type Item = StreamItem;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<StreamItem>> {
            self.polls += 1;
            Poll::Pending
        }
    }

    /// A stream whose first poll finds nothing and whose later polls find the
    /// bytes that arrived since: the shape of a `CommandComplete` that reaches
    /// the socket while the token fires.
    struct AnswersAfterTheFirstPoll {
        polled: bool,
        items: std::vec::IntoIter<StreamItem>,
    }

    impl Stream for AnswersAfterTheFirstPoll {
        type Item = StreamItem;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<StreamItem>> {
            if !std::mem::replace(&mut self.polled, true) {
                return Poll::Pending;
            }
            Poll::Ready(self.items.next())
        }
    }

    /// A stop with nothing known about it: the caller fills in what the test
    /// is about.
    fn stopped(stop: Halt) -> Stopped {
        Stopped {
            stop,
            // A plain data write unless a test says otherwise: the one
            // statement a `57014` proves the rollback of.
            intent: StatementIntent::Write,
            write_sent: false,
            drain_cut: false,
            settle: None,
            affected: 0,
            rows_dropped: false,
            timeout: Duration::from_secs(3),
        }
    }

    /// The error a stop emitted. Panics, with the event, on anything else.
    fn failure(event: Option<CursorEvent>) -> OxynError {
        match event {
            Some(CursorEvent::Failed(error)) => *error,
            other => panic!("expected a failure event, got {other:?}"),
        }
    }

    /// An outcome the user must check by hand: ambiguous, so never replayed.
    fn assert_unknown(error: &OxynError, context: &str) {
        assert!(
            matches!(error, OxynError::OutcomeUnknown(_)),
            "{context}: {error:?}"
        );
        assert!(!error.is_cancelled(), "{context}: {error:?}");
        assert!(!error.is_retryable(), "{context}: {error:?} (I-13)");
        assert_eq!(error.class(), ErrorClass::Ambiguous, "{context}");
    }

    /// Every verdict the read after a cancellation can bring back.
    fn every_settle() -> [Option<AfterCancel>; 6] {
        [
            None,
            Some(AfterCancel::Ended {
                affected: 3,
                rows: false,
            }),
            Some(AfterCancel::Ended {
                affected: 0,
                rows: true,
            }),
            Some(AfterCancel::QueryCanceled),
            Some(AfterCancel::Failed),
            Some(AfterCancel::TimedOut),
        ]
    }

    #[tokio::test]
    async fn a_token_that_already_fired_wins_before_the_statement_is_polled() {
        // The one stop of a write that is truthfully a cancellation: `sqlx` is
        // lazy, nothing was written to the connection.
        let token = CancelToken::new();
        token.cancel();
        let mut stream = Silent { polls: 0 };
        let mut sent = false;

        let step = next_step(&mut stream, &token, None, &mut sent).await;

        assert!(matches!(step, Step::Interrupted(Halt::Cancelled)));
        assert!(!sent, "the statement did not leave");
        assert_eq!(stream.polls, 0, "polling the stream would send the write");
    }

    #[tokio::test]
    async fn a_stop_after_the_first_poll_finds_the_statement_sent() {
        let token = CancelToken::new();
        let mut stream = Silent { polls: 0 };
        let mut sent = false;

        let (step, ()) = tokio::join!(next_step(&mut stream, &token, None, &mut sent), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            token.cancel();
        });

        assert!(matches!(step, Step::Interrupted(Halt::Cancelled)));
        assert!(sent, "the stream was polled: the write may have left");
    }

    #[tokio::test]
    async fn the_deadline_is_a_timeout_and_not_a_cancellation() {
        let token = CancelToken::new();
        let mut stream = Silent { polls: 0 };
        let mut sent = false;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);

        let step = next_step(&mut stream, &token, Some(deadline), &mut sent).await;

        assert!(matches!(step, Step::Interrupted(Halt::TimedOut)));
        assert!(sent);
    }

    #[tokio::test]
    async fn a_command_complete_buffered_when_the_token_fired_is_read_to_its_end() {
        // Issue #220, end to end on a fake stream: the token wins the `biased`
        // select over an answer that reaches the socket at the same moment, and
        // the settling read still finds it.
        let token = CancelToken::new();
        let mut stream = AnswersAfterTheFirstPoll {
            polled: false,
            items: vec![command_complete()].into_iter(),
        };
        let mut sent = false;
        let (step, ()) = tokio::join!(next_step(&mut stream, &token, None, &mut sent), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            token.cancel();
        });
        assert!(matches!(step, Step::Interrupted(Halt::Cancelled)));
        assert!(sent);

        let registry = StatementRegistry::default();
        let verdict = registry.register(StatementHandle::new(), CancelToken::new());
        let answer = settle_stopped_write(
            &mut stream,
            async { Ok(()) },
            &verdict,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(
            answer,
            AfterCancel::Ended {
                affected: 0,
                rows: false
            },
            "the write ran to its end"
        );

        let event = stop_event(Stopped {
            write_sent: sent,
            settle: Some(answer),
            ..stopped(Halt::Cancelled)
        });
        assert!(
            matches!(
                event,
                Some(CursorEvent::Finished {
                    affected_rows: 0,
                    truncated: false
                })
            ),
            "a write that ran is finished, never cancelled: {event:?}"
        );
    }

    #[tokio::test]
    async fn an_answer_that_ends_the_stream_is_read_to_its_end() {
        for items in [vec![], vec![command_complete(), command_complete()]] {
            let mut stream = futures::stream::iter(items);
            let answer = read_after_cancel(
                &mut stream,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await;
            assert_eq!(
                answer,
                AfterCancel::Ended {
                    affected: 0,
                    rows: false
                }
            );
        }
    }

    #[tokio::test]
    async fn a_57014_answer_proves_the_cancellation() {
        // Alone, or behind the `CommandComplete` of a statement whose commit
        // was the part interrupted.
        for items in [
            vec![Err(server_error(QUERY_CANCELED))],
            vec![command_complete(), Err(server_error(QUERY_CANCELED))],
        ] {
            let mut stream = futures::stream::iter(items);
            let answer = read_after_cancel(
                &mut stream,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await;
            assert_eq!(answer, AfterCancel::QueryCanceled);
        }
    }

    #[tokio::test]
    async fn another_error_or_silence_is_not_a_proof() {
        // A constraint violation, a serialization failure, an administrator's
        // shutdown, a cut, a dead worker: none says the write was rolled back
        // because Oxyn asked.
        let errors = [
            server_error("23505"),
            server_error("40001"),
            server_error("57P01"),
            cut(),
            sqlx::Error::WorkerCrashed,
        ];
        for error in errors {
            let mut stream = futures::stream::iter(vec![Err(error), command_complete()]);
            let answer = read_after_cancel(
                &mut stream,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await;
            assert_eq!(answer, AfterCancel::Failed);
        }

        let mut silent = futures::stream::pending::<StreamItem>();
        let answer = read_after_cancel(
            &mut silent,
            tokio::time::Instant::now() + Duration::from_millis(50),
        )
        .await;
        assert_eq!(answer, AfterCancel::TimedOut);
    }

    #[tokio::test]
    async fn the_cancellation_verdict_is_settled_before_the_read_ends() {
        // `Session::cancel` answers whether the cancellation went out; it must
        // not wait for the write's own answer, which can take the whole bound.
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        let verdict = registry.register(handle, CancelToken::new());
        let read_over = AtomicBool::new(false);
        let mut silent = futures::stream::pending::<StreamItem>();

        let (answer, prompt) = tokio::join!(
            async {
                let answer = settle_stopped_write(
                    &mut silent,
                    async { Ok(()) },
                    &verdict,
                    Duration::from_millis(300),
                )
                .await;
                read_over.store(true, Ordering::SeqCst);
                answer
            },
            async {
                registry
                    .cancel(&DriverId::postgres(), handle)
                    .await
                    .expect("the cancellation went out");
                !read_over.load(Ordering::SeqCst)
            }
        );

        assert_eq!(answer, AfterCancel::TimedOut, "the server stayed silent");
        assert!(prompt, "the verdict waited for the end of the read");
    }

    #[tokio::test]
    async fn a_cancellation_that_never_returns_fails_the_verdict_and_leaves_the_answer() {
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        let verdict = registry.register(handle, CancelToken::new());
        let mut stream = futures::stream::iter(vec![command_complete()]);

        let answer = settle_stopped_write(
            &mut stream,
            std::future::pending::<Result<()>>(),
            &verdict,
            Duration::from_millis(50),
        )
        .await;

        assert!(
            matches!(answer, AfterCancel::Ended { .. }),
            "the write's answer is read whatever became of the request: {answer:?}"
        );
        let issue = tokio::time::timeout(
            Duration::from_secs(2),
            registry.cancel(&DriverId::postgres(), handle),
        )
        .await
        .expect("the verdict is always settled");
        let error = issue.expect_err("a request that did not return did not go out");
        assert_eq!(error.class(), ErrorClass::Ambiguous, "{error:?}");
    }

    #[tokio::test]
    async fn a_cancellation_that_fails_is_reported_and_the_read_goes_on() {
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        let verdict = registry.register(handle, CancelToken::new());
        let mut stream = futures::stream::iter(vec![Err(server_error(QUERY_CANCELED))]);

        let answer = settle_stopped_write(
            &mut stream,
            async { Err(OxynError::Connection("refused".to_owned())) },
            &verdict,
            Duration::from_secs(2),
        )
        .await;

        // The statement was interrupted anyway — by a statement timeout, say:
        // the server's own answer is the proof, wherever it came from.
        assert_eq!(answer, AfterCancel::QueryCanceled);
        let issue = registry.cancel(&DriverId::postgres(), handle).await;
        let error = issue.expect_err("the caller learns the request failed");
        assert_eq!(error.class(), ErrorClass::Transient, "{error:?}");
    }

    #[test]
    fn a_finished_write_reports_its_affected_rows() {
        for (before, settled, rows, rows_dropped, total, truncated) in [
            (7, 0, false, false, 7, false),
            (7, 3, false, false, 10, false),
            (0, 0, true, false, 0, true),
            (0, 0, false, true, 0, true),
            (u64::MAX, 5, false, false, u64::MAX, false),
        ] {
            let event = stop_event(Stopped {
                write_sent: true,
                settle: Some(AfterCancel::Ended {
                    affected: settled,
                    rows,
                }),
                affected: before,
                rows_dropped,
                ..stopped(Halt::Cancelled)
            });
            assert!(
                matches!(
                    event,
                    Some(CursorEvent::Finished { affected_rows, truncated: cut })
                        if affected_rows == total && cut == truncated
                ),
                "{before} + {settled}: {event:?}"
            );
        }
    }

    #[test]
    fn a_write_stopped_before_its_first_poll_is_cancelled() {
        // Nothing went out: there is no write to be unsure about.
        for stop in [Halt::Cancelled, Halt::Abandoned] {
            let error = failure(stop_event(stopped(stop)));
            assert!(error.is_cancelled(), "{stop:?}: {error:?}");
        }
    }

    #[test]
    fn a_read_keeps_its_cancellation() {
        // A read never sets `write_sent`, and nothing it could read after the
        // stop changes that: nothing can have changed on the server.
        for stop in [Halt::Cancelled, Halt::Abandoned] {
            for settle in every_settle() {
                let error = failure(stop_event(Stopped {
                    settle,
                    ..stopped(stop)
                }));
                assert!(error.is_cancelled(), "{stop:?}, {settle:?}: {error:?}");
            }
        }
    }

    #[test]
    fn a_write_cut_while_its_rows_were_read_is_ambiguous() {
        for stop in [Halt::Cancelled, Halt::Abandoned] {
            for write_sent in [false, true] {
                for settle in every_settle() {
                    let error = failure(stop_event(Stopped {
                        write_sent,
                        drain_cut: true,
                        settle,
                        ..stopped(stop)
                    }));
                    assert_unknown(&error, &format!("{stop:?}, {write_sent}, {settle:?}"));
                    assert!(error.to_string().contains("returned rows"), "{error}");
                }
            }
        }
    }

    #[test]
    fn an_abandoned_write_that_was_sent_is_never_cancelled() {
        // Nobody reads the answer of a dropped cursor, so nothing proves what
        // became of the write — whatever a settling read may have found.
        for settle in every_settle() {
            let error = failure(stop_event(Stopped {
                write_sent: true,
                settle,
                ..stopped(Halt::Abandoned)
            }));
            assert_unknown(&error, &format!("{settle:?}"));
        }
    }

    #[test]
    fn a_stopped_write_is_cancelled_only_on_57014() {
        let write = |settle| {
            stop_event(Stopped {
                write_sent: true,
                settle,
                ..stopped(Halt::Cancelled)
            })
        };

        let error = failure(write(Some(AfterCancel::QueryCanceled)));
        assert!(error.is_cancelled(), "{error:?}");

        // No read took place, the read failed, the read ran out of time: the
        // write is not proven rolled back.
        for settle in [None, Some(AfterCancel::Failed), Some(AfterCancel::TimedOut)] {
            assert_unknown(&failure(write(settle)), &format!("{settle:?}"));
        }
    }

    #[test]
    fn a_57014_proves_the_rollback_of_a_plain_data_write_only() {
        // A `CALL` is `Unknown` to the classifier, a `DO` block too: the server
        // interrupts the statement it was running, not what the procedure had
        // already committed. Structure and privilege changes are not undone as
        // a whole by every command either.
        for intent in [
            StatementIntent::Unknown,
            StatementIntent::Ddl,
            StatementIntent::Grant,
        ] {
            let error = failure(stop_event(Stopped {
                write_sent: true,
                intent,
                settle: Some(AfterCancel::QueryCanceled),
                ..stopped(Halt::Cancelled)
            }));
            assert_unknown(&error, &format!("{intent:?}"));
            let OxynError::OutcomeUnknown(message) = error else {
                panic!("expected an unknown outcome");
            };
            assert_eq!(message, INTERRUPTED_BUT_KEPT_MESSAGE);
        }
    }

    #[test]
    fn only_a_plain_data_write_interrupted_by_the_server_is_ever_cancelled() {
        // The property under every row above, swept over what the stop can
        // carry: `Cancelled` leaves for a write that was sent in exactly one
        // case.
        let intents = [
            StatementIntent::Read,
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Grant,
            StatementIntent::Unknown,
        ];
        for intent in intents {
            for stop in [Halt::Cancelled, Halt::Abandoned] {
                for settle in every_settle() {
                    let event = stop_event(Stopped {
                        write_sent: true,
                        intent,
                        settle,
                        ..stopped(stop)
                    });
                    let cancelled =
                        matches!(&event, Some(CursorEvent::Failed(error)) if error.is_cancelled());
                    assert_eq!(
                        cancelled,
                        stop == Halt::Cancelled
                            && intent == StatementIntent::Write
                            && settle == Some(AfterCancel::QueryCanceled),
                        "{intent:?}, {stop:?}, {settle:?}: {event:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unconfirmed_stop_says_what_to_do_and_quotes_nothing() {
        let error = failure(stop_event(Stopped {
            write_sent: true,
            settle: Some(AfterCancel::Failed),
            ..stopped(Halt::Cancelled)
        }));
        let OxynError::OutcomeUnknown(message) = error else {
            panic!("expected an unknown outcome: {error:?}");
        };
        assert_eq!(message, UNCONFIRMED_STOP_MESSAGE);
        assert!(message.contains("check the data"), "{message}");
    }

    #[test]
    fn a_timed_out_write_stays_a_timeout() {
        // Ambiguous by construction, whether or not anything was read after.
        for write_sent in [false, true] {
            for settle in every_settle() {
                let error = failure(stop_event(Stopped {
                    write_sent,
                    settle,
                    ..stopped(Halt::TimedOut)
                }));
                assert!(
                    matches!(error, OxynError::Timeout { after } if after == Duration::from_secs(3)),
                    "{error:?}"
                );
                assert!(!error.is_retryable(), "I-13");
            }
        }
    }

    #[test]
    fn the_halts_the_task_finishes_itself_are_left_alone() {
        for stop in [Halt::Failed, Halt::Exhausted, Halt::RowLimit] {
            assert!(stop_event(stopped(stop)).is_none(), "{stop:?}");
        }
    }

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
