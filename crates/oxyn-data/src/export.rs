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
use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::csv::WriterBuilder as CsvWriterBuilder;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::FileWriter as IpcFileWriter;
use arrow::json::{ArrayWriter, LineDelimitedWriter, WriterBuilder as JsonWriterBuilder};
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
    ensure_exportable(buffer, format, opts)?;

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
            // Explicit nulls: Arrow's default drops every null property, so a
            // column NULL in every row vanishes from the file although the
            // grid and CSV show it, and an SQL `NULL` reads as a missing key
            // (UX-SPEC, "Columns and value inspection").
            let mut output: LineDelimitedWriter<_> = JsonWriterBuilder::new()
                .with_explicit_nulls(true)
                .build(&mut counter);
            let summary = source.for_each_batch(|batch| {
                output
                    .write(&*durations_as_text(batch)?)
                    .map_err(DataError::from)
            })?;
            output.finish()?;
            summary
        }
        ExportFormat::Json => {
            // Explicit nulls, as for JSON Lines.
            let mut output: ArrayWriter<_> = JsonWriterBuilder::new()
                .with_explicit_nulls(true)
                .build(&mut counter);
            let summary = source.for_each_batch(|batch| {
                output
                    .write(&*durations_as_text(batch)?)
                    .map_err(DataError::from)
            })?;
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
/// [`DataError::TruncatedResult`], [`DataError::IncompleteResult`], or
/// [`DataError::DuplicateColumnNames`] for JSON and JSON Lines.
pub fn ensure_exportable(
    buffer: &ResultBuffer,
    format: ExportFormat,
    opts: &ExportOptions,
) -> Result<()> {
    // `truncated` first: it is never cleared, and a truncated buffer still
    // open will not become whole by waiting.
    if buffer.stats().truncated {
        return Err(DataError::TruncatedResult);
    }
    if !buffer.is_complete() && !opts.allow_incomplete {
        return Err(DataError::IncompleteResult);
    }
    if matches!(format, ExportFormat::Json | ExportFormat::JsonLines) {
        let names = shared_names(buffer.schema());
        if !names.is_empty() {
            return Err(DataError::DuplicateColumnNames {
                format: format.extension(),
                names,
            });
        }
    }
    Ok(())
}

/// The names carried by more than one column, once each, in the order of
/// their first column.
///
/// Compared exactly: JSON keys are case-sensitive, so `id` and `ID` are two
/// keys and lose nothing. A result tells its columns apart by position —
/// `SELECT *` over a join routinely repeats `id` — and a JSON object by name:
/// writing both would leave the reader one value of the two
/// ([UX-SPEC](../../../docs/UX-SPEC.md#what-is-exported-is-what-is-displayed)).
fn shared_names(schema: &Schema) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut shared: Vec<String> = Vec::new();
    for field in schema.fields() {
        let name = field.name();
        if !seen.insert(name.as_str()) && !shared.iter().any(|known| known == name) {
            shared.push(name.clone());
        }
    }
    shared
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
    source.for_each_batch(|batch| {
        writer
            .write(&*durations_as_text(batch)?)
            .map_err(DataError::from)
    })
}

