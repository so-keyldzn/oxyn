//! Executing a request, on the carrier thread.
//!
//! Everything here runs on the connection's thread: statements and their
//! streamed results never leave it; Arrow 59 batches cross the channel.
//!
//! # A batch of several statements
//!
//! > **Every statement runs, in order; the cursor carries the result of the
//! > last one.**
//!
//! The text is split by `oxyn-query` with the DuckDB profile, and each
//! statement is prepared only once the previous one ran — `CREATE TABLE t;
//! SELECT * FROM t` works. Splitting here rather than letting DuckDB run the
//! batch is what lets the driver check an interruption before each statement
//! starts ([`crate::interrupt`]) and know when an earlier statement already
//! applied. When the splitter is in doubt it merges, and DuckDB's own parser
//! runs the merged text.
//!
//! # What is streamed and what is counted
//!
//! A data write without `RETURNING` (`INSERT`, `UPDATE`, `DELETE`, `COPY`) is
//! executed to its end and reported as a count of affected rows. Everything
//! else is streamed: a `SELECT` and a `RETURNING` alike, chunk by chunk,
//! bounded in bytes.
//!
//! # What is refused
//!
//! * **Bound values with several statements**: nothing says which one they
//!   belong to.
//! * **A typed `USE`**: the session's context is the driver's to declare
//!   ([`oxyn_driver::Session::set_context`]); a `USE` typed in the text would
//!   make [`oxyn_driver::Session::context`] lie.
//! * **A write under `ExecLimits::read_only`.** Two barriers. The statements
//!   must each classify as a read; then the engine itself holds the bound: the
//!   execution runs inside `BEGIN TRANSACTION READ ONLY`, rolled back at the
//!   end, which DuckDB enforces whatever a function in a `SELECT` tries. Inside
//!   a transaction the user opened, that wrapper cannot be used without ending
//!   their transaction, and the execution is refused instead
//!   ([DRIVER-CONTRACT §5](../../../docs/DRIVER-CONTRACT.md)).

use std::sync::Arc;

