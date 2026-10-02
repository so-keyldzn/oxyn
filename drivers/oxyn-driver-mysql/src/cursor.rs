//! The batch stream, and the cancellation that really reaches the server.
//!
//! # Why a task and a channel
//!
//! `mysql_async`'s result borrows the connection: a cursor owning both would
//! be self-referential. The task owns the session's connection — through its
//! lease — for as long as the result is read, and pushes batches into a
//! **one-slot** channel: it decodes the next batch only once the previous one
//! has been taken, so a `SELECT *` over 500 GB never grows memory beyond two
//! batches ([I-06](../../../CLAUDE.md#i-06)).
//!
//! # Stopping for good cancels on the server
//!
//! A tab closed, a Stop, the timeout, the row bound reached with rows still
//! coming: the server is still sending, and left alone it would hold its locks
//! until `net_write_timeout`. The task sends `KILL QUERY` from a second
//! connection while holding its own, then reads on until the server confirms
//! the interruption — only that confirmation lets the connection, and the
//! user's transaction, live on (ADR-0050 §7). `Drop` asks the
//! task to stop; it never aborts it, since an aborted task can kill nothing.
//!
//! # One statement, one result
//!
//! ADR-0050 §5: a `CALL` legitimately returns several result sets — the first
//! is the result, the next ones are counted, and the result is marked
//! truncated so that nothing is dropped in silence. Any other statement that
//! yields a second result set means the server ran more than Oxyn sent: the
//! driver reports it and closes the connection.

use std::sync::Arc;
use std::time::Instant;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::StreamExt as _;
use mysql_async::prelude::{Protocol, Queryable as _};
use mysql_async::{Error as MyError, Params, QueryResult, Row, Statement};
use oxyn_core::{
    CancelToken, DriverId, ErrorClass, ExecLimits, ExecStats, OxynError, Result, StatementHandle,
    StatementIntent, TransactionState,
};
use oxyn_driver::Cursor;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::cancel::{DRAIN_BOUND, DrainVerdict, Killer, StatementRegistry, Verdict, VerdictSender};
use crate::connection::Lease;
use crate::decode::BatchAssembler;
use crate::error::{Bound, MysqlError, is_query_interrupted, map_stream_error};
use crate::session::SQL_READ_WRITE;
use crate::types::{panics_in_binary_protocol, schema_for, type_name};

/// Accumulated bytes beyond which a batch is closed and emitted — in bytes,
/// not rows: a thousand one-megabyte BLOBs make a gigabyte.
pub const BATCH_BYTE_BUDGET: usize = 1 << 20;

/// Rows beyond which a batch is closed whatever its size, so that narrow
/// columns still reach the screen quickly.
pub const BATCH_ROW_CEILING: usize = 8_192;

/// How much of a read's result the task reads past the row bound, discarding
/// it, before it concludes more is coming and kills the statement. A write is
/// never killed at its bound: it is drained to its end.
///
/// A result a few rows longer than the bound has usually been sent whole
/// already: reading it out costs little, where a kill arriving after the
/// statement's end could not be proven spent and would cost the connection —
/// and the user's open transaction with it (ADR-0050 §7).
const READ_AHEAD_ROWS: usize = 1_024;
/// The time bound of the same read-ahead.
const READ_AHEAD_TIME: std::time::Duration = std::time::Duration::from_millis(200);

/// What the stream task pushes to the cursor.
#[derive(Debug)]
enum CursorEvent {
    Batch(RecordBatch),
    Finished { affected_rows: u64, truncated: bool },
    Failed(Box<OxynError>),
}

/// Why the stream stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Halt {
    /// The server has nothing more to send.
    Exhausted,
    /// The row bound is reached **and** more rows were coming.
    RowLimit,
    /// The token fired.
    Cancelled,
    /// [`ExecLimits::timeout`] elapsed.
    TimedOut,
    /// The cursor was dropped.
    Abandoned,
    /// An error was met and recorded.
    Failed,
}

/// A `RecordBatch` stream fed by a MySQL execution.
pub struct MysqlCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    events: mpsc::Receiver<CursorEvent>,
    /// This execution's own token: `Drop` fires it.
    cancel: CancelToken,
    task: JoinHandle<()>,
    stats: ExecStats,
    started: Instant,
    finished: bool,
    projects_columns: bool,
}

impl MysqlCursor {
    /// Is the stream task finished? After a cancellation it must stop on its
    /// own, without having been aborted.
    #[must_use]
    pub fn task_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl std::fmt::Debug for MysqlCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MysqlCursor")
            .field("handle", &self.handle)
            .field("columns", &self.schema.fields().len())
            .field("finished", &self.finished)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Drop for MysqlCursor {
    /// Requests the stop; does not impose it.
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
        }
    }
}

