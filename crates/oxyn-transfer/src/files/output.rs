//! Private staging plus held directory handles and linkat give atomic no-clobber publication.
use crate::{Result, TransferError};
#[cfg(unix)]
use std::ffi::OsString;
use std::fs::File;
use std::path::Path;

pub(crate) struct Output {
    #[cfg(unix)]
    parent: File,
    #[cfg(unix)]
    name: OsString,
    #[cfg(unix)]
    stage: Option<(File, OsString)>,
    data: Option<File>,
    published: bool,
    pub(crate) label: String,
}
impl Output {
    pub(crate) fn select(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let name = path
                .file_name()
                .filter(|name| *name != "." && *name != "..")
                .ok_or(TransferError::File)?
                .to_os_string();
            let label = name
                .to_str()
                .filter(|name| !name.is_empty() && name.len() <= 4096)
                .ok_or(TransferError::File)?
                .to_owned();
            let parent = path
                .parent()
                .ok_or(TransferError::File)?
                .canonicalize()
                .map_err(|_| TransferError::File)?;
            let fd = rustix::fs::open(
                parent,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| TransferError::File)?;
            let selected = Self {
                parent: File::from(fd),
                name,
                stage: None,
                data: None,
                published: false,
                label,
            };
            selected.absent()?;
            Ok(selected)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(TransferError::Unavailable(
                "Native atomic file publication is not implemented on this platform",
            ))
        }
    }
    #[cfg(unix)]
    fn absent(&self) -> Result<()> {
        #[cfg(unix)]
        {
            match rustix::fs::statat(
                &self.parent,
                &self.name,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Err(rustix::io::Errno::NOENT) => Ok(()),
                _ => Err(TransferError::File),
            }
        }
        #[cfg(not(unix))]
        {
            Err(TransferError::File)
        }
    }
    pub(crate) fn writer(&mut self) -> Result<File> {
        #[cfg(unix)]
        {
            self.absent()?;
            if self.stage.is_some() {
                return Err(TransferError::Grant);
            }
            let name = OsString::from(format!(".oxyn-transfer-{}", uuid::Uuid::new_v4()));
            rustix::fs::mkdirat(
                &self.parent,
                &name,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR | rustix::fs::Mode::XUSR,
            )
            .map_err(|_| TransferError::File)?;
            let directory = rustix::fs::openat(
                &self.parent,
                &name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map(File::from)
            .map_err(|_| TransferError::File)?;
            self.stage = Some((directory, name));
            let directory = &self.stage.as_ref().ok_or(TransferError::File)?.0;
            let fd = rustix::fs::openat(
                directory,
                "output.part",
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::CREATE
                    | rustix::fs::OFlags::EXCL
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .map_err(|_| TransferError::File)?;
            let file = File::from(fd);
            let writer = file.try_clone().map_err(|_| TransferError::File)?;
            self.data = Some(file);
            Ok(writer)
        }
        #[cfg(not(unix))]
        {
            Err(TransferError::File)
        }
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.data
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |meta| meta.len())
    }
    pub(crate) fn publish(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            self.data
                .as_ref()
                .ok_or(TransferError::File)?
                .sync_all()
                .map_err(|_| TransferError::File)?;
            let directory = &self.stage.as_ref().ok_or(TransferError::File)?.0;
            let held = rustix::fs::fstat(self.data.as_ref().ok_or(TransferError::File)?)
                .map_err(|_| TransferError::File)?;
            let named = rustix::fs::statat(
                directory,
                "output.part",
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|_| TransferError::File)?;
            if held.st_dev != named.st_dev || held.st_ino != named.st_ino {
                return Err(TransferError::File);
            }
            rustix::fs::linkat(
                directory,
                "output.part",
                &self.parent,
                &self.name,
                rustix::fs::AtFlags::empty(),
            )
            .map_err(|_| TransferError::File)?;
            self.published = true;
            rustix::fs::unlinkat(directory, "output.part", rustix::fs::AtFlags::empty())
                .map_err(|_| TransferError::File)?;
            self.parent.sync_all().map_err(|_| TransferError::File)?;
            self.cleanup()
        }
        #[cfg(not(unix))]
        {
            Err(TransferError::File)
        }
    }
    pub(crate) fn was_published(&self) -> bool {
        self.published
    }
    pub(crate) fn cleanup(&mut self) -> Result<()> {
        self.data.take();
        #[cfg(unix)]
        if let Some((directory, name)) = &self.stage {
            match rustix::fs::unlinkat(directory, "output.part", rustix::fs::AtFlags::empty()) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => {}
                Err(_) => return Err(TransferError::File),
            }
            let held = rustix::fs::fstat(directory).map_err(|_| TransferError::File)?;
            let named =
                rustix::fs::statat(&self.parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|_| TransferError::File)?;
            if held.st_dev != named.st_dev || held.st_ino != named.st_ino {
                return Err(TransferError::File);
            }
            rustix::fs::unlinkat(&self.parent, name, rustix::fs::AtFlags::REMOVEDIR)
                .map_err(|_| TransferError::File)?;
            self.parent.sync_all().map_err(|_| TransferError::File)?;
            self.stage.take();
        }
        Ok(())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(all(test, unix))]
mod tests;
