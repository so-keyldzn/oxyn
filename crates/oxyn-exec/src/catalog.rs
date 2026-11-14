//! Lazy catalog reads and atomic cache publication, confined to the bus.

mod budget;
mod payload;
pub(crate) mod priority;

use std::sync::Arc;

use oxyn_catalog::RelationDefinition;
use oxyn_catalog::{
    CatalogCache, CatalogPath, CatalogProvider, CatalogRef, Constraint, ForeignKey,
    IncomingForeignKey, Index, NamespaceRef, Relation, RelationRef, ServerInfo, SharedCatalog,
};
use oxyn_core::{CancelToken, Capabilities, CatalogRefreshScope, OxynError, Result};
use parking_lot::RwLock;
use tokio::sync::Mutex;

pub(crate) use budget::CatalogBudget;

pub(crate) struct ConnectionCatalog {
    pub cache: SharedCatalog,
    pub refresh: Arc<Mutex<CatalogBudget>>,
    pub queue: Arc<priority::Queue>,
    pub closed: CancelToken,
    /// The first session is reserved for connection-level catalog work.
    pub session: RwLock<Option<oxyn_core::SessionId>>,
}

impl ConnectionCatalog {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(CatalogCache::new())),
            refresh: Arc::new(Mutex::new(CatalogBudget::default())),
            queue: Arc::new(priority::Queue::default()),
            closed: CancelToken::new(),
            session: RwLock::new(None),
        }
    }
}

pub(crate) struct SessionCatalog {
    pub cache: SharedCatalog,
    closed: parking_lot::Mutex<bool>,
}

impl Default for SessionCatalog {
    fn default() -> Self {
        Self {
            cache: Arc::new(RwLock::new(CatalogCache::new())),
            closed: parking_lot::Mutex::new(false),
        }
    }
}

impl SessionCatalog {
    pub fn publish_checked<T>(
        &self,
        generation: oxyn_catalog::CatalogGenerationId,
        publish: impl FnOnce(&mut CatalogCache) -> Result<T>,
    ) -> Result<T> {
        let closed = self.closed.lock();
        if *closed {
            return Err(OxynError::Cancelled);
        }
        let mut cache = self.cache.write();
        if cache.generation() != generation {
            return Err(OxynError::Cancelled);
        }
        publish(&mut cache)
    }

    pub fn close(&self) {
        let mut closed = self.closed.lock();
        *closed = true;
        *self.cache.write() = CatalogCache::new();
    }
}

/// A single successful read, published only after releasing the session guard.
#[derive(Default, serde::Serialize)]
pub(crate) struct CatalogPatch {
    server: Option<ServerInfo>,
    catalogs: Option<Vec<CatalogRef>>,
    namespaces: Option<(Option<String>, Vec<NamespaceRef>)>,
    relations: Option<(CatalogPath, Vec<RelationRef>)>,
    relation: Option<(CatalogPath, Relation)>,
    summary: Option<(CatalogPath, RelationRef)>,
    aggregate: Option<CatalogPath>,
    // Absent means "not read": a source without the capability must never
    // publish an empty vector, which the tabs would read as "no index at all".
    indexes: Option<(CatalogPath, Vec<Index>)>,
    foreign_keys: Option<(CatalogPath, Vec<ForeignKey>)>,
    constraints: Option<(CatalogPath, Vec<Constraint>)>,
    incoming_keys: Option<(CatalogPath, Vec<IncomingForeignKey>)>,
    definition: Option<(CatalogPath, RelationDefinition)>,
    // Absent means "not read" or "the source cannot say", never "empty".
    resolution_path: Option<Vec<String>>,
    #[serde(skip)]
    encoded_size: Option<usize>,
}