use arrow::datatypes::{Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use duckdb::{Connection, Statement};
use oxyn_core::{
    ErrorClass, ExecRequest, OxynError, Result, SqlDialect, StatementIntent, TransactionState,
};
use oxyn_query::{StatementInfo, classify, words};
use tokio::sync::{mpsc, oneshot};

use crate::convert;
use crate::error::{self, DuckDbError, Effect};
use crate::interrupt::Interrupter;
use crate::logical::{self, DuckType};
use crate::options::BatchLimits;
use crate::params;
use crate::transaction;

/// What a batch request brings back.
pub(crate) enum Pulled {
    /// A batch of rows.
    Batch(RecordBatch),
    /// The stream ended.
    Done {
        /// Rows are missing because `ExecLimits::max_rows` cut.
        truncated: bool,
    },
}

/// What the execution returns as soon as the schema is known.
pub(crate) struct StreamStart {
    /// The schema of every batch.
    pub schema: SchemaRef,
    /// Rows affected by the statements that produced no stream.
    pub affected: u64,
    /// What the streamed statement could do: a Stop ends a read as a
    /// cancellation, anything else as an unknown outcome.
    pub effect: Effect,
    /// An earlier statement of the batch ran and could write.
    pub applied: bool,
    /// Batches follow on the channel; `false` when everything already ran.
    pub streaming: bool,
}

/// An execution handed to the carrier thread.
pub(crate) struct StreamJob {
    pub request: ExecRequest,
    pub limits: BatchLimits,
    pub start: oneshot::Sender<Result<StreamStart>>,
    /// Bounded: the thread computes at most one batch ahead of the cursor.
    pub batches: mpsc::Sender<Result<Pulled>>,
}

/// The dialect the driver splits and classifies with.
const DIALECT: SqlDialect = SqlDialect::DuckDb;

/// Runs a request and streams the result of its last statement until the end
/// or until the cursor is dropped.
pub(crate) fn run(connection: &Connection, job: StreamJob, interrupter: &Interrupter) {
    let StreamJob {
        request,
        limits,
        start,
        batches,
    } = job;
    let mut applied = false;
    let mut affected = 0_u64;
    let begun = prepare_last(
        connection,
        &request,
        interrupter,
        &mut applied,
        &mut affected,
    );
    let (statement, effect, _wrapper) = match begun {
        Ok(Some(last)) => last,
        Ok(None) => {
            let _ = start.send(Ok(nothing(affected, applied)));
            return;
        }
        Err(err) => {
            let _ = start.send(Err(err));
            return;
        }
    };
    let mut statement = statement;
    let fail = |err: OxynError| error::after_applied_if(applied, err);

    let values = match params::values(&request.params) {
        Ok(values) => values,
        Err(message) => {
            let _ = start.send(Err(fail(error::driver(
                DuckDbError::Decode(message),
                ErrorClass::Permanent,
            ))));
            return;
        }
    };
    if let Err(err) = expect_parameters(&statement, values.len()) {
        let _ = start.send(Err(fail(err)));
        return;
    }
    if interrupter.tripped() {
        let _ = start.send(Err(stopped(effect, applied)));
        return;
    }

    let executed = statement.stream_arrow(duckdb::params_from_iter(values.iter()));
    if let Err(err) = executed {
        let _ = start.send(Err(engine_failure(&err, effect, applied, interrupter)));
        return;
    }
    let source = statement.schema();
    let types: Vec<DuckType> = (0..source.fields().len())
        .map(|index| logical::describe(&statement.column_logical_type(index)))
        .collect();
    let schema = match convert::target_schema(&source, &types) {
        Ok(schema) => schema,
        Err(message) => {
            let _ = start.send(Err(fail(decode(message))));
            return;
        }
    };
    if schema.fields().is_empty() {
        // DDL, a setting: nothing to stream, but the result is drained so
        // that a late error is not lost.
        let outcome = drain(&statement);
        let _ = start.send(match outcome {
            Ok(()) => Ok(nothing(affected, applied || effect == Effect::Mutating)),
            Err(err) => Err(engine_failure(&err, effect, applied, interrupter)),
        });
        return;
    }
    let opened = StreamStart {
        schema: Arc::clone(&schema),
        affected,
        effect,
        applied,
        streaming: true,
    };
    if start.send(Ok(opened)).is_err() {
        return;
    }
    let context = Failure {
        effect,
        applied,
        interrupter,
    };
    stream_rows(
        &statement, &schema, &types, &request, limits, &batches, &context,
    );
}

/// What a failure of the streamed statement needs to be told correctly.
struct Failure<'a> {
    effect: Effect,
    applied: bool,
    interrupter: &'a Interrupter,
}

/// Fetches, converts, bounds and sends batches until the end, the row limit,
/// an error, or the cursor going away.
fn stream_rows(
    statement: &Statement<'_>,
    schema: &SchemaRef,
    types: &[DuckType],
    request: &ExecRequest,
    limits: BatchLimits,
    batches: &mpsc::Sender<Result<Pulled>>,
    context: &Failure<'_>,
) {
    let mut remaining = request.limits.max_rows;
    loop {
        let chunk = match statement.step() {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                let _ = batches.blocking_send(Ok(Pulled::Done { truncated: false }));
                return;
            }
            Err(err) => {
                let failure =
                    engine_failure(&err, context.effect, context.applied, context.interrupter);
                let _ = batches.blocking_send(Err(failure));
                return;
            }
        };
        let pieces = convert::convert_chunk(&chunk, schema, types)
            .and_then(|batch| convert::bounded(batch, limits));
        let pieces = match pieces {
            Ok(pieces) => pieces,
            Err(message) => {
                let _ = batches.blocking_send(Err(error::after_applied_if(
                    context.applied,
                    decode(message),
                )));
                return;
            }
        };
        for piece in pieces {
            let (piece, cut) = match remaining {
                None => (piece, false),
                Some(left) if piece.num_rows() <= left => {
                    remaining = Some(left - piece.num_rows());
                    (piece, false)
                }
                Some(left) => (piece.slice(0, left), true),
            };
            if piece.num_rows() > 0 && batches.blocking_send(Ok(Pulled::Batch(piece))).is_err() {
                // The cursor is gone: dropping the statement ends the query.
                return;
            }
            if cut || remaining == Some(0) {
                let truncated = cut || more_rows(statement, context.effect);
                let _ = batches.blocking_send(Ok(Pulled::Done { truncated }));
                return;
            }
        }
    }
}

