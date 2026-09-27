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

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::StreamExt as _;
use oxyn_core::{
    CancelToken, DriverId, ErrorClass, ExecLimits, ExecStats, OxynError, Result, StatementHandle,
    StatementIntent,
};
use oxyn_driver::Cursor;
use sqlx::postgres::{PgArguments, PgStatement};
use sqlx::{Either, Statement as _};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::cancel::{BackendCanceller, StatementRegistry, Verdict, VerdictSender};
use crate::decode::BatchAssembler;
use crate::error::{Bound, map_stream_error};
use crate::lease::Lease;
use crate::session::{SQL_RESET_AFTER_WRITE, SQL_RESET_SEARCH_PATH, SQL_ROLLBACK};
use crate::types::PgDecoding;

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
    /// [`ExecLimits::max_rows`] is reached.
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
            Some(CursorEvent::Batch(lot)) => {
                self.stats.record_batch(
                    u64::try_from(lot.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(lot.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                Ok(Some(lot))
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
            Some(CursorEvent::Failed(erreur)) => {
                self.seal(true);
                Err(*erreur)
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
    /// The prepared statement: it is what gave the schema.
    pub(crate) statement: PgStatement,
    /// The bound parameters, already encoded.
    pub(crate) arguments: PgArguments,
    /// Were there caller values among them?
    ///
    /// Recorded by the session before `arguments` is moved into `query_with`:
    /// the server's message can quote a bound value, and `PgArguments` no
    /// longer says, here, where its bytes come from
    /// ([I-03](../../../CLAUDE.md#i-03)).
    pub(crate) bound: Bound,
    /// The batches' schema, known before the first row.
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

/// Starts the stream and returns the cursor that drains it.
///
/// The call returns immediately: the schema is already known — it comes from
/// the prepared statement — so the grid draws its columns while the first row
/// is still travelling.
///
/// `cancel` is the execution's **own** token, already recorded in the registry:
/// dropping this cursor fires it, and so does `Session::cancel`.
pub(crate) fn spawn(request: StreamRequest, cancel: CancelToken) -> PostgresCursor {
    let (envoi, reception) = mpsc::channel(1);

    let schema = SchemaRef::clone(&request.schema);
    let handle = request.handle;
    let projects_columns = !schema.fields().is_empty();
    let jeton = cancel.clone();

    let task = tokio::spawn(async move {
        run(request, jeton, envoi).await;
    });

    PostgresCursor {
        handle,
        schema,
        events: reception,
        cancel,
        task,
        stats: ExecStats::default(),
        started: Instant::now(),
        finished: false,
        projects_columns,
    }
}

/// The body of the stream task.
#[allow(clippy::too_many_lines)]
async fn run(request: StreamRequest, cancel: CancelToken, events: mpsc::Sender<CursorEvent>) {
    let StreamRequest {
        driver,
        mut connection,
        statement,
        arguments,
        bound,
        restore_context,
        opens_transaction,
        schema,
        decodings,
        limits,
        intent,
        backend_pid,
        canceller,
        statements,
        verdict,
        handle,
    } = request;

    let mut assembleur = BatchAssembler::new(schema, &decodings);
    let mut affectees: u64 = 0;
    let mut produites: usize = 0;
    let echeance = limits
        .timeout
        .map(|duree| tokio::time::Instant::now() + duree);

    let arret = {
        // `Statement::query_with` rather than `sqlx::query_statement_with`: the
        // database type there is the statement's, with no inference to bubble
        // up.
        let requete = statement.query_with(arguments);
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
        #[expect(deprecated, reason = "only stream exposing rows_affected(); see above")]
        let mut flux = requete.fetch_many(&mut *connection);

        loop {
            let etape = tokio::select! {
                // `biased`: an already requested cancellation always wins over
                // a ready batch. Without it, a fast stream can make the
                // cancellation wait indefinitely.
                biased;
                () = cancel.cancelled() => Etape::Interrompu(Halt::Cancelled),
                () = attendre(echeance) => Etape::Interrompu(Halt::TimedOut),
                recu = flux.next() => match recu {
                    Some(resultat) => Etape::Recu(resultat),
                    None => Etape::Fin,
                },
            };

            let resultat = match etape {
                Etape::Fin => break Halt::Exhausted,
                Etape::Interrompu(raison) => break raison,
                Etape::Recu(resultat) => resultat,
            };

            let element = match resultat {
                Ok(element) => element,
                Err(erreur) => {
                    let oxyn = map_stream_error(&driver, intent, limits.read_only, bound, erreur);
                    let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
                    break Halt::Failed;
                }
            };

            let ligne = match element {
                // End of a statement: the affected row count is the most useful
                // thing a write has to say.
                Either::Left(resume) => {
                    affectees = affectees.saturating_add(resume.rows_affected());
                    continue;
                }
                Either::Right(ligne) => ligne,
            };

            if let Err(erreur) = assembleur.push(&ligne) {
                let oxyn = OxynError::driver(driver.clone(), ErrorClass::Permanent, erreur);
                let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
                break Halt::Failed;
            }
            produites = produites.saturating_add(1);

            let borne_atteinte = limits.max_rows.is_some_and(|max| produites >= max);
            let lot_plein =
                assembleur.bytes() >= BATCH_BYTE_BUDGET || assembleur.rows() >= BATCH_ROW_CEILING;

            if lot_plein || borne_atteinte {
                // The token is watched **also** during back-pressure: a grid
                // that no longer reads would otherwise let `Session::cancel`
                // wait forever, the query still running on the server.
                let emission = tokio::select! {
                    biased;
                    () = cancel.cancelled() => Emission::Annulee,
                    emission = emettre(&mut assembleur, &driver, &events) => emission,
                };
                match emission {
                    Emission::Poursuivre => {}
                    Emission::Annulee => break Halt::Cancelled,
                    Emission::Abandonne => break Halt::Abandoned,
                    Emission::Echouee => break Halt::Failed,
                }
            }
            if borne_atteinte {
                break Halt::RowLimit;
            }
        }
    };

    // The stop is decided: a token fired after this point changes nothing any
    // more. The task removes itself from the registry, and it — not
    // `Session::cancel` — sends the cancellation if one is needed (see the
    // `cancel` module).
    statements.forget(handle);

    // The stream is dropped: the borrow on the connection is lifted.
    if arret.needs_server_cancel() {
        // A connection whose stream was abandoned may have kept unread bytes:
        // returning it to the pool would desynchronize the next borrower. It is
        // held **until after** the cancellation is sent: as long as it is, no
        // other query can run on `backend_pid`.
        connection.discard();
        match canceller.cancel_backend(backend_pid).await {
            Ok(()) => verdict.settle(Verdict::Cancelled),
            Err(erreur) => {
                // Reported here, and returned to `Session::cancel` if it is
                // waiting: a dropped cursor, for its part, has nobody to tell.
                tracing::warn!(
                    target: "oxyn::driver::postgres",
                    error = %erreur,
                    "server-side cancellation did not succeed"
                );
                verdict.settle(Verdict::CancelFailed(std::sync::Arc::new(erreur)));
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
        let mut remises: Vec<(&'static str, &'static str)> = Vec::with_capacity(2);
        if limits.read_only {
            // The transaction opened by `BEGIN READ ONLY` must be closed before
            // returning to the pool: a connection left `idle in transaction`
            // keeps locks and blocks the `VACUUM` of the whole database. The
            // `ROLLBACK` also undoes any `SET` the user's statement may have set
            // inside the transaction — not the context `SET`, set before it.
            remises.push((
                SQL_ROLLBACK,
                "the read-only transaction could not be closed",
            ));
            if restore_context {
                remises.push((
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
            remises.push((
                SQL_RESET_AFTER_WRITE,
                "the session state could not be reset after a write",
            ));
        }

        let mut remise = !remises.is_empty();
        for (instruction, echec) in remises {
            if let Err(erreur) = sqlx::raw_sql(instruction).execute(&mut *connection).await {
                // Translated before being logged, as everywhere else: a raw
                // `sqlx::Error` can carry the connection URL.
                tracing::warn!(
                    target: "oxyn::driver::postgres",
                    error = %crate::error::map_exec_error(
                        &driver,
                        oxyn_core::StatementIntent::Read,
                        erreur,
                    ),
                    "{echec}"
                );
                remise = false;
                break;
            }
        }
        if remise {
            connection.restored();
        }
    }
    drop(connection);

    match arret {
        Halt::Failed => {}
        Halt::Cancelled | Halt::Abandoned => {
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::Cancelled)))
                .await;
        }
        Halt::TimedOut => {
            let delai = limits.timeout.unwrap_or_default();
            // `Timeout` is ambiguous by construction: the server may have
            // applied the write (I-13). `OxynError::class` says so, not this
            // message.
            let _ = events
                .send(CursorEvent::Failed(Box::new(OxynError::Timeout {
                    after: delai,
                })))
                .await;
        }
        Halt::Exhausted | Halt::RowLimit => {
            if !assembleur.is_empty()
                && let Emission::Echouee | Emission::Abandonne =
                    emettre(&mut assembleur, &driver, &events).await
            {
                return;
            }
            let _ = events
                .send(CursorEvent::Finished {
                    affected_rows: affectees,
                    truncated: arret == Halt::RowLimit,
                })
                .await;
        }
    }
}

/// What one iteration of the loop produced.
enum Etape {
    /// The stream returned an element.
    Recu(
        std::result::Result<
            Either<sqlx::postgres::PgQueryResult, sqlx::postgres::PgRow>,
            sqlx::Error,
        >,
    ),
    /// The stream is exhausted.
    Fin,
    /// An interruption won the race.
    Interrompu(Halt),
}

/// What emitting a batch gave.
enum Emission {
    /// The batch went out, carry on.
    Poursuivre,
    /// The token fired while the channel was full.
    Annulee,
    /// Nobody listens any more: the cursor was dropped.
    Abandonne,
    /// The batch could not be built; the error is already emitted.
    Echouee,
}

/// Closes the current batch and sends it, honoring the channel's
/// back-pressure.
async fn emettre(
    assembleur: &mut BatchAssembler,
    driver: &DriverId,
    events: &mpsc::Sender<CursorEvent>,
) -> Emission {
    let lot = match assembleur.finish() {
        Ok(lot) => lot,
        Err(erreur) => {
            let oxyn = OxynError::driver(driver.clone(), ErrorClass::Permanent, erreur);
            let _ = events.send(CursorEvent::Failed(Box::new(oxyn))).await;
            return Emission::Echouee;
        }
    };
    // `send` on a full channel **waits**: that is where back-pressure applies,
    // and it is what keeps memory from inflating.
    if events.send(CursorEvent::Batch(lot)).await.is_err() {
        return Emission::Abandonne;
    }
    Emission::Poursuivre
}

/// Waits for the deadline, or forever when there is none.
///
/// The instant is **absolute**: recreating this future on every loop turn
/// therefore does not push the delay back, which a relative duration would.
async fn attendre(deadline: Option<tokio::time::Instant>) {
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
        for raison in [
            Halt::Cancelled,
            Halt::TimedOut,
            Halt::RowLimit,
            Halt::Abandoned,
        ] {
            assert!(
                raison.needs_server_cancel(),
                "{raison:?} leaves a query running"
            );
        }
    }
}
