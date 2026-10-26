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
    ensure_exportable(buffer, format, opts)?;
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
mod tests;