#[async_trait]
impl Cursor for MysqlCursor {
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
            None => {
                self.seal(true);
                Err(OxynError::Internal(
                    "the MySQL streaming task stopped without concluding".to_owned(),
                ))
            }
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

impl MysqlCursor {
    fn seal(&mut self, truncated: bool) {
        self.finished = true;
        self.stats.total_time = self.started.elapsed();
        if truncated {
            self.stats.mark_truncated();
        }
    }
}

/// Where an execution's rows come from.
pub(crate) enum Source {
    /// The prepared statement (ADR-0050 §2).
    Prepared {
        statement: Statement,
        params: Params,
        /// Is it a `CALL`, whose extra result sets are the procedure's own?
        call: bool,
    },
    /// The text protocol, after a 1295 on a text split into exactly one
    /// statement (ADR-0050 §3).
    Text { sql: String },
}

/// What the stream task needs.
pub(crate) struct StreamRequest {
    pub(crate) driver: DriverId,
    /// The session's connection, held until the task ends.
    pub(crate) lease: Lease,
    pub(crate) source: Source,
    pub(crate) bound: Bound,
    pub(crate) limits: ExecLimits,
    pub(crate) intent: StatementIntent,
    /// Was `SET SESSION TRANSACTION READ ONLY` issued for this execution?
    pub(crate) restore_read_write: bool,
    pub(crate) killer: Arc<Killer>,
    pub(crate) statements: Arc<StatementRegistry>,
    pub(crate) verdict: VerdictSender,
    pub(crate) handle: StatementHandle,
}

/// Starts the stream and returns the cursor that drains it, once the server
/// answered the execution: its first result set's columns are the schema, and
/// a statement without one leaves it empty.
///
/// The schema is never taken from the prepare. MySQL announces no columns at
/// prepare for `SHOW PROCESSLIST`, `OPTIMIZE TABLE`, `EXPLAIN` and others, and
/// other types than the rows carry for `SHOW INDEX` or a `SELECT ?` bound to a
/// number; MariaDB does the same for some (RESEARCH-NOTES, 2026-10-01). The
/// execution's columns are what the rows are, so they alone describe them.
pub(crate) async fn spawn(request: StreamRequest, cancel: CancelToken) -> MysqlCursor {
    let (send, reception) = mpsc::channel(1);
    let handle = request.handle;
    let token = cancel.clone();
    let (schema_sender, schema_reception) = oneshot::channel();

    let task = tokio::spawn(async move {
        run(request, token, send, schema_sender).await;
    });

    let mut cursor = MysqlCursor {
        handle,
        projects_columns: false,
        schema: Arc::new(arrow::datatypes::Schema::empty()),
        events: reception,
        cancel,
        task,
        stats: ExecStats::default(),
        started: Instant::now(),
        finished: false,
    };
    if let Ok(learnt) = schema_reception.await {
        cursor.projects_columns = !learnt.fields().is_empty();
        cursor.schema = learnt;
    }
    cursor
}

/// The state of the stream loop, handed from stage to stage.
struct Pump {
    driver: DriverId,
    cancel: CancelToken,
    deadline: Option<tokio::time::Instant>,
    events: mpsc::Sender<CursorEvent>,
    schema_sender: Option<oneshot::Sender<SchemaRef>>,
    assembler: BatchAssembler,
    limits: ExecLimits,
    intent: StatementIntent,
    bound: Bound,
    killer: Arc<Killer>,
    id: u64,
    produced: usize,
    affected: u64,
    /// Result sets after the first that carried columns (`CALL`).
    extra_sets: u64,
    /// Did the row bound cut rows off, even when no kill was needed?
    truncated: bool,
    /// The outcome of the kill, once one was needed.
    kill: Option<std::result::Result<DrainVerdict, Arc<OxynError>>>,
    /// Must the connection be closed whatever the kill said?
    close: bool,
    /// Was the connection left with a result the library could only drain by
    /// panicking? It is then leaked rather than closed.
    poisoned: bool,
    /// Binary protocol (prepared) or text protocol (the 1295 fallback)?
    binary: bool,
    /// The first error met, held back until [`Pump::conclude`]: a failure
    /// that costs the connection is reported as the transaction's loss
    /// instead, and a channel of one slot has room for one terminal event.
    failure: Option<OxynError>,
}

/// The body of the stream task.
async fn run(
    request: StreamRequest,
    cancel: CancelToken,
    events: mpsc::Sender<CursorEvent>,
    schema_sender: oneshot::Sender<SchemaRef>,
) {
    let StreamRequest {
        driver,
        mut lease,
        source,
        bound,
        limits,
        intent,
        restore_read_write,
        killer,
        statements,
        verdict,
        handle,
    } = request;

    let before = lease.transaction();
    let mut pump = Pump {
        driver,
        cancel,
        deadline: limits
            .timeout
            .map(|duration| tokio::time::Instant::now() + duration),
        events,
        schema_sender: Some(schema_sender),
        assembler: BatchAssembler::new(Arc::new(arrow::datatypes::Schema::empty()), &[]),
        limits,
        intent,
        bound,
        killer,
        id: lease.id(),
        produced: 0,
        affected: 0,
        extra_sets: 0,
        truncated: false,
        kill: None,
        close: false,
        poisoned: false,
        binary: matches!(source, Source::Prepared { .. }),
        failure: None,
    };

    let (stop, statement, sent) = match lease.conn() {
        // Cancelled before anything was sent: nothing runs, nothing to kill.
        Some(_) if pump.cancel.is_cancelled() => (
            Halt::Cancelled,
            match source {
                Source::Prepared { statement, .. } => Some(statement),
                Source::Text { .. } => None,
            },
            false,
        ),
        None => {
            pump.fail(OxynError::Connection(
                "the connection to the server is closed".to_owned(),
            ));
            (Halt::Failed, None, false)
        }
        Some(conn) => match source {
            Source::Prepared {
                statement,
                params,
                call,
            } => {
                let started = conn.exec_iter(&statement, params);
                let stop = pump.execute(started, call).await;
                (stop, Some(statement), true)
            }
            Source::Text { sql } => {
                let started = conn.query_iter(sql);
                (pump.execute(started, false).await, None, true)
            }
        },
    };

    // The stop is decided: the task leaves the registry, and it — not
    // `Session::cancel` — reports what the kill did.
    statements.forget(handle);
    match &pump.kill {
        None => verdict.settle(Verdict::Finished),
        Some(Ok(_)) => verdict.settle(Verdict::Cancelled),
        Some(Err(error)) => verdict.settle(Verdict::CancelFailed(Arc::clone(error))),
    }

    let lost = pump
        .settle_connection(&mut lease, statement, restore_read_write, before, sent)
        .await;
    drop(lease);
    pump.conclude(stop, lost).await;
}

impl Pump {
    /// Records an error; the first one is the cause, what follows its
    /// consequence.
    fn fail(&mut self, error: OxynError) {
        self.failure.get_or_insert(error);
    }

