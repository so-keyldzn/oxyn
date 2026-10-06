//! The user's agents: the `*.md` files of a directory, read as input.
//!
//! A file here was written by the user, or by whoever could write into their
//! data directory: it is read like a server response
//! ([SECURITY](../../../../docs/SECURITY.md#input-surface), "Workspace
//! files"). Nothing in it is followed, nothing in it is trusted to be small,
//! and nothing of it but its name reaches an error
//! ([I-03](../../../../CLAUDE.md#i-03), [I-09](../../../../CLAUDE.md#i-09)).

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use super::catalog::UserAgentFile;
use super::parse::parse_agent_file;
use super::{AgentFileError, MAX_FILE_BYTES};

/// Most agent files read from the directory.
///
/// A picker longer than this is no longer read by anyone, and the bound keeps
/// a launch to at most 64 × 64 KiB = 4 MiB of reading whatever the directory
/// holds. Files past it, in name order, are listed as one error entry: a
/// missing agent is then explained, not silently dropped.
pub const MAX_USER_AGENT_FILES: usize = 64;

/// Most directory entries looked through, `*.md` or not.
///
/// The files are sorted by name before the [`MAX_USER_AGENT_FILES`] cut, so
/// every name is held at once; this bounds that list. Sixteen times the file
/// bound leaves room for an editor's backups and a `README` next to the
/// agents, and none for a directory that is something else.
pub const MAX_DIRECTORY_ENTRIES: usize = 16 * MAX_USER_AGENT_FILES;

/// Reads the agent files of `dir`, in name order.
///
/// Reads the `*.md` files directly in `dir`, not in its subdirectories, and
/// skips names starting with `.` (an editor's swap file, macOS's `._` files).
/// Each file becomes one [`UserAgentFile`]: its declaration, or why it was
/// refused — not a regular file (a symbolic link is refused, not followed),
/// over [`MAX_FILE_BYTES`] (checked on its size and again while reading, since
/// it can grow in between), not UTF-8, or refused by [`parse_agent_file`].
///
/// A missing `dir` is an empty list: a user who has written no agent has no
/// directory. An unreadable one is a single error entry named after it.
///
/// Blocks on the file system: call it off the interface thread
/// ([I-05](../../../../CLAUDE.md#i-05)). Never panics, whatever the files
/// hold. Allocates the names of at most [`MAX_DIRECTORY_ENTRIES`] entries and
/// the text of one file at a time.
#[must_use]
pub fn read_user_agents(dir: &Path) -> Vec<UserAgentFile> {
    let listing = match list(dir) {
        Listing::Missing => return Vec::new(),
        Listing::Unreadable => {
            return vec![UserAgentFile {
                file_name: directory_label(dir),
                result: Err(AgentFileError::Unreadable {
                    file: directory_label(dir),
                }),
            }];
        }
        Listing::Names(listing) => listing,
    };

    let mut names = listing.names.into_iter();
    let mut files: Vec<UserAgentFile> = names
        .by_ref()
        .take(MAX_USER_AGENT_FILES)
        .map(|name| read_one(dir, &name))
        .collect();

    let mut unread = names;
    if let Some(first) = unread.next() {
        let file = display_name(&first);
        files.push(UserAgentFile {
            file_name: file.clone(),
            result: Err(AgentFileError::TooManyFiles {
                file,
                skipped: unread.count(),
            }),
        });
    }
    if listing.truncated || listing.unreadable_entry {
        let file = directory_label(dir);
        let error = if listing.truncated {
            AgentFileError::TooManyEntries { file: file.clone() }
        } else {
            AgentFileError::Unreadable { file: file.clone() }
        };
        files.push(UserAgentFile {
            file_name: file,
            result: Err(error),
        });
    }
    files
}

/// What the directory holds, before any file is opened.
enum Listing {
    Missing,
    Unreadable,
    Names(Names),
}

