//! Files dropped on the window from the system
//! ([ADR-0041](../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 9 ; UX-SPEC « Souris et glisser »).
//!
//! A dropped path is **untrusted input** ([SECURITY](../../../docs/SECURITY.md#surface-dentrée)):
//! whatever the user dragged, or whatever a page they dragged from put on
//! the pasteboard. So it is classified here, in Rust, and the front only ever
//! receives what the classification decided:
//!
//! - a `.sql` file is read — bounded, strict UTF-8, on the blocking pool —
//!   and opened in a new console. Nothing runs it: `Run` does, once the user
//!   has read it;
//! - a database file that a registered driver reads is *offered* as a
//!   connection: the connection screen opens with the file in the driver's
//!   path field, and nothing is created until the user submits it, in
//!   `production` like any new connection (I-02);
//! - anything else is refused with a reason.
//!
//! A symbolic link is refused rather than followed: its name's extension says
//! nothing of the file it points to, and « open the `.sql` you dropped »
//! must not read `~/.pgpass` because a link was called `notes.sql`.
//!
//! Nothing here emits a `Command`: this is interface plumbing like
//! `subscribe_menu`, and what it produces reaches data only through the
//! console's `Run` or the connection form's submit, which do (I-01).

use std::collections::HashMap;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use parking_lot::Mutex;
use tauri::ipc::Channel;
use tauri::{DragDropEvent, Manager as _, Window, WindowEvent};

use crate::backend::Backend;
use crate::ipc::file_drops::DroppedFile;
use crate::ipc::{DriverChoice, FormFieldKind};

/// The largest `.sql` file opened in a console: 4 MiB.
///
/// A console is a CodeMirror document in the webview, sent there whole in one
/// channel message and autosaved as a draft (ADR-0024). A file past this is a
/// dump, which a console is not the tool for; refusing says so, where
/// accepting would freeze the window for a file nobody meant to edit there.
pub(crate) const MAX_SQL_BYTES: u64 = 4 * 1024 * 1024;

/// The files one drop may carry. Each is announced separately; past this, the
/// rest are refused in one message rather than opened as fifty consoles.
pub(crate) const MAX_FILES_PER_DROP: usize = 16;

/// Extensions a driver may read, and the driver id that reads them. A file is
/// offered only when that driver is registered in this build.
const DATABASE_FILES: &[(&str, &str)] = &[
    ("sqlite", "sqlite"),
    ("sqlite3", "sqlite"),
    ("duckdb", "duckdb"),
];

/// What the application manages: each window's channel, by label.
#[derive(Default)]
pub struct FileDrops {
    channels: Mutex<HashMap<String, Channel<DroppedFile>>>,
}

impl FileDrops {
    /// A window's channel, replacing its previous one: a reloaded page
    /// subscribes again.
    pub fn subscribe(&self, window: &str, channel: Channel<DroppedFile>) {
        self.channels.lock().insert(window.to_owned(), channel);
    }

    /// A closed window: its channel goes.
    pub fn forget(&self, window: &str) {
        self.channels.lock().remove(window);
    }

    /// Sends a classified file to the window it was dropped on, and to it
    /// alone (ADR-0043).
    fn deliver(&self, window: &str, file: DroppedFile) {
        let Some(channel) = self.channels.lock().get(window).cloned() else {
            tracing::debug!("a file was dropped before its window listened");
            return;
        };
        if let Err(error) = channel.send(file) {
            tracing::warn!(%error, "could not hand a dropped file to the front");
        }
    }
}

/// The window event handler: a drop on one of Oxyn's windows is classified
/// on the blocking pool, never on the main thread that delivered it (I-05),
/// and handed to that window.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event else {
        return;
    };
    let app = window.app_handle().clone();
    if paths.is_empty()
        || app
            .state::<Backend>()
            .inner
            .windows
            .key_of(window.label())
            .is_err()
    {
        return;
    }
    let label = window.label().to_owned();
    let paths = paths.clone();
    // Not held: a bounded read that ends by itself and answers through the
    // channel, with nothing a caller could cancel or wait for.
    tauri::async_runtime::spawn_blocking(move || {
        let drivers = app.state::<Backend>().driver_choices();
        let drops = app.state::<FileDrops>();
        for file in classify_all(&paths, &drivers) {
            drops.deliver(&label, file);
        }
    });
}

