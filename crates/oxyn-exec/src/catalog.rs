//! Lazy catalog reads and atomic cache publication, confined to the bus.

use std::collections::HashMap;
use std::sync::Arc;

use oxyn_catalog::{
    CatalogCache, CatalogPath, CatalogProvider, CatalogRef, NamespaceRef, Relation, RelationRef,
    ServerInfo, SharedCatalog,
};
use oxyn_core::{CancelToken, Capabilities, CatalogRefreshScope, OxynError, Result};
use parking_lot::RwLock;
use tokio::sync::Mutex;

pub(crate) struct ConnectionCatalog {
    pub cache: SharedCatalog,
    pub refresh: Mutex<CatalogBudget>,
    pub closed: CancelToken,
}

impl ConnectionCatalog {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(CatalogCache::new())),
            refresh: Mutex::new(CatalogBudget::default()),
            closed: CancelToken::new(),
        }
    }
}

// Product limits, applied before publication; provider vectors remain temporary.
const MAX_CATALOG_SCOPES: usize = 1_024;
const MAX_CATALOG_OBJECTS: usize = 50_000;

#[derive(Default)]
pub(crate) struct CatalogBudget {
    scopes: HashMap<CatalogRefreshScope, usize>,
}

impl CatalogBudget {
    pub fn reserve(&mut self, scope: &CatalogRefreshScope, patch: &CatalogPatch) -> Result<()> {
        let count = patch.object_count();
        let others: usize = self
            .scopes
            .iter()
            .filter(|(key, _)| *key != scope)
            .map(|(_, count)| count)
            .sum();
        if (!self.scopes.contains_key(scope) && self.scopes.len() >= MAX_CATALOG_SCOPES)
            || count.saturating_add(others) > MAX_CATALOG_OBJECTS
        {
            return Err(OxynError::Config(
                "catalog cache limit exceeded (1024 scopes, 50000 metadata objects)".to_owned(),
            ));
        }
        // Keep the high-water mark: list refreshes may retain child details.
        self.scopes
            .entry(scope.clone())
            .and_modify(|old| *old = (*old).max(count))
            .or_insert(count);
        Ok(())
    }
}

/// A single successful read, published only after releasing the session guard.
#[derive(Default)]
pub(crate) struct CatalogPatch {
    server: Option<ServerInfo>,
    catalogs: Option<Vec<CatalogRef>>,
    namespaces: Option<(Option<String>, Vec<NamespaceRef>)>,
    relations: Option<(CatalogPath, Vec<RelationRef>)>,
    relation: Option<(CatalogPath, Relation)>,
}

impl CatalogPatch {
    fn object_count(&self) -> usize {
        usize::from(self.server.is_some())
            + self.catalogs.as_ref().map_or(0, Vec::len)
            + self
                .namespaces
                .as_ref()
                .map_or(0, |(_, values)| values.len())
            + self
                .relations
                .as_ref()
                .map_or(0, |(_, values)| values.len())
            + self
                .relation
                .as_ref()
                .map_or(0, |(_, value)| 1 + value.fields.len())
    }

    pub fn apply(self, cache: &mut CatalogCache) -> Result<()> {
        if let Some(server) = self.server {
            cache.set_server_info(server);
        }
        if let Some(catalogs) = self.catalogs {
            cache.set_catalogs(catalogs);
        }
        if let Some((catalog, namespaces)) = self.namespaces {
            cache.set_namespaces(catalog.as_deref(), namespaces);
        }
        if let Some((path, relations)) = self.relations {
            cache.set_relations(&path, relations)?;
        }
        if let Some((path, relation)) = self.relation {
            cache.set_relation(&path, relation)?;
        }
        Ok(())
    }
}

fn check_cancel(cancel: &CancelToken) -> Result<()> {
    if cancel.is_cancelled() {
        Err(OxynError::Cancelled)
    } else {
        Ok(())
    }
}

