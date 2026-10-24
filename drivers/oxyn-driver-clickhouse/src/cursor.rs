//! The cursor: the batches the execution task sends, one at a time.
//!
//! It holds no connection and decodes nothing: the task in [`crate::stream`]
//! does both, and the one-slot channel between them is the back-pressure.
//! Dropping the cursor closes the channel, which the task reads as "nobody
//! wants the rest" and answers with a kill.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{ExecStats, Result, StatementHandle};
use oxyn_driver::Cursor;
use tokio::sync::mpsc;

/// The stream of one execution.
#[derive(Debug)]
pub struct ClickhouseCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    batches: mpsc::Receiver<Result<RecordBatch>>,
    stats: ExecStats,
    started: Instant,
    /// Set by the task when rows came past the row bound.
    truncated: Arc<AtomicBool>,
}

impl ClickhouseCursor {
    pub(crate) fn new(
        handle: StatementHandle,
        schema: SchemaRef,
        batches: mpsc::Receiver<Result<RecordBatch>>,
        written_rows: Option<u64>,
        truncated: Arc<AtomicBool>,
        started: Instant,
    ) -> Self {
        let mut stats = ExecStats::default();
        if let Some(rows) = written_rows {
            // A write reports the rows it wrote: the summary of its response.
            stats.rows = rows;
        }
        stats.total_time = started.elapsed();
        Self {
            handle,
            schema,
            batches,
            stats,
            started,
            truncated,
        }
    }
}

#[async_trait]
impl Cursor for ClickhouseCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    /// The next batch the task decoded, or `None` once the stream ended
    /// cleanly.
    ///
    /// # Errors
    /// The failure that ended the stream, classified by the task.
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let next = self.batches.recv().await;
        self.stats.total_time = self.started.elapsed();
        match next {
            Some(Ok(batch)) => {
                self.stats.record_batch(
                    u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                Ok(Some(batch))
            }
            Some(Err(error)) => Err(error),
            None => {
                // The task set the flag before it closed the channel.
                if self.truncated.load(Ordering::Acquire) {
                    self.stats.mark_truncated();
                }
                Ok(None)
            }
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}