    /// Sends the statement and reads its result.
    async fn execute<'a, P>(
        &mut self,
        started: futures::future::BoxFuture<
            'a,
            std::result::Result<QueryResult<'a, 'static, P>, MyError>,
        >,
        call: bool,
    ) -> Halt
    where
        P: Protocol + Unpin,
    {
        let mut started = started;
        // `started` is polled first: its first poll writes the command, so a
        // kill decided now reaches a statement already sent rather than an
        // idle connection, where it would be lost.
        let interrupted = tokio::select! {
            biased;
            outcome = &mut started => match outcome {
                Ok(result) => return self.read(result, call).await,
                Err(error) => {
                    self.fail_with(&error);
                    return Halt::Failed;
                }
            },
            () = self.cancel.cancelled() => Some(Halt::Cancelled),
            () = sleep_until(self.deadline) => Some(Halt::TimedOut),
        };
        let Some(reason) = interrupted else {
            return Halt::Failed;
        };
        // The statement runs on the server and has not answered yet: stop it,
        // then wait for its answer before deciding about the connection.
        if !self.send_kill().await {
            return reason;
        }
        let until = tokio::time::Instant::now() + DRAIN_BOUND;
        match tokio::time::timeout_at(until, started).await {
            Ok(Ok(mut result)) => {
                let verdict = drain_all(&mut result, until, self.binary).await;
                self.record_drain(verdict);
            }
            Ok(Err(error)) if is_query_interrupted(&error) => {
                self.record_drain(DrainVerdict::Consumed);
            }
            _ => self.record_drain(DrainVerdict::Unproven),
        }
        reason
    }

    /// Reads every result set of a result.
    async fn read<P>(&mut self, mut result: QueryResult<'_, 'static, P>, call: bool) -> Halt
    where
        P: Protocol + Unpin,
    {
        let mut first = true;
        loop {
            // The block ends the borrow the set holds on `result` before the
            // decisions below need `result` again.
            let flow = {
                let next = tokio::select! {
                    biased;
                    () = self.cancel.cancelled() => Err(Halt::Cancelled),
                    () = sleep_until(self.deadline) => Err(Halt::TimedOut),
                    next = result.stream::<Row>() => Ok(next),
                };
                match next {
                    Err(reason) => Flow::Interrupt(reason),
                    Ok(Err(error)) => Flow::Error(error),
                    Ok(Ok(None)) => Flow::End,
                    Ok(Ok(Some(mut set))) => {
                        let columns = set.columns_ref().to_vec();
                        let affected = set.affected_rows();
                        self.take_set(&mut set, &columns, affected, first, call)
                            .await
                    }
                }
            };
            match flow {
                Flow::Shown => first = false,
                Flow::Skipped => {}
                Flow::End => break,
                Flow::Failed => return Halt::Failed,
                Flow::Error(error) => {
                    self.fail_with(&error);
                    return Halt::Failed;
                }
                Flow::Interrupt(reason) => return self.interrupt(&mut result, reason).await,
                Flow::Killed(reason, verdict) => {
                    let verdict = match verdict {
                        Some(verdict) => verdict,
                        None => {
                            let until = tokio::time::Instant::now() + DRAIN_BOUND;
                            drain_all(&mut result, until, self.binary).await
                        }
                    };
                    self.record_drain(verdict);
                    return reason;
                }
                Flow::KillFailed(reason) => return reason,
                Flow::Refuse(error) => {
                    let until = tokio::time::Instant::now() + DRAIN_BOUND;
                    let _ = drain_all(&mut result, until, self.binary).await;
                    self.close = true;
                    self.fail(error);
                    return Halt::Failed;
                }
            }
        }
        self.affected = self.affected.max(result.affected_rows());
        Halt::Exhausted
    }

