//! An inspected executable is a private snapshot, never a mutable path lookup at run time.
use super::{Fingerprint, regular};
use crate::postgres::ToolVersion;
use crate::{Result, TransferError};
use oxyn_core::{
    CancelToken, DriverId, NativeGrantId, NativeToolIdentity, NativeToolInfo, NativeToolRole,
};
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const MAX_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) struct SelectedTool {
    file: File,
    fingerprint: Fingerprint,
    pub(crate) role: NativeToolRole,
    pub(crate) engine: DriverId,
}
/// Resources stay owned by the grant and every prepared/running reference to it.
pub(crate) struct ToolSnapshot {
    _directory: tempfile::TempDir,
    path: PathBuf,
    pub(crate) info: NativeToolInfo,
    pub(crate) version: ToolVersion,
}
impl SelectedTool {
    pub(crate) fn select(path: &Path, engine: DriverId, role: NativeToolRole) -> Result<Self> {
        if engine.as_str() != "postgres" {
            return Err(TransferError::Unavailable(
                "A native tool adapter is not available for this engine",
            ));
        }
        let mut file = regular(path).map_err(|_| TransferError::Tool)?;
        let metadata = file.metadata().map_err(|_| TransferError::Tool)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(TransferError::Tool);
            }
        }
        if metadata.len() == 0 || metadata.len() > MAX_EXECUTABLE_BYTES {
            return Err(TransferError::Tool);
        }
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)
            .map_err(|_| TransferError::Tool)?;
        if !matches!(
            magic,
            [0x7f, b'E', b'L', b'F']
                | [0xcf, 0xfa, 0xed, 0xfe]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xca, 0xfe, 0xba, 0xbe]
                | [0xca, 0xfe, 0xba, 0xbf]
        ) {
            return Err(TransferError::Tool);
        }
        file.rewind().map_err(|_| TransferError::Tool)?;
        Ok(Self {
            file,
            fingerprint: Fingerprint::of(&metadata),
            role,
            engine,
        })
    }
    pub(crate) fn inspect(
        mut self,
        grant: NativeGrantId,
        cancel: &CancelToken,
    ) -> Result<ToolSnapshot> {
        let directory = tempfile::Builder::new()
            .prefix("oxyn-native-tool-")
            .tempdir()
            .map_err(|_| TransferError::Tool)?;
        let name = match self.role {
            NativeToolRole::Export => "pg_dump",
            NativeToolRole::Restore => "pg_restore",
            _ => return Err(TransferError::Tool),
        };
        let path = directory.path().join(name);
        let mut snapshot = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| TransferError::Tool)?;
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(TransferError::Cancelled);
            }
            let count = self
                .file
                .read(&mut buffer)
                .map_err(|_| TransferError::Tool)?;
            if count == 0 {
                break;
            }
            copied = copied
                .checked_add(u64::try_from(count).map_err(|_| TransferError::Tool)?)
                .filter(|bytes| *bytes <= MAX_EXECUTABLE_BYTES)
                .ok_or(TransferError::Tool)?;
            snapshot
                .write_all(buffer.get(..count).ok_or(TransferError::Tool)?)
                .map_err(|_| TransferError::Tool)?;
        }
        if copied != self.fingerprint.len()
            || Fingerprint::of(&self.file.metadata().map_err(|_| TransferError::Tool)?)
                != self.fingerprint
        {
            return Err(TransferError::Tool);
        }
        snapshot.sync_all().map_err(|_| TransferError::Tool)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            snapshot
                .set_permissions(std::fs::Permissions::from_mode(0o500))
                .map_err(|_| TransferError::Tool)?;
        }
        drop(snapshot);
        let mut command = confined_command(&path, directory.path());
        command.arg("--version").stdin(Stdio::null());
        let observation = crate::process::run(
            command,
            cancel,
            &CancelToken::new(),
            Duration::from_secs(5),
            Some(4096),
            &mut |_| {},
        )?;
        if observation.cancelled {
            return Err(TransferError::Cancelled);
        }
        if observation.deadline {
            return Err(TransferError::Deadline);
        }
        if observation.code != Some(0) || !observation.cleanup || observation.overflow {
            return Err(TransferError::Tool);
        }
        let version = ToolVersion::parse(self.role, &observation.stdout)?;
        let info = NativeToolInfo {
            grant,
            identity: NativeToolIdentity {
                engine: self.engine,
                role: self.role,
                name: name.into(),
                version: version.display(),
            },
        };
        Ok(ToolSnapshot {
            _directory: directory,
            path,
            info,
            version,
        })
    }
}
impl ToolSnapshot {
    pub(crate) fn command(&self, directory: &Path) -> Command {
        confined_command(&self.path, directory)
    }
}

fn confined_command(path: &Path, directory: &Path) -> Command {
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR-0083: the native boundary's single command site clears ambient secrets and runs only on an owned blocking worker"
    )]
    let mut command = Command::new(path);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("HOME", directory)
        .env("PGSYSCONFDIR", directory)
        .env("PGSERVICEFILE", directory.join("service.disabled"))
        .current_dir(directory);
    command
}
