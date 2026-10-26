use std::path::PathBuf;

use oxyn_core::DriverId;

use super::*;
use crate::ipc::FormField;

fn sqlite_driver() -> DriverChoice {
    DriverChoice {
        id: DriverId::sqlite().as_str().to_owned(),
        display_name: "SQLite".to_owned(),
        family: "relational".to_owned(),
        default_port: None,
        aliases: Vec::new(),
        fields: vec![FormField {
            key: "path".to_owned(),
            label: "Database file".to_owned(),
            kind: FormFieldKind::Path,
            required: true,
            secret: false,
            default: None,
            help: None,
        }],
    }
}

fn duckdb_driver() -> DriverChoice {
    DriverChoice {
        id: DriverId::duckdb().as_str().to_owned(),
        display_name: "DuckDB".to_owned(),
        family: "analytical".to_owned(),
        default_port: None,
        aliases: Vec::new(),
        fields: vec![FormField {
            key: "path".to_owned(),
            label: "Database file".to_owned(),
            kind: FormFieldKind::Path,
            required: true,
            secret: false,
            default: None,
            help: None,
        }],
    }
}

fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("the temporary directory is writable");
    path
}

fn reason(file: DroppedFile) -> String {
    match file {
        DroppedFile::Refused { reason, .. } => reason,
        DroppedFile::Sql { name, .. } | DroppedFile::Database { name, .. } => {
            panic!("{name} was accepted")
        }
    }
}

#[test]
fn a_sql_file_is_read_and_never_more() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(&dir, "report.SQL", b"DELETE FROM audit;\n");
    match classify(&path, &[]) {
        DroppedFile::Sql { name, text } => {
            assert_eq!(name, "report.SQL");
            assert_eq!(text, "DELETE FROM audit;\n");
        }
        _ => panic!("a .sql file opens in a console"),
    }
}

#[test]
fn a_sql_file_past_the_bound_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let size = usize::try_from(MAX_SQL_BYTES + 1).expect("4 MiB fits a usize");
    let path = write(&dir, "dump.sql", &vec![b' '; size]);
    assert!(reason(classify(&path, &[])).contains("larger than 4 MiB"));
}

#[test]
fn a_sql_file_at_the_bound_is_opened() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let size = usize::try_from(MAX_SQL_BYTES).expect("4 MiB fits a usize");
    let path = write(&dir, "big.sql", &vec![b' '; size]);
    assert!(matches!(classify(&path, &[]), DroppedFile::Sql { .. }));
}

#[test]
fn a_sql_file_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(&dir, "latin1.sql", b"SELECT '\xe9t\xe9';");
    assert_eq!(reason(classify(&path, &[])), "It is not UTF-8 text.");
}

#[test]
fn a_missing_file_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("gone.sql");
    assert!(reason(classify(&path, &[])).starts_with("It cannot be read"));
}

#[test]
fn a_directory_is_refused_whatever_its_name() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("queries.sql");
    std::fs::create_dir(&path).expect("a directory can be created");
    assert_eq!(reason(classify(&path, &[])), "It is not a file.");
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_refused_and_its_target_not_read() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let secret = write(&dir, "pgpass", b"host:5432:db:user:hunter2\n");
    let link = dir.path().join("notes.sql");
    std::os::unix::fs::symlink(&secret, &link).expect("a link can be created");
    let file = classify(&link, &[]);
    assert!(reason(file).contains("symbolic link"));
}

#[cfg(unix)]
#[test]
fn a_file_swapped_after_the_check_is_not_read() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let dropped = write(&dir, "notes.sql", b"SELECT 1;");
    let checked = std::fs::symlink_metadata(&dropped).expect("the file exists");
    // Between the check and the open, the name now leads elsewhere.
    let secret = write(&dir, "pgpass", b"host:5432:db:user:hunter2\n");
    std::fs::remove_file(&dropped).expect("the file can be removed");
    std::os::unix::fs::symlink(&secret, &dropped).expect("a link can be created");
    assert_eq!(
        read_sql(&dropped, &checked),
        Err("It changed while it was being opened.".to_owned())
    );
}

#[test]
fn an_unknown_extension_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(&dir, "notes.txt", b"SELECT 1;");
    assert!(reason(classify(&path, &[sqlite_driver()])).contains(".sql files"));
    let bare = write(&dir, "Makefile", b"all:");
    assert!(matches!(
        classify(&bare, &[sqlite_driver()]),
        DroppedFile::Refused { .. }
    ));
}

#[test]
fn a_database_file_is_offered_to_the_driver_that_reads_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(&dir, "scratch.sqlite", b"SQLite format 3\0");
    match classify(&path, &[sqlite_driver()]) {
        DroppedFile::Database {
            name,
            driver,
            field,
            path: offered,
        } => {
            assert_eq!(name, "scratch.sqlite");
            assert_eq!(driver, "sqlite");
            assert_eq!(field, "path");
            assert_eq!(offered, path.display().to_string());
        }
        _ => panic!("a .sqlite file is offered as a connection"),
    }
}

#[test]
fn a_database_file_no_registered_driver_reads_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(&dir, "analytics.duckdb", b"");
    assert_eq!(
        reason(classify(&path, &[sqlite_driver()])),
        "No driver in this build reads .duckdb files."
    );
    let sqlite = write(&dir, "scratch.sqlite3", b"");
    assert!(matches!(
        classify(&sqlite, &[]),
        DroppedFile::Refused { .. }
    ));
}

#[test]
fn a_duckdb_file_is_offered_to_the_duckdb_driver() {
    let dir = tempfile::tempdir().expect("the temporary directory is writable");
    let path = write(&dir, "analytics.duckdb", b"");
    match classify(&path, &[duckdb_driver()]) {
        DroppedFile::Database {
            driver,
            field,
            path: offered,
            ..
        } => {
            assert_eq!(driver, "duckdb");
            assert_eq!(field, "path");
            assert_eq!(offered, path.display().to_string());
        }
        _ => panic!("a DuckDB file is offered as a connection"),
    }
}

#[test]
fn one_drop_opens_a_bounded_number_of_files() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = write(&dir, "q.sql", b"SELECT 1;");
    let paths = vec![path; MAX_FILES_PER_DROP + 3];
    let files = classify_all(&paths, &[]);
    assert_eq!(files.len(), MAX_FILES_PER_DROP + 1);
    match files.last() {
        Some(DroppedFile::Refused { name, .. }) => assert_eq!(name, "3 more files"),
        _ => panic!("the files past the bound are refused together"),
    }
}