    /// Handles one result set; what needs the whole result is left to the
    /// caller through the returned [`Flow`].
    async fn take_set<S>(
        &mut self,
        set: &mut S,
        columns: &[mysql_async::Column],
        affected: u64,
        first: bool,
        call: bool,
    ) -> Flow
    where
        S: futures::Stream<Item = std::result::Result<Row, MyError>> + Unpin,
    {
        if columns.is_empty() {
            // An OK packet: a write's count, or the end of a `CALL`.
            self.affected = self.affected.saturating_add(affected);
            return Flow::Skipped;
        }
        if let Some(refusal) = refuse_internal_types(columns, self.binary) {
            // Reading one row would panic the library, and so would draining
            // it: the connection is abandoned as it stands.
            self.poisoned = true;
            self.fail(refusal);
            return Flow::Failed;
        }
        if !first {
            self.extra_sets = self.extra_sets.saturating_add(1);
            if !call {
                // ADR-0050 §5: the server ran more than Oxyn sent.
                return Flow::Refuse(OxynError::driver(
                    self.driver.clone(),
                    ErrorClass::Permanent,
                    MysqlError::protocol(
                        "the server returned a second result for a single statement: it ran \
                         more than Oxyn sent; the connection was closed",
                    ),
                ));
            }
            // A procedure's further result: counted, not shown.
            return match self.skip_set(set).await {
                None => Flow::Skipped,
                Some(Halt::Failed) => Flow::Failed,
                Some(reason) => self.kill_during(set, reason).await,
            };
        }
        if let Some(sender) = self.schema_sender.take() {
            let (learnt, plan) = schema_for(columns);
            self.assembler = BatchAssembler::new(SchemaRef::clone(&learnt), &plan);
            let _ = sender.send(learnt);
        }
        match self.rows(set).await {
            Ok(()) => Flow::Shown,
            Err(Halt::Failed) => Flow::Failed,
            Err(reason) => self.kill_during(set, reason).await,
        }
    }

    /// Kills the statement while a result set is open, and reads on in that
    /// set: that is where the kill's answer arrives.
    async fn kill_during<S>(&mut self, set: &mut S, reason: Halt) -> Flow
    where
        S: futures::Stream<Item = std::result::Result<Row, MyError>> + Unpin,
    {
        if !self.send_kill().await {
            return Flow::KillFailed(reason);
        }
        let until = tokio::time::Instant::now() + DRAIN_BOUND;
        Flow::Killed(reason, drain_set(set, until).await)
    }