/// Whether rows remain past the row limit.
///
/// Probing makes the engine produce one more chunk: harmless for a read. For
/// anything else the answer is "maybe", and the result is marked truncated
/// rather than produce more of a statement that may write.
fn more_rows(statement: &Statement<'_>, effect: Effect) -> bool {
    if effect == Effect::Mutating {
        return true;
    }
    !matches!(statement.step(), Ok(None))
}

/// Reads a result to its end, discarding it.
fn drain(statement: &Statement<'_>) -> std::result::Result<(), duckdb::Error> {
    while statement.step()?.is_some() {}
    Ok(())
}

/// The result of an execution that left nothing to stream.
fn nothing(affected: u64, applied: bool) -> StreamStart {
    StreamStart {
        schema: Arc::new(Schema::empty()),
        affected,
        effect: Effect::ReadOnly,
        applied,
        streaming: false,
    }
}

/// A conversion failure: permanent, the same at every attempt.
fn decode(message: String) -> OxynError {
    error::driver(DuckDbError::Decode(message), ErrorClass::Permanent)
}

/// An engine failure, told apart from the interruption Oxyn asked for.
fn engine_failure(
    err: &duckdb::Error,
    effect: Effect,
    applied: bool,
    interrupter: &Interrupter,
) -> OxynError {
    if interrupter.tripped() && error::is_interrupt(err) {
        return stopped(effect, applied);
    }
    error::after_applied_if(applied, error::engine(err, effect))
}

/// The verdict of a statement the engine stopped on request.
///
/// A read that stopped left nothing: cancelled. Anything else may have left
/// what lives outside a transaction ([`error::stopped_write`]); and after an
/// earlier statement applied, never a cancellation.
fn stopped(effect: Effect, applied: bool) -> OxynError {
    let verdict = match effect {
        Effect::ReadOnly => OxynError::Cancelled,
        Effect::Mutating => error::stopped_write(),
    };
    error::after_applied_if(applied, verdict)
}

/// What a classified statement could do.
pub(crate) fn effect_of(info: &StatementInfo) -> Effect {
    if info.intent == StatementIntent::Read && !info.transaction_control {
        Effect::ReadOnly
    } else {
        Effect::Mutating
    }
}

/// Whether a statement is a data write whose result is a count: `INSERT`,
/// `UPDATE`, `DELETE` or `COPY`, without `RETURNING`.
fn counts_rows(text: &str) -> bool {
    let found = words(text, DIALECT);
    let Some(first) = found.first() else {
        return false;
    };
    let head = first.text.to_ascii_uppercase();
    matches!(head.as_str(), "INSERT" | "UPDATE" | "DELETE" | "COPY")
        && !found
            .iter()
            .any(|word| word.text.eq_ignore_ascii_case("RETURNING"))
}

/// Refuses bound values that do not match the placeholders.
fn expect_parameters(statement: &Statement<'_>, given: usize) -> Result<()> {
    let expected = statement.parameter_count();
    if expected == given {
        return Ok(());
    }
    Err(error::driver(
        DuckDbError::ParameterCount { expected, given },
        ErrorClass::Permanent,
    ))
}

/// A read-only transaction the driver opened, rolled back when dropped —
/// after every statement and result of the execution, declared before them.
pub(crate) struct ReadOnlyTransaction<'c>(&'c Connection);

impl Drop for ReadOnlyTransaction<'_> {
    fn drop(&mut self) {
        // Nothing to undo — the engine refused every write — and a failure
        // here leaves at worst the read-only transaction open, which the next
        // statement's error names.
        let _ = self.0.execute_batch("ROLLBACK");
    }
}

/// The last statement, prepared, after every earlier one ran.
type Last<'c> = (Statement<'c>, Effect, Option<ReadOnlyTransaction<'c>>);

