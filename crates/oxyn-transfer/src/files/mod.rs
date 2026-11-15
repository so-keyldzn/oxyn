//! Host-selected file capabilities. Neither a string nor an extension grants access.
mod input;
mod output;
mod tool;
pub(crate) use input::{Fingerprint, Input};
pub(crate) use output::Output;
pub(crate) use tool::{SelectedTool, ToolSnapshot};

use crate::{Result, TransferError};
use std::fs::File;
use std::path::Path;

pub(crate) fn regular(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        let path = path.canonicalize().map_err(|_| TransferError::File)?;
        let fd = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| TransferError::File)?;
        let file = File::from(fd);
        if !file.metadata().map_err(|_| TransferError::File)?.is_file() {
            return Err(TransferError::File);
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(TransferError::Unavailable(
            "Native file capabilities are not implemented on this platform",
        ))
    }
}
