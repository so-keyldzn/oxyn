use super::*;
use oxyn_core::{ConnectionConfig, DriverId, Environment, PrivacyTier};

fn profile() -> ServerProfile {
    ServerProfile {
        id: ServerProfileId::new(),
        name: "Development server".into(),
        driver: DriverId::postgres(),
        params: [("host".into(), "localhost".into())].into_iter().collect(),
    }
}

#[test]
fn grouping_and_removing_a_profile_never_rewrites_children_or_their_secrets() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("grouping").expect("workspace").id;
    let child = ConnectionConfig::new("child", DriverId::postgres())
        .with_environment(Environment::Production)
        .with_privacy_tier(PrivacyTier::Local)
        .with_param("database", "example")
        .with_secret_ref("synthetic-reference-not-a-credential")
        .read_only();
    store.connections().save(workspace, &child).expect("child");
    let profile = profile();
    store
        .server_profiles()
        .save(workspace, &profile, None)
        .expect("profile");
    store
        .server_profiles()
        .set_membership(workspace, child.id, Some(profile.id), None)
        .expect("link");
    let listed = store.server_profiles().list(workspace).expect("list");
    assert_eq!(listed[0].connections, vec![child.id]);
    assert_eq!(
        store.connections().get(child.id).expect("child"),
        Some(child.clone())
    );
    store
        .server_profiles()
        .remove(workspace, profile.id, 1)
        .expect("remove group");
    assert!(
        store
            .server_profiles()
            .list(workspace)
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        store.connections().get(child.id).expect("child"),
        Some(child)
    );
}

#[test]
fn stale_edits_and_cross_workspace_writes_are_refused() {
    let store = Store::open_in_memory().expect("store");
    let a = store.workspaces().create("a").expect("workspace").id;
    let b = store.workspaces().create("b").expect("workspace").id;
    let mut profile = profile();
    store
        .server_profiles()
        .save(a, &profile, None)
        .expect("create");
    profile.name = "new name".into();
    assert_eq!(
        store
            .server_profiles()
            .save(a, &profile, Some(1))
            .expect("update")
            .revision,
        2
    );
    assert!(matches!(
        store.server_profiles().save(a, &profile, Some(1)),
        Err(StoreError::ServerProfileConflict)
    ));
    assert!(matches!(
        store.server_profiles().save(b, &profile, Some(2)),
        Err(StoreError::ServerProfileConflict)
    ));
    assert!(store.server_profiles().remove(b, profile.id, 2).is_err());
    assert!(store.server_profiles().remove(a, profile.id, 1).is_err());
    assert!(
        store
            .server_profiles()
            .list(b)
            .expect("other workspace")
            .is_empty()
    );
    assert_eq!(
        store.server_profiles().list(a).expect("list")[0].revision,
        2
    );
}

#[test]
fn membership_requires_workspace_protocol_and_the_observed_previous_group() {
    let store = Store::open_in_memory().expect("store");
    let a = store.workspaces().create("a").expect("workspace").id;
    let b = store.workspaces().create("b").expect("workspace").id;
    let first = profile();
    let mut second = profile();
    second.name = "second".into();
    for profile in [&first, &second] {
        store
            .server_profiles()
            .save(a, profile, None)
            .expect("profile");
    }
    let foreign = ConnectionConfig::new("foreign", DriverId::postgres());
    let wrong_driver = ConnectionConfig::new("wrong protocol", DriverId::sqlite());
    let child = ConnectionConfig::new("child", DriverId::postgres());
    store.connections().save(b, &foreign).expect("foreign");
    store
        .connections()
        .save(a, &wrong_driver)
        .expect("wrong driver");
    store.connections().save(a, &child).expect("child");
    assert!(
        store
            .server_profiles()
            .set_membership(a, foreign.id, Some(first.id), None)
            .is_err()
    );
    assert!(
        store
            .server_profiles()
            .set_membership(a, wrong_driver.id, Some(first.id), None)
            .is_err()
    );
    store
        .server_profiles()
        .set_membership(a, child.id, Some(first.id), None)
        .expect("first");
    assert!(matches!(
        store
            .server_profiles()
            .set_membership(a, child.id, Some(second.id), None),
        Err(StoreError::ServerProfileConflict)
    ));
    store
        .server_profiles()
        .set_membership(a, child.id, Some(second.id), Some(first.id))
        .expect("move");
    assert!(
        store
            .server_profiles()
            .set_membership(a, child.id, None, Some(first.id))
            .is_err()
    );
    store
        .server_profiles()
        .set_membership(a, child.id, None, Some(second.id))
        .expect("unlink");
    assert!(
        store
            .server_profiles()
            .list(a)
            .expect("list")
            .iter()
            .all(|p| p.connections.is_empty())
    );
}

