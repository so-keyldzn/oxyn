//! The cursor: the batches of an answer that is already whole.
//!
//! Hrana over HTTP sends the result in one document, read under a budget
//! before the cursor exists ([`crate::http`]). The cursor therefore holds the
//! parsed rows and builds Arrow batches from them as the caller drains it —
//! a batch at a time, closed by bytes ([`crate::convert`]) — so the Arrow
//! copy of a large result is never held next to its source.

use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{ExecStats, Result, StatementHandle};
use oxyn_driver::Cursor;

use crate::convert::Batches;

/// The stream of one execution.
#[derive(Debug)]
pub struct TursoCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    batches: Batches,
    stats: ExecStats,
    started: Instant,
    /// Rows past the row bound were dropped.
    truncated: bool,
}

impl TursoCursor {
    pub(crate) fn new(
        handle: StatementHandle,
        schema: SchemaRef,
        batches: Batches,
        written_rows: Option<u64>,
        server_time_ms: f64,
        truncated: bool,
        started: Instant,
    ) -> Self {
        let mut stats = ExecStats::default();
        if let Some(rows) = written_rows {
            // A write reports the rows it changed.
            stats.rows = rows;
        }
        // A hostile or absent value is no duration: `None`, never a panic.
        stats.server_time = Duration::try_from_secs_f64(server_time_ms / 1000.0).ok();
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
impl Cursor for TursoCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    /// The next batch, or `None` once the rows are exhausted.
    ///
    /// # Errors
    /// [`oxyn_core::OxynError::Internal`] if a value does not fit its column,
    /// which the plan that read every value makes impossible.
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let next = self.batches.next_batch()?;
        self.stats.total_time = self.started.elapsed();
        match next {
            Some(batch) => {
                self.stats.record_batch(
                    u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                Ok(Some(batch))
            }
            None => {
                if self.truncated {
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
