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
mod tests;
