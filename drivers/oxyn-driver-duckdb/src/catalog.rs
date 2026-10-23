//! Minimal DuckDB catalog boundary.

use async_trait::async_trait;
use oxyn_catalog::model::{Relation, RelationRef, ServerInfo};
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{CancelToken, Capabilities, OxynError, Result};

/// Catalog access bound to one DuckDB session.
#[derive(Debug)]
pub(crate) struct DuckDbCatalog {
    capabilities: Capabilities,
}

impl DuckDbCatalog {
    pub(crate) const fn new(capabilities: Capabilities) -> Self {
        Self { capabilities }
    }
}

#[async_trait]
impl CatalogProvider for DuckDbCatalog {
    async fn server_info(&self, cancel: &CancelToken) -> Result<ServerInfo> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        Ok(ServerInfo::new(
            "DuckDB",
            duckdb::version(),
            self.capabilities,
        ))
    }

    async fn list_relations(
        &self,
        _namespace: &CatalogPath,
        _cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        Err(OxynError::NotSupported {
            capability: "DuckDB catalog while execution support is incomplete".to_owned(),
        })
    }

    async fn describe_relation(
        &self,
        _relation: &CatalogPath,
        _cancel: &CancelToken,
    ) -> Result<Relation> {
        Err(OxynError::NotSupported {
            capability: "DuckDB catalog while execution support is incomplete".to_owned(),
        })
    }
}