/// Runs every statement but the last, and prepares the last.
///
/// Counted writes run here to their end, the last included: `None` then means
/// everything ran.
fn prepare_last<'c>(
    connection: &'c Connection,
    request: &ExecRequest,
    interrupter: &Interrupter,
    applied: &mut bool,
    affected: &mut u64,
) -> Result<Option<Last<'c>>> {
    let statements = classify(&request.text, DIALECT).statements;
    if statements.is_empty() {
        return Ok(None);
    }
    if !request.params.is_empty() && statements.len() > 1 {
        return Err(error::driver(
            DuckDbError::ParametersWithBatch,
            ErrorClass::Permanent,
        ));
    }
    if let Some(info) = statements.iter().find(|info| info.context_change) {
        return Err(OxynError::NotSupported {
            capability: format!(
                "`{}` changes the session's database or schema; choose it with the console's \
                 context selector, which keeps what Oxyn shows true",
                first_words(&info.text)
            ),
        });
    }
    let wrapper = if request.limits.read_only {
        Some(read_only_wrapper(connection, &statements)?)
    } else {
        None
    };
    let count = statements.len();
    for (index, info) in statements.iter().enumerate() {
        let effect = effect_of(info);
        if interrupter.tripped() {
            return Err(stopped(effect, *applied));
        }
        let last = index + 1 == count;
        let mut statement = connection.prepare(&info.text).map_err(|err| {
            error::after_applied_if(*applied, error::engine(&err, Effect::ReadOnly))
        })?;
        if last && !counts_rows(&info.text) {
            return Ok(Some((statement, effect, wrapper)));
        }
        let values = if last {
            params::values(&request.params).map_err(|message| {
                error::after_applied_if(
                    *applied,
                    error::driver(DuckDbError::Decode(message), ErrorClass::Permanent),
                )
            })?
        } else {
            Vec::new()
        };
        expect_parameters(&statement, values.len())
            .map_err(|err| error::after_applied_if(*applied, err))?;
        if counts_rows(&info.text) {
            let changed = statement
                .execute(duckdb::params_from_iter(values.iter()))
                .map_err(|err| engine_failure(&err, effect, *applied, interrupter))?;
            *affected = affected.saturating_add(u64::try_from(changed).unwrap_or(u64::MAX));
        } else {
            statement
                .stream_arrow(duckdb::params_from_iter(values.iter()))
                .map(drop)
                .and_then(|()| drain(&statement))
                .map_err(|err| engine_failure(&err, effect, *applied, interrupter))?;
        }
        *applied |= effect == Effect::Mutating;
    }
    drop(wrapper);
    Ok(None)
}

/// Checks that every statement reads, then opens the engine's read-only
/// transaction around them.
fn read_only_wrapper<'c>(
    connection: &'c Connection,
    statements: &[StatementInfo],
) -> Result<ReadOnlyTransaction<'c>> {
    if let Some(info) = statements
        .iter()
        .find(|info| effect_of(info) == Effect::Mutating)
    {
        return Err(OxynError::PolicyDenied {
            reason: format!(
                "this execution is read-only, and `{}` is not a read ({})",
                first_words(&info.text),
                info.intent
            ),
        });
    }
    match transaction::state(connection) {
        TransactionState::Idle => {}
        TransactionState::Open => {
            return Err(OxynError::PolicyDenied {
                reason: "this execution is read-only, and DuckDB can only hold that bound \
                         outside a transaction: commit or roll back the open transaction first"
                    .to_owned(),
            });
        }
        _ => {
            return Err(OxynError::PolicyDenied {
                reason: "this execution is read-only, and the session's transaction state \
                         could not be read to hold that bound"
                    .to_owned(),
            });
        }
    }
    connection
        .execute_batch("BEGIN TRANSACTION READ ONLY")
        .map_err(|err| error::engine(&err, Effect::ReadOnly))?;
    Ok(ReadOnlyTransaction(connection))
}

/// The first words of a statement, to name it in a refusal without copying a
/// whole script.
fn first_words(text: &str) -> String {
    let mut words = text
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    if words.chars().count() > 60 {
        words = words.chars().take(60).collect::<String>() + "…";
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_writes_without_returning_are_counted() {
        for text in [
            "INSERT INTO t VALUES (1)",
            "update t set v = 2",
            "DELETE FROM t",
            "COPY t TO 'out.csv'",
        ] {
            assert!(counts_rows(text), "{text}");
        }
        for text in [
            "INSERT INTO t VALUES (1) RETURNING *",
            "SELECT * FROM t",
            "EXPLAIN ANALYZE DELETE FROM t",
            "CREATE TABLE t (v INTEGER)",
            "-- INSERT\nSELECT 1",
        ] {
            assert!(!counts_rows(text), "{text}");
        }
    }

    #[test]
    fn a_refusal_names_the_statement_briefly() {
        let long = format!("SELECT {}", "x".repeat(200));
        assert!(first_words(&long).chars().count() <= 61);
        assert_eq!(first_words("DROP   TABLE audit"), "DROP TABLE audit");
    }
}
