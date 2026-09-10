//! Atomic, monotonic snapshots stored as readable JSON beside a workspace.

use crate::{Result, Store, StoreError};
use chrono::Utc;
use oxyn_core::{PreferencesSnapshot, WorkspaceId};
use rusqlite::{OptionalExtension, params};

/// Typed access to local display preferences. All methods may block.
#[derive(Debug)]
pub struct Preferences<'a> {
    store: &'a Store,
}

impl<'a> Preferences<'a> {
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Loads an existing workspace's snapshot, or defaults when no row was saved.
    pub fn load(&self, workspace: WorkspaceId) -> Result<PreferencesSnapshot> {
        self.store
            .with_connection(|connection| read(connection, workspace))
    }

    /// Stores a newer snapshot atomically; an older write returns the current state.
    /// Equal revisions with different content are rejected instead of overwriting.
    pub fn save(
        &self,
        workspace: WorkspaceId,
        snapshot: &PreferencesSnapshot,
    ) -> Result<PreferencesSnapshot> {
        snapshot
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        if snapshot.revision == 0 {
            return Err(invalid("a saved revision must be positive"));
        }
        let revision =
            i64::try_from(snapshot.revision).map_err(|_| invalid("revision out of range"))?;
        let payload = serde_json::to_string(&snapshot.preferences)?;
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            read(&transaction, workspace)?;
            transaction.execute("INSERT INTO workspace_preferences (workspace_id, revision, payload, updated_at) VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(workspace_id) DO UPDATE SET revision=excluded.revision, payload=excluded.payload, updated_at=excluded.updated_at
                WHERE excluded.revision > workspace_preferences.revision", params![workspace.to_string(), revision, payload, Utc::now()])?;
            let current = read(&transaction, workspace)?;
            if current.revision == snapshot.revision && current.preferences != snapshot.preferences {
                return Err(invalid("preference revision conflict; reload before saving again"));
            }
            transaction.commit()?;
            Ok(current)
        })
    }
}

fn invalid(detail: impl Into<String>) -> StoreError {
    StoreError::Corrupted {
        field: "workspace_preferences",
        detail: detail.into(),
    }
}

fn read(connection: &rusqlite::Connection, workspace: WorkspaceId) -> Result<PreferencesSnapshot> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?1)",
        [workspace.to_string()],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(invalid("workspace does not exist"));
    }
    let row: Option<(i64, Option<String>)> = connection.query_row("SELECT revision, CASE WHEN length(CAST(payload AS BLOB)) <= 4096 THEN payload ELSE NULL END FROM workspace_preferences WHERE workspace_id=?1", [workspace.to_string()], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let Some((revision, payload)) = row else {
        return Ok(PreferencesSnapshot::default());
    };
    let payload = payload.ok_or_else(|| invalid("payload exceeds the supported size"))?;
    let preferences =
        serde_json::from_str(&payload).map_err(|_| invalid("invalid JSON preference payload"))?;
    let snapshot = PreferencesSnapshot {
        revision: u64::try_from(revision).map_err(|_| invalid("negative revision"))?,
        preferences,
    };
    snapshot
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{Appearance, ReadingDensity};

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
}
