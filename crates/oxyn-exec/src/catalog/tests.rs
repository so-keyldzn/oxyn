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
    async fn list_relations(&self, _: &CatalogPath, _: &CancelToken) -> Result<Vec<RelationRef>> {
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

    async fn list_relations(&self, _: &CatalogPath, _: &CancelToken) -> Result<Vec<RelationRef>> {
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
async fn incoming_keys_are_capability_gated_and_cancellation_never_publishes_partial_metadata() {
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
