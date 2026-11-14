use super::*;
use oxyn_core::{ConnectionConfig, DriverId, Environment, MetadataLevel};

fn fixture() -> (Store, WorkspaceId, ConnectionConfig) {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("metadata").expect("workspace").id;
    let connection = ConnectionConfig::new("source", DriverId::sqlite())
        .with_param("path", "synthetic.sqlite")
        .with_secret_ref("synthetic-reference")
        .with_environment(Environment::Production)
        .read_only();
    store
        .connections()
        .save(workspace, &connection)
        .expect("source");
    (store, workspace, connection)
}

#[test]
fn policy_persistence_keeps_authentication_authority_and_missing_defaults_separate() {
    let (store, workspace, source) = fixture();
    assert_eq!(
        store
            .metadata_policies()
            .load(workspace, source.id)
            .expect("defaults"),
        MetadataPolicySnapshot::default()
    );
    let policy = MetadataLoadingPolicy {
        default: MetadataLevel::Extended,
        ..MetadataLoadingPolicy::default()
    };
    let saved = store
        .metadata_policies()
        .save(workspace, source.id, &policy, 0)
        .expect("save");
    assert_eq!(saved.revision, 1);
    assert_eq!(
        store
            .metadata_policies()
            .load(workspace, source.id)
            .expect("load"),
        saved
    );
    assert_eq!(
        store.connections().get(source.id).expect("source"),
        Some(source)
    );
}

#[test]
fn stale_revisions_foreign_workspaces_and_exhaustion_are_refused() {
    let (store, workspace, source) = fixture();
    let policy = MetadataLoadingPolicy::default();
    store
        .metadata_policies()
        .save(workspace, source.id, &policy, 0)
        .expect("first");
    assert!(
        store
            .metadata_policies()
            .save(workspace, source.id, &policy, 0)
            .is_err()
    );
    let other = store.workspaces().create("other").expect("other").id;
    assert!(store.metadata_policies().load(other, source.id).is_err());
    assert!(
        store
            .metadata_policies()
            .save(other, source.id, &policy, 1)
            .is_err()
    );
    assert!(
        store
            .metadata_policies()
            .save(workspace, source.id, &policy, MAX_METADATA_POLICY_REVISION)
            .is_err()
    );
    assert_eq!(
        store
            .metadata_policies()
            .load(workspace, source.id)
            .expect("unchanged")
            .revision,
        1
    );
}

#[test]
fn corrupt_unknown_and_oversized_payloads_fail_without_disclosing_content() {
    let (store, workspace, source) = fixture();
    for payload in [
        "synthetic-sensitive-invalid-json".to_owned(),
        r#"{"version":2,"default":"minimal","overrides":[]}"#.to_owned(),
        r#"{"version":1,"default":"minimal","overrides":[],"password":"synthetic-sensitive"}"#
            .to_owned(),
        "synthetic-sensitive".repeat(MAX_METADATA_POLICY_BYTES),
    ] {
        store
            .with_connection(|sql| {
                sql.execute(
                    "INSERT OR REPLACE INTO metadata_loading_policies VALUES (?1, 1, ?2)",
                    params![source.id.to_string(), payload],
                )?;
                Ok(())
            })
            .expect("tampered fixture");
        let failure = store
            .metadata_policies()
            .load(workspace, source.id)
            .expect_err("invalid state");
        assert!(!failure.to_string().contains("synthetic-sensitive"));
    }
}

#[test]
fn migration_from_twenty_keeps_connection_identity_and_uses_lazy_defaults() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("workspace.sqlite3");
    let sql = crate::schema::file_at_version(&path, 20);
    let workspace = WorkspaceId::new();
    let connection = ConnectionId::new();
    sql.execute(
        "INSERT INTO workspaces VALUES (?1, 'existing', '2026-11-14', '2026-11-14')",
        [workspace.to_string()],
    )
    .expect("workspace");
    sql.execute("INSERT INTO connections(id, workspace_id, name, driver, environment, params, secret_ref, read_only, created_at, updated_at, privacy_tier) VALUES (?1, ?2, 'existing', 'sqlite', 'production', '{}', 'synthetic-ref', 1, '2026-11-14', '2026-11-14', 'local')", params![connection.to_string(), workspace.to_string()]).expect("connection");
    drop(sql);
    let store = Store::open_at(&path).expect("migration");
    assert_eq!(
        store
            .metadata_policies()
            .load(workspace, connection)
            .expect("default"),
        MetadataPolicySnapshot::default()
    );
    let restored = store
        .connections()
        .get(connection)
        .expect("source")
        .expect("retained");
    assert_eq!(restored.secret_ref.as_deref(), Some("synthetic-ref"));
    assert!(restored.read_only);
}
