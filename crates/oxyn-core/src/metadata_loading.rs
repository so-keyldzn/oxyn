//! Manual catalog-loading preferences, independent from connection authority.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{OxynError, Result};
mod preload;
pub use preload::{
    MetadataFacet, MetadataPreloadCursor, MetadataPreloadIssue, MetadataPreloadProgress,
    MetadataPreloadRequest, MetadataPreloadState,
};

/// Largest exact revision carried through JavaScript IPC.
pub const MAX_METADATA_POLICY_REVISION: u64 = (1_u64 << 53) - 1;
/// Namespace overrides retained by one connection.
pub const MAX_METADATA_OVERRIDES: usize = 32;
/// UTF-8 bytes accepted for one exact catalog/namespace identifier.
pub const MAX_METADATA_SCOPE_BYTES: usize = 1024;
/// Encoded policy bytes accepted at the persistence boundary.
pub const MAX_METADATA_POLICY_BYTES: usize = 128 * 1024;
/// Relations one explicit preload may describe; excess stays reported unread.
pub const MAX_PRELOAD_RELATIONS: usize = 128;

/// Facets requested by an explicit preload, never a permission to read rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MetadataLevel {
    /// Names in one explicitly selected scope.
    #[default]
    Minimal,
    /// Names, descriptions, supported indexes and outgoing keys.
    Structure,
    /// Structure plus supported object definitions.
    Extended,
}

/// Exact native hierarchy; missing levels remain absent rather than guessed.
#[derive(Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataScope {
    /// Database/catalog, absent on engines without that level.
    pub catalog: Option<String>,
    /// Schema/database namespace, absent on engines without that level.
    pub namespace: Option<String>,
}

impl fmt::Debug for MetadataScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetadataScope")
            .field("catalog_present", &self.catalog.is_some())
            .field("namespace_present", &self.namespace.is_some())
            .finish()
    }
}

impl MetadataScope {
    /// Refuses excessive or invalid identifiers without echoing their content.
    ///
    /// # Errors
    /// An empty, oversized or control-bearing supplied segment.
    pub fn validate(&self) -> Result<()> {
        for segment in self.catalog.iter().chain(self.namespace.iter()) {
            validate_segment(segment)?;
        }
        Ok(())
    }
}

fn validate_segment(segment: &str) -> Result<()> {
    if segment.is_empty()
        || segment.len() > MAX_METADATA_SCOPE_BYTES
        || segment.chars().any(char::is_control)
    {
        return Err(OxynError::Config(
            "invalid metadata scope identifier".into(),
        ));
    }
    Ok(())
}

/// A namespace's manual exception to its connection's default preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataOverride {
    /// Full native namespace identity, including its catalog where present.
    pub scope: MetadataScope,
    /// Explicit preset; it starts no read when saved.
    pub level: MetadataLevel,
}

/// Readable, versioned loading state; contains no authentication setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataLoadingPolicy {
    /// Unknown versions fail rather than silently dropping preferences.
    pub version: u32,
    /// Preset inherited by scopes without an exact override.
    pub default: MetadataLevel,
    /// Distinct native catalog/namespace pairs, without case folding.
    pub overrides: Vec<MetadataOverride>,
}

impl Default for MetadataLoadingPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            default: MetadataLevel::Minimal,
            overrides: Vec::new(),
        }
    }
}

impl MetadataLoadingPolicy {
    /// Checks version, namespace identity and bounded local state.
    ///
    /// # Errors
    /// Unknown versions, duplicate scopes or excess overrides.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.overrides.len() > MAX_METADATA_OVERRIDES {
            return Err(OxynError::Config("invalid metadata loading policy".into()));
        }
        let mut seen = HashSet::new();
        for item in &self.overrides {
            item.scope.validate()?;
            if (item.scope.catalog.is_none() && item.scope.namespace.is_none())
                || !seen.insert(&item.scope)
            {
                return Err(OxynError::Config(
                    "invalid or duplicate metadata override".into(),
                ));
            }
        }
        Ok(())
    }

    /// Resolves exact native scope identity; never introspects or changes a session.
    #[must_use]
    pub fn effective(&self, scope: &MetadataScope) -> MetadataLevel {
        self.overrides
            .iter()
            .find(|item| item.scope == *scope)
            .map_or(self.default, |item| item.level)
    }
}

/// Missing persisted state has revision zero and an on-demand Minimal default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataPolicySnapshot {
    /// Monotonic compare-and-set token, exactly representable in JavaScript.
    pub revision: u64,
    /// Loading preference, independent of credentials and privacy tier.
    pub policy: MetadataLoadingPolicy,
}

#[cfg(test)]
mod tests;
