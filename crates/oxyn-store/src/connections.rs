//! The `connections` table: **metadata, never a secret**.
//!
//! This module is the last point in the product where a password can be
//! prevented from reaching the disk in clear. What is persisted here is what
//! [`ConnectionConfig`] carries: a name, a driver, an environment marking,
//! non-secret parameters, and a secret **reference** resolved elsewhere by
//! `oxyn-secrets` against the system keychain (SECURITY, I-03).
//!
//! # The safeguard, and why it refuses instead of warning
//!
//! [`Connections::save`] **refuses** a configuration one of whose parameters
//! carries a secret's name — `password`, `sslpassword`, `api_key`,
//! `client_secret`, `access_token`… A comment in the schema saying "never a
//! secret here" prevents nothing: a driver written six months later will store
//! the password in `params` because it was the convenient field, and nobody
//! will see it until the day the user opens their state file.
//!
//! The refusal is on the **key**, never on the value: deciding by looking at
//! the value would mean reading it, logging it on error, and letting through
//! a password that looks like a host name. A parameter named `password` but
//! empty is refused too — the name is the signal.
//!
//! Certificate file names (`sslkey`, `sslcert`, `sslrootcert`) are **not**
//! refused: they are paths, and forbidding `sslkey` would make PostgreSQL
//! client-certificate authentication impossible.
//!
//! On **reading back**, a suspicious key is not an error — the row already
//! exists — but it emits a `warn` naming the connection and the key, never
//! the value.

use chrono::{DateTime, Utc};
use oxyn_core::{ConnectionConfig, ConnectionId, DriverId, WorkspaceId};
use rusqlite::{OptionalExtension, Row, params};

use crate::encoding::{environment_from_text, parse_id, privacy_tier_from_column};
use crate::error::{Result, StoreError};
use crate::store::Store;

/// Fragments that, when present in a normalized parameter name, mark it as a
/// secret.
///
/// The list targets the names actually found in DBMS and cloud SDK connection
/// strings. It is deliberately short: each entry forbids a potential
/// legitimate parameter name, and a list that is too broad pushes people to
/// work around the safeguard.
const SECRET_KEY_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "secret",
    "token",
    "credential",
    "apikey",
    "api_key",
    "private_key",
    "privatekey",
];

/// Typed access to the `connections` table.
#[derive(Debug)]
pub struct Connections<'a> {
    store: &'a Store,
}

