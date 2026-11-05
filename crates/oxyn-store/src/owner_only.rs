//! Owner-only permissions for the state file.
//!
//! The store holds the query history, the audit journal's statements, the
//! assistant's transcripts and connection parameters. SQLite creates its file
//! with 0644 and `create_dir_all` honours the umask (0755 with the usual 022):
//! on a Linux home directory that is itself traversable, any other local
//! account could read all of it. The log directory is already 0700/0600 for the
//! weaker reason that "a journal names connections and paths"; the state holds
//! strictly more.
//!
//! SQLite derives the mode of `-wal`, `-shm` and the rollback journal from the
//! main database file, so creating that file with 0600 before SQLite opens it
//! is enough for the siblings. On Windows the per-user profile is the boundary
//! and these functions do nothing.

use std::io;
use std::path::Path;

/// Suffixes of the files SQLite keeps next to the database in WAL mode.
#[cfg(unix)]
const SIBLING_SUFFIXES: [&str; 2] = ["-wal", "-shm"];

/// Creates the **leaf** directory with mode 0700 if it is missing.
///
/// Only the leaf is Oxyn's: its ancestors (`~/.local/share`) are created with
/// the default mode and never tightened.
pub(crate) fn create_directory(directory: &Path) -> io::Result<()> {
    if let Some(grandparent) = directory.parent()
        && !grandparent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(grandparent)?;
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && directory.is_dir() => Ok(()),
        Err(error) => Err(error),
    }
}

/// Makes sure the database file exists with mode 0600 before SQLite opens it,
/// and tightens an existing file and its `-wal`/`-shm` siblings.
///
/// Tightening an existing file is best effort: a store we cannot chmod (owned
/// by someone else) is still usable, so the failure is logged, not fatal.
/// Creating it is not best effort: an error there is returned.
pub(crate) fn prepare_database_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        drop(options.open(path)?);
        tighten(path, 0o600);
        for suffix in SIBLING_SUFFIXES {
            let mut sibling = path.as_os_str().to_owned();
            sibling.push(suffix);
            let sibling = Path::new(&sibling);
            if sibling.exists() {
                tighten(sibling, 0o600);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Restricts an existing directory Oxyn owns — the default data directory —
/// to 0700. Not applied to a caller-chosen path: its parent may be shared.
pub(crate) fn tighten_directory(directory: &Path) {
    #[cfg(unix)]
    tighten(directory, 0o700);
    #[cfg(not(unix))]
    let _ = directory;
}

#[cfg(unix)]
fn tighten(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;

    let current = match std::fs::metadata(path) {
        Ok(metadata) => metadata.permissions().mode() & 0o777,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "cannot read state permissions");
            return;
        }
    };
    if current == mode {
        return;
    }
    if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
        tracing::warn!(%error, path = %path.display(), "cannot restrict state permissions");
    }
}
