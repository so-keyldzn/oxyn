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
        let maintenant = Utc::now();

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
                    maintenant,
                    maintenant,
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
                |row| Ok(depuis_ligne(row)),
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
            let mut requete = conn.prepare(
                "SELECT id, name, driver, environment, params, secret_ref, read_only,
                        privacy_tier
                 FROM connections WHERE workspace_id = ?1 ORDER BY name, id",
            )?;
            let lignes = requete.query_and_then(params![workspace.to_string()], depuis_ligne)?;
            lignes.collect()
        })
    }

    /// The workspace a connection belongs to.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] or [`StoreError::Corrupted`].
    pub fn workspace_of(&self, id: ConnectionId) -> Result<Option<WorkspaceId>> {
        self.store.with_connection(|conn| {
            let brut: Option<String> = conn
                .query_row(
                    "SELECT workspace_id FROM connections WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            brut.as_deref()
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
            let touchees = conn.execute(
                "DELETE FROM connections WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(touchees > 0)
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
    let normalise = normalise_key(key);
    SECRET_KEY_MARKERS
        .iter()
        .any(|marqueur| normalise.contains(*marqueur))
}

/// Refuses a configuration that would store a secret in its parameters.
fn refuse_secrets(config: &ConnectionConfig) -> Result<()> {
    for cle in config.params.keys() {
        if is_secret_key(cle) {
            return Err(StoreError::SecretInParams { key: cle.clone() });
        }
    }
    Ok(())
}

/// Rebuilds a [`ConnectionConfig`] from a row.
///
/// An unreadable environment falls back to `production`
/// ([`environment_from_text`]): it is the most restrictive value, and it is
/// what SECURITY requires of a missing or doubtful marking.
fn depuis_ligne(row: &Row<'_>) -> Result<ConnectionConfig> {
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
    let niveau: Option<String> = row.get("privacy_tier")?;
    config.privacy_tier = privacy_tier_from_column(niveau.as_deref());

    signale_les_cles_suspectes(&config);
    Ok(config)
}

/// Warns if an already written row carries a secret key.
///
/// Does not fail: the row exists, and refusing it would make the connection
/// unusable without erasing anything. The message names the connection and
/// the key, never the value.
fn signale_les_cles_suspectes(config: &ConnectionConfig) {
    for cle in config.params.keys() {
        if is_secret_key(cle) {
            tracing::warn!(
                connection = %config.name,
                param = %cle,
                "stored connection parameter looks like a secret; secrets belong in the OS keychain"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::Environment;

    fn store_avec_workspace() -> (Store, WorkspaceId) {
        let store = Store::open_in_memory().expect("open");
        let workspace = store.workspaces().create("atelier").expect("workspace");
        (store, workspace.id)
    }

    #[test]
    fn a_connection_round_trips() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Staging)
            .with_param("host", "db.interne")
            .with_param("port", "5432")
            .with_param("dbname", "facturation")
            .with_secret_ref("keychain://oxyn/base-client")
            .read_only();

        store.connections().save(workspace, &config).expect("write");
        let relu = store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("the connection exists");

        assert_eq!(relu.id, config.id);
        assert_eq!(relu.name, "base client");
        assert_eq!(relu.driver, DriverId::postgres());
        assert_eq!(relu.environment, Environment::Staging);
        assert_eq!(
            relu.secret_ref.as_deref(),
            Some("keychain://oxyn/base-client")
        );
        assert!(relu.read_only);
        assert_eq!(relu.params.len(), 3);
    }

    #[test]
    fn parameter_order_is_preserved() {
        // A configuration file that reorders itself produces unreadable
        // diffs; `ConnectionConfig` relies on this order.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("ordre", DriverId::sqlite())
            .with_param("zeta", "1")
            .with_param("alpha", "2")
            .with_param("mu", "3");

        store.connections().save(workspace, &config).expect("write");
        let relu = store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present");

        let cles: Vec<&str> = relu.params.keys().map(String::as_str).collect();
        assert_eq!(cles, ["zeta", "alpha", "mu"]);
    }

    #[test]
    fn a_parameter_named_like_a_secret_is_refused() {
        let (store, workspace) = store_avec_workspace();

        for cle in [
            "password",
            "PASSWORD",
            "sslpassword",
            "passwd",
            "pwd",
            "passphrase",
            "api_key",
            "API-KEY",
            "apikey",
            "client_secret",
            "access_token",
            "aws_credentials",
            "private_key",
        ] {
            let config = ConnectionConfig::new("essai", DriverId::postgres())
                .with_param("host", "localhost")
                .with_param(cle, "peu importe");

            let erreur = store
                .connections()
                .save(workspace, &config)
                .expect_err("a parameter named like a secret must be refused");
            assert!(
                matches!(erreur, StoreError::SecretInParams { .. }),
                "`{cle}`: {erreur}"
            );
            assert!(
                !erreur.to_string().contains("peu importe"),
                "the value must never appear"
            );
            assert!(
                store.connections().get(config.id).expect("read").is_none(),
                "`{cle}`: nothing must have been written"
            );
        }
    }

    #[test]
    fn an_empty_parameter_named_like_a_secret_is_refused_too() {
        // The name is the signal: looking at the value would mean reading it,
        // and a password that looks like a host name would get through.
        let (store, workspace) = store_avec_workspace();
        let config =
            ConnectionConfig::new("essai", DriverId::postgres()).with_param("password", "");
        assert!(store.connections().save(workspace, &config).is_err());
    }

    #[test]
    fn certificate_paths_stay_allowed() {
        // Refusing `sslkey` would make PostgreSQL client-certificate
        // authentication impossible: these are paths, not keys.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("tls", DriverId::postgres())
            .with_param("sslmode", "verify-full")
            .with_param("sslkey", "/etc/ssl/client.key")
            .with_param("sslcert", "/etc/ssl/client.crt")
            .with_param("sslrootcert", "/etc/ssl/ca.crt")
            .with_param("host", "db.interne")
            .with_param("application_name", "oxyn");

        store
            .connections()
            .save(workspace, &config)
            .expect("none of these parameters is a secret");
    }

    #[test]
    fn a_connection_without_environment_reads_back_as_production() {
        let (store, workspace) = store_avec_workspace();
        // No `with_environment`: `ConnectionConfig`'s default applies.
        let config = ConnectionConfig::new("ajoutée à la hâte", DriverId::postgres());
        assert!(config.is_production());

        store.connections().save(workspace, &config).expect("write");
        let relu = store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present");
        assert!(relu.is_production());
    }

    #[test]
    fn an_unreadable_environment_reads_back_as_production() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("trafiquée", DriverId::postgres())
            .with_environment(Environment::Local);
        store.connections().save(workspace, &config).expect("write");

        // Someone opens the file with `sqlite3` and writes anything.
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE connections SET environment = 'presque-du-dev' WHERE id = ?1",
                    params![config.id.to_string()],
                )?;
                Ok(())
            })
            .expect("tampering");

        let relu = store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present");
        assert!(
            relu.is_production(),
            "SECURITY: an unreadable marking counts as production"
        );
    }

    #[test]
    fn a_connection_without_workspace_is_refused() {
        let store = Store::open_in_memory().expect("open");
        let config = ConnectionConfig::new("orpheline", DriverId::sqlite());
        assert!(
            store
                .connections()
                .save(WorkspaceId::new(), &config)
                .is_err(),
            "the foreign key must refuse a nonexistent workspace"
        );
    }

    #[test]
    fn the_list_is_scoped_to_the_workspace() {
        let store = Store::open_in_memory().expect("open");
        let a = store.workspaces().create("a").expect("workspace");
        let b = store.workspaces().create("b").expect("workspace");

        store
            .connections()
            .save(a.id, &ConnectionConfig::new("dans a", DriverId::sqlite()))
            .expect("write");
        store
            .connections()
            .save(b.id, &ConnectionConfig::new("dans b", DriverId::sqlite()))
            .expect("write");

        let dans_a = store.connections().list(a.id).expect("list");
        assert_eq!(dans_a.len(), 1);
        assert_eq!(dans_a[0].name, "dans a");
    }

    #[test]
    fn deleting_a_workspace_takes_its_connections() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("éphémère", DriverId::sqlite());
        store.connections().save(workspace, &config).expect("write");

        assert!(store.workspaces().delete(workspace).expect("deletion"));
        assert!(
            store.connections().get(config.id).expect("read").is_none(),
            "the cascade must apply: PRAGMA foreign_keys is on"
        );
    }

    #[test]
    fn an_update_creates_no_duplicate() {
        let (store, workspace) = store_avec_workspace();
        let mut config = ConnectionConfig::new("avant", DriverId::sqlite());
        store.connections().save(workspace, &config).expect("write");

        config.name = "après".to_owned();
        store
            .connections()
            .save(workspace, &config)
            .expect("update");

        let liste = store.connections().list(workspace).expect("list");
        assert_eq!(liste.len(), 1);
        assert_eq!(liste[0].name, "après");
    }

    #[test]
    fn debug_of_a_read_back_connection_still_masks_values() {
        // The guarantee comes from `oxyn-core`, but it must hold after a round
        // trip through the disk: that is the real path.
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("prod", DriverId::postgres())
            .with_param("host", "db-secret.interne")
            .with_secret_ref("keychain://oxyn/prod");
        store.connections().save(workspace, &config).expect("write");

        let relu = store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present");
        let rendu = format!("{relu:?}");
        assert!(!rendu.contains("db-secret.interne"), "{rendu}");
        assert!(!rendu.contains("keychain://oxyn/prod"), "{rendu}");
    }

    /// ADR-0006: the tier belongs to the connection, so it survives it.
    ///
    /// The defect this test closes was silent and permissive: without a
    /// column, every connection read back went back to `Metadata`. A user
    /// setting `Local` on a customer database, closing Oxyn and reopening it
    /// saw the DDL and column names go to a remote provider again, with no
    /// message anywhere.
    #[test]
    fn the_privacy_tier_survives_closing() {
        let (store, workspace) = store_avec_workspace();
        let mut config =
            ConnectionConfig::new("base client", DriverId::new("postgres").expect("driver"));
        config.privacy_tier = oxyn_core::PrivacyTier::Local;
        store.connections().save(workspace, &config).expect("write");

        let relu = store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present");
        assert_eq!(
            relu.privacy_tier,
            oxyn_core::PrivacyTier::Local,
            "the tier belongs to the connection, not to the session"
        );
    }

    /// An absence and an unreadable value do not mean the same thing.
    ///
    /// Absent, the column says "this binary is older than this setting": the
    /// user never chose one, and ADR-0006's default applies. Unreadable, it
    /// says "a setting existed and its meaning was lost": the most restrictive
    /// applies, because a database for which it is no longer known what it
    /// allowed does not get the benefit of the doubt.
    #[test]
    fn an_unreadable_value_is_not_an_absence() {
        let (store, workspace) = store_avec_workspace();
        let config = ConnectionConfig::new("héritée", DriverId::new("sqlite").expect("driver"));
        store.connections().save(workspace, &config).expect("write");

        // The case of the row older than the migration.
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE connections SET privacy_tier = NULL WHERE id = ?1",
                    params![config.id.to_string()],
                )?;
                Ok(())
            })
            .expect("reset");
        assert_eq!(
            store
                .connections()
                .get(config.id)
                .expect("read")
                .expect("present")
                .privacy_tier,
            oxyn_core::PrivacyTier::Metadata,
            "no value written: ADR-0006's default applies"
        );

        // The case of the value that can no longer be read.
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE connections SET privacy_tier = 'confidentiel' WHERE id = ?1",
                    params![config.id.to_string()],
                )?;
                Ok(())
            })
            .expect("unknown value");
        assert_eq!(
            store
                .connections()
                .get(config.id)
                .expect("read")
                .expect("present")
                .privacy_tier,
            oxyn_core::PrivacyTier::Local,
            "a setting whose meaning was lost falls back to the most restrictive"
        );
    }
}