    /// Reads the rows of the first result set into batches.
    ///
    /// `Err(reason)` when the stream must stop with the server still sending;
    /// `Err(Halt::Failed)` once an error is emitted.
    async fn rows<S>(&mut self, set: &mut S) -> std::result::Result<(), Halt>
    where
        S: futures::Stream<Item = std::result::Result<Row, MyError>> + Unpin,
    {
        loop {
            let received = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Err(Halt::Cancelled),
                () = sleep_until(self.deadline) => return Err(Halt::TimedOut),
                received = set.next() => received,
            };
            let row = match received {
                None => return Ok(()),
                Some(Err(error)) => {
                    self.fail_with(&error);
                    return Err(Halt::Failed);
                }
                Some(Ok(row)) => row,
            };
            if self.produced >= self.limits.max_rows.unwrap_or(usize::MAX) {
                // One row beyond the bound: the result is really longer.
                self.truncated = true;
                // A write is drained to its end, under the same token and
                // deadline, neither decoded nor kept: killing it here would
                // roll back an autocommit statement whose first rows the user
                // is shown, and report nothing.
                if self.intent.is_mutating() {
                    continue;
                }
                return if read_ahead(set).await {
                    Ok(())
                } else {
                    Err(Halt::RowLimit)
                };
            }
            if let Err(error) = self.assembler.push(row.unwrap_raw()) {
                self.fail(OxynError::driver(
                    self.driver.clone(),
                    ErrorClass::Permanent,
                    error,
                ));
                return Err(Halt::Failed);
            }
            self.produced = self.produced.saturating_add(1);
            let full = self.assembler.bytes() >= BATCH_BYTE_BUDGET
                || self.assembler.rows() >= BATCH_ROW_CEILING
                || self.limits.max_rows.is_some_and(|max| self.produced >= max);
            if full {
                // The token is watched during back-pressure too: a grid that
                // no longer reads must not keep the server waiting.
                let emission = tokio::select! {
                    biased;
                    () = self.cancel.cancelled() => Emission::Cancelled,
                    emission = emit(&mut self.assembler, &self.events) => emission,
                };
                match emission {
                    Emission::Proceed => {}
                    Emission::Cancelled => return Err(Halt::Cancelled),
                    Emission::Abandoned => return Err(Halt::Abandoned),
                    Emission::Failed(error) => {
                        self.fail(OxynError::driver(
                            self.driver.clone(),
                            ErrorClass::Permanent,
                            error,
                        ));
                        return Err(Halt::Failed);
                    }
                }
            }
        }
    }

    /// Reads and discards a procedure's further result set.
    async fn skip_set<S>(&mut self, set: &mut S) -> Option<Halt>
    where
        S: futures::Stream<Item = std::result::Result<Row, MyError>> + Unpin,
    {
        loop {
            let received = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Some(Halt::Cancelled),
                () = sleep_until(self.deadline) => return Some(Halt::TimedOut),
                received = set.next() => received,
            };
            match received {
                None => return None,
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    self.fail_with(&error);
                    return Some(Halt::Failed);
                }
            }
        }
    }

    /// Stops the statement between two result sets.
    async fn interrupt<P>(&mut self, result: &mut QueryResult<'_, 'static, P>, reason: Halt) -> Halt
    where
        P: Protocol + Unpin,
    {
        if reason == Halt::Failed {
            return reason;
        }
        if self.send_kill().await {
            let until = tokio::time::Instant::now() + DRAIN_BOUND;
            let verdict = drain_all(result, until, self.binary).await;
            self.record_drain(verdict);
        }
        reason
    }

    /// Sends `KILL QUERY`; `false` when it could not go out.
    async fn send_kill(&mut self) -> bool {
        match self.killer.kill_query(self.id).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    target: "oxyn::driver::mysql",
                    error = %error,
                    "server-side cancellation did not succeed"
                );
                self.kill = Some(Err(Arc::new(error)));
                false
            }
        }
    }

    fn record_drain(&mut self, verdict: DrainVerdict) {
        self.kill = Some(Ok(verdict));
    }

    fn fail_with(&mut self, error: &MyError) {
        let mapped = map_stream_error(
            &self.driver,
            self.intent,
            self.limits.read_only,
            self.bound,
            error,
        );
        if crate::error::breaks_connection(error) {
            self.close = true;
        }
        self.fail(mapped);
    }

    /// Leaves the connection as the next operation may find it, and says
    /// whether an open transaction was lost with it.
    async fn settle_connection(
        &mut self,
        lease: &mut Lease,
        statement: Option<Statement>,
        restore_read_write: bool,
        before: TransactionState,
        sent: bool,
    ) -> bool {
        let unproven = !matches!(self.kill, None | Some(Ok(DrainVerdict::Consumed)));
        if self.poisoned {
            lease.abandon();
            return before == TransactionState::Open;
        }
        if unproven || self.close {
            let lost =
                before == TransactionState::Open || lease.transaction() == TransactionState::Open;
            if matches!(self.kill, Some(Ok(DrainVerdict::Unproven))) {
                // The kill may have been lost: end the connection on the
                // server, which ends its statement too.
                let _ = self.killer.kill_connection(self.id).await;
            }
            lease.discard();
            return lost;
        }
        if sent && (self.kill.is_some() || self.failure.is_some()) {
            // An error packet — the consumed kill's, or the statement's own —
            // erased the last status, and an error that keeps the connection
            // leaves an open transaction open: the server says which, as
            // `Link::after_error` asks it after any other operation.
            lease.refresh().await;
            if lease.conn().is_none() {
                // The ping found the connection gone, and the transaction
                // with it: `discard` made the state unknown.
                return before == TransactionState::Open;
            }
        } else if sent {
            // Unsent, the last packet is still the prepare's, which says
            // nothing of the transaction (see `Shared::prepare`): the state
            // observed before stands.
            lease.record_status();
        }
        let mut healthy = true;
        if let Some(conn) = lease.conn() {
            if let Some(statement) = statement {
                // ADR-0050 §2: with the cache disabled, closing is the
                // driver's job; a statement left open counts against
                // `max_prepared_stmt_count`.
                healthy &= conn.close(statement).await.is_ok();
            }
            if restore_read_write && healthy {
                healthy &= conn.query_drop(SQL_READ_WRITE).await.is_ok();
            }
        }
        if !healthy {
            // A connection that cannot be put back in its default state is not
            // trusted with the next statement.
            let lost = lease.transaction() == TransactionState::Open;
            lease.discard();
            return lost;
        }
        false
    }

    /// Emits the last event — exactly one, whatever went wrong before.
    async fn conclude(mut self, stop: Halt, lost: bool) {
        // `spawn` may still be waiting for the schema of a statement that
        // failed before giving one: it must learn there is none before the
        // terminal event is queued, or it would never read the channel.
        drop(self.schema_sender.take());
        let failure = self.failure.take();
        // Nothing was sent, or the kill was answered on the statement itself.
        let stop_proven = matches!(self.kill, None | Some(Ok(DrainVerdict::Consumed)));
        if let Some(error) = terminal_error(
            &self.driver,
            self.intent,
            self.limits.timeout,
            (stop, stop_proven),
            lost,
            failure,
        ) {
            let _ = self.events.send(CursorEvent::Failed(Box::new(error))).await;
            return;
        }
        if !matches!(stop, Halt::Exhausted | Halt::RowLimit) {
            return;
        }
        if !self.assembler.is_empty() {
            match emit(&mut self.assembler, &self.events).await {
                Emission::Proceed | Emission::Cancelled => {}
                Emission::Abandoned => return,
                Emission::Failed(error) => {
                    let error =
                        OxynError::driver(self.driver.clone(), ErrorClass::Permanent, error);
                    let _ = self.events.send(CursorEvent::Failed(Box::new(error))).await;
                    return;
                }
            }
        }
        if self.extra_sets > 0 {
            tracing::info!(
                target: "oxyn::driver::mysql",
                extra_result_sets = self.extra_sets,
                "a CALL returned more than one result set; only the first is shown"
            );
        }
        let _ = self
            .events
            .send(CursorEvent::Finished {
                affected_rows: self.affected,
                truncated: stop == Halt::RowLimit || self.truncated || self.extra_sets > 0,
            })
            .await;
    }
}

