//! A retained file descriptor prevents path replacement from changing the archive.
use super::regular;
use crate::{Result, TransferError};
use std::fs::{File, Metadata};
use std::io::{Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    changed: (i64, i64),
}
impl Fingerprint {
    pub(crate) fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            dev: metadata.dev(),
            #[cfg(unix)]
            ino: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
    pub(crate) fn len(&self) -> u64 {
        self.len
    }
}
pub(crate) struct Input {
    file: File,
    fingerprint: Fingerprint,
    pub(crate) label: String,
}
impl Input {
    pub(crate) fn select(path: &Path) -> Result<Self> {
        let file = regular(path)?;
        let metadata = file.metadata().map_err(|_| TransferError::File)?;
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.len() <= 4096)
            .ok_or(TransferError::File)?
            .to_owned();
        Ok(Self {
            file,
            fingerprint: Fingerprint::of(&metadata),
            label,
        })
    }
    pub(crate) fn reader(&mut self) -> Result<File> {
        if Fingerprint::of(&self.file.metadata().map_err(|_| TransferError::File)?)
            != self.fingerprint
        {
            return Err(TransferError::Archive);
        }
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| TransferError::File)?;
        self.file.try_clone().map_err(|_| TransferError::File)
    }
}