/// The candidate names, sorted.
struct Names {
    names: Vec<OsString>,
    /// The directory held more than [`MAX_DIRECTORY_ENTRIES`] entries.
    truncated: bool,
    /// An entry could not be read; the others were.
    unreadable_entry: bool,
}

fn list(dir: &Path) -> Listing {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Listing::Missing,
        Err(_) => return Listing::Unreadable,
    };
    let mut names = Vec::new();
    let mut truncated = false;
    let mut unreadable_entry = false;
    for (seen, entry) in entries.enumerate() {
        if seen >= MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            unreadable_entry = true;
            continue;
        };
        let name = entry.file_name();
        if is_agent_file_name(&name) {
            names.push(name);
        }
    }
    // Byte order of the name: the same on every launch and every machine,
    // whatever order the file system lists in.
    names.sort_unstable();
    Listing::Names(Names {
        names,
        truncated,
        unreadable_entry,
    })
}

fn is_agent_file_name(name: &OsStr) -> bool {
    let hidden = name.as_encoded_bytes().first() == Some(&b'.');
    !hidden && Path::new(name).extension() == Some(OsStr::new("md"))
}

fn read_one(dir: &Path, name: &OsStr) -> UserAgentFile {
    let Some(file_name) = name.to_str() else {
        let file = display_name(name);
        return UserAgentFile {
            file_name: file.clone(),
            result: Err(AgentFileError::FileNameNotUtf8 { file }),
        };
    };
    let result =
        read_text(&dir.join(name), file_name).and_then(|text| parse_agent_file(file_name, &text));
    UserAgentFile {
        file_name: file_name.to_owned(),
        result,
    }
}

/// The file's text, if it is a regular UTF-8 file within the size cap.
fn read_text(path: &Path, file_name: &str) -> Result<String, AgentFileError> {
    let unreadable = || AgentFileError::Unreadable {
        file: file_name.to_owned(),
    };
    let not_regular = || AgentFileError::NotARegularFile {
        file: file_name.to_owned(),
    };

    // `symlink_metadata` does not follow: a link is seen as a link.
    let listed = fs::symlink_metadata(path).map_err(|_| unreadable())?;
    if !listed.file_type().is_file() {
        return Err(not_regular());
    }
    if listed.len() > u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX) {
        return Err(AgentFileError::TooLarge {
            file: file_name.to_owned(),
            bytes: usize::try_from(listed.len()).unwrap_or(usize::MAX),
        });
    }

    // The name can be swapped for a link or a FIFO between the check above
    // and the open: the open itself refuses to follow, and never waits.
    let file = open_no_follow(path).map_err(|_| match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => not_regular(),
        _ => unreadable(),
    })?;
    let opened = file.metadata().map_err(|_| unreadable())?;
    if !opened.file_type().is_file() {
        return Err(not_regular());
    }

    let bytes = read_bounded(file, file_name)?;
    String::from_utf8(bytes).map_err(|_| AgentFileError::NotUtf8 {
        file: file_name.to_owned(),
    })
}

/// Reads at most [`MAX_FILE_BYTES`] + 1 bytes: one more than allowed is
/// enough to know the file is too large, whatever its size said before.
pub(super) fn read_bounded(reader: impl Read, file_name: &str) -> Result<Vec<u8>, AgentFileError> {
    let limit = u64::try_from(MAX_FILE_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| AgentFileError::Unreadable {
            file: file_name.to_owned(),
        })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AgentFileError::TooLarge {
            file: file_name.to_owned(),
            bytes: bytes.len(),
        });
    }
    Ok(bytes)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    // `NONBLOCK`: a FIFO put in place of the file would otherwise hold the
    // open until a writer comes. It changes nothing for a regular file.
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    Ok(File::from(fd))
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> io::Result<File> {
    File::open(path)
}

fn display_name(name: &OsStr) -> String {
    name.to_string_lossy().into_owned()
}

/// What an error about the directory itself names: its last component,
/// never the full path, which holds the user's account name.
fn directory_label(dir: &Path) -> String {
    dir.file_name()
        .map_or_else(|| "agents".to_owned(), display_name)
}