/// The one error an execution ends on, or `None` when it ends on its rows.
///
/// A lost transaction outranks the failure that cost the connection: it is
/// what the user must act upon, and the failure is kept in its message.
///
/// `stop` comes with whether it is proven to have reached the statement: a
/// write stopped by a kill the server did not answer on that very statement —
/// or that never went out — may have run to its end and committed.
fn terminal_error(
    driver: &DriverId,
    intent: StatementIntent,
    timeout: Option<std::time::Duration>,
    (stop, stop_proven): (Halt, bool),
    lost: bool,
    failure: Option<OxynError>,
) -> Option<OxynError> {
    if lost {
        let cause = failure
            .map(|error| format!(" The statement had failed first: {error}"))
            .unwrap_or_default();
        return Some(if intent == StatementIntent::Read {
            OxynError::driver(
                driver.clone(),
                ErrorClass::Permanent,
                MysqlError::protocol(format!(
                    "the connection had to be closed, and the server rolled back the \
                     transaction that was open on it: nothing of it was committed.{cause}"
                )),
            )
        } else {
            // The interrupted statement may itself have ended the transaction
            // — a `COMMIT`, a DDL's implicit commit — before the connection
            // closed: claiming a rollback would invite a replay.
            OxynError::OutcomeUnknown(format!(
                "the connection had to be closed while a transaction was open on it: the \
                 server either rolled the transaction back or the interrupted statement \
                 committed it; check the data before running it again.{cause}"
            ))
        });
    }
    if failure.is_some() {
        return failure;
    }
    match stop {
        // Not `Cancelled`: that would read as "nothing happened" and invite
        // running the write again (I-13).
        Halt::Cancelled | Halt::Abandoned if intent.is_mutating() && !stop_proven => {
            Some(OxynError::OutcomeUnknown(
                "the write was stopped, but the server did not confirm it interrupted it: \
                 it may have run to its end; check the data before running it again"
                    .to_owned(),
            ))
        }
        Halt::Cancelled | Halt::Abandoned => Some(OxynError::Cancelled),
        // Ambiguous by construction: a write may have been applied.
        Halt::TimedOut => Some(OxynError::Timeout {
            after: timeout.unwrap_or_default(),
        }),
        Halt::Failed | Halt::Exhausted | Halt::RowLimit => None,
    }
}

/// Refuses a binary result set carrying a type the library panics on.
fn refuse_internal_types(columns: &[mysql_async::Column], binary: bool) -> Option<OxynError> {
    if !binary {
        return None;
    }
    let column = columns
        .iter()
        .find(|column| panics_in_binary_protocol(column.column_type()))?;
    Some(OxynError::driver(
        DriverId::mysql(),
        ErrorClass::Permanent,
        MysqlError::protocol(format!(
            "the server announced column `{}` with {}, a type internal to the server that \
             the client library cannot decode; the result was refused and the connection \
             abandoned",
            column.name_str(),
            type_name(column.column_type(), column.flags())
        )),
    ))
}

