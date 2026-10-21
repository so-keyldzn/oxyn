//! Executing a request, on the worker thread side.
//!
//! Everything here runs **on the connection's thread**: it is the only place
//! where a `Statement` and its `Rows` exist, and they never leave it. What
//! crosses the channel are Arrow `RecordBatch`es.
//!
//! # A batch of several statements
//!
//! Splitting a script belongs to `oxyn-query`
//! ([ARCHITECTURE §3](../../../docs/ARCHITECTURE.md)); this driver nevertheless
//! accepts several statements in one submission, and the rule fits in one
//! sentence:
//!
//! > **Every statement is executed, in order; the cursor carries the result of
//! > the last one.**
//!
//! A result set produced by a statement that is not the last is consumed and
//! thrown away — the statement runs all the same, because its effects count.
//! `SELECT 1; INSERT INTO t VALUES (2);` therefore does insert, and returns an
//! empty result.
//!
//! Statements are prepared **as they come**, not all in advance: preparing
//! `SELECT * FROM t` before running the `CREATE TABLE t` that precedes it would
//! fail.
//!
//! # When an earlier statement already applied
//!
//! Each statement is its own atomic unit: one that completed has changed the
//! data, whatever the next one does. `applied` records it as the batch goes,
//! **in place**, so that it survives the error that ends the batch — a later
//! statement that cannot even be prepared (a lock, a schema change, a table
//! that never existed) is as much "after an applied statement" as one that
//! fails while running. From then on every failure that leaves the worker thread
//! goes through `error::after_applied`: never a cancellation, never retryable.
//!
//! That wrapping happens at **one point per exit** — the error returns of `run`
//! and of `stream_rows` — and nowhere below: the functions that read or step
//! return the engine's own verdict, so nothing is wrapped twice.
//!
//! # What is refused, and why
//!
//! * **Bound parameters with a batch of several statements.** Nothing says which
//!   one they relate to. Guessing would mean binding values to a statement the
//!   user did not target.
//! * **A write under `ExecLimits::read_only`.** The question is not asked of the
//!   text but of the engine: `sqlite3_stmt_readonly` knows what the compiled
//!   statement can do, where a lexical analysis would be fooled by a view, a
//!   trigger or a virtual table.

