//! Dated client floors are sourced in RESEARCH-NOTES, not inferred from a driver ID.
use crate::{Result, TransferError};
use oxyn_core::NativeToolRole;

/// Parsed public PostgreSQL client version; no arbitrary program output is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolVersion {
    /// Major protocol/tool generation.
    pub major: u16,
    /// Maintenance release within the major.
    pub patch: u16,
}
impl ToolVersion {
    /// Parse only the requested canonical tool's bounded version line.
    ///
    /// # Errors
    /// Wrong role/program, malformed output or an unsupported security baseline.
    pub fn parse(role: NativeToolRole, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 4096 {
            return Err(TransferError::Tool);
        }
        let name = match role {
            NativeToolRole::Export => "pg_dump",
            NativeToolRole::Restore => "pg_restore",
            _ => return Err(TransferError::Tool),
        };
        let output = std::str::from_utf8(bytes).map_err(|_| TransferError::Tool)?;
        let version = output
            .strip_prefix(&format!("{name} (PostgreSQL) "))
            .and_then(|tail| tail.split_whitespace().next())
            .ok_or(TransferError::Tool)?;
        let mut components = version.split('.');
        let major = components
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or(TransferError::Version)?;
        let patch = components
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or(TransferError::Version)?;
        if components.next().is_some() {
            return Err(TransferError::Version);
        }
        let parsed = Self { major, patch };
        if !matches!(
            parsed,
            Self {
                major: 17,
                patch: 12..
            } | Self {
                major: 18,
                patch: 7..
            }
        ) {
            return Err(TransferError::Version);
        }
        Ok(parsed)
    }
    /// Normalized public display value, not the original output line.
    #[must_use]
    pub fn display(self) -> String {
        format!("{}.{}", self.major, self.patch)
    }
}
