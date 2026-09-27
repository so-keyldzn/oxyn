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
    pub fn with_null_text(mut self, texte: impl Into<Cow<'static, str>>) -> Self {
        self.null_text = texte.into();
        self
    }

    /// Changes the timestamp pattern.
    #[must_use]
    pub fn with_timestamp_format(mut self, motif: impl Into<Option<Cow<'static, str>>>) -> Self {
        self.timestamp_format = motif.into();
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
        lots: buffer.batch_count(),
    };
    let mut compteur = CountingWriter::new(writer);

    let mut resume = match format {
        ExportFormat::Csv => ecrire_delimite(&source, &mut compteur, opts, b',')?,
        ExportFormat::Tsv => ecrire_delimite(&source, &mut compteur, opts, b'\t')?,
        ExportFormat::JsonLines => {
            let mut sortie = LineDelimitedWriter::new(&mut compteur);
            let resume =
                source.pour_chaque_lot(|lot| sortie.write(lot).map_err(DataError::from))?;
            sortie.finish()?;
            resume
        }
        ExportFormat::Json => {
            let mut sortie = ArrayWriter::new(&mut compteur);
            let resume =
                source.pour_chaque_lot(|lot| sortie.write(lot).map_err(DataError::from))?;
            // Without `finish`, the JSON array is never closed: the file is
            // unreadable and nothing reported it.
            sortie.finish()?;
            resume
        }
        ExportFormat::ArrowIpc => {
            let mut sortie = IpcFileWriter::try_new(&mut compteur, buffer.schema().as_ref())?;
            let resume =
                source.pour_chaque_lot(|lot| sortie.write(lot).map_err(DataError::from))?;
            // The footer carries the block index: without it, the file is not
            // an Arrow file.
            sortie.finish()?;
            resume
        }
        // TODO(phase 1, opened on 2026-09-05): Parquet waits for the `parquet`
        // crate to be added to the workspace manifest; SQL waits for
        // identifier quoting from `oxyn-query` (I-10); Markdown waits for the
        // formatting decided by the interface. None of the three is missing
        // code here: each waits for a dependency that does not exist yet.
        autre => {
            return Err(DataError::UnsupportedFormat {
                format: autre.extension(),
            });
        }
    };

    compteur.flush()?;
    resume.bytes = compteur.bytes;
    Ok(resume)
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
    lots: usize,
}

impl Source<'_> {
    /// Reads the snapshot's batches back and passes them to `ecrire`, checking
    /// cancellation between each.
    fn pour_chaque_lot<F>(&self, mut ecrire: F) -> Result<ExportSummary>
    where
        F: FnMut(&RecordBatch) -> Result<()>,
    {
        let mut resume = ExportSummary::default();
        for position in 0..self.lots {
            // Between two batches, not in the middle: a file cut in the middle
            // of encoding cannot be recovered, whereas a file cut on a batch
            // boundary is a valid prefix.
            if self.ct.is_cancelled() {
                return Err(DataError::Cancelled);
            }
            let Some(lot) = self.buffer.batch(BatchIndex::new(position))? else {
                // The number of batches was read before the loop; a gap
                // signals a broken invariant, not a normal race.
                return Err(DataError::Spill(std::io::Error::other(
                    "a batch vanished from the buffer during export",
                )));
            };
            ecrire(&lot)?;
            resume.rows = resume.rows.saturating_add(lot.num_rows());
            resume.batches = resume.batches.saturating_add(1);
        }
        Ok(resume)
    }
}