/// Reads past the row bound without decoding: `true` when the set ends within
/// [`READ_AHEAD_ROWS`] and [`READ_AHEAD_TIME`], so no kill is needed.
///
/// An error in those rows is not the user's result any more: the set is then
/// treated as still running, and the kill settles the connection.
async fn read_ahead<S>(set: &mut S) -> bool
where
    S: futures::Stream<Item = std::result::Result<Row, MyError>> + Unpin,
{
    let until = tokio::time::Instant::now() + READ_AHEAD_TIME;
    for _ in 0..READ_AHEAD_ROWS {
        match tokio::time::timeout_at(until, set.next()).await {
            Ok(None) => return true,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Err(_) => return false,
        }
    }
    false
}

/// Reads the rest of the current result set after a kill. `Some` once the
/// verdict is known; `None` when the set ended without an answer, so the next
/// sets must be read too.
async fn drain_set<S>(set: &mut S, until: tokio::time::Instant) -> Option<DrainVerdict>
where
    S: futures::Stream<Item = std::result::Result<Row, MyError>> + Unpin,
{
    loop {
        match tokio::time::timeout_at(until, set.next()).await {
            Err(_) => return Some(DrainVerdict::Unproven),
            Ok(None) => return None,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(error))) if is_query_interrupted(&error) => {
                return Some(DrainVerdict::Consumed);
            }
            Ok(Some(Err(_))) => return Some(DrainVerdict::Unproven),
        }
    }
}

/// Reads every remaining result set after a kill, until the server confirms
/// it, ends, or the bound elapses.
async fn drain_all<P>(
    result: &mut QueryResult<'_, 'static, P>,
    until: tokio::time::Instant,
    binary: bool,
) -> DrainVerdict
where
    P: Protocol + Unpin,
{
    loop {
        let next = match tokio::time::timeout_at(until, result.stream::<Row>()).await {
            Err(_) => return DrainVerdict::Unproven,
            Ok(Err(error)) if is_query_interrupted(&error) => return DrainVerdict::Consumed,
            Ok(Err(_) | Ok(None)) => return DrainVerdict::Unproven,
            Ok(Ok(Some(set))) => set,
        };
        if refuse_internal_types(next.columns_ref(), binary).is_some() {
            return DrainVerdict::Unproven;
        }
        let mut set = next;
        if let Some(verdict) = drain_set(&mut set, until).await {
            return verdict;
        }
    }
}

/// What handling one result set leaves for the loop over the result.
enum Flow {
    /// The first result set was read into batches.
    Shown,
    /// An OK packet, or a procedure's further result set, counted.
    Skipped,
    /// No result set left.
    End,
    /// An error was already emitted.
    Failed,
    /// The server's error, to classify and emit.
    Error(MyError),
    /// Interrupted between two result sets: the kill is still to send.
    Interrupt(Halt),
    /// The kill went out inside a set; `None` when that set ended without the
    /// server's answer, so the following sets must be read too.
    Killed(Halt, Option<DrainVerdict>),
    /// The kill could not go out.
    KillFailed(Halt),
    /// The result is refused: drain it, emit the error, close the connection.
    Refuse(OxynError),
}

/// What emitting a batch gave.
enum Emission {
    Proceed,
    Cancelled,
    Abandoned,
    /// The batch could not be closed; the caller reports it.
    Failed(arrow::error::ArrowError),
}

/// Closes the current batch and sends it, honoring the channel's
/// back-pressure.
async fn emit(assembler: &mut BatchAssembler, events: &mpsc::Sender<CursorEvent>) -> Emission {
    let batch = match assembler.finish() {
        Ok(batch) => batch,
        Err(error) => return Emission::Failed(error),
    };
    if events.send(CursorEvent::Batch(batch)).await.is_err() {
        return Emission::Abandoned;
    }
    Emission::Proceed
}

