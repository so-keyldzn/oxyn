//! The cursor: one Arrow batch at a time, and a Stop that reaches the engine.
//!
//! # Cancellation reaches the engine
//!
//! The token is checked before each batch and watched while waiting for one.
//! When it fires, `duckdb_interrupt` goes to the engine, targeted at this
//! stream's task: a chunk being computed stops with DuckDB's `INTERRUPT`
//! error, the carrier thread drops the statement, and the session is free.
//! Dropping the cursor while the thread computes does the same: a cursor that
//! did not reach the end of its stream interrupts its task when dropped.
//!
//! DuckDB runs in Oxyn's process: there is no server to send a cancellation to,
//! and the session does not declare `SERVER_SIDE_CANCEL`
//! ([`crate::session`]). The interruption is nonetheless the engine's own, not
//! a dropped future.
//!
//! # What a Stop claims
//!
//! A read that stops changed nothing: [`OxynError::Cancelled`]. Anything else
//! — a `RETURNING` write, a `CALL`, a `PRAGMA` — may already have applied, and
//! what lives outside a transaction stays: the Stop answers
//! [`OxynError::OutcomeUnknown`], ambiguous, never retried
//! ([I-13](../../../CLAUDE.md#i-13)). The same goes for a read whose batch
//! already ran a statement that could write.

use std::pin::pin;
use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::future::{Either, select};
use oxyn_core::{CancelToken, ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor;
use tokio::sync::mpsc;

use crate::error::{self, Effect};
use crate::interrupt::WorkId;
use crate::stream::{Pulled, StreamStart};
use crate::worker::WorkerHandle;

/// A stream of `RecordBatch` fed by a DuckDB statement.
pub struct DuckDbCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    batches: Option<mpsc::Receiver<Result<Pulled>>>,
    worker: WorkerHandle,
    work: WorkId,
    effect: Effect,
    applied: bool,
    cancel: CancelToken,
    stats: ExecStats,
    started: Instant,
    elapsed: Option<Duration>,
    finished: bool,
}

impl std::fmt::Debug for DuckDbCursor {
    /// Neither the schema nor the batches: where the stream stands.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbCursor")
            .field("handle", &self.handle)
            .field("columns", &self.schema.fields().len())
            .field("finished", &self.finished)
            .field("rows", &self.stats.rows)
            .finish_non_exhaustive()
    }
}

/// What a cursor is built from.
pub(crate) struct Opened {
    pub handle: StatementHandle,
    pub start: StreamStart,
    pub batches: mpsc::Receiver<Result<Pulled>>,
    pub worker: WorkerHandle,
    pub work: WorkId,
    pub cancel: CancelToken,
    pub started: Instant,
}

impl DuckDbCursor {
    pub(crate) fn new(opened: Opened) -> Self {
        let Opened {
            handle,
            start,
            batches,
            worker,
            work,
            cancel,
            started,
        } = opened;
        let mut stats = ExecStats::default();
        if !start.streaming {
            // No column: the user expects the count of affected rows, which
            // `ExecStats::rows` carries.
            stats.rows = start.affected;
        }
        let mut cursor = Self {
            handle,
            schema: start.schema,
            batches: Some(batches),
            worker,
            work,
            effect: start.effect,
            applied: start.applied,
            cancel,
            stats,
            started,
            elapsed: None,
            finished: false,
        };
        if !start.streaming {
            cursor.close(false);
        }
        cursor
    }

    fn record(&mut self, batch: &RecordBatch) {
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
    }

    /// Marks the stream finished, lets the carrier thread go, freezes the
    /// duration.
    fn close(&mut self, truncated: bool) {
        self.finished = true;
        self.batches = None;
        if truncated {
            self.stats.mark_truncated();
        }
        if self.elapsed.is_none() {
            self.elapsed = Some(self.started.elapsed());
        }
    }

    /// Stops the stream on the token: interrupts the engine, then says what
    /// the Stop may claim.
    fn stop(&mut self) -> OxynError {
        self.worker.interrupt(self.work);
        self.close(true);
        let verdict = match self.effect {
            Effect::ReadOnly => OxynError::Cancelled,
            Effect::Mutating => error::stopped_write(),
        };
        error::after_applied_if(self.applied, verdict)
    }
}

#[async_trait]
impl Cursor for DuckDbCursor {
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
        if self.cancel.is_cancelled() {
            return Err(self.stop());
        }
        let cancel = self.cancel.clone();
        let Some(batches) = self.batches.as_mut() else {
            self.close(true);
            return Err(error::closed());
        };
        let outcome = match select(pin!(batches.recv()), pin!(cancel.cancelled())).await {
            Either::Left((received, _)) => Some(received),
            Either::Right(((), _)) => None,
        };
        let Some(received) = outcome else {
            return Err(self.stop());
        };
        match received {
            Some(Ok(Pulled::Batch(batch))) => {
                self.record(&batch);
                Ok(Some(batch))
            }
            Some(Ok(Pulled::Done { truncated })) => {
                self.close(truncated);
                Ok(None)
            }
            Some(Err(err)) => {
                self.close(true);
                Err(err)
            }
            None => {
                self.close(true);
                Err(error::closed())
            }
        }
    }

    fn stats(&self) -> ExecStats {
        let mut stats = self.stats;
        stats.total_time = self.elapsed.unwrap_or_else(|| self.started.elapsed());
        // `server_time` stays `None`: the engine runs in this process, and
        // there is no server clock to set against the client's.
        stats
    }
}

impl Drop for DuckDbCursor {
    /// A cursor dropped before the end of its stream interrupts its task: the
    /// carrier thread may be computing the next chunk of an aggregate nobody
    /// will read.
    fn drop(&mut self) {
        if !self.finished {
            self.worker.interrupt(self.work);
        }
    }
}
