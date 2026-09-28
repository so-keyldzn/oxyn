//! The `workspaces` table: the unit of persistence of user state.
//!
//! Deleting a workspace removes its connections, documents and preferences.
//! The append-only audit remains independent and is never deleted with it.

use chrono::{DateTime, Utc};
use oxyn_core::WorkspaceId;
use rusqlite::{OptionalExtension, Row, params};

use crate::encoding::parse_id;
use crate::error::Result;
use crate::store::Store;

/// A workspace, as persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Internal identifier, stable from one opening to the next.
    pub id: WorkspaceId,
    /// Name given by the user. It is what is displayed.
    pub name: String,
    /// Creation date.
    pub created_at: DateTime<Utc>,
    /// Date of the last recorded modification.
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    /// Builds a new workspace, not yet persisted.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: WorkspaceId::new(),
            name: name.into(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Typed access to the `workspaces` table.
#[derive(Debug)]
pub struct Workspaces<'a> {
    store: &'a Store,
}

impl<'a> Workspaces<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Creates and persists a workspace.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the write fails.
    pub fn create(&self, name: impl Into<String>) -> Result<Workspace> {
        let workspace = Workspace::new(name);
        self.save(&workspace)?;
        Ok(workspace)
    }

    /// Inserts or updates a workspace.
    ///
    /// `created_at` is never overwritten by an update: a workspace's creation
    /// date does not change because it was renamed.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the write fails.
    pub fn save(&self, workspace: &Workspace) -> Result<()> {
        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO workspaces (id, name, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     name       = excluded.name,
                     updated_at = excluded.updated_at",
                params![
                    workspace.id.to_string(),
                    workspace.name,
                    workspace.created_at,
                    workspace.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Reads a workspace back by its identifier.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn get(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        self.store.with_connection(|conn| {
            conn.query_row(
                "SELECT id, name, created_at, updated_at FROM workspaces WHERE id = ?1",
                params![id.to_string()],
                |row| Ok(from_row(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Lists workspaces, by name.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn list(&self) -> Result<Vec<Workspace>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(
                "SELECT id, name, created_at, updated_at FROM workspaces ORDER BY name, id",
            )?;
            let rows = query.query_and_then([], from_row)?;
            rows.collect()
        })
    }

    /// Deletes a workspace and, in cascade, its connections, their catalog
    /// caches and its documents.
    ///
    /// **The audit journal is not touched**: its rows carry no foreign key to
    /// a workspace. Deleting a workspace does not erase what an agent did in
    /// it.
    ///
    /// Returns `true` if a row was deleted.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the deletion fails.
    pub fn delete(&self, id: WorkspaceId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touched = conn.execute(
                "DELETE FROM workspaces WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(touched > 0)
        })
    }
}

/// Rebuilds a [`Workspace`] from a row.
fn from_row(row: &Row<'_>) -> Result<Workspace> {
    let id: String = row.get("id")?;
    Ok(Workspace {
        id: parse_id(&id, "workspaces.id")?,
        name: row.get("name")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_round_trips() {
        let store = Store::open_in_memory().expect("open");
        let created = store.workspaces().create("workshop").expect("creation");

        let read_back = store
            .workspaces()
            .get(created.id)
            .expect("read")
            .expect("the workspace exists");
        assert_eq!(read_back.id, created.id);
        assert_eq!(read_back.name, "workshop");
        // Timestamps are serialized through SQLite: the comparison holds to
        // the millisecond, not to the nanosecond.
        assert_eq!(
            read_back.created_at.timestamp_millis(),
            created.created_at.timestamp_millis()
        );
    }

    #[test]
    fn renaming_does_not_change_the_creation_date() {
        let store = Store::open_in_memory().expect("open");
        let mut workspace = store.workspaces().create("before").expect("creation");
        let creation = workspace.created_at;

        workspace.name = "after".to_owned();
        workspace.updated_at = Utc::now();
        workspace.created_at = Utc::now(); // even if the caller gets it wrong
        store.workspaces().save(&workspace).expect("update");

        let read_back = store
            .workspaces()
            .get(workspace.id)
            .expect("read")
            .expect("present");
        assert_eq!(read_back.name, "after");
        assert_eq!(
            read_back.created_at.timestamp_millis(),
            creation.timestamp_millis()
        );
    }

    #[test]
    fn a_missing_workspace_returns_none() {
        let store = Store::open_in_memory().expect("open");
        assert!(
            store
                .workspaces()
                .get(WorkspaceId::new())
                .expect("read")
                .is_none()
        );
        assert!(
            !store
                .workspaces()
                .delete(WorkspaceId::new())
                .expect("deletion")
        );
    }

    #[test]
    fn the_list_is_ordered_by_name() {
        let store = Store::open_in_memory().expect("open");
        for name in ["gamma", "alpha", "beta"] {
            store.workspaces().create(name).expect("creation");
        }
        let names: Vec<String> = store
            .workspaces()
            .list()
            .expect("list")
            .into_iter()
            .map(|w| w.name)
            .collect();
        assert_eq!(names, ["alpha", "beta", "gamma"]);
    }
}
