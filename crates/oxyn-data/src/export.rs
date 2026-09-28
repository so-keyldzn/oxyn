//! Export of a [`ResultBuffer`] to a file.
//!
//! **Streamed, never in one block.** The export reads batches back one by one
//! — including those that spilled to disk — and writes them as it goes.
//! Exporting 40 million rows takes no more memory than exporting a thousand
//! ([I-06](../../../CLAUDE.md#i-06)).
//!
//! **What is written reads back without Oxyn.** CSV, JSON and Arrow IPC are
//! public formats, produced by `arrow-rs`'s writers — the same ones that
//! format the grid, so the file and the screen do not diverge
//! ([I-11](../../../CLAUDE.md#i-11)).
//!
//! **A partial export is refused by default.** A truncated file that looks
//! like a complete file is silent data loss; the caller that accepts it
//! declares so with [`ExportOptions::allow_incomplete`].
//!
//! **A truncated result is always refused.** A buffer closed by a row limit,
//! saturation, a cancellation or a timeout has finished loading: nothing on
//! screen tells it from a whole result, which is why no option allows it
//! ([UX-SPEC](../../../docs/UX-SPEC.md#what-is-exported-is-what-is-displayed)).

use std::borrow::Cow;
use std::io::Write;

use arrow::csv::WriterBuilder as CsvWriterBuilder;
use arrow::ipc::writer::FileWriter as IpcFileWriter;
use arrow::json::{ArrayWriter, LineDelimitedWriter};
use arrow::record_batch::RecordBatch;
use oxyn_core::{CancelToken, ExportFormat};
use serde::{Deserialize, Serialize};

use crate::buffer::{BatchIndex, ResultBuffer};
use crate::error::{DataError, Result};

/// Settings of an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportOptions {
    /// Write a header line. Only concerns CSV and TSV.
    pub header: bool,
    /// What to write in place of an absent value, for CSV and TSV.
    ///
    /// Empty by default, which is the CSV convention: `a,,c`. Putting `NULL`
    /// there makes the file ambiguous as soon as a text column contains that word.
    pub null_text: Cow<'static, str>,
    /// Timestamp formatting pattern, in the `chrono` sense.
    ///
    /// `None` = RFC 3339, which is what spreadsheets and databases read back.
    pub timestamp_format: Option<Cow<'static, str>>,
    /// Accept exporting a result still being received.
    ///
    /// The export then takes a snapshot of the batches received at call time.
    pub allow_incomplete: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            header: true,
            null_text: Cow::Borrowed(""),
            timestamp_format: None,
            allow_incomplete: false,
        }
    }
}

impl ExportOptions {
    /// Whether to write the header line.
    #[must_use]
    pub fn with_header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// Changes the text of absent values.
    #[must_use]
    pub fn with_null_text(mut self, text: impl Into<Cow<'static, str>>) -> Self {
        self.null_text = text.into();
        self
    }

    /// Changes the timestamp pattern.
    #[must_use]
    pub fn with_timestamp_format(mut self, pattern: impl Into<Option<Cow<'static, str>>>) -> Self {
        self.timestamp_format = pattern.into();
        self
    }

    /// Allows exporting an incomplete result.
    #[must_use]
    pub fn allowing_incomplete(mut self) -> Self {
        self.allow_incomplete = true;
        self
    }
}

/// What an export produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ExportSummary {
    /// Rows written.
    pub rows: usize,
    /// Batches read back.
    pub batches: usize,
    /// Bytes written.
    pub bytes: u64,
}

/// Is this format really written by [`export`]?
///
/// Exists so that the interface does not offer a format it cannot produce:
/// without it, the action starts, the file picker opens, the user names their
/// destination — and the failure only comes afterwards, leaving an empty file
/// on their disk. What is not available is announced before the click.
///
/// Stays aligned with [`export`]'s `match`: the test
/// `every_format_declared_writable_is_really_written` fails otherwise.
#[must_use]
pub const fn is_supported(format: ExportFormat) -> bool {
    matches!(
        format,
        ExportFormat::Csv
            | ExportFormat::Tsv
            | ExportFormat::Json
            | ExportFormat::JsonLines
            | ExportFormat::ArrowIpc
    )
}