fn path(
    catalog: &Option<String>,
    namespace: &Option<String>,
    relation: Option<String>,
) -> Result<CatalogPath> {
    CatalogPath::from_levels(catalog.clone(), namespace.clone(), relation)
        .map_err(|_| OxynError::Config("invalid catalog scope identifier".to_owned()))
}

pub(crate) async fn read(
    provider: &dyn CatalogProvider,
    capabilities: Capabilities,
    scope: &CatalogRefreshScope,
    cancel: &CancelToken,
) -> Result<CatalogPatch> {
    check_cancel(cancel)?;
    let mut patch = CatalogPatch::default();
    match scope {
        CatalogRefreshScope::Root => {
            let mut server = provider.server_info(cancel).await?;
            check_cancel(cancel)?;
            // Session capabilities are authoritative even if metadata disagrees.
            server.capabilities = capabilities;
            patch.server = Some(server);
            let catalogs = provider.list_catalogs(cancel).await?;
            check_cancel(cancel)?;
            if catalogs.is_empty() {
                if capabilities.contains(Capabilities::SCHEMAS) {
                    patch.namespaces = Some((None, provider.list_namespaces(None, cancel).await?));
                } else {
                    let root = CatalogPath::empty();
                    let relations = provider.list_relations(&root, cancel).await?;
                    patch.relations = Some((root, relations));
                }
            }
            patch.catalogs = Some(catalogs);
        }
        CatalogRefreshScope::Namespaces { catalog } => {
            path(catalog, &None, None)?;
            if !capabilities.contains(Capabilities::SCHEMAS) {
                return Err(OxynError::NotSupported {
                    capability: "SCHEMAS".to_owned(),
                });
            }
            let namespaces = provider.list_namespaces(catalog.as_deref(), cancel).await?;
            patch.namespaces = Some((catalog.clone(), namespaces));
        }
        CatalogRefreshScope::Relations { catalog, namespace } => {
            let parent = path(catalog, namespace, None)?;
            let relations = provider.list_relations(&parent, cancel).await?;
            patch.relations = Some((parent, relations));
        }
        CatalogRefreshScope::Relation {
            catalog,
            namespace,
            relation,
        } => {
            let target = path(catalog, namespace, Some(relation.clone()))?;
            let detail = provider.describe_relation(&target, cancel).await?;
            patch.relation = Some((target, detail));
        }
        _ => {
            return Err(OxynError::NotSupported {
                capability: "catalog scope".to_owned(),
            });
        }
    }
    check_cancel(cancel)?;
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_refuses_oversized_patch_without_losing_previous_reservations() {
        let mut budget = CatalogBudget::default();
        let patch = CatalogPatch {
            server: Some(ServerInfo::new("fixture", "", Capabilities::empty())),
            ..CatalogPatch::default()
        };
        budget
            .reserve(&CatalogRefreshScope::Root, &patch)
            .expect("initial reservation");
        let oversized = CatalogPatch {
            catalogs: Some(vec![
                CatalogRef::new("db").expect("valid name");
                MAX_CATALOG_OBJECTS + 1
            ]),
            ..CatalogPatch::default()
        };
        assert!(matches!(
            budget.reserve(&CatalogRefreshScope::Root, &oversized),
            Err(OxynError::Config(_))
        ));
        assert_eq!(budget.scopes.get(&CatalogRefreshScope::Root), Some(&1));
        budget
            .reserve(&CatalogRefreshScope::Root, &patch)
            .expect("repeat refresh is not charged twice");
    }

    #[test]
    fn budget_bounds_distinct_scopes_even_when_their_lists_are_empty() {
        let mut budget = CatalogBudget::default();
        for i in 0..MAX_CATALOG_SCOPES {
            budget
                .reserve(
                    &CatalogRefreshScope::Namespaces {
                        catalog: Some(i.to_string()),
                    },
                    &CatalogPatch::default(),
                )
                .expect("within scope budget");
        }
        assert!(
            budget
                .reserve(&CatalogRefreshScope::Root, &CatalogPatch::default())
                .is_err()
        );
        assert_eq!(budget.scopes.len(), MAX_CATALOG_SCOPES);
    }
}
