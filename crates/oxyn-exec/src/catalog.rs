//! Lazy catalog reads and atomic cache publication, confined to the bus.

mod budget;

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
    pub refresh: Mutex<CatalogBudget>,
    pub closed: CancelToken,
    /// The first session is reserved for connection-level catalog work.
    pub session: RwLock<Option<oxyn_core::SessionId>>,
}

impl ConnectionCatalog {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(CatalogCache::new())),
            refresh: Mutex::new(CatalogBudget::default()),
            closed: CancelToken::new(),
            session: RwLock::new(None),
        }
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
    // Absent means "not read": a source without the capability must never
    // publish an empty vector, which the tabs would read as "no index at all".
    indexes: Option<(CatalogPath, Vec<Index>)>,
    foreign_keys: Option<(CatalogPath, Vec<ForeignKey>)>,
    constraints: Option<(CatalogPath, Vec<Constraint>)>,
    incoming_keys: Option<(CatalogPath, Vec<IncomingForeignKey>)>,
    definition: Option<(CatalogPath, RelationDefinition)>,
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
            + self
                .constraints
                .as_ref()
                .map_or(0, |(_, values)| values.len())
            + self
                .incoming_keys
                .as_ref()
                .map_or(0, |(_, values)| values.len())
            + usize::from(self.definition.is_some())
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
            patch.relation = Some((target, detail));
        }
        CatalogRefreshScope::Constraints {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::CONSTRAINTS)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            // Describe first so a direct bus request can publish to an empty cache.
            let detail = provider.describe_relation(&target, cancel).await?;
            check_cancel(cancel)?;
            let constraints = provider.list_constraints(&target, cancel).await?;
            patch.constraints = Some((target.clone(), constraints));
            patch.relation = Some((target, detail));
        }
        CatalogRefreshScope::IncomingForeignKeys {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::INCOMING_FOREIGN_KEYS)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            let detail = provider.describe_relation(&target, cancel).await?;
            check_cancel(cancel)?;
            let keys = provider.list_incoming_foreign_keys(&target, cancel).await?;
            patch.incoming_keys = Some((target.clone(), keys));
            patch.relation = Some((target, detail));
        }
        CatalogRefreshScope::Definition {
            catalog,
            namespace,
            relation,
        } => {
            capabilities.require(Capabilities::OBJECT_DEFINITION)?;
            let target = path(catalog, namespace, Some(relation.clone()))?;
            let detail = provider.describe_relation(&target, cancel).await?;
            check_cancel(cancel)?;
            let definition = provider.relation_definition(&target, cancel).await?;
            definition.validate()?;
            patch.definition = Some((target.clone(), definition));
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
    use oxyn_catalog::DefinitionSource;

    struct ConstraintProvider {
        cancel_after_read: bool,
    }
    #[async_trait::async_trait]
    impl CatalogProvider for ConstraintProvider {
        async fn server_info(&self, _: &CancelToken) -> Result<ServerInfo> {
            Ok(ServerInfo::new("fixture", "", Capabilities::CONSTRAINTS))
        }
        async fn list_relations(
            &self,
            _: &CatalogPath,
            _: &CancelToken,
        ) -> Result<Vec<RelationRef>> {
            Ok(vec![])
        }
        async fn describe_relation(&self, path: &CatalogPath, _: &CancelToken) -> Result<Relation> {
            Ok(Relation::new(
                path.relation().unwrap_or_default(),
                oxyn_catalog::RelationKind::Table,
            ))
        }
        async fn list_incoming_foreign_keys(
            &self,
            path: &CatalogPath,
            cancel: &CancelToken,
        ) -> Result<Vec<IncomingForeignKey>> {
            if self.cancel_after_read {
                cancel.cancel();
            }
            Ok(vec![IncomingForeignKey {
                source: CatalogPath::for_relation(None, Some("main"), "child")?,
                key: ForeignKey::new(
                    "declared",
                    vec!["parent_id".into()],
                    oxyn_catalog::ForeignKeyTarget {
                        relation: path.clone(),
                        fields: vec!["id".into()],
                    },
                ),
                source_unique: Some(false),
            }])
        }
        async fn list_constraints(
            &self,
            _: &CatalogPath,
            cancel: &CancelToken,
        ) -> Result<Vec<Constraint>> {
            if self.cancel_after_read {
                cancel.cancel();
            }
            Ok(vec![Constraint::new(
                "real_key",
                oxyn_catalog::ConstraintKind::PrimaryKey,
                vec!["id".into()],
            )])
        }
    }

    struct DefinitionProvider {
        cancel_after_read: bool,
        definition: RelationDefinition,
    }

    #[async_trait::async_trait]
    impl CatalogProvider for DefinitionProvider {
        async fn server_info(&self, _: &CancelToken) -> Result<ServerInfo> {
            Ok(ServerInfo::new(
                "fixture",
                "",
                Capabilities::OBJECT_DEFINITION,
            ))
        }

        async fn list_relations(
            &self,
            _: &CatalogPath,
            _: &CancelToken,
        ) -> Result<Vec<RelationRef>> {
            Ok(vec![])
        }

        async fn describe_relation(&self, path: &CatalogPath, _: &CancelToken) -> Result<Relation> {
            Ok(Relation::new(
                path.relation().unwrap_or_default(),
                oxyn_catalog::RelationKind::View,
            )
            .with_comment("identity from describe"))
        }

        async fn relation_definition(
            &self,
            _: &CatalogPath,
            cancel: &CancelToken,
        ) -> Result<RelationDefinition> {
            if self.cancel_after_read {
                cancel.cancel();
            }
            Ok(self.definition.clone())
        }
    }

    fn definition_fixture() -> RelationDefinition {
        RelationDefinition {
            sql: "CREATE VIEW \"odd\" AS SELECT 1".into(),
            source: DefinitionSource::Reconstructed,
            notes: vec!["check dependent objects".into()],
        }
    }

    #[tokio::test]
    async fn definition_scope_requires_capability_and_publishes_identity_atomically() {
        let scope = CatalogRefreshScope::Definition {
            catalog: None,
            namespace: Some("main".into()),
            relation: "odd\"; name".into(),
        };
        let provider = DefinitionProvider {
            cancel_after_read: false,
            definition: definition_fixture(),
        };
        assert!(matches!(
            read(
                &provider,
                Capabilities::empty(),
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::NotSupported { .. })
        ));

        let path = CatalogPath::for_relation(None, Some("main"), "odd\"; name").expect("path");
        let mut cache = CatalogCache::new();
        let patch = read(
            &provider,
            Capabilities::OBJECT_DEFINITION,
            &scope,
            &CancelToken::new(),
        )
        .await
        .expect("read");
        assert_eq!(patch.object_count(), 2);
        patch.apply(&mut cache).expect("publish");

        assert_eq!(
            cache.relation(&path).expect("relation").kind,
            oxyn_catalog::RelationKind::View
        );
        assert_eq!(
            cache.relation(&path).expect("relation").comment.as_deref(),
            Some("identity from describe")
        );
        let expected = definition_fixture();
        assert_eq!(cache.definition(&path), Some(&expected));
    }

    #[tokio::test]
    async fn definition_scope_rejects_invalid_payload_before_publication() {
        let scope = CatalogRefreshScope::Definition {
            catalog: None,
            namespace: Some("main".into()),
            relation: "invalid".into(),
        };
        let provider = DefinitionProvider {
            cancel_after_read: false,
            definition: RelationDefinition {
                sql: String::new(),
                source: DefinitionSource::Stored,
                notes: vec![],
            },
        };
        let cache = CatalogCache::new();
        assert!(matches!(
            read(
                &provider,
                Capabilities::OBJECT_DEFINITION,
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::CatalogUnavailable(_))
        ));
        let path = CatalogPath::for_relation(None, Some("main"), "invalid").expect("path");
        assert!(cache.relation(&path).is_none());
        assert!(cache.definition(&path).is_none());
    }

    #[tokio::test]
    async fn definition_scope_does_not_publish_after_provider_cancellation() {
        let scope = CatalogRefreshScope::Definition {
            catalog: None,
            namespace: Some("main".into()),
            relation: "cancelled".into(),
        };
        let provider = DefinitionProvider {
            cancel_after_read: true,
            definition: definition_fixture(),
        };
        let cache = CatalogCache::new();
        assert!(matches!(
            read(
                &provider,
                Capabilities::OBJECT_DEFINITION,
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::Cancelled)
        ));
        let path = CatalogPath::for_relation(None, Some("main"), "cancelled").expect("path");
        assert!(cache.relation(&path).is_none());
        assert!(cache.definition(&path).is_none());
    }

    #[tokio::test]
    async fn constraints_scope_is_explicit_capability_gated_and_atomically_published() {
        let provider = ConstraintProvider {
            cancel_after_read: false,
        };
        let path = CatalogPath::for_relation(None, Some("main"), "odd\"; name").expect("path");
        let scope = CatalogRefreshScope::Constraints {
            catalog: None,
            namespace: Some("main".into()),
            relation: "odd\"; name".into(),
        };
        assert!(matches!(
            read(
                &provider,
                Capabilities::empty(),
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::NotSupported { .. })
        ));
        let mut cache = CatalogCache::new();
        let patch = read(
            &provider,
            Capabilities::CONSTRAINTS,
            &scope,
            &CancelToken::new(),
        )
        .await
        .expect("read");
        assert_eq!(patch.object_count(), 2);
        patch
            .apply(&mut cache)
            .expect("publish to an initially empty cache");
        assert_eq!(
            cache
                .constraints(&path)
                .expect("read")
                .first()
                .expect("key")
                .name,
            "real_key"
        );
        assert!(matches!(
            read(
                &ConstraintProvider {
                    cancel_after_read: true
                },
                Capabilities::CONSTRAINTS,
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::Cancelled)
        ));
        assert_eq!(
            cache
                .constraints(&path)
                .expect("previous read preserved")
                .len(),
            1
        );
        let relation_scope = CatalogRefreshScope::Relation {
            catalog: None,
            namespace: Some("main".into()),
            relation: "odd\"; name".into(),
        };
        let ordinary = read(
            &provider,
            Capabilities::CONSTRAINTS,
            &relation_scope,
            &CancelToken::new(),
        )
        .await
        .expect("ordinary description");
        assert!(
            ordinary.constraints.is_none(),
            "opening a table does not request constraints"
        );
    }

    #[tokio::test]
    async fn incoming_keys_are_capability_gated_and_cancellation_never_publishes_partial_metadata()
    {
        let scope = CatalogRefreshScope::IncomingForeignKeys {
            catalog: None,
            namespace: Some("main".into()),
            relation: "parent".into(),
        };
        let provider = ConstraintProvider {
            cancel_after_read: false,
        };
        assert!(matches!(
            read(
                &provider,
                Capabilities::empty(),
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::NotSupported { .. })
        ));
        let mut cache = CatalogCache::new();
        read(
            &provider,
            Capabilities::INCOMING_FOREIGN_KEYS,
            &scope,
            &CancelToken::new(),
        )
        .await
        .expect("read")
        .apply(&mut cache)
        .expect("publish");
        let path = CatalogPath::for_relation(None, Some("main"), "parent").expect("path");
        assert_eq!(cache.incoming_foreign_keys(&path).expect("keys").len(), 1);
        assert!(
            cache.foreign_keys(&path).is_none(),
            "direction is never confused"
        );
        assert!(matches!(
            read(
                &ConstraintProvider {
                    cancel_after_read: true
                },
                Capabilities::INCOMING_FOREIGN_KEYS,
                &scope,
                &CancelToken::new()
            )
            .await,
            Err(OxynError::Cancelled)
        ));
        assert_eq!(
            cache
                .incoming_foreign_keys(&path)
                .expect("previous metadata preserved")
                .len(),
            1
        );
    }
}
