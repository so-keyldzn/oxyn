//! The user's agents: the `*.md` files of a directory, read as input.
//!
//! A file here was written by the user, or by whoever could write into their
//! data directory: it is read like a server response
//! ([SECURITY](../../../../docs/SECURITY.md#input-surface), "Workspace
//! files"). Nothing in it is followed, nothing in it is trusted to be small,
//! and nothing of it but its name reaches an error
//! ([I-03](../../../../CLAUDE.md#i-03), [I-09](../../../../CLAUDE.md#i-09)).

use std::ffi::{OsStr, OsString};
use std::fs::File;
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
/// `dir` itself must be a directory and not a symbolic link to one: on macOS
/// and Linux it is opened once without following a link, and every file is
/// then looked up in what was opened, so swapping the directory for a link
/// halfway through reads nothing from the link.
///
/// Each file becomes one [`UserAgentFile`]: its declaration, or why it was
/// refused — not a regular file (a symbolic link is refused, not followed),
/// over [`MAX_FILE_BYTES`] (checked on its size and again while reading, since
/// it can grow in between), not UTF-8, or refused by [`parse_agent_file`].
/// A missing `dir` is an empty list: a user who has written no agent has no
/// directory. A `dir` that cannot be read is a single error entry named after
/// its last component, never its full path, which holds the account name.
///
/// Blocks on the file system: call it off the interface thread
/// ([I-05](../../../../CLAUDE.md#i-05)). Never panics, whatever the files
/// hold. Allocates the names of at most [`MAX_DIRECTORY_ENTRIES`] entries and
/// the text of one file at a time.
#[must_use]
pub fn read_user_agents(dir: &Path) -> Vec<UserAgentFile> {
    let label = directory_label(dir);
    let directory = match Directory::open(dir) {
        Opened::Missing => return Vec::new(),
        Opened::Refused => {
            return vec![refused(
                &label,
                AgentFileError::NotADirectory {
                    file: label.clone(),
                },
            )];
        }
        Opened::Unreadable => {
            return vec![refused(
                &label,
                AgentFileError::Unreadable {
                    file: label.clone(),
                },
            )];
        }
        Opened::Directory(directory) => directory,
    };
    let listing = directory.names();

    let mut names = listing.names.into_iter();
    let mut files: Vec<UserAgentFile> = names
        .by_ref()
        .take(MAX_USER_AGENT_FILES)
        .map(|name| read_one(&directory, &name))
        .collect();

    let mut unread = names;
    if let Some(first) = unread.next() {
        let file = display_name(&first);
        files.push(refused(
            &file,
            AgentFileError::TooManyFiles {
                file: file.clone(),
                skipped: unread.count(),
            },
        ));
    }
    if listing.truncated {
        files.push(refused(
            &label,
            AgentFileError::TooManyEntries {
                file: label.clone(),
            },
        ));
    } else if listing.unreadable_entry {
        files.push(refused(
            &label,
            AgentFileError::Unreadable {
                file: label.clone(),
            },
        ));
    }
    files
}

/// The entry of a file, or of the directory, that could not be read.
fn refused(file: &str, error: AgentFileError) -> UserAgentFile {
    UserAgentFile {
        file_name: file.to_owned(),
        result: Err(error),
    }
}

/// The candidate names, sorted.
struct Names {
    names: Vec<OsString>,
    /// The directory held more than [`MAX_DIRECTORY_ENTRIES`] entries.
    truncated: bool,
    /// An entry could not be read; the others were.
    unreadable_entry: bool,
}

impl Names {
    /// Keeps the agent file names of `entries`, at most
    /// [`MAX_DIRECTORY_ENTRIES`] of them looked at, sorted.
    fn collect(entries: impl Iterator<Item = io::Result<OsString>>) -> Self {
        let mut names = Vec::new();
        let mut truncated = false;
        let mut unreadable_entry = false;
        for (seen, entry) in entries.enumerate() {
            if seen >= MAX_DIRECTORY_ENTRIES {
                truncated = true;
                break;
            }
            match entry {
                Ok(name) if is_agent_file_name(&name) => names.push(name),
                Ok(_) => {}
                Err(_) => unreadable_entry = true,
            }
        }
        // Byte order of the name: the same on every launch and every
        // machine, whatever order the file system lists in.
        names.sort_unstable();
        Self {
            names,
            truncated,
            unreadable_entry,
        }
    }
}

fn is_agent_file_name(name: &OsStr) -> bool {
    let hidden = name.as_encoded_bytes().first() == Some(&b'.');
    !hidden && Path::new(name).extension() == Some(OsStr::new("md"))
}

fn read_one(directory: &Directory, name: &OsStr) -> UserAgentFile {
    let file_name = display_name(name);
    if name.to_str().is_none() {
        return refused(
            &file_name,
            AgentFileError::FileNameNotUtf8 {
                file: file_name.clone(),
            },
        );
    }
    let result =
        read_text(directory, name, &file_name).and_then(|text| parse_agent_file(&file_name, &text));
    UserAgentFile { file_name, result }
}

/// The file's text, if it is a regular UTF-8 file within the size cap.
fn read_text(
    directory: &Directory,
    name: &OsStr,
    file_name: &str,
) -> Result<String, AgentFileError> {
    let listed = directory.entry(name, file_name)?;
    if !listed.regular {
        return Err(AgentFileError::NotARegularFile {
            file: file_name.to_owned(),
        });
    }
    if listed.len > u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX) {
        return Err(AgentFileError::TooLarge {
            file: file_name.to_owned(),
            bytes: usize::try_from(listed.len).unwrap_or(usize::MAX),
        });
    }
    let file = directory.open_regular(name, file_name)?;
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

/// What the directory says of an entry, without following it.
struct Listed {
    regular: bool,
    len: u64,
}