impl<'a> Connections<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Inserts or updates a connection in a workspace.
    ///
    /// `created_at` is preserved on update.
    ///
    /// # Errors
    /// * [`StoreError::SecretInParams`] if a parameter carries a secret's
    ///   name. **Nothing is written** in that case;
    /// * [`StoreError::Sqlite`] if the workspace does not exist — the foreign
    ///   key refuses it — or if the write fails;
    /// * [`StoreError::Json`] if the parameters are not serializable.
    pub fn save(&self, workspace: WorkspaceId, config: &ConnectionConfig) -> Result<()> {
        refuse_secrets(config)?;

        let params_json = serde_json::to_string(&config.params)?;
        let now = Utc::now();

        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO connections
                     (id, workspace_id, name, driver, environment,
                      params, secret_ref, read_only, created_at, updated_at,
                      privacy_tier)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                     workspace_id = excluded.workspace_id,
                     name         = excluded.name,
                     driver       = excluded.driver,
                     environment  = excluded.environment,
                     params       = excluded.params,
                     secret_ref   = excluded.secret_ref,
                     read_only    = excluded.read_only,
                     privacy_tier = excluded.privacy_tier,
                     updated_at   = excluded.updated_at",
                params![
                    config.id.to_string(),
                    workspace.to_string(),
                    config.name,
                    config.driver.as_str(),
                    config.environment.as_str(),
                    params_json,
                    config.secret_ref,
                    config.read_only,
                    now,
                    now,
                    config.privacy_tier.as_str(),
                ],
            )?;
            Ok(())
        })
    }

    /// Reads a connection back by its identifier.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], [`StoreError::Corrupted`] or
    /// [`StoreError::Json`].
    pub fn get(&self, id: ConnectionId) -> Result<Option<ConnectionConfig>> {
        self.store.with_connection(|conn| {
            conn.query_row(
                "SELECT id, name, driver, environment, params, secret_ref, read_only,
                        privacy_tier
                 FROM connections WHERE id = ?1",
                params![id.to_string()],
                |row| Ok(from_row(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Lists a workspace's connections, by name.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], [`StoreError::Corrupted`] or
    /// [`StoreError::Json`].
    pub fn list(&self, workspace: WorkspaceId) -> Result<Vec<ConnectionConfig>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(
                "SELECT id, name, driver, environment, params, secret_ref, read_only,
                        privacy_tier
                 FROM connections WHERE workspace_id = ?1 ORDER BY name, id",
            )?;
            let rows = query.query_and_then(params![workspace.to_string()], from_row)?;
            rows.collect()
        })
    }

    /// The workspace a connection belongs to.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] or [`StoreError::Corrupted`].
    pub fn workspace_of(&self, id: ConnectionId) -> Result<Option<WorkspaceId>> {
        self.store.with_connection(|conn| {
            let stored: Option<String> = conn
                .query_row(
                    "SELECT workspace_id FROM connections WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            stored
                .as_deref()
                .map(|raw| parse_id(raw, "connections.workspace_id"))
                .transpose()
        })
    }

    /// Date of this connection's last write.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the read fails.
    pub fn updated_at(&self, id: ConnectionId) -> Result<Option<DateTime<Utc>>> {
        self.store.with_connection(|conn| {
            Ok(conn
                .query_row(
                    "SELECT updated_at FROM connections WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )
                .optional()?)
        })
    }

    /// Deletes a connection.
    ///
    /// The documents that targeted it remain, their connection becoming
    /// `NULL`; the history and the audit journal are **not** touched.
    ///
    /// Returns `true` if a row was deleted.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the deletion fails.
    pub fn delete(&self, id: ConnectionId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touched = conn.execute(
                "DELETE FROM connections WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(touched > 0)
        })
    }
}

/// Normalizes a parameter name before comparison: lowercase, separators
/// folded to an underscore. `Access-Token` and `ACCESS TOKEN` become
/// `access_token`.
fn normalise_key(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            '-' | ' ' | '.' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Does this parameter name designate a secret?
fn is_secret_key(key: &str) -> bool {
    let normalised = normalise_key(key);
    SECRET_KEY_MARKERS
        .iter()
        .any(|marker| normalised.contains(*marker))
}

/// Refuses a configuration that would store a secret in its parameters.
fn refuse_secrets(config: &ConnectionConfig) -> Result<()> {
    for key in config.params.keys() {
        if is_secret_key(key) {
            return Err(StoreError::SecretInParams { key: key.clone() });
        }
    }
    Ok(())
}

/// Rebuilds a [`ConnectionConfig`] from a row.
///
/// An unreadable environment falls back to `production`
/// ([`environment_from_text`]): it is the most restrictive value, and it is
/// what SECURITY requires of a missing or doubtful marking.
fn from_row(row: &Row<'_>) -> Result<ConnectionConfig> {
    let id: String = row.get("id")?;
    let driver: String = row.get("driver")?;
    let environment: String = row.get("environment")?;
    let params_json: String = row.get("params")?;
    let name: String = row.get("name")?;

    let driver = DriverId::new(&driver).map_err(|err| StoreError::Corrupted {
        field: "connections.driver",
        detail: err.detail().to_owned(),
    })?;

    let mut config = ConnectionConfig::new(name, driver);
    config.id = parse_id(&id, "connections.id")?;
    config.environment = environment_from_text(&environment);
    // The type of `params` is the field's: inferring it avoids naming
    // `IndexMap`, which is not a dependency of this crate. Entry order is
    // preserved by serialization as well as by reading back.
    config.params = serde_json::from_str(&params_json)?;
    config.secret_ref = row.get("secret_ref")?;
    config.read_only = row.get("read_only")?;
    let tier: Option<String> = row.get("privacy_tier")?;
    config.privacy_tier = privacy_tier_from_column(tier.as_deref());

    flags_suspicious_keys(&config);
    Ok(config)
}

/// Warns if an already written row carries a secret key.
///
/// Does not fail: the row exists, and refusing it would make the connection
/// unusable without erasing anything. The message names the connection and
/// the key, never the value.
fn flags_suspicious_keys(config: &ConnectionConfig) {
    for key in config.params.keys() {
        if is_secret_key(key) {
            tracing::warn!(
                connection = %config.name,
                param = %key,
                "stored connection parameter looks like a secret; secrets belong in the OS keychain"
            );
        }
    }
}

#[cfg(test)]
mod tests;