#[test]
fn secret_defaults_and_corrupt_stored_defaults_are_never_returned() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("a").expect("workspace").id;
    let mut profile = profile();
    profile
        .params
        .insert("password".into(), "synthetic-sensitive-value".into());
    let error = store
        .server_profiles()
        .save(workspace, &profile, None)
        .expect_err("refused");
    assert!(!error.to_string().contains("synthetic-sensitive-value"));
    profile.params.shift_remove("password");
    store
        .server_profiles()
        .save(workspace, &profile, None)
        .expect("valid");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE server_profiles SET params = ?1",
                [r#"{"password":"synthetic-sensitive-value"}"#],
            )?;
            Ok(())
        })
        .expect("corrupt fixture");
    let error = store
        .server_profiles()
        .list(workspace)
        .expect_err("corruption explicit");
    assert!(!error.to_string().contains("synthetic-sensitive-value"));
}

#[test]
fn database_itself_refuses_reparenting_across_workspace_boundaries() {
    let store = Store::open_in_memory().expect("store");
    let a = store.workspaces().create("a").expect("workspace").id;
    let b = store.workspaces().create("b").expect("workspace").id;
    let profile = profile();
    let child = ConnectionConfig::new("child", DriverId::postgres());
    store.connections().save(b, &child).expect("child");
    store
        .server_profiles()
        .save(a, &profile, None)
        .expect("profile");
    assert!(
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO server_profile_members VALUES (?1, ?2)",
                    params![child.id.to_string(), profile.id.to_string()],
                )?;
                Ok(())
            })
            .is_err()
    );
}

#[test]
fn migration_from_nineteen_keeps_existing_connection_identity() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("workspace.sqlite3");
    let connection = crate::schema::file_at_version(&path, 19);
    let workspace = WorkspaceId::new();
    let child = ConnectionId::new();
    connection
        .execute(
            "INSERT INTO workspaces VALUES (?1, 'existing', '2026-11-13', '2026-11-13')",
            [workspace.to_string()],
        )
        .expect("legacy workspace");
    connection.execute(
        "INSERT INTO connections(id, workspace_id, name, driver, environment, params, secret_ref,
        read_only, created_at, updated_at, privacy_tier)
        VALUES (?1, ?2, 'existing', 'postgres', 'production', '{}', 'synthetic-ref', 1,
        '2026-11-13', '2026-11-13', 'local')",
        params![child.to_string(), workspace.to_string()],
    ).expect("legacy connection");
    drop(connection);
    let store = Store::open_at(&path).expect("migration");
    assert_eq!(
        store.schema_version().expect("version"),
        crate::schema::latest_version()
    );
    assert!(
        store
            .server_profiles()
            .list(workspace)
            .expect("empty groups")
            .is_empty()
    );
    let restored = store
        .connections()
        .get(child)
        .expect("read")
        .expect("preserved");
    assert_eq!(restored.id, child);
    assert_eq!(restored.secret_ref.as_deref(), Some("synthetic-ref"));
    assert!(restored.read_only);
}

