//! One bounded page over the retained cursor or its Arrow history.

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor;
use std::sync::Arc;
use tokio::sync::OwnedMutexGuard;

use super::{BATCHES, State, Stream, expired, quota};

pub(super) struct Page {
    stream: Arc<Stream>,
    state: OwnedMutexGuard<State>,
    position: usize,
    remaining: usize,
    finished: bool,
    stats: ExecStats,
    started: std::time::Instant,
}

impl Page {
    pub(super) fn new(
        stream: Arc<Stream>,
        state: OwnedMutexGuard<State>,
        position: usize,
        remaining: usize,
    ) -> Self {
        Self {
            stream,
            state,
            position,
            remaining,
            finished: false,
            stats: ExecStats::default(),
            started: std::time::Instant::now(),
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // A dropped next_batch future may have consumed protocol bytes. Its
        // cursor is never reused, even when a previous page remains retained.
        if !self.finished {
            self.stream.cancel.cancel();
            self.state.cursor.take();
            self.state.lease.take();
            self.state.permit.take();
        }
    }
}

#[async_trait]
impl Cursor for Page {
    fn handle(&self) -> StatementHandle {
        self.stream.handle
    }
    fn schema(&self) -> SchemaRef {
        self.stream.history.schema().clone()
    }
    fn stats(&self) -> ExecStats {
        self.stats
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.stream.cancel.is_cancelled() {
            return Err(expired());
        }
        if self.remaining == 0 {
            self.finished = true;
            self.stats.total_time = self.started.elapsed();
            return Ok(None);
        }
        loop {
            if let Some((index, local)) = self.stream.history.locate(self.position) {
                let history = Arc::clone(&self.stream.history);
                let batch = tokio::task::spawn_blocking(move || history.batch(index))
                    .await
                    .map_err(|_| OxynError::Internal("preview history reader stopped".into()))??
                    .ok_or_else(|| OxynError::Internal("preview history batch is absent".into()))?;
                let count = self.remaining.min(batch.num_rows().saturating_sub(local));
                let page = batch.slice(local, count);
                self.position = self.position.saturating_add(count);
                self.remaining -= count;
                self.stats.record_batch(
                    u64::try_from(count).unwrap_or(u64::MAX),
                    u64::try_from(page.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                return Ok(Some(page));
            }
            let Some(cursor) = &mut self.state.cursor else {
                self.finished = true;
                self.stats.total_time = self.started.elapsed();
                return Ok(None);
            };
            match cursor.next_batch().await? {
                Some(batch) if batch.num_rows() == 0 => continue,
                Some(batch) => {
                    if self.stream.history.batch_count() >= BATCHES {
                        return Err(quota());
                    }
                    let history = Arc::clone(&self.stream.history);
                    let count = batch.num_rows();
                    let accepted = tokio::task::spawn_blocking(move || {
                        let before = history.row_count();
                        history.push(batch)?;
                        Ok::<_, OxynError>(history.row_count().saturating_sub(before))
                    })
                    .await
                    .map_err(|_| OxynError::Internal("preview history writer stopped".into()))??;
                    if accepted != count {
                        return Err(quota());
                    }
                }
                None => {
                    self.state.cursor.take();
                    if let Some(lease) = self.state.lease.take() {
                        lease.finish().await?;
                    }
                    self.state.permit.take();
                }
            }
        }
    }
}
