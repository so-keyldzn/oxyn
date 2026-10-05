//! The update preference: one open JSON file for the whole application, not
//! per workspace ([I-11](../../../../CLAUDE.md#i-11)), and the lock an
//! administrator sets through the environment.
//!
//! `app_config_dir()/updates.json`:
//!
//! ```json
//! {"format":1,"automatic":true}
//! ```

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub(crate) const FILE: &str = "updates.json";

/// The variable that turns updates off on a managed or offline workstation.
pub(crate) const LOCK_VARIABLE: &str = "OXYN_UPDATES";

const FORMAT: u32 = 1;

/// The file is a few bytes; anything larger was not written by Oxyn.
const LONGEST: u64 = 64 * 1024;

/// What is stored. Unknown fields are ignored: a later format may add some
/// without an older Oxyn losing the preference.
#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    format: u32,
    automatic: bool,
}

/// Whether `OXYN_UPDATES` turns updates off. Read once at launch and passed
/// in, so tests never touch the process environment.
pub(crate) fn locked(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("off"))
}

/// The preference `bytes` hold: automatic when the file is missing, and when
/// it cannot be read — a corrupt file must not silently stop updates.
pub(crate) fn parse(bytes: Option<&[u8]>) -> bool {
    let Some(bytes) = bytes else {
        return true;
    };
    match serde_json::from_slice::<Stored>(bytes) {
        Ok(stored) => {
            if stored.format != FORMAT {
                tracing::warn!(
                    format = stored.format,
                    "update preference written by another version of Oxyn; reading it as format 1"
                );
            }
            stored.automatic
        }
        Err(error) => {
            tracing::warn!(%error, "unreadable update preference; automatic updates stay on");
            true
        }
    }
}

pub(crate) fn serialize(automatic: bool) -> Vec<u8> {
    let stored = Stored {
        format: FORMAT,
        automatic,
    };
    // A struct of a number and a boolean always serializes.
    serde_json::to_vec(&stored).unwrap_or_default()
}

/// Reads the preference in `folder`. Blocking: launch, or the blocking pool.
pub(crate) fn read(folder: &Path) -> bool {
    match read_bounded(&folder.join(FILE)) {
        Ok(bytes) => parse(bytes.as_deref()),
        Err(error) => {
            tracing::warn!(%error, "update preference not read; automatic updates stay on");
            true
        }
    }
}

/// Writes the preference in `folder`, through a temporary file renamed onto
/// it: a crash leaves the old preference or the new one, never half.
///
/// # Errors
/// The folder or the file cannot be written.
pub(crate) fn write(folder: &Path, automatic: bool) -> io::Result<()> {
    write_atomically(folder, FILE, &serialize(automatic))
}

/// The bytes of `path`, `None` when it does not exist.
pub(crate) fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(LONGEST).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Writes `bytes` to `folder/name` through a temporary file of this write's
/// own: named after the process and a counter, and created exclusively, so a
/// concurrent write neither truncates, renames nor removes it. A failure
/// removes only that file and leaves `name` as it was.
pub(crate) fn write_atomically(folder: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    static WRITES: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(folder)?;
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    let temporary = folder.join(format!(".{name}.{}.{write}.part", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)
        .and_then(|()| {
            drop(file);
            std::fs::rename(&temporary, folder.join(name))
        })
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
}