#[test]
fn unreadable_and_excessively_encoded_defaults_fail_without_echoing_values() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("bounded").expect("workspace").id;
    let mut profile = profile();
    profile.params = (0..32)
        .map(|i| (format!("field_{i}"), "\\".repeat(4096)))
        .collect();
    assert!(
        store
            .server_profiles()
            .save(workspace, &profile, None)
            .is_err()
    );
    assert!(
        store
            .server_profiles()
            .list(workspace)
            .expect("no partial write")
            .is_empty()
    );
    profile.params.clear();
    store
        .server_profiles()
        .save(workspace, &profile, None)
        .expect("valid");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE server_profiles SET params = ?1",
                [r#"{"host":987654321}"#],
            )?;
            Ok(())
        })
        .expect("malformed local fixture");
    let error = store
        .server_profiles()
        .list(workspace)
        .expect_err("explicit failure");
    assert!(!error.to_string().contains("987654321"));
}

#[test]
fn revision_exhaustion_never_rounds_through_javascript() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("revision").expect("workspace").id;
    let profile = profile();
    store
        .server_profiles()
        .save(workspace, &profile, None)
        .expect("create");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE server_profiles SET revision = ?1",
                [sql_revision(MAX_PROFILE_REVISION)?],
            )?;
            Ok(())
        })
        .expect("last safe revision");
    assert!(
        store
            .server_profiles()
            .save(workspace, &profile, Some(MAX_PROFILE_REVISION))
            .is_err()
    );
    assert_eq!(
        store.server_profiles().list(workspace).expect("unchanged")[0].revision,
        MAX_PROFILE_REVISION
    );
}

#[test]
fn discovered_child_creation_is_atomic_and_never_reuses_seed_credentials() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("children").expect("workspace").id;
    let mut group = profile();
    group
        .params
        .insert("user".into(), "future_default_role".into());
    store
        .server_profiles()
        .save(workspace, &group, None)
        .expect("group");
    let seed = ConnectionConfig::new("seed", DriverId::postgres())
        .with_param("host", "localhost")
        .with_param("database", "seed")
        .with_param("user", "source_role")
        .with_secret_ref("synthetic-seed-reference");
    store.connections().save(workspace, &seed).expect("seed");
    let child = ConnectionConfig::new("child", DriverId::postgres())
        .with_param("host", "localhost")
        .with_param("database", "other")
        .with_param("user", "child_role")
        .with_secret_ref("synthetic-child-reference");
    let mut invalid = child.clone();
    invalid.secret_ref = seed.secret_ref.clone();
    assert!(
        store
            .server_profiles()
            .create_child(workspace, group.id, seed.id, &invalid)
            .is_err()
    );
    assert!(store.connections().get(child.id).expect("read").is_none());
    invalid = child.clone();
    invalid
        .params
        .insert("host".into(), "elsewhere.invalid".into());
    assert!(
        store
            .server_profiles()
            .create_child(workspace, group.id, seed.id, &invalid)
            .is_err()
    );
    assert!(store.connections().get(child.id).expect("read").is_none());
    store
        .server_profiles()
        .create_child(workspace, group.id, seed.id, &child)
        .expect("create");
    assert_eq!(
        store.server_profiles().list(workspace).expect("groups")[0].connections,
        vec![child.id]
    );
    assert_eq!(
        store.connections().get(seed.id).expect("seed"),
        Some(seed.clone())
    );
    assert!(
        store
            .server_profiles()
            .create_child(workspace, group.id, seed.id, &child)
            .is_err()
    );
    let other = store.workspaces().create("other").expect("workspace").id;
    let fresh = ConnectionConfig {
        id: ConnectionId::new(),
        ..child
    };
    assert!(
        store
            .server_profiles()
            .create_child(other, group.id, seed.id, &fresh)
            .is_err()
    );
    assert!(store.connections().get(fresh.id).expect("read").is_none());
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET params = ?1 WHERE id = ?2",
                params![
                    serde_json::to_string("synthetic-sensitive-seed-value")?,
                    seed.id.to_string()
                ],
            )?;
            Ok(())
        })
        .expect("malformed workspace fixture");
    let error = store
        .server_profiles()
        .create_child(workspace, group.id, seed.id, &fresh)
        .expect_err("refuse corrupt seed");
    assert!(!error.to_string().contains("synthetic-sensitive-seed-value"));
    assert!(
        store
            .connections()
            .get(fresh.id)
            .expect("no partial child")
            .is_none()
    );
}