/// Writes the content of `buffer` into `writer`.
///
/// Reads batches back one by one, including from the spill file, and checks
/// cancellation between each batch. Never materializes more than one batch at
/// a time.
///
/// A result with no row at all produces an **empty** file, without even the
/// header line: it is the behavior of Arrow's CSV writer, which only writes
/// the header with the first batch. A caller that wants a header-only file
/// must compose it itself.
///
/// # Errors
///
/// * [`DataError::IncompleteResult`] if the result is still flowing and
///   [`ExportOptions::allow_incomplete`] is false;
/// * [`DataError::TruncatedResult`] when rows of the result are missing;
/// * [`DataError::UnsupportedFormat`] for Parquet, SQL and Markdown;
/// * [`DataError::Cancelled`] if the token is triggered — the partially
///   written file is left to the caller, who alone knows whether it must be
///   deleted;
/// * [`DataError::Io`], [`DataError::Arrow`] or [`DataError::Spill`] otherwise.
pub fn export<W: Write>(
    buffer: &ResultBuffer,
    format: ExportFormat,
    writer: W,
    opts: &ExportOptions,
    ct: &CancelToken,
) -> Result<ExportSummary> {
    ensure_exportable(buffer, opts)?;

    // Snapshot: the number of batches is read only once, so that the export
    // of a result still in progress has a defined end.
    let source = Source {
        buffer,
        ct,
        batch_count: buffer.batch_count(),
    };
    let mut counter = CountingWriter::new(writer);

    let mut summary = match format {
        ExportFormat::Csv => write_delimited(&source, &mut counter, opts, b',')?,
        ExportFormat::Tsv => write_delimited(&source, &mut counter, opts, b'\t')?,
        ExportFormat::JsonLines => {
            let mut output = LineDelimitedWriter::new(&mut counter);
            let summary =
                source.for_each_batch(|batch| output.write(batch).map_err(DataError::from))?;
            output.finish()?;
            summary
        }
        ExportFormat::Json => {
            let mut output = ArrayWriter::new(&mut counter);
            let summary =
                source.for_each_batch(|batch| output.write(batch).map_err(DataError::from))?;
            // Without `finish`, the JSON array is never closed: the file is
            // unreadable and nothing reported it.
            output.finish()?;
            summary
        }
        ExportFormat::ArrowIpc => {
            let mut output = IpcFileWriter::try_new(&mut counter, buffer.schema().as_ref())?;
            let summary =
                source.for_each_batch(|batch| output.write(batch).map_err(DataError::from))?;
            // The footer carries the block index: without it, the file is not
            // an Arrow file.
            output.finish()?;
            summary
        }
        // TODO(phase 1, opened on 2026-09-05): Parquet waits for the `parquet`
        // crate to be added to the workspace manifest; SQL waits for
        // identifier quoting from `oxyn-query` (I-10); Markdown waits for the
        // formatting decided by the interface. None of the three is missing
        // code here: each waits for a dependency that does not exist yet.
        other => {
            return Err(DataError::UnsupportedFormat {
                format: other.extension(),
            });
        }
    };

    counter.flush()?;
    summary.bytes = counter.bytes;
    Ok(summary)
}

/// Refuses what [`export`] would refuse, without writing anything.
///
/// For the caller that prepares a destination before writing: the refusal
/// then comes before any file is created.
///
/// # Errors
///
/// [`DataError::TruncatedResult`] or [`DataError::IncompleteResult`].
pub fn ensure_exportable(buffer: &ResultBuffer, opts: &ExportOptions) -> Result<()> {
    // `truncated` first: it is never cleared, and a truncated buffer still
    // open will not become whole by waiting.
    if buffer.stats().truncated {
        return Err(DataError::TruncatedResult);
    }
    if !buffer.is_complete() && !opts.allow_incomplete {
        return Err(DataError::IncompleteResult);
    }
    Ok(())
}

/// The snapshot of the batches to write.
///
/// The number of batches is read **once**, at construction: without that, an
/// export started on a result still in progress would have no defined end.
#[derive(Debug, Clone, Copy)]
struct Source<'a> {
    buffer: &'a ResultBuffer,
    ct: &'a CancelToken,
    batch_count: usize,
}

impl Source<'_> {
    /// Reads the snapshot's batches back and passes them to `ecrire`, checking
    /// cancellation between each.
    fn for_each_batch<F>(&self, mut emit: F) -> Result<ExportSummary>
    where
        F: FnMut(&RecordBatch) -> Result<()>,
    {
        let mut summary = ExportSummary::default();
        for position in 0..self.batch_count {
            // Between two batches, not in the middle: a file cut in the middle
            // of encoding cannot be recovered, whereas a file cut on a batch
            // boundary is a valid prefix.
            if self.ct.is_cancelled() {
                return Err(DataError::Cancelled);
            }
            let Some(rb) = self.buffer.batch(BatchIndex::new(position))? else {
                // The number of batches was read before the loop; a gap
                // signals a broken invariant, not a normal race.
                return Err(DataError::Spill(std::io::Error::other(
                    "a batch vanished from the buffer during export",
                )));
            };
            emit(&rb)?;
            summary.rows = summary.rows.saturating_add(rb.num_rows());
            summary.batches = summary.batches.saturating_add(1);
        }
        Ok(summary)
    }
}

