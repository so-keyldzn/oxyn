use super::*;
use oxyn_core::{
    Appearance, ConnectionId, ObjectLocation, ObjectSection, ReadingDensity, WorkspacePreferences,
};

#[test]
fn preferences_survive_reopening_and_old_writes_cannot_replace_newer_ones() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("state.sqlite3");
    let store = Store::open_at(&path).expect("store");
    let workspace = store
        .workspaces()
        .create("preferences")
        .expect("workspace")
        .id;
    let mut newer = PreferencesSnapshot {
        revision: 2,
        ..Default::default()
    };
    newer.preferences.reading_density = ReadingDensity::Comfortable;
    newer.preferences.inspector_width = 340;
    store
        .preferences()
        .save(workspace, &newer)
        .expect("save newer");
    let older = PreferencesSnapshot {
        revision: 1,
        ..Default::default()
    };
    assert_eq!(
        store
            .preferences()
            .save(workspace, &older)
            .expect("stale write"),
        newer
    );
    let mut conflict = newer.clone();
    conflict.preferences.appearance = Appearance::Light;
    assert!(store.preferences().save(workspace, &conflict).is_err());
    drop(store);
    let reopened = Store::open_at(path).expect("reopen");
    assert_eq!(
        reopened
            .preferences()
            .load(workspace)
            .expect("persisted snapshot"),
        newer
    );
    reopened
        .workspaces()
        .delete(workspace)
        .expect("delete workspace");
    assert!(reopened.preferences().load(workspace).is_err());
}

#[test]
fn the_longest_storable_location_still_fits_under_the_payload_cap() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store
        .workspaces()
        .create("preferences")
        .expect("workspace")
        .id;
    // The payload cap is a CHECK: a location that passes validation must
    // never be the reason an unrelated preference fails to save.
    let path = format!(
        "db.public.{}",
        "n".repeat(ObjectLocation::MAX_PATH_BYTES - 11)
    );
    let snapshot = PreferencesSnapshot {
        revision: 1,
        preferences: WorkspacePreferences {
            null_text: "x".repeat(64),
            object_location: ObjectLocation::new(
                ConnectionId::new(),
                path,
                ObjectSection::Constraints,
            ),
            ..Default::default()
        },
    };
    assert_eq!(
        store
            .preferences()
            .save(workspace, &snapshot)
            .expect("saved location"),
        snapshot
    );
    assert_eq!(
        store.preferences().load(workspace).expect("reloaded"),
        snapshot
    );
}

#[test]
fn defaults_and_invalid_payloads_are_explicit_without_echoing_stored_text() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store
        .workspaces()
        .create("preferences")
        .expect("workspace")
        .id;
    assert_eq!(
        store.preferences().load(workspace).expect("defaults"),
        PreferencesSnapshot::default()
    );
    let mut invalid_snapshot = PreferencesSnapshot {
        revision: 1,
        ..Default::default()
    };
    invalid_snapshot.preferences.null_text = "x".repeat(65);
    assert!(
        store
            .preferences()
            .save(workspace, &invalid_snapshot)
            .is_err()
    );
    assert_eq!(
        store
            .preferences()
            .load(workspace)
            .expect("unchanged")
            .revision,
        0
    );
    store
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO workspace_preferences VALUES (?1, 1, ?2, ?3)",
                params![
                    workspace.to_string(),
                    r#"{"appearance":"do not echo this stored text"}"#,
                    Utc::now()
                ],
            )?;
            Ok(())
        })
        .expect("external malformed state");
    let error = store
        .preferences()
        .load(workspace)
        .expect_err("invalid enum")
        .to_string();
    assert!(!error.contains("do not echo"));
    let repair = PreferencesSnapshot {
        revision: 2,
        ..Default::default()
    };
    assert!(
        store.preferences().save(workspace, &repair).is_err(),
        "unreadable state is never silently replaced"
    );
    store
        .with_connection(|connection| {
            let payload: String = connection.query_row(
                "SELECT payload FROM workspace_preferences WHERE workspace_id=?1",
                [workspace.to_string()],
                |row| row.get(0),
            )?;
            assert!(payload.contains("do not echo"));
            Ok(())
        })
        .expect("payload preserved");
}