impl CatalogPatch {
    fn object_count(&self) -> usize {
        usize::from(self.server.is_some())
            + usize::from(self.summary.is_some())
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
            + self
                .constraints
                .as_ref()
                .map_or(0, |(_, values)| values.len())
            + self
                .incoming_keys
                .as_ref()
                .map_or(0, |(_, values)| values.len())
            + usize::from(self.definition.is_some())
            + self.resolution_path.as_ref().map_or(0, Vec::len)
            + self.indexes.as_ref().map_or(0, |(_, values)| values.len())
            + self
                .foreign_keys
                .as_ref()
                .map_or(0, |(_, values)| values.len())
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
        if let Some((path, summary)) = self.summary {
            cache.set_relation_summary(&path, summary)?;
        }
        if let Some((path, relation)) = self.relation {
            cache.set_relation(&path, relation)?;
        }
        // After `set_relation`, never before: the cache refuses to attach
        // indexes to a relation it has neither listed nor described.
        if let Some((path, indexes)) = self.indexes {
            cache.set_indexes(&path, indexes)?;
        }
        if let Some((path, keys)) = self.foreign_keys {
            cache.set_foreign_keys(&path, keys)?;
        }
        if let Some((path, constraints)) = self.constraints {
            cache.set_constraints(&path, constraints)?;
        }
        if let Some((path, keys)) = self.incoming_keys {
            cache.set_incoming_foreign_keys(&path, keys)?;
        }
        if let Some((path, definition)) = self.definition {
            cache.set_definition(&path, definition)?;
        }
        if let Some(path) = self.aggregate {
            cache.mark_relation_aggregate(&path)?;
        }
        if let Some(namespaces) = self.resolution_path {
            cache.set_resolution_path(namespaces);
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

pub(crate) fn relation_target(scope: &CatalogRefreshScope) -> Result<Option<CatalogPath>> {
    match scope {
        CatalogRefreshScope::Relation {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::Description {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::SampledDescription {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::Indexes {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::OutgoingForeignKeys {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::Constraints {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::IncomingForeignKeys {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::Definition {
            catalog,
            namespace,
            relation,
        } => path(catalog, namespace, Some(relation.clone())).map(Some),
        _ => Ok(None),
    }
}

async fn observed_summary(
    provider: &dyn CatalogProvider,
    target: &CatalogPath,
    known: Option<RelationRef>,
    cancel: &CancelToken,
) -> Result<RelationRef> {
    if let Some(summary) = known {
        return Ok(summary);
    }
    let parent = target
        .parent()
        .ok_or_else(|| OxynError::Config("relation needs a parent scope".into()))?;
    provider
        .list_relations(&parent, cancel)
        .await?
        .into_iter()
        .find(|summary| Some(summary.name()) == target.relation())
        .ok_or_else(|| {
            OxynError::Query("the requested object is no longer listed in its namespace".into())
        })
}

pub(crate) async fn read(
    provider: &dyn CatalogProvider,
    capabilities: Capabilities,
    scope: &CatalogRefreshScope,
    cancel: &CancelToken,
    summary: Option<RelationRef>,
) -> Result<CatalogPatch> {
    check_cancel(cancel)?;
    if capabilities.contains(Capabilities::INFERRED_SCHEMA)
        && matches!(
            scope,
            CatalogRefreshScope::Relation { .. } | CatalogRefreshScope::Description { .. }
        )
    {
        return Err(OxynError::NotSupported {
            capability: "explicit schema sampling required".into(),
        });
    }
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
            let mut detail = provider.describe_relation(&target, cancel).await?;
            oxyn_catalog::nesting::bound(&mut detail.fields);
            // Session capabilities gate these two reads. Calling anyway would
            // turn `NotSupported` into a failed refresh of the whole relation,
            // and swallowing that error would leave the tabs empty with no
            // explanation — which a human reads as "this table has none".
            if capabilities.contains(Capabilities::INDEXES) {
                check_cancel(cancel)?;
                let indexes = provider.list_indexes(&target, cancel).await?;
                patch.indexes = Some((target.clone(), indexes));
            }
            if capabilities.contains(Capabilities::FOREIGN_KEYS) {
                check_cancel(cancel)?;
                let keys = provider.list_foreign_keys(&target, cancel).await?;
                patch.foreign_keys = Some((target.clone(), keys));
            }
            patch.aggregate = Some(target.clone());
            patch.relation = Some((target, detail));
        }
        CatalogRefreshScope::Description {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::SampledDescription {
            catalog,
            namespace,
            relation,
        } => {
            if matches!(scope, CatalogRefreshScope::SampledDescription { .. }) {
                capabilities.require(Capabilities::INFERRED_SCHEMA)?;
                provider
                    .schema_sample_limits()
                    .ok_or_else(|| OxynError::NotSupported {
                        capability: "bounded schema inference".into(),
                    })?
                    .validate()?;
            }
            let target = path(catalog, namespace, Some(relation.clone()))?;
            let mut detail = provider.describe_relation(&target, cancel).await?;
            oxyn_catalog::nesting::bound(&mut detail.fields);
            patch.relation = Some((target, detail));
        }
        CatalogRefreshScope::Indexes {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::INDEXES)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            patch.summary = Some((
                target.clone(),
                observed_summary(provider, &target, summary, cancel).await?,
            ));
            patch.indexes = Some((
                target.clone(),
                provider.list_indexes(&target, cancel).await?,
            ));
        }
        CatalogRefreshScope::OutgoingForeignKeys {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::FOREIGN_KEYS)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            patch.summary = Some((
                target.clone(),
                observed_summary(provider, &target, summary, cancel).await?,
            ));
            patch.foreign_keys = Some((
                target.clone(),
                provider.list_foreign_keys(&target, cancel).await?,
            ));
        }
        CatalogRefreshScope::Constraints {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::CONSTRAINTS)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            patch.summary = Some((
                target.clone(),
                observed_summary(provider, &target, summary, cancel).await?,
            ));
            check_cancel(cancel)?;
            let constraints = provider.list_constraints(&target, cancel).await?;
            patch.constraints = Some((target.clone(), constraints));
        }
        CatalogRefreshScope::IncomingForeignKeys {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::INCOMING_FOREIGN_KEYS)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            patch.summary = Some((
                target.clone(),
                observed_summary(provider, &target, summary, cancel).await?,
            ));
            check_cancel(cancel)?;
            let keys = provider.list_incoming_foreign_keys(&target, cancel).await?;
            patch.incoming_keys = Some((target.clone(), keys));
        }
        CatalogRefreshScope::Definition {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::OBJECT_DEFINITION)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            patch.summary = Some((
                target.clone(),
                observed_summary(provider, &target, summary, cancel).await?,
            ));
            check_cancel(cancel)?;
            let definition = provider.relation_definition(&target, cancel).await?;
            definition.validate()?;
            patch.definition = Some((target.clone(), definition));
        }
        CatalogRefreshScope::ResolutionPath => match provider.resolution_path(cancel).await {
            Ok(namespaces) => patch.resolution_path = Some(namespaces),
            // A source that cannot say publishes nothing: the cache keeps
            // "unknown". Failing the pass, or publishing an empty path, would
            // each tell the completion something the source never said.
            Err(OxynError::NotSupported { .. }) => {}
            Err(other) => return Err(other),
        },
        _ => {
            return Err(OxynError::NotSupported {
                capability: "catalog scope".to_owned(),
            });
        }
    }
    check_cancel(cancel)?;
    patch.encoded_size = Some(patch.encoded_bytes()?);
    Ok(patch)
}

#[cfg(test)]
mod tests;
