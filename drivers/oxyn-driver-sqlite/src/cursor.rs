//! The cursor: one Arrow batch at a time, and a cancellation that really cuts.
//!
//! # Cancellation, here, is not decorative
//!
//! The token is checked **before each batch request** and watched **during the
//! wait**. When it fires, `sqlite3_interrupt` goes to the engine: a
//! `sqlite3_step` already launched on a four-minute aggregation stops, the
//! worker thread becomes available again, and the cursor becomes unusable.
//!
//! Dropping the future would not be enough — it is exactly the failure that
//! [`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md) describes. It is also
//! why waiting for a batch interrupts its task when its future is destroyed
//! (`WorkerHandle::await_reply`): a cursor destroyed while it waits for a batch
//! interrupts what is still running. Destroyed at rest, it only closes the
//! channel, and the worker thread leaves its streaming loop.
//!
//! # What this cursor does not claim to be
//!
//! `sqlite3_interrupt` is a **local** cancellation: SQLite has no server, it runs
//! in Oxyn's process. The session therefore does not declare
//! [`Capabilities::SERVER_SIDE_CANCEL`](oxyn_core::Capabilities::SERVER_SIDE_CANCEL),
//! and [`SqliteSession::cancel`](crate::SqliteSession) refuses. The cancellation
//! path is the [`CancelToken`], and it is the only one.
//!
//! # A Stop does not decide how a write ended
//!
//! The token firing proves nothing about a statement that may write: a
//! `RETURNING` makes all its changes at its first step, before its rows are
//! read. A read stopped is cancelled; a write is reported as the engine ends
//! it. Its last batch carries the end of the statement
//! ([`Pulled::Last`]), so a Stop arriving after it finds the cursor finished;
//! a Stop arriving before, while a batch is requested, waits for the worker
//! thread's verdict; and a Stop at rest, with the statement still open, is an
//! [`OxynError::OutcomeUnknown`] — never a cancellation the engine did not
//! report ([I-13](../../../CLAUDE.md#i-13)).
//!
//! # After a cancellation, the cursor does not resume
//!
//! An interrupted statement leaves the engine at a point nothing allows resuming
//! cleanly. The cursor marks itself finished and refuses what follows: a resume
//! point is designed, not improvised.

use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{CancelToken, ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor;
use tokio::sync::{mpsc, oneshot};

use crate::error::{self, Effect};
use crate::interrupt::WorkId;
use crate::stream::{Pull, Pulled, StreamStart};
use crate::worker::WorkerHandle;

/// A stream of `RecordBatch` fed by an SQLite statement.
pub struct SqliteCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    /// The first batch, produced before the cursor even exists: resolving the
    /// column types required reading rows.
    pending: Option<RecordBatch>,
    pulls: mpsc::UnboundedSender<Pull>,
    worker: WorkerHandle,
    /// The worker thread task that streams this flow: an interruption targets
    /// only it.
    work: WorkId,
    /// What the statement could do: decides what a Stop may claim.
    effect: Effect,
    cancel: CancelToken,
    stats: ExecStats,
    started: Instant,
    elapsed: Option<Duration>,
    finished: bool,
}

impl std::fmt::Debug for SqliteCursor {
    /// Neither the schema nor the batches: a cursor `Debug` is for knowing where
    /// the stream stands, not for printing data.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteCursor")
            .field("handle", &self.handle)
            .field("columns", &self.schema.fields().len())
            .field("finished", &self.finished)
            .field("rows", &self.stats.rows)
            .finish_non_exhaustive()
    }
}

