//! Bound encoded metadata before publishing, without allocating another payload.

use super::CatalogPatch;
use oxyn_core::{OxynError, Result};
use std::io::{self, Write};

/// Input/publication guard, not an automatic prefetch threshold.
const MAX_METADATA_PATCH_BYTES: usize = 16 * 1024 * 1024;

struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_METADATA_PATCH_BYTES)
            .ok_or_else(|| io::Error::other("metadata byte limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl CatalogPatch {
    pub(crate) fn encoded_bytes(&self) -> Result<usize> {
        if let Some(bytes) = self.encoded_size {
            return Ok(bytes);
        }
        encoded_bytes(self)
    }
}

/// Shared publication guard for metadata snapshots, without a second encoded buffer.
pub(crate) fn encoded_bytes(value: &impl serde::Serialize) -> Result<usize> {
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| {
        OxynError::Config("metadata response exceeds its encoded byte budget".into())
    })?;
    Ok(counter.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_catalog::{CatalogPath, Relation, RelationKind};
    #[test]
    fn excessive_metadata_is_refused_without_exposing_its_content() {
        let path = CatalogPath::for_relation(None, None, "fixture").expect("path");
        let mut relation = Relation::new("fixture", RelationKind::Table);
        relation.comment = Some("synthetic-private".repeat(MAX_METADATA_PATCH_BYTES / 16 + 1));
        let patch = CatalogPatch {
            relation: Some((path, relation)),
            ..CatalogPatch::default()
        };
        let error = patch.encoded_bytes().expect_err("bounded publication");
        assert!(!error.to_string().contains("synthetic-private"));
    }
}
