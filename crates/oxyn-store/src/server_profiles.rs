//! Workspace-scoped groups. Membership never changes a child's configuration.

use oxyn_core::server_profile::{
    MAX_DATABASE_NAME_BYTES, MAX_PROFILE_JSON_BYTES, MAX_PROFILE_MEMBERS, MAX_PROFILE_NAME_BYTES,
    MAX_PROFILE_REVISION, MAX_SERVER_PROFILES,
};
use oxyn_core::{ConnectionId, ServerProfile, ServerProfileId, ServerProfileSnapshot, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::connections::is_secret_key;
use crate::encoding::parse_id;
use crate::{Result, Store, StoreError};

/// Blocking access to local server profiles; callers use the blocking pool.
#[derive(Debug)]
pub struct ServerProfiles<'a> {
    store: &'a Store,
}

fn invalid(detail: &'static str) -> StoreError {
    StoreError::InvalidServerProfile { detail }
}

fn sql_limit(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid("limit exceeds SQLite integer range"))
}

fn sql_revision(value: u64) -> Result<i64> {
    if value == 0 || value > MAX_PROFILE_REVISION {
        return Err(invalid("invalid revision"));
    }
    i64::try_from(value).map_err(|_| invalid("revision exceeds SQLite integer range"))
}

fn read_revision(value: i64) -> Result<u64> {
    let revision = u64::try_from(value).map_err(|_| invalid("invalid stored revision"))?;
    sql_revision(revision)?;
    Ok(revision)
}

fn validate(profile: &ServerProfile) -> Result<()> {
    profile
        .validate()
        .map_err(|_| invalid("invalid defaults or name"))?;
    if profile.params.keys().any(|key| is_secret_key(key)) {
        return Err(invalid("secret fields cannot be stored in defaults"));
    }
    Ok(())
}

fn from_row(row: &Row<'_>) -> Result<(ServerProfile, u64)> {
    let id: String = row.get("id")?;
    let driver: String = row.get("driver")?;
    let encoded: Option<String> = row.get("params")?;
    let profile = ServerProfile {
        id: parse_id(&id, "server_profiles.id")?,
        name: row.get("name")?,
        driver: driver
            .parse()
            .map_err(|_| invalid("invalid stored driver"))?,
        params: serde_json::from_str(
            &encoded.ok_or_else(|| invalid("stored defaults exceed their byte limit"))?,
        )
        .map_err(|_| invalid("invalid stored defaults"))?,
    };
    validate(&profile)?;
    let revision = read_revision(row.get("revision")?)?;
    Ok((profile, revision))
}

fn members(conn: &Connection, profile: ServerProfileId) -> Result<Vec<ConnectionId>> {
    let mut query = conn.prepare(
        "SELECT CASE WHEN length(CAST(connection_id AS BLOB)) <= 36 THEN connection_id END
         FROM server_profile_members WHERE profile_id = ?1
         ORDER BY connection_id LIMIT ?2",
    )?;
    let rows = query.query_map(
        params![profile.to_string(), sql_limit(MAX_PROFILE_MEMBERS + 1)?],
        |row| row.get::<_, String>(0),
    )?;
    let mut result = Vec::new();
    for row in rows {
        if result.len() == MAX_PROFILE_MEMBERS {
            return Err(invalid("too many member connections"));
        }
        result.push(parse_id(&row?, "server_profile_members.connection_id")?);
    }
    Ok(result)
}

fn membership(conn: &Connection, connection: ConnectionId) -> Result<Option<ServerProfileId>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT profile_id FROM server_profile_members WHERE connection_id = ?1",
            [connection.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|id| parse_id(&id, "server_profile_members.profile_id"))
        .transpose()
}

