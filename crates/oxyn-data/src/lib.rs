//! Oxyn's result buffers.
//!
//! "First display under 100 ms, stable memory over 10 M rows" is held or lost
//! here. This crate implements [ADR-0002]: a driver produces Arrow
//! [`RecordBatch`](arrow::record_batch::RecordBatch)es, and nothing converts
//! them again until the screen or the export.
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`buffer`] | bounded accumulation, spill to disk, `locate` in O(log n) | ADR-0002, PERFORMANCE |
//! | [`cell`] | rendering a cell for the grid | UX-SPEC |
//! | [`sink`] | back-pressure between a cursor and a buffer | ARCHITECTURE §9 |
//! | [`mod@export`] | CSV, TSV, JSON, JSON lines, Arrow IPC | I-11 |
//! | [`error`] | the layer's error boundary | rust.md |
//!
//! # The complete path
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxyn_core::CancelToken;
//! use oxyn_data::{BatchSink, BatchSource, ResultBuffer, format_cell, FormatOptions};
//!
//! # async fn example(mut cursor: Box<dyn BatchSource>) -> Result<(), Box<dyn std::error::Error>> {
//! // The buffer is created as soon as the schema is known: the grid draws its
//! // columns before a single row arrives.
//! let buffer = Arc::new(ResultBuffer::new(cursor.schema(), 256 * 1024 * 1024));
//! let sink = BatchSink::new(Arc::clone(&buffer));
//! let cancel = CancelToken::new();
//!
//! // The task drains; the interface reads the same `Arc` without ever waiting for it.
//! let outcome = sink.drain(cursor.as_mut(), &cancel).await?;
//!
//! let options = FormatOptions::default();
//! if let Some((position, offset)) = buffer.locate(0) {
//!     if let Some(batch) = buffer.batch(position)? {
//!         let cell = format_cell(&batch, offset, 0, &options);
//!         println!("{}", cell.display_with(&options));
//!     }
//! }
//! # let _ = outcome;
//! # Ok(())
//! # }
//! ```
//!
//! # Local page storage
//!
//! [ADR-0012] specifies positioned reads of autonomous Arrow IPC streams,
//! off the UI thread. `cached_batch` never performs I/O or waits for a lock;
//! `load_page` populates the byte-bounded cache with cooperative cancellation.
//! Initial batches and decoded pages share one retention budget. Decoder
//! temporaries and clones held by readers are separate from cache retention.
//!
//! [ADR-0012]: ../../../docs/adr/0012-lecture-pages-resultats.md
//! [ADR-0002]: ../../../docs/adr/0002-arrow-result-model.md

pub mod buffer;
pub mod cell;
mod duration;
pub mod error;
pub mod export;
pub mod export_file;
pub mod find;
pub mod sink;
mod spill;
pub mod value_page;

pub use buffer::{BatchIndex, BufferLimits, DEFAULT_MEMORY_BUDGET, Pressure, ResultBuffer};
pub use cell::{
    BinaryDisplay, CellValue, DEFAULT_MAX_LEN, FormatOptions, GROUP_SEPARATOR, NumberGrouping,
    TimestampDisplay, format_cell, format_value, timestamp_display,
};
pub use error::{DataError, Result};
pub use export::{ExportOptions, ExportSummary, ensure_exportable, export, is_supported};
pub use export_file::export_to_path;
pub use find::{FindOutcome, find_rows};
pub use sink::{BatchProgress, BatchSink, BatchSource, SinkOutcome};

/// What you import in one go when working with results.
pub mod prelude {
    pub use crate::buffer::{BatchIndex, BufferLimits, Pressure, ResultBuffer};
    pub use crate::cell::{BinaryDisplay, CellValue, FormatOptions, NumberGrouping, format_cell};
    pub use crate::error::{DataError, Result};
    pub use crate::export::{ExportOptions, ExportSummary, export, is_supported};
    pub use crate::sink::{BatchProgress, BatchSink, BatchSource, SinkOutcome};
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::record_batch::RecordBatch;
    use futures::executor::block_on;
    use futures::future::BoxFuture;
    use oxyn_core::{CancelToken, ExecStats, ExportFormat, OxynError};

    use crate::prelude::*;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
        ]))
    }

    fn batch_of(start: i32, rows: usize) -> RecordBatch {
        let ids: Vec<i32> = (0..rows)
            .map(|i| start.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        let names: Vec<Option<String>> = ids.iter().map(|i| Some(format!("n{i}"))).collect();
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(names)),
            ],
        )
        .expect("the columns match the schema built just above")
    }

    /// Simulated cursor: many batches, none of which fits in the budget.
    #[derive(Debug)]
    struct CountdownSource {
        remaining: usize,
    }

    impl BatchSource for CountdownSource {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(
            &mut self,
        ) -> BoxFuture<'_, std::result::Result<Option<RecordBatch>, OxynError>> {
            let rank = self.remaining;
            self.remaining = self.remaining.saturating_sub(1);
            Box::pin(async move {
                if rank == 0 {
                    return Ok(None);
                }
                let start = i32::try_from(rank).unwrap_or(i32::MAX).saturating_mul(100);
                Ok(Some(batch_of(start, 50)))
            })
        }

        fn stats(&self) -> ExecStats {
            ExecStats::default()
        }
    }

    /// The crate's complete path, with a budget small enough to force a
    /// spill: draining, locating, rendering, export.
    ///
    /// It is the reduced version of phase 0's exit criterion: memory stays
    /// bounded, nothing is lost, and what is displayed is what is exported.
    #[test]
    fn the_complete_path_fits_in_a_tiny_budget() {
        let buf = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_memory_budget(2_048),
        ));
        let batch_sink = BatchSink::new(Arc::clone(&buf));
        let mut cursor = CountdownSource { remaining: 40 };
        let cancel = CancelToken::new();

        let outcome = block_on(batch_sink.drain(&mut cursor, &cancel)).expect("drain");
        assert_eq!(outcome, SinkOutcome::Exhausted);
        assert!(outcome.is_complete());
        assert_eq!(buf.row_count(), 40 * 50);
        assert!(
            buf.spilled_batches() > 0,
            "the budget must have been exceeded"
        );
        // The memory invariant, in one line: what stays resident never
        // exceeds the budget, whatever the volume that went through.
        assert!(
            buf.resident_bytes() <= 2_048,
            "{} resident bytes for a budget of 2,048",
            buf.resident_bytes()
        );

        // A row taken far into the result is read again without re-execution.
        let options = FormatOptions::default();
        let (position, offset) = buf.locate(1_999).expect("the row exists");
        let rb = buf.batch(position).expect("read back").expect("the batch");
        let value_cell = format_cell(&rb, offset, 1, &options);
        assert!(
            value_cell.text().is_some_and(|t| t.starts_with('n')),
            "{value_cell:?}"
        );

        // And what is displayed is what is exported.
        let mut output: Vec<u8> = Vec::new();
        let summary = export(
            &buf,
            ExportFormat::Csv,
            &mut output,
            &ExportOptions::default(),
            &cancel,
        )
        .expect("export");
        assert_eq!(summary.rows, 2_000);

        let txt = String::from_utf8(output).expect("CSV in UTF-8");
        let expected = value_cell.text().unwrap_or_default();
        assert!(txt.contains(expected), "{expected} missing from the export");
    }
}