impl SqliteCursor {
    /// Builds the cursor from what the worker thread has already produced.
    pub(crate) fn new(
        handle: StatementHandle,
        start: StreamStart,
        pulls: mpsc::UnboundedSender<Pull>,
        worker: WorkerHandle,
        work: WorkId,
        cancel: CancelToken,
        started: Instant,
    ) -> Self {
        let mut stats = ExecStats::default();
        if start.schema.fields().is_empty() {
            // No column: what the user expects is the count of affected rows.
            // `ExecStats::rows` carries both meanings.
            stats.rows = start.affected;
        }

        let mut cursor = Self {
            handle,
            schema: start.schema,
            pending: None,
            pulls,
            worker,
            work,
            effect: start.effect,
            cancel,
            stats,
            started,
            elapsed: None,
            finished: false,
        };

        match start.first {
            Pulled::Batch(batch) => {
                cursor.record(&batch);
                cursor.pending = Some(batch);
            }
            Pulled::Last { batch, truncated } => {
                cursor.record(&batch);
                cursor.pending = Some(batch);
                cursor.close(truncated);
            }
            Pulled::Done { truncated } => cursor.close(truncated),
        }
        cursor
    }

    /// Records a batch in the statistics.
    fn record(&mut self, batch: &RecordBatch) {
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
    }

    /// Marks the stream finished and freezes the duration.
    fn close(&mut self, truncated: bool) {
        self.finished = true;
        if truncated {
            // Must reach the screen: a truncated result that looks complete leads to
            // wrong conclusions on real data.
            self.stats.mark_truncated();
        }
        if self.elapsed.is_none() {
            self.elapsed = Some(self.started.elapsed());
        }
    }

    /// Returns the stop requested before any batch was claimed.
    ///
    /// **Without interrupting**: no request is in flight, so the worker thread is
    /// not in `sqlite3_step`. Setting the interruption flag on an engine at rest
    /// would cancel nothing and could land on the next statement.
    ///
    /// A read is cancelled. A write is not known to be: its statement is still
    /// open, after a first step that may already have applied it, and what
    /// releasing it keeps is not established here.
    fn abort(&mut self) -> OxynError {
        self.close(true);
        match self.effect {
            Effect::ReadOnly => OxynError::Cancelled,
            Effect::Mutating => stopped_write(),
        }
    }
}

/// The error of a write stopped while its statement was still open.
fn stopped_write() -> OxynError {
    OxynError::OutcomeUnknown(
        "the write was stopped after it started, and SQLite may already have applied it; \
         check the data before running it again"
            .to_owned(),
    )
}

#[async_trait]
impl Cursor for SqliteCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        SchemaRef::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Some(batch) = self.pending.take() {
            return Ok(Some(batch));
        }
        if self.finished {
            return Ok(None);
        }
        if self.cancel.is_cancelled() {
            return Err(self.abort());
        }

        let (reply, answer) = oneshot::channel();
        if self.pulls.send(reply).is_err() {
            self.close(true);
            return Err(error::closed());
        }

        // The token is cloned: the wait borrows it, and `self` is already
        // mutably borrowed. From here on, the worker thread may be in
        // `sqlite3_step`: if this future is abandoned, the wait interrupts the
        // stream's task. A read lets go at the Stop; a write waits for what the
        // engine says of it.
        let cancel = self.cancel.clone();
        let pulled = match self.effect {
            Effect::ReadOnly => self.worker.await_reply(answer, &cancel, self.work).await,
            Effect::Mutating => self.worker.await_verdict(answer, &cancel, self.work).await,
        };

        match pulled {
            Ok(Pulled::Batch(batch)) => {
                self.record(&batch);
                Ok(Some(batch))
            }
            Ok(Pulled::Last { batch, truncated }) => {
                self.record(&batch);
                self.close(truncated);
                Ok(Some(batch))
            }
            Ok(Pulled::Done { truncated }) => {
                self.close(truncated);
                Ok(None)
            }
            Err(err) => {
                // Including cancellation: `await_reply` has already interrupted the
                // engine. The cursor does not resume.
                self.close(true);
                Err(err)
            }
        }
    }

    fn stats(&self) -> ExecStats {
        let mut stats = self.stats;
        stats.total_time = self.elapsed.unwrap_or_else(|| self.started.elapsed());
        // `server_time` stays `None`: SQLite runs in Oxyn's process, there is no
        // server clock to set against the client clock. Measuring it per
        // statement would cost two clock reads per row, on the hottest path of
        // the driver.
        stats
    }
}