/// CSV and TSV: same writer, but for the separator.
///
/// `arrow::csv::Writer` flushes its internal buffer at every batch written: an
/// output abandoned halfway stays a valid prefix.
fn write_delimited<W: Write>(
    source: &Source<'_>,
    output: &mut CountingWriter<W>,
    opts: &ExportOptions,
    delimiter: u8,
) -> Result<ExportSummary> {
    let mut builder = CsvWriterBuilder::new()
        .with_header(opts.header)
        .with_delimiter(delimiter)
        .with_null(opts.null_text.as_ref().to_owned());
    if let Some(pattern) = opts.timestamp_format.as_deref() {
        builder = builder
            .with_timestamp_format(pattern.to_owned())
            .with_timestamp_tz_format(pattern.to_owned());
    }

    let mut writer = builder.build(output);
    source.for_each_batch(|batch| writer.write(batch).map_err(DataError::from))
}

/// Writer that counts what goes through it.
///
/// The count feeds [`ExportSummary`] and the progress display: without it, a
/// 4 GB export has no landmark to show.
#[derive(Debug)]
struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> CountingWriter<W> {
    const fn new(inner: W) -> Self {
        Self { inner, bytes: 0 }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::ipc::reader::FileReader;
    use oxyn_core::ExecStats;

    use super::*;
    use crate::buffer::BufferLimits;

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
        let names: Vec<Option<String>> = ids
            .iter()
            .map(|i| {
                if i % 2 == 0 {
                    Some(format!("n{i}"))
                } else {
                    None
                }
            })
            .collect();
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(names)),
            ],
        )
        .expect("the columns match the schema built just above")
    }

    /// Closed buffer, with a budget so small that everything spilled to disk:
    /// it is the case that matters, since the export must read back from the
    /// temporary file.
    fn spilled_buffer() -> ResultBuffer {
        let buffer =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_memory_budget(1));
        buffer.push(batch_of(0, 3)).expect("batch accepted");
        buffer.push(batch_of(100, 2)).expect("batch accepted");
        buffer.mark_complete(ExecStats::default());
        assert_eq!(buffer.spilled_batches(), 2);
        buffer
    }

    fn exporter(format: ExportFormat, opts: &ExportOptions) -> (String, ExportSummary) {
        let buffer = spilled_buffer();
        let mut output: Vec<u8> = Vec::new();
        let summary = export(&buffer, format, &mut output, opts, &CancelToken::new())
            .expect("export without error");
        (
            String::from_utf8(output).expect("the tested formats are UTF-8"),
            summary,
        )
    }

    #[test]
    fn csv_carries_its_header_and_all_its_rows() {
        let (text, summary) = exporter(ExportFormat::Csv, &ExportOptions::default());
        let row_count: Vec<&str> = text.lines().collect();

        assert_eq!(row_count.first(), Some(&"id,name"));
        assert_eq!(row_count.len(), 6, "a header and five rows: {text}");
        assert_eq!(summary.rows, 5);
        assert_eq!(summary.batches, 2);
        assert!(summary.bytes > 0);
    }

    /// Spilling to disk must change nothing in the exported content.
    #[test]
    fn the_export_reads_back_spilled_batches() {
        let (text, _) = exporter(ExportFormat::Csv, &ExportOptions::default());
        for expected in ["0,n0", "1,", "2,n2", "100,n100", "101,"] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
    }

    #[test]
    fn tsv_uses_the_tab() {
        let (text, _) = exporter(ExportFormat::Tsv, &ExportOptions::default());
        assert!(text.starts_with("id\tname"), "{text}");
    }

    #[test]
    fn the_header_is_optional() {
        let (text, _) = exporter(
            ExportFormat::Csv,
            &ExportOptions::default().with_header(false),
        );
        assert!(!text.starts_with("id,name"), "{text}");
        assert_eq!(text.lines().count(), 5);
    }

    #[test]
    fn the_absent_value_text_is_configurable() {
        let (text, _) = exporter(
            ExportFormat::Csv,
            &ExportOptions::default().with_null_text("\\N"),
        );
        assert!(text.contains("1,\\N"), "{text}");
    }

    #[test]
    fn json_lines_produces_one_object_per_row() {
        let (text, summary) = exporter(ExportFormat::JsonLines, &ExportOptions::default());
        let row_count: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(row_count.len(), 5, "{text}");
        assert!(
            row_count.first().is_some_and(|l| l.starts_with('{')),
            "{text}"
        );
        assert_eq!(summary.rows, 5);
    }

    #[test]
    fn json_produces_a_closed_array() {
        let (text, _) = exporter(ExportFormat::Json, &ExportOptions::default());
        assert!(text.starts_with('['), "{text}");
        assert!(text.trim_end().ends_with(']'), "{text}");
    }

    /// The Arrow IPC round trip is the only export that converts nothing: what
    /// comes out must be exactly what went in.
    #[test]
    fn arrow_ipc_makes_an_exact_round_trip() {
        let buffer = spilled_buffer();
        let mut output: Vec<u8> = Vec::new();
        let summary = export(
            &buffer,
            ExportFormat::ArrowIpc,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export without error");
        assert_eq!(summary.rows, 5);

        let reader = FileReader::try_new(std::io::Cursor::new(output), None)
            .expect("the written file must be a valid Arrow file");
        assert_eq!(reader.schema().fields(), schema().fields());

        let reread: Vec<RecordBatch> = reader
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("reading batches back");
        assert_eq!(reread.len(), 2);
        assert_eq!(reread.first(), Some(&batch_of(0, 3)));
        assert_eq!(reread.get(1), Some(&batch_of(100, 2)));
    }

    #[test]
    fn parquet_is_refused_explicitly() {
        let buffer = spilled_buffer();
        let mut output: Vec<u8> = Vec::new();
        match export(
            &buffer,
            ExportFormat::Parquet,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        ) {
            Err(DataError::UnsupportedFormat { format }) => assert_eq!(format, "parquet"),
            other => panic!("expected UnsupportedFormat, got {other:?}"),
        }
        assert!(output.is_empty(), "nothing must be written");
    }

    /// The failure mode to avoid: a partial file that looks complete.
    #[test]
    fn a_result_in_progress_is_not_exported_by_accident() {
        let buffer = ResultBuffer::new(schema(), 1 << 20);
        buffer.push(batch_of(0, 3)).expect("batch accepted");
        let mut output: Vec<u8> = Vec::new();

        match export(
            &buffer,
            ExportFormat::Csv,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        ) {
            Err(DataError::IncompleteResult) => {}
            other => panic!("expected IncompleteResult, got {other:?}"),
        }

        // Declared explicitly, it is allowed.
        let summary = export(
            &buffer,
            ExportFormat::Csv,
            &mut output,
            &ExportOptions::default().allowing_incomplete(),
            &CancelToken::new(),
        )
        .expect("explicit export of a partial result");
        assert_eq!(summary.rows, 3);
    }

    /// Closed but truncated: the case nothing on screen tells from a whole
    /// result. No option allows it, not even `allowing_incomplete`.
    #[test]
    fn a_truncated_result_never_exports() {
        let buffer = spilled_buffer();
        buffer.mark_truncated();

        for opts in [
            ExportOptions::default(),
            ExportOptions::default().allowing_incomplete(),
        ] {
            let mut output: Vec<u8> = Vec::new();
            match export(
                &buffer,
                ExportFormat::Csv,
                &mut output,
                &opts,
                &CancelToken::new(),
            ) {
                Err(DataError::TruncatedResult) => {}
                other => panic!("expected TruncatedResult, got {other:?}"),
            }
            assert!(output.is_empty(), "nothing may be written");
        }
    }

    #[test]
    fn a_cancellation_interrupts_the_export() {
        let buffer = spilled_buffer();
        let ct = CancelToken::new();
        ct.cancel();
        let mut output: Vec<u8> = Vec::new();

        match export(
            &buffer,
            ExportFormat::Csv,
            &mut output,
            &ExportOptions::default(),
            &ct,
        ) {
            Err(DataError::Cancelled) => {}
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn every_format_declared_writable_is_really_written() {
        // `is_supported` governs what the interface offers. A divergence
        // between this list and `export`'s `match` breaks nothing here: it
        // breaks three clicks later, after the user named their file, and
        // leaves an empty file behind.
        let buffer = spilled_buffer();
        for format in [
            ExportFormat::Csv,
            ExportFormat::Tsv,
            ExportFormat::Json,
            ExportFormat::JsonLines,
            ExportFormat::Parquet,
            ExportFormat::ArrowIpc,
            ExportFormat::Sql,
            ExportFormat::Markdown,
        ] {
            let mut output: Vec<u8> = Vec::new();
            let written = export(
                &buffer,
                format,
                &mut output,
                &ExportOptions::default(),
                &CancelToken::new(),
            )
            .is_ok();
            assert_eq!(
                written,
                is_supported(format),
                "{format} : `is_supported` dit {}, `export` dit {written}",
                is_supported(format)
            );
        }
    }

    #[test]
    fn an_empty_result_produces_an_empty_file_not_an_error() {
        let buffer = ResultBuffer::new(schema(), 1 << 20);
        buffer.mark_complete(ExecStats::default());
        let mut output: Vec<u8> = Vec::new();

        let summary = export(
            &buffer,
            ExportFormat::Csv,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export without error");

        assert_eq!(summary.rows, 0);
        assert_eq!(summary.batches, 0);
        assert!(output.is_empty());
    }
}
