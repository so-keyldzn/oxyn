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

use crate::error;
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

    /// Returns the cancellation requested before any batch was claimed.
    ///
    /// **Without interrupting**: no request is in flight, so the worker thread is
    /// not in `sqlite3_step`. Setting the interruption flag on an engine at rest
    /// would cancel nothing and could land on the next statement.
    fn abort(&mut self) -> OxynError {
        self.close(true);
        OxynError::Cancelled
    }
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

        // The token is cloned: `await_reply` borrows it, and `self` is already
        // mutably borrowed. From here on, the worker thread may be in
        // `sqlite3_step`: if this future is abandoned, `await_reply` interrupts
        // the stream's task.
        let cancel = self.cancel.clone();
        let pulled = self.worker.await_reply(answer, &cancel, self.work).await;

        match pulled {
            Ok(Pulled::Batch(batch)) => {
                self.record(&batch);
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