/// CSV and TSV: same writer, but for the separator.
///
/// `arrow::csv::Writer` flushes its internal buffer at every batch written: an
/// output abandoned halfway stays a valid prefix.
fn ecrire_delimite<W: Write>(
    source: &Source<'_>,
    sortie: &mut CountingWriter<W>,
    opts: &ExportOptions,
    delimiteur: u8,
) -> Result<ExportSummary> {
    let mut constructeur = CsvWriterBuilder::new()
        .with_header(opts.header)
        .with_delimiter(delimiteur)
        .with_null(opts.null_text.as_ref().to_owned());
    if let Some(motif) = opts.timestamp_format.as_deref() {
        constructeur = constructeur
            .with_timestamp_format(motif.to_owned())
            .with_timestamp_tz_format(motif.to_owned());
    }

    let mut ecrivain = constructeur.build(sortie);
    source.pour_chaque_lot(|lot| ecrivain.write(lot).map_err(DataError::from))
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
        let ecrits = self.inner.write(buf)?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(ecrits).unwrap_or(u64::MAX));
        Ok(ecrits)
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
            Field::new("nom", DataType::Utf8, true),
        ]))
    }

    fn lot(depart: i32, lignes: usize) -> RecordBatch {
        let ids: Vec<i32> = (0..lignes)
            .map(|i| depart.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        let noms: Vec<Option<String>> = ids
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
                Arc::new(StringArray::from(noms)),
            ],
        )
        .expect("the columns match the schema built just above")
    }

    /// Closed buffer, with a budget so small that everything spilled to disk:
    /// it is the case that matters, since the export must read back from the
    /// temporary file.
    fn tampon_deborde() -> ResultBuffer {
        let tampon =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_memory_budget(1));
        tampon.push(lot(0, 3)).expect("batch accepted");
        tampon.push(lot(100, 2)).expect("batch accepted");
        tampon.mark_complete(ExecStats::default());
        assert_eq!(tampon.spilled_batches(), 2);
        tampon
    }

    fn exporter(format: ExportFormat, opts: &ExportOptions) -> (String, ExportSummary) {
        let tampon = tampon_deborde();
        let mut sortie: Vec<u8> = Vec::new();
        let resume = export(&tampon, format, &mut sortie, opts, &CancelToken::new())
            .expect("export without error");
        (
            String::from_utf8(sortie).expect("the tested formats are UTF-8"),
            resume,
        )
    }

    #[test]
    fn csv_carries_its_header_and_all_its_rows() {
        let (texte, resume) = exporter(ExportFormat::Csv, &ExportOptions::default());
        let lignes: Vec<&str> = texte.lines().collect();

        assert_eq!(lignes.first(), Some(&"id,nom"));
        assert_eq!(lignes.len(), 6, "a header and five rows: {texte}");
        assert_eq!(resume.rows, 5);
        assert_eq!(resume.batches, 2);
        assert!(resume.bytes > 0);
    }

    /// Spilling to disk must change nothing in the exported content.
    #[test]
    fn the_export_reads_back_spilled_batches() {
        let (texte, _) = exporter(ExportFormat::Csv, &ExportOptions::default());
        for attendu in ["0,n0", "1,", "2,n2", "100,n100", "101,"] {
            assert!(texte.contains(attendu), "{attendu} missing from:\n{texte}");
        }
    }

    #[test]
    fn tsv_uses_the_tab() {
        let (texte, _) = exporter(ExportFormat::Tsv, &ExportOptions::default());
        assert!(texte.starts_with("id\tnom"), "{texte}");
    }

    #[test]
    fn the_header_is_optional() {
        let (texte, _) = exporter(
            ExportFormat::Csv,
            &ExportOptions::default().with_header(false),
        );
        assert!(!texte.starts_with("id,nom"), "{texte}");
        assert_eq!(texte.lines().count(), 5);
    }

    #[test]
    fn the_absent_value_text_is_configurable() {
        let (texte, _) = exporter(
            ExportFormat::Csv,
            &ExportOptions::default().with_null_text("\\N"),
        );
        assert!(texte.contains("1,\\N"), "{texte}");
    }

    #[test]
    fn json_lines_produces_one_object_per_row() {
        let (texte, resume) = exporter(ExportFormat::JsonLines, &ExportOptions::default());
        let lignes: Vec<&str> = texte.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lignes.len(), 5, "{texte}");
        assert!(
            lignes.first().is_some_and(|l| l.starts_with('{')),
            "{texte}"
        );
        assert_eq!(resume.rows, 5);
    }

    #[test]
    fn json_produces_a_closed_array() {
        let (texte, _) = exporter(ExportFormat::Json, &ExportOptions::default());
        assert!(texte.starts_with('['), "{texte}");
        assert!(texte.trim_end().ends_with(']'), "{texte}");
    }

    /// The Arrow IPC round trip is the only export that converts nothing: what
    /// comes out must be exactly what went in.
    #[test]
    fn arrow_ipc_makes_an_exact_round_trip() {
        let tampon = tampon_deborde();
        let mut sortie: Vec<u8> = Vec::new();
        let resume = export(
            &tampon,
            ExportFormat::ArrowIpc,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export without error");
        assert_eq!(resume.rows, 5);

        let lecteur = FileReader::try_new(std::io::Cursor::new(sortie), None)
            .expect("the written file must be a valid Arrow file");
        assert_eq!(lecteur.schema().fields(), schema().fields());

        let relus: Vec<RecordBatch> = lecteur
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("reading batches back");
        assert_eq!(relus.len(), 2);
        assert_eq!(relus.first(), Some(&lot(0, 3)));
        assert_eq!(relus.get(1), Some(&lot(100, 2)));
    }

    #[test]
    fn parquet_is_refused_explicitly() {
        let tampon = tampon_deborde();
        let mut sortie: Vec<u8> = Vec::new();
        match export(
            &tampon,
            ExportFormat::Parquet,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        ) {
            Err(DataError::UnsupportedFormat { format }) => assert_eq!(format, "parquet"),
            autre => panic!("attendu UnsupportedFormat, obtenu {autre:?}"),
        }
        assert!(sortie.is_empty(), "nothing must be written");
    }

    /// The failure mode to avoid: a partial file that looks complete.
    #[test]
    fn a_result_in_progress_is_not_exported_by_accident() {
        let tampon = ResultBuffer::new(schema(), 1 << 20);
        tampon.push(lot(0, 3)).expect("batch accepted");
        let mut sortie: Vec<u8> = Vec::new();

        match export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        ) {
            Err(DataError::IncompleteResult) => {}
            autre => panic!("attendu IncompleteResult, obtenu {autre:?}"),
        }

        // Declared explicitly, it is allowed.
        let resume = export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default().allowing_incomplete(),
            &CancelToken::new(),
        )
        .expect("explicit export of a partial result");
        assert_eq!(resume.rows, 3);
    }

    /// Closed but truncated: the case nothing on screen tells from a whole
    /// result. No option allows it, not even `allowing_incomplete`.
    #[test]
    fn a_truncated_result_never_exports() {
        let tampon = tampon_deborde();
        tampon.mark_truncated();

        for opts in [
            ExportOptions::default(),
            ExportOptions::default().allowing_incomplete(),
        ] {
            let mut sortie: Vec<u8> = Vec::new();
            match export(
                &tampon,
                ExportFormat::Csv,
                &mut sortie,
                &opts,
                &CancelToken::new(),
            ) {
                Err(DataError::TruncatedResult) => {}
                autre => panic!("expected TruncatedResult, got {autre:?}"),
            }
            assert!(sortie.is_empty(), "nothing may be written");
        }
    }

    #[test]
    fn a_cancellation_interrupts_the_export() {
        let tampon = tampon_deborde();
        let ct = CancelToken::new();
        ct.cancel();
        let mut sortie: Vec<u8> = Vec::new();

        match export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &ct,
        ) {
            Err(DataError::Cancelled) => {}
            autre => panic!("attendu Cancelled, obtenu {autre:?}"),
        }
    }

    #[test]
    fn every_format_declared_writable_is_really_written() {
        // `is_supported` governs what the interface offers. A divergence
        // between this list and `export`'s `match` breaks nothing here: it
        // breaks three clicks later, after the user named their file, and
        // leaves an empty file behind.
        let tampon = tampon_deborde();
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
            let mut sortie: Vec<u8> = Vec::new();
            let ecrit = export(
                &tampon,
                format,
                &mut sortie,
                &ExportOptions::default(),
                &CancelToken::new(),
            )
            .is_ok();
            assert_eq!(
                ecrit,
                is_supported(format),
                "{format} : `is_supported` dit {}, `export` dit {ecrit}",
                is_supported(format)
            );
        }
    }

    #[test]
    fn an_empty_result_produces_an_empty_file_not_an_error() {
        let tampon = ResultBuffer::new(schema(), 1 << 20);
        tampon.mark_complete(ExecStats::default());
        let mut sortie: Vec<u8> = Vec::new();

        let resume = export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export without error");

        assert_eq!(resume.rows, 0);
        assert_eq!(resume.batches, 0);
        assert!(sortie.is_empty());
    }
}