impl<'a> ServerProfiles<'a> {
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Reads only this workspace, refusing oversized or corrupt rows explicitly.
    ///
    /// # Errors
    /// Corrupt state, exceeded bounds or a database read failure.
    pub fn list(&self, workspace: WorkspaceId) -> Result<Vec<ServerProfileSnapshot>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(
                "SELECT
                 CASE WHEN length(CAST(id AS BLOB)) <= 36 THEN id END AS id,
                 CASE WHEN length(CAST(name AS BLOB)) <= ?4 THEN name END AS name,
                 CASE WHEN length(CAST(driver AS BLOB)) <= 128 THEN driver END AS driver,
                 revision,
                 CASE WHEN length(CAST(params AS BLOB)) <= ?3 THEN params END AS params
                 FROM server_profiles WHERE workspace_id = ?1 ORDER BY name, id LIMIT ?2",
            )?;
            let rows = query.query_and_then(
                params![
                    workspace.to_string(),
                    sql_limit(MAX_SERVER_PROFILES + 1)?,
                    sql_limit(MAX_PROFILE_JSON_BYTES)?,
                    sql_limit(MAX_PROFILE_NAME_BYTES)?
                ],
                from_row,
            )?;
            let mut profiles = Vec::new();
            for row in rows {
                if profiles.len() == MAX_SERVER_PROFILES {
                    return Err(invalid("too many server profiles"));
                }
                let (profile, revision) = row?;
                let connections = members(conn, profile.id)?;
                profiles.push(ServerProfileSnapshot {
                    profile,
                    revision,
                    connections,
                });
            }
            Ok(profiles)
        })
    }

    /// Creates with no revision, or updates exactly the revision the caller read.
    ///
    /// Driver and workspace identity cannot change. Child settings and secrets
    /// are neither copied nor updated.
    ///
    /// # Errors
    /// Invalid input, stale revision, identity mismatch, bounds or storage failure.
    pub fn save(
        &self,
        workspace: WorkspaceId,
        profile: &ServerProfile,
        expected_revision: Option<u64>,
    ) -> Result<ServerProfileSnapshot> {
        validate(profile)?;
        self.store.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let previous: Option<(String, String, i64)> = tx
                .query_row(
                    "SELECT workspace_id, driver, revision FROM server_profiles WHERE id = ?1",
                    [profile.id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let revision = match (&previous, expected_revision) {
                (None, None) => {
                    let count: i64 = tx.query_row(
                        "SELECT count(*) FROM server_profiles WHERE workspace_id = ?1",
                        [workspace.to_string()],
                        |row| row.get(0),
                    )?;
                    if count >= sql_limit(MAX_SERVER_PROFILES)? {
                        return Err(invalid("too many server profiles"));
                    }
                    1
                }
                (Some((owner, driver, current)), Some(expected))
                    if owner == &workspace.to_string()
                        && driver == profile.driver.as_str()
                        && u64::try_from(*current).ok() == Some(expected) =>
                {
                    read_revision(*current)?
                        .checked_add(1)
                        .filter(|n| *n <= MAX_PROFILE_REVISION)
                        .ok_or_else(|| invalid("revision exhausted"))?
                }
                _ => return Err(StoreError::ServerProfileConflict),
            };
            let encoded = serde_json::to_string(&profile.params)?;
            if encoded.len() > MAX_PROFILE_JSON_BYTES {
                return Err(invalid("encoded defaults exceed their byte limit"));
            }
            tx.execute(
                "INSERT INTO server_profiles(id, workspace_id, name, driver, params, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                    params = excluded.params, revision = excluded.revision",
                params![
                    profile.id.to_string(),
                    workspace.to_string(),
                    profile.name,
                    profile.driver.as_str(),
                    encoded,
                    sql_revision(revision)?
                ],
            )?;
            let connections = members(&tx, profile.id)?;
            tx.commit()?;
            Ok(ServerProfileSnapshot {
                profile: profile.clone(),
                revision,
                connections,
            })
        })
    }

    /// Deletes only the profile and membership, never connections or their secrets.
    ///
    /// # Errors
    /// A stale revision, another workspace's profile, or storage failure.
    pub fn remove(
        &self,
        workspace: WorkspaceId,
        profile: ServerProfileId,
        expected_revision: u64,
    ) -> Result<()> {
        self.store.with_connection(|conn| {
            let changed = conn.execute(
                "DELETE FROM server_profiles WHERE id = ?1 AND workspace_id = ?2 AND revision = ?3",
                params![
                    profile.to_string(),
                    workspace.to_string(),
                    sql_revision(expected_revision)?
                ],
            )?;
            if changed == 0 {
                return Err(StoreError::ServerProfileConflict);
            }
            Ok(())
        })
    }

    /// Replaces the membership the caller observed, or unlinks with `None`.
    ///
    /// The file enforces workspace and protocol ownership. Reparenting is
    /// compare-and-set, so an old settings panel cannot undo a newer move.
    ///
    /// # Errors
    /// Wrong ownership, stale membership, exceeded bounds or storage failure.
    pub fn set_membership(
        &self,
        workspace: WorkspaceId,
        connection: ConnectionId,
        profile: Option<ServerProfileId>,
        expected_profile: Option<ServerProfileId>,
    ) -> Result<()> {
        self.store.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id = ?1 AND workspace_id = ?2)",
                params![connection.to_string(), workspace.to_string()],
                |row| row.get(0),
            )?;
            if !known {
                return Err(invalid("connection is not in this workspace"));
            }
            if membership(&tx, connection)? != expected_profile {
                return Err(StoreError::ServerProfileConflict);
            }
            if let Some(profile) = profile {
                let permitted: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM server_profiles p JOIN connections c
                     ON c.workspace_id = p.workspace_id AND c.driver = p.driver
                     WHERE p.id = ?1 AND p.workspace_id = ?2 AND c.id = ?3)",
                    params![
                        profile.to_string(),
                        workspace.to_string(),
                        connection.to_string()
                    ],
                    |row| row.get(0),
                )?;
                if !permitted {
                    return Err(invalid("profile does not match the workspace and protocol"));
                }
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM server_profile_members WHERE profile_id = ?1",
                    [profile.to_string()],
                    |row| row.get(0),
                )?;
                if expected_profile != Some(profile) && count >= sql_limit(MAX_PROFILE_MEMBERS)? {
                    return Err(invalid("too many member connections"));
                }
                tx.execute(
                    "INSERT INTO server_profile_members(connection_id, profile_id) VALUES (?1, ?2)
                     ON CONFLICT(connection_id) DO UPDATE SET profile_id = excluded.profile_id",
                    params![connection.to_string(), profile.to_string()],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM server_profile_members WHERE connection_id = ?1",
                    [connection.to_string()],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;

mod children;
