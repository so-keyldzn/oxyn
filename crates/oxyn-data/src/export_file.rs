//! Export to a file the user named, without ever damaging the one that was
//! there.
//!
//! `File::create` truncates the destination when it opens it: an export
//! cancelled, or failing at its tenth batch, would destroy the document the
//! user agreed to overwrite — but only in exchange for the new one. The export
//! therefore writes to a temporary file **in the same folder**, and renames it
//! onto the destination only once everything is written: the rename, atomic
//! within one file system, leaves the destination either its old content or
//! the whole new one.
//!
//! **No symbolic link is followed.** The temporary file is created exclusively
//! (`O_EXCL`), and the rename replaces the destination's directory entry
//! without writing through it: a link placed there does not redirect the
//! export to its target.

use std::fs;
use std::io::BufWriter;
use std::path::Path;

use oxyn_core::{CancelToken, ExportFormat};
use tempfile::{Builder, NamedTempFile};

use crate::buffer::ResultBuffer;
use crate::error::{DataError, Result};
use crate::export::{ExportOptions, ExportSummary, ensure_exportable, export};

/// Exports `buffer` to `destination`, replacing the file there only if the
/// export succeeds entirely.
///
/// Blocks while writing: call it from the blocking pool. A replaced file keeps
/// its permissions, special bits aside; a read-only file is refused, as
/// `File::create` refused it. A new file is readable by its owner alone on
/// Unix, since it carries a database's data; elsewhere it inherits the
/// folder's access rights.
///
/// A process killed between writing and renaming leaves a
/// `.oxyn-export-*.part` in the folder: nothing cleans it up at startup.
///
/// # Errors
///
/// Those of [`export`]; the ones that depend on the result are checked before
/// any file is created. On an error or a cancellation, the destination keeps
/// its bytes and the temporary file is deleted.
pub fn export_to_path(
    buffer: &ResultBuffer,
    format: ExportFormat,
    destination: &Path,
    opts: &ExportOptions,
    ct: &CancelToken,
) -> Result<ExportSummary> {
    ensure_exportable(buffer, opts)?;
    replace_atomically(destination, ct, |file| {
        export(buffer, format, BufWriter::new(file), opts, ct)
    })
}

/// Writes through `write` into a temporary file next to `destination`, then
/// renames it onto it.
fn replace_atomically<T>(
    destination: &Path,
    ct: &CancelToken,
    write: impl FnOnce(&fs::File) -> Result<T>,
) -> Result<T> {
    if ct.is_cancelled() {
        return Err(DataError::Cancelled);
    }
    let folder = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let kept = match fs::symlink_metadata(destination) {
        Ok(existing) if existing.is_file() => {
            // What `File::create` refused, the rename would allow: it only
            // needs the right to write in the folder.
            if existing.permissions().readonly() {
                return Err(DataError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "the destination file is read-only",
                )));
            }
            Some(kept_permissions(&existing))
        }
        _ => None,
    };
    // Deleted when dropped: every error return below cleans it up.
    let temporary: NamedTempFile = Builder::new()
        .prefix(".oxyn-export-")
        .suffix(".part")
        .tempfile_in(folder)?;
    if let Some(permissions) = kept {
        temporary.as_file().set_permissions(permissions)?;
    }

    let written = write(temporary.as_file())?;
    // Without `sync_all`, a power cut right after the rename can leave an
    // empty destination: the rename would be durable before the data.
    temporary.as_file().sync_all()?;
    // The last point where a cancellation can still keep the old file.
    if ct.is_cancelled() {
        return Err(DataError::Cancelled);
    }
    temporary
        .persist(destination)
        .map_err(|failure| DataError::Io(failure.error))?;
    Ok(written)
}