/// Waits for the deadline, or forever when there is none. The instant is
/// absolute: recreating this future does not push the delay back.
async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(instant) => tokio::time::sleep_until(instant).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mysql_async::Column;
    use mysql_async::consts::ColumnType;
    use std::time::Duration;

    #[test]
    fn a_binary_result_with_an_internal_type_is_refused_before_any_row() {
        // I-09: `mysql_common` panics decoding these codes in the binary
        // protocol; the text protocol reads every value as bytes.
        let columns = [
            Column::new(ColumnType::MYSQL_TYPE_LONG),
            Column::new(ColumnType::MYSQL_TYPE_DATETIME2),
        ];
        let refusal = refuse_internal_types(&columns, true).expect("refusal expected");
        assert!(refusal.to_string().contains("type code 18"), "{refusal}");
        assert!(refuse_internal_types(&columns, false).is_none());
        assert!(
            refuse_internal_types(&[Column::new(ColumnType::MYSQL_TYPE_VECTOR)], true).is_none()
        );
    }

    /// A pump as `run` builds it, before the result gave its schema.
    fn pump_awaiting_its_schema(
        intent: StatementIntent,
    ) -> (
        Pump,
        mpsc::Receiver<CursorEvent>,
        oneshot::Receiver<SchemaRef>,
    ) {
        let spec = crate::options::ConnectSpec::from_config(
            &crate::driver::mysql_metadata(),
            &oxyn_core::ConnectionConfig::new("trial", DriverId::mysql())
                .with_param("host", "127.0.0.1")
                .with_param("user", "app")
                .with_environment(oxyn_core::Environment::Local),
            &oxyn_driver::Credentials::new(),
        )
        .expect("a complete configuration");
        let (events, reception) = mpsc::channel(1);
        let (schema_sender, schema_reception) = oneshot::channel();
        let pump = Pump {
            driver: DriverId::mysql(),
            cancel: CancelToken::new(),
            deadline: None,
            events,
            schema_sender: Some(schema_sender),
            assembler: BatchAssembler::new(Arc::new(arrow::datatypes::Schema::empty()), &[]),
            limits: ExecLimits::default(),
            intent,
            bound: Bound::Internal,
            killer: Arc::new(Killer::new(spec, DriverId::mysql())),
            id: 0,
            produced: 0,
            affected: 0,
            extra_sets: 0,
            truncated: false,
            kill: None,
            close: true,
            poisoned: false,
            binary: false,
            failure: None,
        };
        (pump, reception, schema_reception)
    }

    #[tokio::test]
    async fn a_failure_before_the_schema_that_loses_the_transaction_ends_on_one_error() {
        // The path that deadlocked: the statement fails before its schema,
        // the connection goes with the open transaction, and `spawn` — which
        // reads the channel only once the schema is settled — waits on it.
        let (mut pump, mut events, schema) = pump_awaiting_its_schema(StatementIntent::Write);
        pump.fail(OxynError::Connection("lost mid-statement".to_owned()));
        let task = tokio::spawn(pump.conclude(Halt::Failed, true));
        let reader = async {
            assert!(
                schema.await.is_err(),
                "no schema: the statement never gave one"
            );
            let mut received = Vec::new();
            while let Some(event) = events.recv().await {
                received.push(event);
            }
            received
        };
        let received = tokio::time::timeout(Duration::from_secs(5), reader)
            .await
            .expect("the execution concludes instead of waiting on itself");
        task.await.expect("the task ends");
        let [CursorEvent::Failed(error)] = received.as_slice() else {
            panic!("exactly one terminal event expected: {received:?}");
        };
        assert!(matches!(**error, OxynError::OutcomeUnknown(_)), "{error:?}");
        assert!(error.to_string().contains("lost mid-statement"), "{error}");
    }

    #[test]
    fn the_first_failure_is_the_one_reported_when_the_transaction_survives() {
        let (mut pump, _events, _schema) = pump_awaiting_its_schema(StatementIntent::Read);
        pump.fail(OxynError::Query("the cause".to_owned()));
        pump.fail(OxynError::Query("a consequence".to_owned()));
        let error = terminal_error(
            &pump.driver,
            pump.intent,
            None,
            (Halt::Failed, true),
            false,
            pump.failure.take(),
        )
        .expect("an error");
        assert!(error.to_string().contains("the cause"), "{error}");
        let lost = terminal_error(
            &DriverId::mysql(),
            StatementIntent::Read,
            None,
            (Halt::Cancelled, true),
            true,
            None,
        )
        .expect("an error");
        assert_eq!(
            lost.class(),
            ErrorClass::Permanent,
            "a read commits nothing"
        );
        assert!(
            terminal_error(
                &DriverId::mysql(),
                StatementIntent::Read,
                None,
                (Halt::Exhausted, true),
                false,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn a_stopped_write_is_cancelled_only_when_the_server_confirmed_the_kill() {
        let stopped = |intent, proven| {
            terminal_error(
                &DriverId::mysql(),
                intent,
                None,
                (Halt::Cancelled, proven),
                false,
                None,
            )
            .expect("a stop is an error")
        };
        // The kill missed, or never went out: the write may have committed.
        let unknown = stopped(StatementIntent::Write, false);
        assert!(
            matches!(unknown, OxynError::OutcomeUnknown(_)),
            "{unknown:?}"
        );
        assert!(!unknown.is_retryable(), "never replayed (I-13)");
        // `ER_QUERY_INTERRUPTED` on the statement itself: nothing applied.
        assert!(matches!(
            stopped(StatementIntent::Write, true),
            OxynError::Cancelled
        ));
        // A read has nothing to commit.
        assert!(matches!(
            stopped(StatementIntent::Read, false),
            OxynError::Cancelled
        ));
    }
}
