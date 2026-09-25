//! The journal on disk, in the application's log directory.
//!
//! Launched from the Finder or a desktop entry, Oxyn has no terminal: stderr
//! goes nowhere, and a user asked for a journal has nothing to send. The file
//! is that journal.
//!
//! A line is handed to a thread of its own and written there. The caller may
//! be the UI thread, which does no I/O ([I-05](../../../../CLAUDE.md#i-05)); if
//! the writer falls behind, lines are dropped and counted rather than waited
//! for.
//!
//! The directory is bounded: [`Limits::files`] files of about
//! [`Limits::file_bytes`] each. Each launch starts a new file, so the last
//! launch's journal — the failed one — is in `oxyn.1.log` until this launch
//! first rotates.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;
use std::time::Duration;

use tracing_subscriber::fmt::MakeWriter;

/// How much of the disk the journal may take.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Past this size, the file is rotated before the next line.
    pub(crate) file_bytes: u64,
    /// Files kept, the one being written included.
    pub(crate) files: usize,
}

/// Fifty megabytes at most: a week of `info`, a few minutes of `trace`.
pub(crate) const LIMITS: Limits = Limits {
    file_bytes: 10 * 1024 * 1024,
    files: 5,
};

/// Lines waiting for the writer before new ones are dropped.
const QUEUE: usize = 4096;

const STEM: &str = "oxyn";

enum Message {
    Line(Vec<u8>),
    Flush(SyncSender<()>),
}

/// A handle on the journal's writer thread; cloning it shares the thread.
#[derive(Clone)]
pub(crate) struct FileJournal {
    directory: PathBuf,
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

impl FileJournal {
    /// Rotates the previous launch's file and starts the writer thread.
    ///
    /// # Errors
    /// If the directory or the file cannot be created, or the thread started.
    pub(crate) fn open(directory: &Path, limits: Limits) -> io::Result<Self> {
        let files = Files::open(directory, limits)?;
        let (queue, lines) = sync_channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&dropped);
        thread::Builder::new()
            .name("oxyn-log".to_owned())
            .spawn(move || files.drain(&lines, &counted))?;
        Ok(Self {
            directory: directory.to_path_buf(),
            queue,
            dropped,
        })
    }

    /// Where the files are.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Waits, at most `within`, for every line sent so far to be written.
    ///
    /// For an exit: what was logged just before is what explains it.
    pub(crate) fn flush(&self, within: Duration) {
        let (ack, written) = sync_channel(1);
        // Not `send`: a full queue behind a stalled disk would block without
        // bound, and a full queue already means the journal is dropping lines.
        if self.queue.try_send(Message::Flush(ack)).is_ok() {
            // A timeout leaves the lines to the thread; nothing more to do.
            let _ = written.recv_timeout(within);
        }
    }
}

impl<'a> MakeWriter<'a> for FileJournal {
    type Writer = Line<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        Line {
            journal: self,
            bytes: Vec::new(),
        }
    }
}

/// One event, sent whole when the formatter is done with it.
pub(crate) struct Line<'a> {
    journal: &'a FileJournal,
    bytes: Vec<u8>,
}

impl io::Write for Line<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Line<'_> {
    fn drop(&mut self) {
        if self.bytes.is_empty() {
            return;
        }
        let line = Message::Line(std::mem::take(&mut self.bytes));
        if self.journal.queue.try_send(line).is_err() {
            self.journal.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The writer thread's side: the open file and the rotation.
struct Files {
    directory: PathBuf,
    limits: Limits,
    file: File,
    written: u64,
}

impl Files {
    fn open(directory: &Path, limits: Limits) -> io::Result<Self> {
        create_directory(directory)?;
        shift(directory, limits.files)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            limits,
            file: create(&path(directory, 0))?,
            written: 0,
        })
    }

    fn drain(mut self, messages: &Receiver<Message>, dropped: &AtomicU64) {
        for message in messages {
            match message {
                Message::Line(bytes) => {
                    let lost = dropped.swap(0, Ordering::Relaxed);
                    if lost > 0 {
                        let notice = format!("{lost} log lines dropped: the journal fell behind\n");
                        self.write(notice.as_bytes(), dropped);
                    }
                    self.write(&bytes, dropped);
                }
                Message::Flush(ack) => {
                    let _ = ack.send(());
                }
            }
        }
    }

    /// A failure here cannot be logged — it would come back here. It is
    /// counted as a dropped line, which the next line that gets through
    /// reports.
    fn write(&mut self, bytes: &[u8], dropped: &AtomicU64) {
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if self.written > 0
            && self.written.saturating_add(size) > self.limits.file_bytes
            && self.rotate().is_err()
        {
            dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        match self.file.write_all(bytes) {
            Ok(()) => self.written = self.written.saturating_add(size),
            Err(_) => {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn rotate(&mut self) -> io::Result<()> {
        shift(&self.directory, self.limits.files)?;
        self.file = create(&path(&self.directory, 0))?;
        self.written = 0;
        Ok(())
    }
}

/// `oxyn.log` becomes `oxyn.1.log`, and so on; the oldest is removed so that
/// `files` remain once a new `oxyn.log` is created.
fn shift(directory: &Path, files: usize) -> io::Result<()> {
    let oldest = files.saturating_sub(1);
    remove_if_present(&path(directory, oldest))?;
    for index in (0..oldest).rev() {
        let from = path(directory, index);
        // Not `exists`, which follows a link: a dangling `oxyn.log` link left
        // in place would be written through by `create`.
        if fs::symlink_metadata(&from).is_ok() {
            fs::rename(from, path(directory, index + 1))?;
        }
    }
    Ok(())
}

fn remove_if_present(file: &Path) -> io::Result<()> {
    match fs::remove_file(file) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn path(directory: &Path, index: usize) -> PathBuf {
    if index == 0 {
        directory.join(format!("{STEM}.log"))
    } else {
        directory.join(format!("{STEM}.{index}.log"))
    }
}

/// On Unix, the directory and its files are its owner's only: a journal names
/// connections and paths. On Windows, the per-user `AppData\Local` is.
fn create_directory(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(directory)
}

/// `create_new`: never through a link, and never an older file that would
/// keep its own permissions — [`shift`] has moved `oxyn.log` out of the way.
fn create(file: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(file)
}

#[cfg(test)]
mod tests;