use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::datatypes::{Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use oxyn_core::{ErrorClass, ExecRequest, OxynError, Result};
use rusqlite::fallible_iterator::FallibleIterator as _;
use rusqlite::{Batch, Connection, Row, Rows, Statement};
use tokio::sync::{mpsc, oneshot};

use crate::convert::{ColumnBuilder, ColumnPlan, Observed, ProbeValue, schema_of, value_bytes};
use crate::error::{self, Bound, Effect, SqliteError};
use crate::interrupt::Interrupter;
use crate::options::BatchLimits;
use crate::params;

/// A batch request, from the cursor.
pub(crate) type Pull = oneshot::Sender<Result<Pulled>>;

/// What a batch request brings back.
pub(crate) enum Pulled {
    /// A batch of rows.
    Batch(RecordBatch),
    /// The last batch of rows: nothing more will be read from the statement —
    /// it ran to its end, or `ExecLimits::max_rows` cut it — and the worker
    /// thread has already let go of it.
    ///
    /// Distinct from [`Batch`](Self::Batch) followed by [`Done`](Self::Done)
    /// because a write is over when its last row is read, not when the cursor
    /// asks once more: a Stop landing between the two must find the cursor
    /// already finished, not report a cancellation of what was applied
    /// (issue #181, I-13).
    Last {
        /// The rows.
        batch: RecordBatch,
        /// Rows are missing, because `ExecLimits::max_rows` cut.
        truncated: bool,
    },
    /// The stream is finished.
    Done {
        /// Rows are missing, because `ExecLimits::max_rows` cut.
        truncated: bool,
    },
}

/// What the execution returns as soon as the schema is known.
///
/// The first batch is **already there**: resolving it required reading rows, and
/// throwing them away to read them again would be absurd.
pub(crate) struct StreamStart {
    /// The schema of the batches, stable for the whole duration of the stream.
    pub schema: SchemaRef,
    /// The first batch, or the end of the stream if there is no row.
    pub first: Pulled,
    /// Rows affected by the statements that produced no columns.
    pub affected: u64,
    /// What the streamed statement could do: a Stop ends a read as a
    /// cancellation, a write only on what the engine reports.
    pub effect: Effect,
    /// An earlier statement of the batch ran to its end and could write.
    ///
    /// Kept apart from `effect`, which speaks of the streamed statement alone:
    /// the row-limit probe of a read must not change because of what came
    /// before it. The cursor reads this to refuse a "cancelled" for a batch that
    /// had already changed data.
    pub applied: bool,
}

/// An execution handed to the worker thread.
pub(crate) struct StreamJob {
    /// The request, as the caller phrased it.
    pub request: ExecRequest,
    /// The bounds of an Arrow batch.
    pub limits: BatchLimits,
    /// Where to reply once the schema is known.
    pub start: oneshot::Sender<Result<StreamStart>>,
    /// Through which the cursor requests what follows.
    pub pulls: mpsc::UnboundedReceiver<Pull>,
}

/// Executes a request and streams its batches until exhaustion or destruction of
/// the cursor.
///
/// `interrupter` is consulted just before each statement starts: an interruption
/// set during a preparation would otherwise be cleared by the engine at the first
/// step of the next statement ([`crate::interrupt`]).
pub(crate) fn run(connection: &Connection, job: StreamJob, interrupter: &Interrupter) {
    let StreamJob {
        request,
        limits,
        start,
        mut pulls,
    } = job;

    // Taken once for the whole execution: from `params::bind` on, the engine
    // may quote one of these values in its message, and this must be known
    // when classifying the error rather than guessed from its text (I-03).
    let bound = Bound::of(&request.params);
    let mut affected = 0_u64;
    let mut applied = false;
    let mut batch = Batch::new(connection, &request.text);
    let last = match select_last(
        connection,
        &mut batch,
        &request,
        &mut affected,
        &mut applied,
        interrupter,
    ) {
        Ok(statement) => statement,
        Err(err) => {
            let _ = start.send(Err(error::after_applied_if(applied, err)));
            return;
        }
    };
    // No `drop(batch)` here: `rusqlite::Batch` does not implement `Drop`, so the
    // call freed nothing — the borrow of the connection ends anyway at its last
    // use, just above.
    let Some(mut statement) = last else {
        // An empty text, or only comments.
        let _ = start.send(Ok(nothing(affected, applied)));
        return;
    };

    // Everything before the last statement has run: `applied` no longer moves
    // until the last statement itself completes.
    let fail = |err: OxynError| error::after_applied_if(applied, err);
    if let Err(err) = guard_read_only(&statement, &request) {
        let _ = start.send(Err(fail(err)));
        return;
    }
    if let Err(err) = params::bind(&mut statement, &request.params) {
        let _ = start.send(Err(fail(error::driver(err, ErrorClass::Permanent))));
        return;
    }

    let effect = effect_of(&statement);
    if let Err(err) = interrupter.checkpoint(effect, bound) {
        let _ = start.send(Err(fail(err)));
        return;
    }
    if statement.column_count() == 0 {
        // Write or DDL: no columns, hence no stream. What the user expects is
        // the count of affected rows.
        let before = connection.total_changes();
        if let Err(err) = statement.raw_execute() {
            let _ = start.send(Err(fail(error::engine_bound(err, effect, bound))));
            return;
        }
        let changed = changes_since(connection, before);
        affected = affected.saturating_add(changed);
        let _ = start.send(Ok(nothing(affected, applied || wrote(effect, changed))));
        return;
    }

    let context = FailureContext {
        effect,
        bound,
        applied,
    };
    stream_rows(statement, &request, limits, context, start, &mut pulls);
}

/// The result of an execution that produces no column.
fn nothing(affected: u64, applied: bool) -> StreamStart {
    StreamStart {
        schema: Arc::new(Schema::empty()),
        first: Pulled::Done { truncated: false },
        affected,
        // Every statement has already run: there is no stream left to stop.
        effect: Effect::ReadOnly,
        applied,
    }
}

/// Did a statement that ran to its end change the database?
///
/// Two tests, because neither is enough alone. The engine's read-only flag is a
/// promise about what the statement *may* do: it is false for `CREATE TABLE`,
/// which `total_changes` never counts, and for an `UPDATE` that matched no row,
/// which over-reports — the safe side for a verdict that forbids a replay.
/// The counter is what actually happened, triggers included.
const fn wrote(effect: Effect, changed: u64) -> bool {
    matches!(effect, Effect::Mutating) || changed > 0
}

/// What a failure of the streamed statement needs to be told correctly.
///
/// Fixed before the first row and the same for the whole stream. Grouped to keep
/// [`stream_rows`] within a readable number of parameters.
#[derive(Debug, Clone, Copy)]
struct FailureContext {
    /// What the streamed statement could do.
    effect: Effect,
    /// Where its bound values come from.
    bound: Bound,
    /// An earlier statement of the batch applied.
    applied: bool,
}

/// Executes every statement but the last, and returns that one.
///
/// Returns `None` if the text contains no statement.
///
/// `applied` is raised **in place** when a statement completes and could write,
/// so the caller still reads it when this function returns the error of a later
/// statement.
fn select_last<'conn>(
    connection: &'conn Connection,
    batch: &mut Batch<'conn, '_>,
    request: &ExecRequest,
    affected: &mut u64,
    applied: &mut bool,
    interrupter: &Interrupter,
) -> Result<Option<Statement<'conn>>> {
    let mut pending = next_statement(batch)?;
    loop {
        let Some(mut statement) = pending.take() else {
            return Ok(None);
        };

        // Without bound parameters, a statement without columns runs before the
        // next one is prepared. That is what makes
        // `CREATE TABLE t; SELECT * FROM t;` work: the second is prepared only
        // once the table is created. Nothing being bound here, the engine message
        // can quote only the submitted SQL: `error::engine` is enough in this
        // function.
        if request.params.is_empty() && statement.column_count() == 0 {
            guard_read_only(&statement, request)?;
            let effect = effect_of(&statement);
            interrupter.checkpoint(effect, Bound::Internal)?;
            let before = connection.total_changes();
            statement
                .raw_execute()
                .map_err(|err| error::engine(err, effect))?;
            let changed = changes_since(connection, before);
            *affected = affected.saturating_add(changed);
            // Before preparing the next statement: that preparation is where a
            // lock or a schema change surfaces, and it is already "after".
            *applied |= wrote(effect, changed);
            pending = next_statement(batch)?;
            continue;
        }

        // We must know whether a statement remains after this one. Preparing it
        // now is harmless: everything before it has already run, and preparing
        // executes nothing.
        let next = next_statement(batch)?;
        if next.is_none() {
            return Ok(Some(statement));
        }
        if !request.params.is_empty() {
            return Err(error::driver(
                SqliteError::ParametersWithBatch,
                ErrorClass::Permanent,
            ));
        }
        guard_read_only(&statement, request)?;
        // After preparing the next one, which clears the engine's flag.
        interrupter.checkpoint(effect_of(&statement), Bound::Internal)?;
        // This result set is overwritten by the next statement's. It is run all
        // the same — its effects count — and its rows are thrown away.
        discard(&mut statement, connection, affected, applied)?;
        pending = next;
    }
}

