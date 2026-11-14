//! Provider-declared inference bounds belong to the catalog contract.

use oxyn_core::{OxynError, Result};
use serde::{Deserialize, Serialize};

/// Declared inference bounds, captured when the owning session opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaSampleLimits {
    /// Returned documents inspected, not a promise about a server's query plan.
    pub max_documents: u32,
    /// Bytes retained for inference; protocol batching can receive more.
    pub max_retained_bytes: u64,
    /// Server-side execution deadline in milliseconds.
    pub max_duration_ms: u64,
}

impl SchemaSampleLimits {
    /// Rejects a provider whose declared bound exceeds the host's admission cap.
    ///
    /// # Errors
    /// Zero or excessive limits; the host never silently lowers a provider's claim.
    pub fn validate(&self) -> Result<()> {
        if self.max_documents == 0
            || self.max_documents > 1_000
            || self.max_retained_bytes == 0
            || self.max_retained_bytes > 16 * 1024 * 1024
            || self.max_duration_ms == 0
            || self.max_duration_ms > 60_000
        {
            return Err(OxynError::Config(
                "schema sampling bounds are unavailable".into(),
            ));
        }
        Ok(())
    }
}
