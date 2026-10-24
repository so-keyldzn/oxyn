//! The driver: what it says about itself, and how it opens a session.
//!
//! # The connection in five steps
//!
//! 1. the configuration becomes a [`ConnectSpec`], which refuses a weak TLS
//!    mode outside a local database and has no talkative `Debug`;
//! 2. the client options are built — a DNS seed list resolved — and the pool
//!    created, with the library's retries off;
//! 3. `buildInfo` proves the server reachable and the credentials accepted,
//!    and gives the version;
//! 4. a `killSessions` naming nobody tells whether the session can stop its
//!    commands on the server;
//! 5. `connectionStatus` with its privileges tells whether the account can
//!    write at all.
//!
//! Each step races `cancel`: before a command of the user exists, dropping
//! the client is the whole cleanup.

use std::sync::Arc;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use mongodb::Client;
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result, StatementIntent,
};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::cancel;
use crate::error::map_error;
use crate::options::{ConnectSpec, DEFAULT_PORT, Mechanism, TlsMode};
use crate::session::{MongoSession, Shared};

/// What every MongoDB session declares, before the account's privileges and
/// the server's support for `killSessions` are known.
///
/// `SCHEMAS` and `TABLES` are the catalog's two levels, in MongoDB's words
/// databases and collections: without `SCHEMAS`, the tree would ask for the
/// relations of an empty path, which names no database.
#[must_use]
pub const fn base_capabilities() -> Capabilities {
    Capabilities::MONGO_QUERY
        .union(Capabilities::SCHEMAS)
        .union(Capabilities::TABLES)
        .union(Capabilities::OBJECT_DEFINITION)
        // The `queryPlanner` verbosity only: `EXPLAIN_ANALYZE` would be
        // `executionStats`, which runs the plan.
        .union(Capabilities::EXPLAIN)
        .union(Capabilities::DOCUMENT)
        .union(Capabilities::SCHEMALESS)
        .union(Capabilities::INFERRED_SCHEMA)
        .union(Capabilities::STREAMING)
        .union(Capabilities::VIEWS)
        .union(Capabilities::INDEXES)
        .union(Capabilities::ROW_COUNT_ESTIMATE)
        .union(Capabilities::SESSION_CONTEXT)
        .union(Capabilities::PREVIEW_SORT)
        .union(Capabilities::DML)
        .union(Capabilities::DDL)
        .union(Capabilities::GRANT_REVOKE)
}

/// The ceiling: what a session may declare at best.
#[must_use]
pub const fn driver_capabilities() -> Capabilities {
    base_capabilities()
        .union(Capabilities::SERVER_SIDE_CANCEL)
        .union(Capabilities::READ_ONLY_SESSION)
}

/// A session of an account whose privileges hold no write: the server
/// enforces read-only, and the session says so the way SQLite's read-only
/// file does.
#[must_use]
pub const fn read_only(capabilities: Capabilities) -> Capabilities {
    capabilities
        .difference(Capabilities::DML)
        .difference(Capabilities::DDL)
        .difference(Capabilities::GRANT_REVOKE)
        .union(Capabilities::READ_ONLY_SESSION)
}

/// The privilege actions of the built-in `read` and `readAnyDatabase` roles
/// (MongoDB manual, built-in roles, consulted on 2026-10-22). An account whose
/// every privilege is among them cannot write; any other action, known or
/// not, makes no such claim.
const READ_ACTIONS: &[&str] = &[
    "changeStream",
    "collStats",
    "dbHash",
    "dbStats",
    "find",
    "killCursors",
    "listCollections",
    "listIndexes",
    "listSearchIndexes",
    "planCacheRead",
    "listDatabases",
];

/// The MongoDB driver.
#[derive(Debug, Clone)]
pub struct MongoDriver {
    metadata: DriverMetadata,
}

impl MongoDriver {
    /// Builds the driver and its connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: mongodb_metadata(),
        }
    }
}

