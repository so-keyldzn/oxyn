//! Connection-owned metadata preferences; all access may block on SQLite.

use oxyn_core::metadata_loading::{MAX_METADATA_POLICY_BYTES, MAX_METADATA_POLICY_REVISION};
use oxyn_core::{ConnectionId, MetadataLoadingPolicy, MetadataPolicySnapshot, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Result, Store, StoreError};

/// Blocking local-state access, deliberately separate from connection parameters.
#[derive(Debug)]
pub struct MetadataPolicies<'a> {
    store: &'a Store,
}

fn invalid(detail: &'static str) -> StoreError {
    StoreError::InvalidMetadataPolicy { detail }
}

impl<'a> MetadataPolicies<'a> {
    pub(crate) const fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// A missing row inherits the on-demand Minimal default without writing one.
    ///
    /// # Errors
    /// A foreign/missing connection, invalid JSON/version or an oversized row.
    pub fn load(
        &self,
        workspace: WorkspaceId,
        connection: ConnectionId,
    ) -> Result<MetadataPolicySnapshot> {
        self.store
            .with_connection(|sql| read(sql, workspace, connection))
    }

    /// Atomically compares the observed revision and saves only loading state.
    ///
    /// # Errors
    /// Stale edits, invalid policy, revision exhaustion or a storage failure.
    pub fn save(
        &self,
        workspace: WorkspaceId,
        connection: ConnectionId,
        policy: &MetadataLoadingPolicy,
        expected_revision: u64,
    ) -> Result<MetadataPolicySnapshot> {
        policy.validate().map_err(|_| invalid("invalid policy"))?;
        let payload =
            serde_json::to_string(policy).map_err(|_| invalid("policy encoding failed"))?;
        if payload.len() > MAX_METADATA_POLICY_BYTES {
            return Err(invalid("policy exceeds its byte limit"));
        }
        let next = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_METADATA_POLICY_REVISION)
            .ok_or_else(|| invalid("metadata policy revision exhausted"))?;
        let stored_revision = i64::try_from(next).map_err(|_| invalid("invalid revision"))?;
        self.store.with_connection(|sql| {
            let transaction = sql.unchecked_transaction()?;
            let current = read(&transaction, workspace, connection)?;
            if current.revision != expected_revision { return Err(invalid("policy changed; reload it before saving")); }
            transaction.execute(
                "INSERT INTO metadata_loading_policies(connection_id, revision, payload) VALUES (?1, ?2, ?3)
                 ON CONFLICT(connection_id) DO UPDATE SET revision = excluded.revision, payload = excluded.payload",
                params![connection.to_string(), stored_revision, payload],
            )?;
            transaction.commit()?;
            Ok(MetadataPolicySnapshot { revision: next, policy: policy.clone() })
        })
    }
}

fn read(
    sql: &Connection,
    workspace: WorkspaceId,
    connection: ConnectionId,
) -> Result<MetadataPolicySnapshot> {
    let owned: bool = sql.query_row(
        "SELECT EXISTS(SELECT 1 FROM connections WHERE id = ?1 AND workspace_id = ?2)",
        params![connection.to_string(), workspace.to_string()],
        |row| row.get(0),
    )?;
    if !owned {
        return Err(invalid("connection is not in this workspace"));
    }
    let limit =
        i64::try_from(MAX_METADATA_POLICY_BYTES).map_err(|_| invalid("invalid byte limit"))?;
    let row: Option<(i64, Option<String>)> = sql
        .query_row(
            "SELECT revision, CASE WHEN length(CAST(payload AS BLOB)) <= ?2 THEN payload END
         FROM metadata_loading_policies WHERE connection_id = ?1",
            params![connection.to_string(), limit],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((revision, payload)) = row else {
        return Ok(MetadataPolicySnapshot::default());
    };
    let revision = u64::try_from(revision)
        .ok()
        .filter(|value| *value > 0 && *value <= MAX_METADATA_POLICY_REVISION)
        .ok_or_else(|| invalid("invalid stored revision"))?;
    let policy: MetadataLoadingPolicy = serde_json::from_str(
        &payload.ok_or_else(|| invalid("stored policy exceeds its byte limit"))?,
    )
    .map_err(|_| invalid("invalid stored policy"))?;
    policy
        .validate()
        .map_err(|_| invalid("invalid stored policy"))?;
    Ok(MetadataPolicySnapshot { revision, policy })
}

#[cfg(test)]
mod tests;