/// The next statement of the batch, prepared.
fn next_statement<'conn>(batch: &mut Batch<'conn, '_>) -> Result<Option<Statement<'conn>>> {
    // Preparing modifies nothing: a preparation error is never ambiguous.
    batch
        .next()
        .map_err(|err| error::engine(err, Effect::ReadOnly))
}

/// Executes a statement and throws its rows away.
///
/// Called only from [`select_last`], which refused bound parameters before
/// getting here: hence [`Bound::Internal`].
fn discard(
    statement: &mut Statement<'_>,
    connection: &Connection,
    affected: &mut u64,
    applied: &mut bool,
) -> Result<()> {
    let effect = effect_of(statement);
    let before = connection.total_changes();
    let mut rows = statement.raw_query();
    while step(&mut rows, effect, Bound::Internal)?.is_some() {}
    drop(rows);
    let changed = changes_since(connection, before);
    *affected = affected.saturating_add(changed);
    *applied |= wrote(effect, changed);
    Ok(())
}

/// Streams the rows of a statement.
///
/// `bound` goes with `effect` down to the read functions: they do not have the
/// request at hand, and guessing it at their level would be one more
/// assumption. `applied` stops at this function: it wraps every error that
/// leaves it, so the read functions below return the engine's own verdict.
fn stream_rows(
    mut statement: Statement<'_>,
    request: &ExecRequest,
    limits: BatchLimits,
    context: FailureContext,
    start: oneshot::Sender<Result<StreamStart>>,
    pulls: &mut mpsc::UnboundedReceiver<Pull>,
) {
    let FailureContext {
        effect,
        bound,
        applied,
    } = context;
    // Taken before borrowing the statement for reading: `columns()` borrows,
    // `raw_query()` borrows mutably.
    let declared: Vec<(String, Option<String>)> = statement
        .columns()
        .iter()
        .map(|column| {
            (
                column.name().to_owned(),
                column.decl_type().map(str::to_owned),
            )
        })
        .collect();
    let width = declared.len();
    let mut remaining = request.limits.max_rows;
    let mut rows = statement.raw_query();

    let probed = match probe(&mut rows, width, limits, &mut remaining, effect, bound) {
        Ok(probed) => probed,
        Err(err) => {
            let _ = start.send(Err(error::after_applied_if(applied, err)));
            return;
        }
    };

    let plans: Vec<ColumnPlan> = declared
        .into_iter()
        .zip(probed.observed)
        .map(|((name, declared), observed)| ColumnPlan::resolve(name, declared, observed))
        .collect();
    let schema = schema_of(&plans);
    for (index, plan) in plans.iter().enumerate() {
        if plan.observed.is_mixed() {
            // Neither the column name nor any value: only the index and the
            // classes encountered (I-03). The signal meant for the interface is
            // in the field's metadata; this one is for diagnostics.
            tracing::debug!(
                column = index,
                storage_classes = %plan.observed.names(),
                resolved = %plan.kind,
                "column with mixed storage classes: the chosen rendering is a fallback"
            );
        }
    }
    let capacity = probed.rows.max(64);
    let mut builders: Vec<ColumnBuilder> = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| ColumnBuilder::new(plan.kind, index, capacity))
        .collect();

    let finished = probed.finished;
    let mut truncated = probed.truncated;

    let first = if probed.rows == 0 {
        Pulled::Done { truncated }
    } else {
        match assemble(&mut builders, &probed.values, &schema) {
            Ok(batch) if finished => Pulled::Last { batch, truncated },
            Ok(batch) => Pulled::Batch(batch),
            Err(err) => {
                let _ = start.send(Err(error::after_applied_if(applied, err)));
                return;
            }
        }
    };

    if start
        .send(Ok(StreamStart {
            schema: Arc::clone(&schema),
            first,
            affected: 0,
            effect,
            applied,
        }))
        .is_err()
        || finished
    {
        // Either the caller gave up before even reading the first batch, or
        // the source is exhausted. In the second case, staying in the loop
        // would **hold the worker thread** until the cursor is dropped: a
        // cursor left in a scope after being drained then blocked every
        // following execution on the same session — the driver has a single
        // thread. The cursor knows it is finished and will not pull anymore.
        return;
    }

    while let Some(reply) = pulls.blocking_recv() {
        let filled = match fill(
            &mut rows,
            &mut builders,
            limits,
            &mut remaining,
            effect,
            bound,
        ) {
            Ok(filled) => filled,
            Err(err) => {
                let _ = reply.send(Err(error::after_applied_if(applied, err)));
                return;
            }
        };
        truncated |= filled.truncated;

        let answer = if filled.rows == 0 {
            Ok(Pulled::Done { truncated })
        } else if filled.finished {
            finish_batch(&mut builders, &schema).map(|batch| Pulled::Last { batch, truncated })
        } else {
            finish_batch(&mut builders, &schema).map(Pulled::Batch)
        }
        .map_err(|err| error::after_applied_if(applied, err));
        let failed = answer.is_err();
        // A finished source joins the two other stop causes for the same reason
        // as above: once the end is announced, keeping the thread only serves
        // to block the next query.
        if reply.send(answer).is_err() || failed || filled.finished {
            return;
        }
    }
}