/// Every path of one drop, in order, at most [`MAX_FILES_PER_DROP`] opened.
pub(crate) fn classify_all(
    paths: &[std::path::PathBuf],
    drivers: &[DriverChoice],
) -> Vec<DroppedFile> {
    let mut files: Vec<DroppedFile> = paths
        .iter()
        .take(MAX_FILES_PER_DROP)
        .map(|path| classify(path, drivers))
        .collect();
    let extra = paths.len().saturating_sub(MAX_FILES_PER_DROP);
    if extra > 0 {
        files.push(DroppedFile::Refused {
            name: format!("{extra} more files"),
            reason: format!("At most {MAX_FILES_PER_DROP} files are opened from one drop."),
        });
    }
    files
}

/// What one dropped path becomes. Never panics, whatever the path is.
pub(crate) fn classify(path: &Path, drivers: &[DriverChoice]) -> DroppedFile {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let refused = |reason: String| DroppedFile::Refused {
        name: name.clone(),
        reason,
    };
    // `symlink_metadata`, not `metadata`: a link is judged as a link.
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return refused(format!("It cannot be read: {error}.")),
    };
    if metadata.file_type().is_symlink() {
        return refused("It is a symbolic link. Drop the file it points to.".to_owned());
    }
    if !metadata.is_file() {
        return refused("It is not a file.".to_owned());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if extension == "sql" {
        return match read_sql(path, &metadata) {
            Ok(text) => DroppedFile::Sql { name, text },
            Err(reason) => refused(reason),
        };
    }
    let Some(&(_, driver)) = DATABASE_FILES.iter().find(|(known, _)| *known == extension) else {
        return refused(
            "Oxyn opens .sql files in a console, and .sqlite or .duckdb files as connections."
                .to_owned(),
        );
    };
    let field = drivers
        .iter()
        .find(|choice| choice.id == driver)
        .and_then(|choice| {
            choice
                .fields
                .iter()
                .find(|field| matches!(field.kind, FormFieldKind::Path))
        });
    let Some(field) = field else {
        return refused(format!("No driver in this build reads .{extension} files."));
    };
    // `display()` would replace what is not UTF-8, and the form would then
    // hold the path of another file, or of none.
    let Some(text) = path.to_str() else {
        return refused("Its path is not UTF-8 text.".to_owned());
    };
    DroppedFile::Database {
        name,
        driver: driver.to_owned(),
        field: field.key.clone(),
        path: text.to_owned(),
    }
}

/// The text of a `.sql` file, bounded by [`MAX_SQL_BYTES`] **as read**: the
/// size the metadata gave may have grown since.
///
/// `checked` is what `symlink_metadata` saw. The file opened is compared with
/// it, on the descriptor: a link or another file swapped in under the same
/// name between the check and the open is a different file, and is refused
/// rather than read. What remains is a FIFO swapped in at that instant, on
/// which `open` waits for a writer: it holds one thread of the blocking pool,
/// never the window, and the descriptor check refuses it once opened.
fn read_sql(path: &Path, checked: &std::fs::Metadata) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("It cannot be read: {error}."))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("It cannot be read: {error}."))?;
    if !opened.is_file() || !same_file(checked, &opened) {
        return Err("It changed while it was being opened.".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SQL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("It cannot be read: {error}."))?;
    if u64::try_from(bytes.len()).map_or(true, |read| read > MAX_SQL_BYTES) {
        return Err(format!(
            "It is larger than {} MiB, which a console does not open.",
            MAX_SQL_BYTES / (1024 * 1024)
        ));
    }
    // Strict: a replacement character in a statement is a different
    // statement, and nothing would show where it came from.
    String::from_utf8(bytes).map_err(|_| "It is not UTF-8 text.".to_owned())
}

/// Whether two metadata describe the same file: same device, same inode.
#[cfg(unix)]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    left.dev() == right.dev() && left.ino() == right.ino()
}

/// Windows exposes no stable file identity in `std` (`file_index` is still
/// unstable): the check stops at `is_file` on the descriptor, which a link
/// swapped in to another regular file passes.
#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
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
}