impl Default for MongoDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// MongoDB's connection form.
///
/// The fields are the parts of a connection string, never the string: the
/// password goes to the keychain like every driver's
/// ([I-03](../../../CLAUDE.md#i-03)). The database is optional; it is where a
/// console sends a command that names none.
#[must_use]
pub fn mongodb_metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::mongodb(), "MongoDB", DriverFamily::Document)
        .with_default_port(DEFAULT_PORT)
        .with_fields([
            ConnectionField::new("host", "Hosts", FieldKind::Text)
                .required()
                .with_default("localhost")
                .with_help(
                    "host or host:port; several, comma-separated, for a replica set seed list",
                ),
            ConnectionField::new("port", "Port", FieldKind::Number)
                .with_default(DEFAULT_PORT.to_string())
                .with_help("For the hosts written without a port."),
            ConnectionField::new("srv", "DNS seed list (mongodb+srv)", FieldKind::Bool)
                .with_help("The host is a DNS name whose SRV records list the servers."),
            ConnectionField::new("database", "Database", FieldKind::Text)
                .with_help("Where a console command goes when it names no database."),
            ConnectionField::new("user", "User", FieldKind::Text),
            ConnectionField::new("password", "Password", FieldKind::Password)
                .with_help("Stored in the system keyring."),
            ConnectionField::new("authSource", "Authentication database", FieldKind::Text)
                .with_help("Defaults to the database above, then admin."),
            ConnectionField::new(
                "authMechanism",
                "Authentication mechanism",
                FieldKind::Choice(
                    Mechanism::ALL
                        .iter()
                        .map(|mechanism| mechanism.name().to_owned())
                        .collect(),
                ),
            )
            .with_default(Mechanism::Default.name()),
            ConnectionField::new("replicaSet", "Replica set", FieldKind::Text),
            ConnectionField::new("directConnection", "Direct connection", FieldKind::Bool)
                .with_help("Talk to this one server, without discovering the replica set."),
            ConnectionField::new(
                "tls",
                "TLS mode",
                FieldKind::Choice(
                    TlsMode::ALL
                        .iter()
                        .map(|mode| mode.name().to_owned())
                        .collect(),
                ),
            )
            .with_default(TlsMode::VerifyFull.name()),
            ConnectionField::new("tlsCAFile", "CA file", FieldKind::Path)
                .with_help("PEM certificates of the authority that signed the server's."),
            ConnectionField::new(
                "tlsCertificateKeyFile",
                "Client certificate",
                FieldKind::Path,
            )
            .with_help("PEM file holding the client certificate and its unencrypted key."),
        ])
}

/// Whether `connectionStatus` says the authenticated account cannot write.
///
/// Without access control the list of users is empty and everything is
/// allowed: no claim. A claim needs at least one privilege, each of whose
/// actions is a read.
fn reads_only(status: &Document) -> bool {
    let Ok(info) = status.get_document("authInfo") else {
        return false;
    };
    let authenticated = info
        .get_array("authenticatedUsers")
        .is_ok_and(|users| !users.is_empty());
    let Ok(privileges) = info.get_array("authenticatedUserPrivileges") else {
        return false;
    };
    authenticated
        && !privileges.is_empty()
        && privileges.iter().all(|privilege| {
            privilege
                .as_document()
                .and_then(|p| p.get_array("actions").ok())
                .is_some_and(|actions| {
                    actions.iter().all(|action| {
                        matches!(action, Bson::String(name) if READ_ACTIONS.contains(&name.as_str()))
                    })
                })
        })
}

/// Races `cancel` before any user command exists: dropping is the cleanup.
async fn before<T>(cancel: &CancelToken, call: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::select! {
        outcome = call => outcome,
        () = cancel.cancelled() => Err(OxynError::Cancelled),
    }
}

#[async_trait]
impl Driver for MongoDriver {
    fn id(&self) -> DriverId {
        DriverId::mongodb()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Opens the pool, identifies the server and the account, and checks
    /// that commands can be stopped on the server.
    ///
    /// # Errors
    /// [`OxynError::Config`] for an incomplete configuration or a TLS mode
    /// refused outside a local database; [`OxynError::Authentication`] if the
    /// server refuses the credentials; [`OxynError::Connection`] if no server
    /// answers; [`OxynError::Cancelled`] if `cancel` fires.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let driver = self.id();
        let read =
            |error: mongodb::error::Error| map_error(&driver, StatementIntent::Read, &error, false);
        let (client, stopper) = before(cancel, async {
            let options = spec.client_options().await.map_err(read)?;
            // The stopper's single connection is opened on its first use: a
            // session that never stops a command never pays for it.
            let mut stopping = options.clone();
            stopping.max_pool_size = Some(1);
            Ok((
                Client::with_options(options).map_err(read)?,
                Client::with_options(stopping).map_err(read)?,
            ))
        })
        .await?;
        let admin = client.database("admin");
        let build = before(cancel, async {
            admin
                .run_command(doc! { "buildInfo": 1 })
                .await
                .map_err(read)
        })
        .await?;
        let version = build.get_str("version").unwrap_or("unknown").to_owned();
        let can_kill = before(cancel, async { Ok(cancel::probe(&stopper).await) }).await?;
        let status = before(cancel, async {
            admin
                .run_command(doc! { "connectionStatus": 1, "showPrivileges": true })
                .await
                .map_err(read)
        })
        .await?;

        let mut capabilities = base_capabilities();
        if can_kill {
            capabilities = capabilities.union(Capabilities::SERVER_SIDE_CANCEL);
        }
        if reads_only(&status) {
            capabilities = read_only(capabilities);
        }
        tracing::debug!(
            target: "oxyn::driver::mongodb",
            version = %version,
            server_side_cancel = can_kill,
            read_only = capabilities.contains(Capabilities::READ_ONLY_SESSION),
            "session opened"
        );
        let shared = Arc::new(Shared {
            client,
            stopper,
            driver,
            default_database: spec.database().map(str::to_owned),
            capabilities,
        });
        Ok(Box::new(MongoSession::new(shared, version)))
    }
}

#[cfg(test)]
mod tests;