/// What the first-batch probe learned.
struct Probe {
    /// The values of the first batch, row by row, column by column.
    values: Vec<ProbeValue>,
    /// The storage classes seen, column by column.
    observed: Vec<Observed>,
    /// Rows read.
    rows: usize,
    /// The source is exhausted, or the row bound is reached.
    finished: bool,
    /// Rows are missing.
    truncated: bool,
}

/// Reads the first batch **as raw values**, to decide the column types.
///
/// It is the price of honesty about SQLite's dynamic typing: without this probe,
/// the type of an undeclared column could come only from the first row, and a
/// column mixing integers and text would be typed on its first sample. The copy
/// concerns only the first batch, whose size is bounded in rows **and** in
/// bytes.
fn probe(
    rows: &mut Rows<'_>,
    width: usize,
    limits: BatchLimits,
    remaining: &mut Option<usize>,
    effect: Effect,
    bound: Bound,
) -> Result<Probe> {
    let mut values = Vec::new();
    let mut observed = vec![Observed::default(); width];
    let mut count = 0_usize;
    let mut bytes = 0_usize;
    let mut finished = false;
    let mut truncated = false;

    loop {
        if *remaining == Some(0) {
            truncated = past_the_bound(rows, effect, bound)?;
            finished = true;
            break;
        }
        let Some(row) = step(rows, effect, bound)? else {
            finished = true;
            break;
        };
        for (index, seen) in observed.iter_mut().enumerate() {
            let value = row
                .get_ref(index)
                .map_err(|err| error::engine_bound(err, effect, bound))?;
            seen.observe(value);
            bytes = bytes.saturating_add(value_bytes(value));
            values.push(ProbeValue::capture(value));
        }
        count = count.saturating_add(1);
        consume_one(remaining);
        if limits.reached(count, bytes) {
            break;
        }
    }

    Ok(Probe {
        values,
        observed,
        rows: count,
        finished,
        truncated,
    })
}

