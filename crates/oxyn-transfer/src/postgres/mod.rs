//! PostgreSQL argument construction is not a generic process-launch service.
mod target;
mod version;
pub use target::Target;
pub use version::ToolVersion;

/// Non-executing format check, not a certificate of archive contents.
pub(crate) fn check_archive_header(reader: &mut impl std::io::Read) -> crate::Result<()> {
    let mut magic = [0u8; 5];
    reader
        .read_exact(&mut magic)
        .map_err(|_| crate::TransferError::Archive)?;
    if &magic != b"PGDMP" {
        return Err(crate::TransferError::Archive);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