/// The replaced file's permissions, without setuid, setgid or sticky: the new
/// file belongs to the user exporting, not to the old one's owner.
#[cfg(unix)]
fn kept_permissions(existing: &fs::Metadata) -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    fs::Permissions::from_mode(existing.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn kept_permissions(existing: &fs::Metadata) -> fs::Permissions {
    existing.permissions()
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;

    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use oxyn_core::ExecStats;

    use super::*;

    const EXISTING: &str = "EXISTING USER DOCUMENT\n";

    fn buffer(batches: usize) -> ResultBuffer {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
        let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
        for start in 0..batches {
            let start = i32::try_from(start).expect("a few test batches");
            buffer
                .push(
                    RecordBatch::try_new(
                        Arc::clone(&schema),
                        vec![Arc::new(Int32Array::from(vec![start, start + 100]))],
                    )
                    .expect("the column matches the schema"),
                )
                .expect("batch accepted");
        }
        buffer.mark_complete(ExecStats::default());
        buffer
    }

    /// A folder holding nothing but the existing document.
    fn folder_with_document() -> (tempfile::TempDir, std::path::PathBuf) {
        let folder = tempfile::tempdir().expect("temporary folder");
        let destination = folder.path().join("export.csv");
        fs::write(&destination, EXISTING).expect("existing document");
        (folder, destination)
    }

    /// What the folder holds: the temporary file must never stay there.
    fn entries(folder: &tempfile::TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder.path())
            .expect("readable folder")
            .map(|entry| {
                entry
                    .expect("readable entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// A writer that cancels after its first write: the export itself notices,
    /// between two batches.
    struct CancelAfterFirstWrite<'a> {
        file: &'a fs::File,
        ct: &'a CancelToken,
    }

    impl Write for CancelAfterFirstWrite<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let written = self.file.write(bytes)?;
            self.ct.cancel();
            Ok(written)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }

    #[test]
    fn a_nominal_export_replaces_the_document_and_leaves_no_temporary_file() {
        let (folder, destination) = folder_with_document();

        let summary = export_to_path(
            &buffer(2),
            ExportFormat::Csv,
            &destination,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export");

        assert_eq!(summary.rows, 4);
        assert_eq!(
            fs::read_to_string(&destination).expect("export reread"),
            "n\n0\n100\n1\n101\n"
        );
        assert_eq!(entries(&folder), ["export.csv"]);
    }

    #[test]
    fn an_export_cancelled_before_it_starts_touches_nothing() {
        let (folder, destination) = folder_with_document();
        let ct = CancelToken::new();
        ct.cancel();

        let outcome = export_to_path(
            &buffer(2),
            ExportFormat::Csv,
            &destination,
            &ExportOptions::default(),
            &ct,
        );

        assert!(matches!(outcome, Err(DataError::Cancelled)), "{outcome:?}");
        assert_eq!(fs::read_to_string(&destination).expect("reread"), EXISTING);
        assert_eq!(entries(&folder), ["export.csv"]);
    }

    #[test]
    fn an_export_cancelled_midway_keeps_the_document() {
        let (folder, destination) = folder_with_document();
        let ct = CancelToken::new();
        let rows = buffer(3);

        let outcome = replace_atomically(&destination, &ct, |file| {
            export(
                &rows,
                ExportFormat::Csv,
                CancelAfterFirstWrite { file, ct: &ct },
                &ExportOptions::default(),
                &ct,
            )
        });

        assert!(matches!(outcome, Err(DataError::Cancelled)), "{outcome:?}");
        assert_eq!(fs::read_to_string(&destination).expect("reread"), EXISTING);
        assert_eq!(entries(&folder), ["export.csv"]);
    }

    #[test]
    fn an_encoding_error_midway_keeps_the_document() {
        let (folder, destination) = folder_with_document();

        let outcome: Result<()> =
            replace_atomically(&destination, &CancelToken::new(), |mut file| {
                file.write_all(b"n\n0\n")?;
                Err(DataError::Arrow(arrow::error::ArrowError::CsvError(
                    "encoding failed on the second batch".into(),
                )))
            });

        assert!(matches!(outcome, Err(DataError::Arrow(_))), "{outcome:?}");
        assert_eq!(fs::read_to_string(&destination).expect("reread"), EXISTING);
        assert_eq!(entries(&folder), ["export.csv"]);
    }

    #[test]
    fn a_truncated_result_is_refused_before_any_file_is_created() {
        let (folder, destination) = folder_with_document();
        let rows = buffer(1);
        rows.mark_truncated();

        let outcome = export_to_path(
            &rows,
            ExportFormat::Csv,
            &destination,
            &ExportOptions::default(),
            &CancelToken::new(),
        );

        assert!(
            matches!(outcome, Err(DataError::TruncatedResult)),
            "{outcome:?}"
        );
        assert_eq!(fs::read_to_string(&destination).expect("reread"), EXISTING);
        assert_eq!(entries(&folder), ["export.csv"]);
    }

    #[test]
    fn a_missing_folder_fails_without_creating_anything() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let destination = folder.path().join("missing").join("export.csv");

        let outcome = export_to_path(
            &buffer(1),
            ExportFormat::Csv,
            &destination,
            &ExportOptions::default(),
            &CancelToken::new(),
        );

        assert!(matches!(outcome, Err(DataError::Io(_))), "{outcome:?}");
        assert!(entries(&folder).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (_folder, destination) = folder_with_document();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o640))
            .expect("permissions set");

        export_to_path(
            &buffer(1),
            ExportFormat::Csv,
            &destination,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export");

        let mode = fs::metadata(&destination)
            .expect("reread")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    /// The rename would overwrite a write-protected file: refused, as
    /// `File::create` refused it, leaving nothing in the folder.
    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_refused_and_kept() {
        use std::os::unix::fs::PermissionsExt;

        let (folder, destination) = folder_with_document();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o444))
            .expect("permissions set");

        let outcome = export_to_path(
            &buffer(1),
            ExportFormat::Csv,
            &destination,
            &ExportOptions::default(),
            &CancelToken::new(),
        );

        assert!(
            matches!(&outcome, Err(DataError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied),
            "{outcome:?}"
        );
        assert_eq!(fs::read_to_string(&destination).expect("reread"), EXISTING);
        assert_eq!(entries(&folder), ["export.csv"]);
    }

    /// A link at the destination does not redirect the export to its target:
    /// the folder entry is what gets replaced.
    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_at_the_destination_is_not_followed() {
        let (folder, target) = folder_with_document();
        let link = folder.path().join("link.csv");
        std::os::unix::fs::symlink(&target, &link).expect("symbolic link");

        export_to_path(
            &buffer(1),
            ExportFormat::Csv,
            &link,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export");

        assert_eq!(
            fs::read_to_string(&target).expect("target reread"),
            EXISTING
        );
        let entry = fs::symlink_metadata(&link).expect("destination reread");
        assert!(
            entry.is_file(),
            "the destination is a file, no longer a link"
        );
        assert_eq!(entries(&folder), ["export.csv", "link.csv"]);
    }
}
