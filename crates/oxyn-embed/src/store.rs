//! Where the model lives on disk, and in what state.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::EmbedError;
use crate::hash::size_matches;
use crate::pinned::{CONVERTED, PinnedFile, SAFETENSORS, TOKENIZER};

/// The directory the pinned model's files live in, under the root the
/// caller chose. Named after the revision: a future model gets a directory
/// of its own and never reads this one's files.
const MODEL_DIRECTORY: &str = "granite-embedding-97m-multilingual-r2-835ad140";

/// Suffix of a file being written. Only ever renamed into place once
/// verified, so a crash leaves a `.part`, never a damaged final file.
pub(crate) const PART_SUFFIX: &str = ".part";

/// The lock file, `<root>/<MODEL_DIRECTORY>.lock`: beside the model's
/// directory, never in it.
///
/// A file lock belongs to an open file, not to a path. Kept inside the
/// directory, it would be unlinked by [`ModelStore::remove`] while a
/// download in another process still holds it; a third process would then
/// create a fresh `.lock`, lock that one, and write beside the first — the
/// serialization would be gone without an error anywhere. Outside, the path
/// outlives every removal, so every process locks the same file.
const LOCK_SUFFIX: &str = ".lock";

/// What [`ModelStore::status`] finds, from file sizes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelStatus {
    /// Nothing usable: never downloaded, removed, or a download interrupted
    /// before its end. [`ModelStore::download`] brings it to `Ready`.
    Absent,
    /// The tokenizer and the converted model are present at their pinned
    /// sizes. Their checksums are verified by [`Embedder::load`](crate::Embedder::load).
    Ready,
    /// A final file is present at the wrong size — truncated by a full disk,
    /// or replaced. [`ModelStore::download`] replaces it;
    /// [`ModelStore::remove`] clears it.
    Corrupt,
}

/// The model's data directory.
///
/// Cheap to clone: clones share the lock that serializes
/// [`download`](Self::download), so one store per root is enough and two
/// windows asking for the download at once do not write the same `.part`.
/// Between processes, [`download`](Self::download) and
/// [`remove`](Self::remove) exclude each other through a lock file beside the
/// directory.
#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
    dir: PathBuf,
    pub(crate) download_lock: Arc<tokio::sync::Mutex<()>>,
}

impl ModelStore {
    /// A store under `root`, which the caller owns: Oxyn's data directory
    /// gives it, typically `<data dir>/models`. Nothing is created until
    /// [`download`](Self::download).
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            dir: root.as_ref().join(MODEL_DIRECTORY),
            download_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// The caller's root, which holds the model's directory and its lock.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn lock_path(&self) -> PathBuf {
        self.root.join(format!("{MODEL_DIRECTORY}{LOCK_SUFFIX}"))
    }

    /// Takes the lock file, exclusively and without waiting; `root` must
    /// exist.
    ///
    /// The lock is released when the returned handle drops — at the end of
    /// the operation, on an error, on a cancellation, and by the system if
    /// the process dies. The file itself stays: it is empty and only ever
    /// locked. **Blocks** on an open and a non-blocking `flock`.
    ///
    /// Within one process it excludes too: the lock belongs to the open
    /// file, and a second open is a second owner.
    pub(crate) fn lock_exclusive(&self) -> Result<File, EmbedError> {
        let path = self.lock_path();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|err| EmbedError::io("open", &path, err))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(TryLockError::WouldBlock) => Err(EmbedError::DownloadInProgress),
            Err(TryLockError::Error(err)) => Err(EmbedError::io("lock", &path, err)),
        }
    }

    /// The directory holding this model's files.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn path(&self, file: &PinnedFile) -> PathBuf {
        self.dir.join(file.name)
    }

    pub(crate) fn part_path(&self, file: &PinnedFile) -> PathBuf {
        self.dir.join(format!("{}{PART_SUFFIX}", file.name))
    }

    /// The state of the files, from their sizes.
    ///
    /// **Blocks** on two `stat` calls — negligible, but disk I/O: from the UI
    /// thread, ask a worker. It does not hash: that is
    /// [`Embedder::load`](crate::Embedder::load)'s job, which reads the files
    /// anyway.
    ///
    /// # Errors
    /// [`EmbedError::Io`] when the directory cannot be inspected — permission
    /// denied, for instance; not finding it is [`ModelStatus::Absent`].
    pub fn status(&self) -> Result<ModelStatus, EmbedError> {
        let tokenizer = size_matches(&self.path(&TOKENIZER), &TOKENIZER)?;
        let model = size_matches(&self.path(&CONVERTED), &CONVERTED)?;
        Ok(match (tokenizer, model) {
            (Some(true), Some(true)) => ModelStatus::Ready,
            (Some(false), _) | (_, Some(false)) => ModelStatus::Corrupt,
            _ => ModelStatus::Absent,
        })
    }

    /// Deletes this model's directory and everything in it, `.part` files
    /// included. Absent already is not an error.
    ///
    /// **Blocks** on disk I/O. A loaded [`Embedder`](crate::Embedder) keeps
    /// working: on macOS and Linux a removed file stays readable to whoever
    /// mapped it. The caller unloads first when the point is to free memory —
    /// [`OnDemandEmbedder::unload`](crate::OnDemandEmbedder::unload).
    ///
    /// Refused while a download holds the directory, in this process or
    /// another: deleting the files under it would make it fail on its next
    /// write, and let another process start a second one beside it.
    ///
    /// # Errors
    /// [`EmbedError::DownloadInProgress`] while a download runs — cancel it,
    /// wait for it, then remove; [`EmbedError::Io`] when a file cannot be
    /// removed.
    pub fn remove(&self) -> Result<(), EmbedError> {
        if !self.root.is_dir() {
            return Ok(());
        }
        let _exclusive = self.lock_exclusive()?;
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EmbedError::io("remove", &self.dir, err)),
        }
    }

    /// The upstream weights: present only between their download and the
    /// end of the conversion, which deletes them.
    pub(crate) fn safetensors_path(&self) -> PathBuf {
        self.path(&SAFETENSORS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_len(path: &Path, len: u64) -> Result<(), EmbedError> {
        let file = std::fs::File::create(path).map_err(|e| EmbedError::io("create", path, e))?;
        file.set_len(len)
            .map_err(|e| EmbedError::io("extend", path, e))
    }

    #[test]
    fn status_follows_the_final_files_only() -> Result<(), EmbedError> {
        let root = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
        let store = ModelStore::new(root.path());
        assert_eq!(store.status()?, ModelStatus::Absent);

        std::fs::create_dir_all(store.dir()).map_err(|e| EmbedError::io("create", "dir", e))?;
        // An interrupted download leaves only `.part` files: nothing to repair.
        write_len(&store.part_path(&CONVERTED), 10)?;
        write_len(&store.path(&TOKENIZER), TOKENIZER.size)?;
        assert_eq!(store.status()?, ModelStatus::Absent);

        write_len(&store.path(&CONVERTED), CONVERTED.size)?;
        assert_eq!(store.status()?, ModelStatus::Ready);

        // Truncated by a full disk.
        write_len(&store.path(&CONVERTED), CONVERTED.size / 2)?;
        assert_eq!(store.status()?, ModelStatus::Corrupt);

        store.remove()?;
        assert_eq!(store.status()?, ModelStatus::Absent);
        // Removing twice is not an error.
        store.remove()?;
        Ok(())
    }
}