/// `batch` with its duration columns rewritten as the grid's text.
///
/// The text formats only: Arrow's CSV and JSON writers write a duration in
/// ISO 8601 (`-PT3023999S`), which is neither what the grid shows nor what a
/// MySQL `TIME` reads back from. Arrow IPC keeps the native type — it is the
/// format that converts nothing.
///
/// Borrows the batch when it has no duration column, which is almost always:
/// the copy is paid only by the results that need it, once per batch, never
/// per value. Only top-level columns: a duration inside a struct or a list
/// stays Arrow's, since CSV refuses nested columns anyway.
fn durations_as_text(batch: &RecordBatch) -> Result<Cow<'_, RecordBatch>> {
    let schema = batch.schema();
    if !schema
        .fields()
        .iter()
        .any(|field| matches!(field.data_type(), DataType::Duration(_)))
    {
        return Ok(Cow::Borrowed(batch));
    }
    let mut fields = Vec::with_capacity(schema.fields().len());
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(batch.num_columns());
    for (field, column) in schema.fields().iter().zip(batch.columns()) {
        match crate::duration::as_text_column(column.as_ref()) {
            Some(text) => {
                // Name, nullability and metadata kept: only the type changes.
                fields.push(Arc::new(
                    Field::new(field.name(), DataType::Utf8, field.is_nullable())
                        .with_metadata(field.metadata().clone()),
                ));
                columns.push(Arc::new(text));
            }
            None => {
                fields.push(Arc::clone(field));
                columns.push(Arc::clone(column));
            }
        }
    }
    let schema = Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()));
    Ok(Cow::Owned(RecordBatch::try_new(schema, columns)?))
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

    /// Every Arrow type the MySQL driver emits (ADR-0050), one column each.
    fn mysql_buffer() -> ResultBuffer {
        use arrow::array::{
            BinaryArray, Date32Array, Decimal128Array, Decimal256Array, DurationMicrosecondArray,
            Float32Array, Float64Array, Int8Array, Int16Array, Int64Array,
            TimestampMicrosecondArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
        };
        use arrow::datatypes::{TimeUnit, i256};

        let wide =
            i256::from_string("12345678901234567890123456789012345123456789012345678901234567890")
                .expect("65 digits fit in 256 bits");
        let geometry = std::collections::HashMap::from([(
            "oxyn:mysql_type".to_owned(),
            "GEOMETRY".to_owned(),
        )]);
        let schema = Arc::new(Schema::new(vec![
            Field::new("i8", DataType::Int8, true),
            Field::new("i16", DataType::Int16, true),
            Field::new("i32", DataType::Int32, true),
            Field::new("i64", DataType::Int64, true),
            Field::new("u8", DataType::UInt8, true),
            Field::new("year", DataType::UInt16, true),
            Field::new("u32", DataType::UInt32, true),
            Field::new("bit", DataType::UInt64, true),
            Field::new("f32", DataType::Float32, true),
            Field::new("f64", DataType::Float64, true),
            Field::new("d128", DataType::Decimal128(10, 2), true),
            Field::new("d256", DataType::Decimal256(65, 30), true),
            Field::new("d256_whole", DataType::Decimal256(65, 0), true),
            Field::new("day", DataType::Date32, true),
            Field::new(
                "datetime",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                true,
            ),
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                true,
            ),
            Field::new("time", DataType::Duration(TimeUnit::Microsecond), true),
            Field::new("text", DataType::Utf8, true),
            Field::new("geom", DataType::Binary, true).with_metadata(geometry),
        ]));
        let columns: Vec<ArrayRef> = vec![
            Arc::new(Int8Array::from(vec![Some(-128), None])),
            Arc::new(Int16Array::from(vec![Some(-32_768), None])),
            Arc::new(Int32Array::from(vec![Some(i32::MIN), None])),
            Arc::new(Int64Array::from(vec![Some(i64::MIN), None])),
            Arc::new(UInt8Array::from(vec![Some(255), None])),
            Arc::new(UInt16Array::from(vec![Some(2155), None])),
            Arc::new(UInt32Array::from(vec![Some(u32::MAX), None])),
            Arc::new(UInt64Array::from(vec![Some(u64::MAX), None])),
            Arc::new(Float32Array::from(vec![Some(1.5), None])),
            Arc::new(Float64Array::from(vec![Some(-0.25), None])),
            Arc::new(
                Decimal128Array::from(vec![Some(12_345), None])
                    .with_precision_and_scale(10, 2)
                    .expect("valid DECIMAL(10, 2)"),
            ),
            Arc::new(
                Decimal256Array::from(vec![Some(wide.wrapping_neg()), None])
                    .with_precision_and_scale(65, 30)
                    .expect("valid DECIMAL(65, 30)"),
            ),
            Arc::new(
                Decimal256Array::from(vec![Some(wide), None])
                    .with_precision_and_scale(65, 0)
                    .expect("valid DECIMAL(65, 0)"),
            ),
            Arc::new(Date32Array::from(vec![Some(18_628), None])),
            Arc::new(TimestampMicrosecondArray::from(vec![Some(0), None])),
            Arc::new(TimestampMicrosecondArray::from(vec![Some(0), None]).with_timezone("UTC")),
            Arc::new(DurationMicrosecondArray::from(vec![
                Some(-3_020_399_000_000),
                None,
            ])),
            Arc::new(StringArray::from(vec![Some("x"), None])),
            Arc::new(BinaryArray::from(vec![Some(b"\x01".as_slice()), None])),
        ];
        let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
        buffer
            .push(RecordBatch::try_new(schema, columns).expect("columns match the schema"))
            .expect("batch accepted");
        buffer.mark_complete(ExecStats::default());
        buffer
    }

    fn export_text(buffer: &ResultBuffer, format: ExportFormat) -> String {
        let mut output: Vec<u8> = Vec::new();
        export(
            buffer,
            format,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .unwrap_or_else(|error| panic!("{format} exports every MySQL type: {error}"));
        String::from_utf8(output).expect("text formats are UTF-8")
    }

    #[test]
    fn every_mysql_type_exports_in_every_text_format() {
        let buffer = mysql_buffer();
        let negative = "-12345678901234567890123456789012345.123456789012345678901234567890";
        let whole = "12345678901234567890123456789012345123456789012345678901234567890";
        for format in [ExportFormat::Csv, ExportFormat::Tsv] {
            let text = export_text(&buffer, format);
            // The grid's text, not ISO 8601's `-PT3023999S`.
            assert!(text.contains("-838:59:59.000000"), "{format}: {text}");
            assert!(!text.contains("PT"), "{format}: {text}");
            assert!(text.contains(negative), "{format}: {text}");
            assert!(text.contains(whole), "{format}: {text}");
        }
        for format in [ExportFormat::Json, ExportFormat::JsonLines] {
            let text = export_text(&buffer, format);
            // A string in JSON, like every text the grid shows.
            assert!(
                text.contains(r#""time":"-838:59:59.000000""#),
                "{format}: {text}"
            );
            // Decimals stay bare numbers, digit for digit.
            assert!(
                text.contains(&format!(r#""d256":{negative}"#)),
                "{format}: {text}"
            );
            assert!(
                text.contains(&format!(r#""d256_whole":{whole}"#)),
                "{format}: {text}"
            );
        }
    }

    /// Arrow IPC converts nothing: the duration stays a duration.
    #[test]
    fn arrow_ipc_keeps_mysql_types_native() {
        let buffer = mysql_buffer();
        let mut output: Vec<u8> = Vec::new();
        export(
            &buffer,
            ExportFormat::ArrowIpc,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export without error");
        let reader = FileReader::try_new(std::io::Cursor::new(output), None)
            .expect("the written file must be a valid Arrow file");
        assert_eq!(&reader.schema(), buffer.schema());
    }

    /// A NULL is written `null`, never dropped: a column NULL in every row
    /// stays in every record, and `""` stays distinct from `null`. Parsed
    /// back over two batches, in both JSON formats.
    #[test]
    fn json_keeps_null_properties_and_all_null_columns() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("note", DataType::Utf8, true),
            Field::new("optional", DataType::Utf8, true),
        ]));
        let batch = |ids: Vec<i32>, notes: Vec<Option<&str>>| {
            let empty: Vec<Option<&str>> = vec![None; ids.len()];
            RecordBatch::try_new(
                Arc::clone(&schema),
                vec![
                    Arc::new(Int32Array::from(ids)),
                    Arc::new(StringArray::from(notes)),
                    Arc::new(StringArray::from(empty)),
                ],
            )
            .expect("the columns match the schema")
        };
        let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
        buffer
            .push(batch(vec![1, 2], vec![Some("a"), Some("")]))
            .expect("batch accepted");
        buffer
            .push(batch(vec![3, 4], vec![None, Some("d")]))
            .expect("batch accepted");
        buffer.mark_complete(ExecStats::default());

        for format in [ExportFormat::Json, ExportFormat::JsonLines] {
            let text = export_text(&buffer, format);
            let records: Vec<serde_json::Value> = if format == ExportFormat::Json {
                serde_json::from_str(&text).expect("a JSON array")
            } else {
                text.lines()
                    .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
                    .collect()
            };
            assert_eq!(records.len(), 4, "{format}: {text}");
            for record in &records {
                let keys: Vec<&str> = record
                    .as_object()
                    .expect("each record is an object")
                    .keys()
                    .map(String::as_str)
                    .collect();
                assert_eq!(keys, ["id", "note", "optional"], "{format}: {text}");
                assert_eq!(record["optional"], serde_json::Value::Null, "{format}");
            }
            let notes: Vec<&serde_json::Value> = records.iter().map(|r| &r["note"]).collect();
            assert_eq!(
                notes,
                [
                    &serde_json::json!("a"),
                    &serde_json::json!(""),
                    &serde_json::Value::Null,
                    &serde_json::json!("d"),
                ],
                "{format}: {text}"
            );
        }
    }

    /// A complete buffer of one row over `names`, column `i` holding `i`.
    fn one_row_named(names: &[&str]) -> ResultBuffer {
        let schema = Arc::new(Schema::new(
            names
                .iter()
                .map(|name| Field::new(*name, DataType::Int32, false))
                .collect::<Vec<_>>(),
        ));
        let columns = (0..names.len())
            .map(|i| {
                let value = i32::try_from(i).expect("a few test columns");
                Arc::new(Int32Array::from(vec![value])) as ArrayRef
            })
            .collect();
        let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
        buffer
            .push(RecordBatch::try_new(schema, columns).expect("the columns match the schema"))
            .expect("batch accepted");
        buffer.mark_complete(ExecStats::default());
        buffer
    }

    /// Duplicate names — an alias repeated, or `SELECT *` over a join — are
    /// refused in JSON and JSON Lines, named once each, and the message asks
    /// for aliases (issue #183).
    #[test]
    fn json_refuses_duplicate_column_names_and_names_them() {
        let buffer = one_row_named(&["id", "name", "id", "name", "id", "total"]);
        for format in [ExportFormat::Json, ExportFormat::JsonLines] {
            let mut output: Vec<u8> = Vec::new();
            let outcome = export(
                &buffer,
                format,
                &mut output,
                &ExportOptions::default(),
                &CancelToken::new(),
            );
            let Err(DataError::DuplicateColumnNames { names, .. }) = &outcome else {
                panic!("{format}: expected a refusal, got {outcome:?}");
            };
            assert_eq!(names, &["id", "name"], "{format}");
            let message = outcome.expect_err("refused").to_string();
            assert!(message.contains("\"id\", \"name\""), "{message}");
            assert!(message.contains("aliases"), "{message}");
            assert!(output.is_empty(), "{format}: nothing is written");
        }
    }

    /// The formats that keep columns by position still export them all.
    #[test]
    fn positional_formats_keep_duplicate_column_names() {
        let buffer = one_row_named(&["id", "id"]);
        assert_eq!(export_text(&buffer, ExportFormat::Csv), "id,id\n0,1\n");
        assert_eq!(export_text(&buffer, ExportFormat::Tsv), "id\tid\n0\t1\n");
        let mut output: Vec<u8> = Vec::new();
        export(
            &buffer,
            ExportFormat::ArrowIpc,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("Arrow IPC keeps duplicate names");
    }

    /// Distinct names stay distinct: case counts, and a name that merely looks
    /// like a suffixed copy is its own key.
    #[test]
    fn json_accepts_names_that_differ_only_by_case_or_suffix() {
        let buffer = one_row_named(&["id", "ID", "id_2"]);
        let text = export_text(&buffer, ExportFormat::Json);
        let records: Vec<serde_json::Value> = serde_json::from_str(&text).expect("a JSON array");
        assert_eq!(
            records,
            [serde_json::json!({"id": 0, "ID": 1, "id_2": 2})],
            "{text}"
        );
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
