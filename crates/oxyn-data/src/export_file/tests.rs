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

    let outcome: Result<()> = replace_atomically(&destination, &CancelToken::new(), |mut file| {
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
fn duplicate_column_names_refuse_json_before_any_file_is_created() {
    // `SELECT 1 AS id, 2 AS id`: a JSON object would keep one of the two
    // (issue #183). The refusal comes before the destination is touched.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("id", DataType::Int32, false),
    ]));
    let rows = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
    rows.push(
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1])),
                Arc::new(Int32Array::from(vec![2])),
            ],
        )
        .expect("the columns match the schema"),
    )
    .expect("batch accepted");
    rows.mark_complete(ExecStats::default());

    for format in [ExportFormat::Json, ExportFormat::JsonLines] {
        let (folder, destination) = folder_with_document();
        let outcome = export_to_path(
            &rows,
            format,
            &destination,
            &ExportOptions::default(),
            &CancelToken::new(),
        );

        assert!(
            matches!(&outcome, Err(DataError::DuplicateColumnNames { names, .. }) if names == &["id"]),
            "{format}: {outcome:?}"
        );
        assert_eq!(fs::read_to_string(&destination).expect("reread"), EXISTING);
        assert_eq!(entries(&folder), ["export.csv"]);
    }
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
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o640)).expect("permissions set");

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
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o444)).expect("permissions set");

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