enum Opened {
    Missing,
    /// A symbolic link, or not a directory.
    Refused,
    Unreadable,
    Directory(Directory),
}

/// The agents directory, opened once: every file is looked up in it.
#[cfg(unix)]
pub(super) struct Directory(std::os::fd::OwnedFd);

#[cfg(unix)]
impl Directory {
    fn open(dir: &Path) -> Opened {
        use rustix::fs::{Mode, OFlags};
        match rustix::fs::open(
            dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => Opened::Directory(Self(fd)),
            Err(rustix::io::Errno::NOENT) => Opened::Missing,
            // A link to a directory fails `NOFOLLOW` with `ELOOP` on Linux
            // and macOS; a file, or a link to one, fails `DIRECTORY`.
            Err(rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) => Opened::Refused,
            Err(_) => Opened::Unreadable,
        }
    }

    fn names(&self) -> Names {
        use std::os::unix::ffi::OsStrExt;
        let Ok(dir) = rustix::fs::Dir::read_from(&self.0) else {
            return Names {
                names: Vec::new(),
                truncated: false,
                unreadable_entry: true,
            };
        };
        Names::collect(
            dir.map(|entry| {
                entry
                    .map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_owned())
                    .map_err(io::Error::from)
            })
            .filter(|name| {
                !matches!(name, Ok(name) if name.as_encoded_bytes() == b"." || name.as_encoded_bytes() == b"..")
            }),
        )
    }

    fn entry(&self, name: &OsStr, file_name: &str) -> Result<Listed, AgentFileError> {
        use rustix::fs::{AtFlags, FileType};
        let stat = rustix::fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| {
            AgentFileError::Unreadable {
                file: file_name.to_owned(),
            }
        })?;
        Ok(Listed {
            regular: FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile,
            // A negative size is no size a file has: refuse it as too large.
            len: u64::try_from(stat.st_size).unwrap_or(u64::MAX),
        })
    }

    /// Opens `name` if it is a regular file, never waiting and never
    /// following a link: the entry can be swapped for a link, a FIFO or a
    /// device between [`Self::entry`] and here.
    pub(super) fn open_regular(
        &self,
        name: &OsStr,
        file_name: &str,
    ) -> Result<File, AgentFileError> {
        use rustix::fs::{Mode, OFlags};
        let not_regular = || AgentFileError::NotARegularFile {
            file: file_name.to_owned(),
        };
        let fd = rustix::fs::openat(
            &self.0,
            name,
            // `NONBLOCK`: a FIFO would otherwise hold the open until a writer
            // comes; it changes nothing for a regular file. `NOCTTY`: a
            // terminal put there does not become Oxyn's.
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|errno| match errno {
            rustix::io::Errno::LOOP => not_regular(),
            _ => AgentFileError::Unreadable {
                file: file_name.to_owned(),
            },
        })?;
        let file = File::from(fd);
        match file.metadata() {
            Ok(meta) if meta.file_type().is_file() => Ok(file),
            Ok(_) => Err(not_regular()),
            Err(_) => Err(AgentFileError::Unreadable {
                file: file_name.to_owned(),
            }),
        }
    }
}

/// The agents directory, by path: no system call here opens a directory and
/// a file relative to it without following links. No Windows build ships
/// (SECURITY, "Workspace files"); a port starts here.
#[cfg(not(unix))]
pub(super) struct Directory(std::path::PathBuf);

#[cfg(not(unix))]
impl Directory {
    fn open(dir: &Path) -> Opened {
        match std::fs::symlink_metadata(dir) {
            Ok(meta) if meta.file_type().is_dir() => Opened::Directory(Self(dir.to_owned())),
            Ok(_) => Opened::Refused,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Opened::Missing,
            Err(_) => Opened::Unreadable,
        }
    }

    fn names(&self) -> Names {
        match std::fs::read_dir(&self.0) {
            Ok(entries) => {
                Names::collect(entries.map(|entry| entry.map(|entry| entry.file_name())))
            }
            Err(_) => Names {
                names: Vec::new(),
                truncated: false,
                unreadable_entry: true,
            },
        }
    }

    fn entry(&self, name: &OsStr, file_name: &str) -> Result<Listed, AgentFileError> {
        let meta = std::fs::symlink_metadata(self.0.join(name)).map_err(|_| {
            AgentFileError::Unreadable {
                file: file_name.to_owned(),
            }
        })?;
        Ok(Listed {
            regular: meta.file_type().is_file(),
            len: meta.len(),
        })
    }

    pub(super) fn open_regular(
        &self,
        name: &OsStr,
        file_name: &str,
    ) -> Result<File, AgentFileError> {
        let file = File::open(self.0.join(name)).map_err(|_| AgentFileError::Unreadable {
            file: file_name.to_owned(),
        })?;
        match file.metadata() {
            Ok(meta) if meta.file_type().is_file() => Ok(file),
            _ => Err(AgentFileError::NotARegularFile {
                file: file_name.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
impl Directory {
    pub(super) fn open_for_test(dir: &Path) -> Option<Self> {
        match Self::open(dir) {
            Opened::Directory(directory) => Some(directory),
            _ => None,
        }
    }
}

/// A name as the picker and an error show it: invalid UTF-8, control
/// characters and bidirectional overrides replaced by `�`, so that a file
/// name can neither break a line of a log nor read as another name.
fn display_name(name: &OsStr) -> String {
    name.to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_control() || is_bidi_control(c) {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect()
}

/// The characters that reorder text around them (Unicode's `Bidi_Control`).
const fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// What an error about the directory itself names: its last component,
/// never the full path, which holds the user's account name.
fn directory_label(dir: &Path) -> String {
    dir.file_name()
        .map_or_else(|| "agents".to_owned(), display_name)
}