/// What a steady-state batch brought back.
struct Filled {
    /// Rows added.
    rows: usize,
    /// Nothing more will come.
    finished: bool,
    /// Rows are missing.
    truncated: bool,
}

/// Fills a batch in steady state: the column types are already decided.
fn fill(
    rows: &mut Rows<'_>,
    builders: &mut [ColumnBuilder],
    limits: BatchLimits,
    remaining: &mut Option<usize>,
    effect: Effect,
    bound: Bound,
) -> Result<Filled> {
    let mut count = 0_usize;
    let mut bytes = 0_usize;
    let mut finished = false;
    let mut truncated = false;

    loop {
        if *remaining == Some(0) {
            truncated = past_the_bound(rows, effect, bound)?;
            finished = true;
            break;
        }
        let Some(row) = step(rows, effect, bound)? else {
            finished = true;
            break;
        };
        for (index, builder) in builders.iter_mut().enumerate() {
            let value = row
                .get_ref(index)
                .map_err(|err| error::engine_bound(err, effect, bound))?;
            bytes = bytes.saturating_add(value_bytes(value));
            builder
                .append(value)
                .map_err(|err| error::driver(err, ErrorClass::Permanent))?;
        }
        count = count.saturating_add(1);
        consume_one(remaining);
        if limits.reached(count, bytes) {
            break;
        }
    }

    Ok(Filled {
        rows: count,
        finished,
        truncated,
    })
}

