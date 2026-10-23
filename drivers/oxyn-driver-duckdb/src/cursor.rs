//! Cursor types are deliberately kept separate from session state.

use std::sync::Arc;

use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{ExecStats, Result, StatementHandle};
use oxyn_driver::Cursor;

/// A completed statement with no result rows.
pub(crate) struct EmptyCursor {
    handle: StatementHandle,
    schema: Arc<Schema>,
    stats: ExecStats,
}

impl EmptyCursor {
    pub(crate) fn new(handle: StatementHandle) -> Self {
        Self {
            handle,
            schema: Arc::new(Schema::empty()),
            stats: ExecStats::default(),
        }
    }
}

#[async_trait]
impl Cursor for EmptyCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> arrow::datatypes::SchemaRef {
        Arc::clone(&self.schema)
    }
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(None)
    }
    fn stats(&self) -> ExecStats {
        self.stats
    }
}
