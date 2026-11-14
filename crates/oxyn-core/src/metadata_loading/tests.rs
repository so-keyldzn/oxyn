use super::*;

#[test]
fn exact_scope_inheritance_preserves_homonyms_and_optional_native_levels() {
    let first = MetadataScope {
        catalog: Some("a".into()),
        namespace: Some("public".into()),
    };
    let second = MetadataScope {
        catalog: Some("b".into()),
        namespace: Some("public".into()),
    };
    let sqlite = MetadataScope {
        catalog: None,
        namespace: Some("main".into()),
    };
    let redis = MetadataScope {
        catalog: Some("db0".into()),
        namespace: None,
    };
    let policy = MetadataLoadingPolicy {
        overrides: vec![
            MetadataOverride {
                scope: first.clone(),
                level: MetadataLevel::Extended,
            },
            MetadataOverride {
                scope: sqlite.clone(),
                level: MetadataLevel::Structure,
            },
            MetadataOverride {
                scope: redis.clone(),
                level: MetadataLevel::Extended,
            },
        ],
        ..MetadataLoadingPolicy::default()
    };
    policy.validate().expect("native scopes");
    assert_eq!(policy.effective(&first), MetadataLevel::Extended);
    assert_eq!(policy.effective(&second), MetadataLevel::Minimal);
    assert_eq!(policy.effective(&sqlite), MetadataLevel::Structure);
    assert_eq!(policy.effective(&redis), MetadataLevel::Extended);
    assert_eq!(
        policy.effective(&MetadataScope::default()),
        MetadataLevel::Minimal
    );
}

#[test]
fn malformed_duplicate_and_excessive_policies_are_refused_without_echoing_identifiers() {
    let mut policy = MetadataLoadingPolicy {
        version: 2,
        ..Default::default()
    };
    assert!(policy.validate().is_err());
    policy.version = 1;
    let scope = MetadataScope {
        catalog: None,
        namespace: Some("synthetic-private-namespace".into()),
    };
    let item = MetadataOverride {
        scope,
        level: MetadataLevel::Structure,
    };
    policy.overrides = vec![item.clone(), item.clone()];
    assert!(policy.validate().is_err());
    assert!(!format!("{policy:?}").contains("synthetic-private-namespace"));
    policy.overrides = vec![item; MAX_METADATA_OVERRIDES + 1];
    assert!(policy.validate().is_err());
    let bad = MetadataScope {
        catalog: None,
        namespace: Some("synthetic\nprivate".into()),
    };
    assert!(
        !bad.validate()
            .expect_err("control")
            .to_string()
            .contains("synthetic")
    );
    assert!(
        MetadataScope {
            catalog: Some("x".repeat(MAX_METADATA_SCOPE_BYTES + 1)),
            namespace: None
        }
        .validate()
        .is_err()
    );
}

#[test]
fn continuation_is_bound_to_the_reviewed_native_scope_and_policy() {
    let scope = MetadataScope {
        catalog: None,
        namespace: Some("main".into()),
    };
    let mut request = super::MetadataPreloadRequest {
        connection: crate::ConnectionId::new(),
        session: crate::SessionId::new(),
        scope: scope.clone(),
        expected_revision: 1,
        refresh: false,
        after: Some(super::MetadataPreloadCursor {
            generation: uuid::Uuid::now_v7(),
            scope,
            policy_revision: 1,
            after: "orders".into(),
        }),
    };
    request.validate().expect("same page");
    request.expected_revision = 2;
    assert!(request.validate().is_err());
    request.expected_revision = 1;
    request.scope.namespace = Some("attached".into());
    assert!(
        request.validate().is_err(),
        "homonymous object in another native scope"
    );
}