/// Advances by one row.
fn step<'a, 'stmt>(
    rows: &'a mut Rows<'stmt>,
    effect: Effect,
    bound: Bound,
) -> Result<Option<&'a Row<'stmt>>> {
    rows.next()
        .map_err(|err| error::engine_bound(err, effect, bound))
}

/// Is there a row beyond the `ExecLimits::max_rows` bound?
///
/// One more step tells exactly N rows from more than N, and only a statement
/// the engine knows to be read-only takes it, the rule the executor applies to
/// its own end probe. A `RETURNING` makes all its changes at the first step, so
/// the rule costs a write nothing but exactness: stopped at its bound, it stays
/// truncated.
fn past_the_bound(rows: &mut Rows<'_>, effect: Effect, bound: Bound) -> Result<bool> {
    match effect {
        Effect::ReadOnly => Ok(step(rows, effect, bound)?.is_some()),
        Effect::Mutating => Ok(true),
    }
}

/// Counts a row against the `ExecLimits::max_rows` bound, when there is one.
fn consume_one(remaining: &mut Option<usize>) {
    if let Some(left) = remaining {
        *left = left.saturating_sub(1);
    }
}

/// The number of rows modified since a reading.
///
/// `total_changes` is monotonic over the connection's lifetime, unlike
/// `changes()`, which speaks only of the last statement — and keeps its previous
/// value after a `SELECT` or a `CREATE TABLE`. The count includes the rows
/// touched by triggers, which is what the user wants to know.
fn changes_since(connection: &Connection, before: u64) -> u64 {
    connection.total_changes().saturating_sub(before)
}

/// Pours the values set aside by the probe into the builders.
fn assemble(
    builders: &mut [ColumnBuilder],
    values: &[ProbeValue],
    schema: &SchemaRef,
) -> Result<RecordBatch> {
    let width = builders.len();
    if width > 0 {
        for line in values.chunks(width) {
            for (builder, value) in builders.iter_mut().zip(line) {
                builder
                    .append(value.borrow())
                    .map_err(|err| error::driver(err, ErrorClass::Permanent))?;
            }
        }
    }
    finish_batch(builders, schema)
}

/// Finishes the builders and assembles the `RecordBatch`.
fn finish_batch(builders: &mut [ColumnBuilder], schema: &SchemaRef) -> Result<RecordBatch> {
    let columns: Vec<ArrayRef> = builders.iter_mut().map(ColumnBuilder::finish).collect();
    RecordBatch::try_new(Arc::clone(schema), columns)
        .map_err(|err| error::driver(SqliteError::Arrow(err), ErrorClass::Permanent))
}

/// What the compiled statement can do to the database.
fn effect_of(statement: &Statement<'_>) -> Effect {
    if statement.readonly() {
        Effect::ReadOnly
    } else {
        Effect::Mutating
    }
}

/// Refuses a write when the request declares itself read-only.
///
/// The question is asked of the **engine** (`sqlite3_stmt_readonly`), not of the
/// text: a lexical analysis would be fooled by a view, a trigger or a virtual
/// table that writes.
fn guard_read_only(statement: &Statement<'_>, request: &ExecRequest) -> Result<()> {
    if request.limits.read_only && !statement.readonly() {
        return Err(OxynError::PolicyDenied {
            reason: "the execution is declared read-only and the statement may write".to_owned(),
        });
    }
    Ok(())
}
