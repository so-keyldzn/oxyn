//! A publication epoch is catalog coordination, never execution authority.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Retired when a live cache's scope changes; omitted from persisted cache content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CatalogGenerationId(Uuid);

impl CatalogGenerationId {
    /// A fresh epoch, independent of connection/session execution identifiers.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// The opaque wire token carried by core command DTOs, not an authority grant.
    #[must_use]
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }
}
impl Default for CatalogGenerationId {
    fn default() -> Self {
        Self::new()
    }
}
