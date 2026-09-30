//! The driver: what it says about itself, and how it opens a session.
//!
//! # One driver per protocol
//!
//! MySQL and MariaDB speak one protocol: one crate, and what differs is read
//! from the server at connection time ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # The connection in four steps
//!
//! 1. the configuration becomes a [`ConnectSpec`], which refuses a weak TLS
//!    mode outside a local database and has no talkative `Debug`;
//! 2. the connection opens, racing cancellation, and the session setup runs:
//!    UTC time zone, `utf8mb4` (ADR-0050 §6);
//! 3. `VERSION()` tells MySQL from MariaDB;
//! 4. a second connection of the same account is opened and closed: if it
//!    opens, the session can cancel on the server and says so (ADR-0050 §7).

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::cancel::Killer;
use crate::connection::{Link, SQL_VERSION, Shared, first_value, open, race_cancel, value_text};
use crate::error::map_exec_error;
use crate::options::{ConnectSpec, DEFAULT_PORT, TlsMode};
use crate::session::MysqlSession;
use crate::variant::{MysqlVariant, driver_capabilities};

/// The MySQL and MariaDB driver.
#[derive(Debug, Clone)]
pub struct MysqlDriver {
    metadata: DriverMetadata,
}

impl MysqlDriver {
    /// Builds the driver and its connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: mysql_metadata(),
        }
    }
}

impl Default for MysqlDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// MySQL's connection form.
///
/// The database is **optional**: a MySQL connection opens on the server, and
/// its databases are the tree's first level. The password has no default: it
/// would be written in clear in the binary.
#[must_use]
pub fn mysql_metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::mysql(), "MySQL", DriverFamily::Relational)
        .with_default_port(DEFAULT_PORT)
        .with_fields([
            ConnectionField::new("host", "Host", FieldKind::Text)
                .required()
                .with_default("localhost"),
            ConnectionField::new("port", "Port", FieldKind::Number)
                .with_default(DEFAULT_PORT.to_string()),
            ConnectionField::new("database", "Database", FieldKind::Text),
            ConnectionField::new("user", "User", FieldKind::Text).required(),
            ConnectionField::new("password", "Password", FieldKind::Password)
                .with_help("Stored in the system keyring."),
            ConnectionField::new(
                "sslmode",
                "TLS mode",
                FieldKind::Choice(
                    TlsMode::ALL
                        .iter()
                        .map(|mode| mode.name().to_owned())
                        .collect(),
                ),
            )
            .with_default(TlsMode::VerifyFull.name()),
        ])
}

#[async_trait]
impl Driver for MysqlDriver {
    fn id(&self) -> DriverId {
        DriverId::mysql()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// The ceiling: what a session may declare. The session's own list is
    /// authoritative.
    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Opens the session's connection, identifies the server, and checks that
    /// a second connection can open for cancellation.
    ///
    /// # Errors
    /// [`OxynError::Config`] for an incomplete configuration or a TLS mode
    /// refused outside a local database; [`OxynError::Authentication`] if the
    /// server refuses the credentials; [`OxynError::Connection`] if it is
    /// unreachable or TLS fails; [`OxynError::Cancelled`] if `cancel` fires.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let driver = self.id();
        let (mut conn, id) = race_cancel(cancel, open(&spec, &driver)).await?;
        let banner = race_cancel(cancel, async {
            first_value(&mut conn, SQL_VERSION)
                .await
                .map_err(|error| map_exec_error(&driver, oxyn_core::StatementIntent::Read, &error))
        })
        .await?
        .as_ref()
        .and_then(value_text)
        .ok_or_else(|| OxynError::Connection("the server did not say its version".to_owned()))?;

        let killer = Arc::new(Killer::new(spec.clone(), driver.clone()));
        let can_kill = race_cancel(cancel, async { Ok(killer.probe().await) }).await?;
        let variant = MysqlVariant::detect(&banner, can_kill);
        tracing::debug!(
            target: "oxyn::driver::mysql",
            product = variant.product(),
            version = %variant.server_version,
            server_side_cancel = can_kill,
            "session opened"
        );

        let shared = Arc::new(Shared::new(driver, spec, Link::new(conn, id), killer));
        Ok(Box::new(MysqlSession::new(shared, variant)))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn the_driver_declares_itself_consistently() {
        let driver = MysqlDriver::new();
        assert_eq!(driver.id(), driver.metadata().id);
        driver.metadata().check().expect("consistent form");
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(MysqlDriver::new()))
            .expect("registration");
        assert!(registry.contains(&DriverId::mysql()));
    }

    #[test]
    fn the_form_is_the_contract_the_interface_builds_on() {
        // The front end's fixtures are built from this list: order, kinds,
        // requirement and defaults are a contract.
        let metadata = mysql_metadata();
        assert_eq!(metadata.id.as_str(), "mysql");
        assert_eq!(metadata.display_name, "MySQL");
        assert_eq!(metadata.family, DriverFamily::Relational);
        assert_eq!(metadata.default_port, Some(3306));
        let fields: Vec<(&str, &FieldKind, bool, Option<&str>)> = metadata
            .connection_fields
            .iter()
            .map(|field| {
                (
                    field.key.as_str(),
                    &field.kind,
                    field.required,
                    field.default.as_deref(),
                )
            })
            .collect();
        let choice = FieldKind::Choice(vec![
            "verify-full".to_owned(),
            "require".to_owned(),
            "prefer".to_owned(),
            "disable".to_owned(),
        ]);
        assert_eq!(
            fields,
            [
                ("host", &FieldKind::Text, true, Some("localhost")),
                ("port", &FieldKind::Number, false, Some("3306")),
                ("database", &FieldKind::Text, false, None),
                ("user", &FieldKind::Text, true, None),
                ("password", &FieldKind::Password, false, None),
                ("sslmode", &choice, false, Some("verify-full")),
            ]
        );
        assert_eq!(
            metadata.field("password").and_then(|f| f.help.as_deref()),
            Some("Stored in the system keyring.")
        );
    }

    #[test]
    fn the_password_is_the_only_secret_and_has_no_default() {
        let metadata = mysql_metadata();
        let secrets: Vec<&str> = metadata
            .secret_fields()
            .map(|field| field.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);
    }

    #[test]
    fn a_connection_without_database_is_valid() {
        let config = ConnectionConfig::new("server", DriverId::mysql())
            .with_param("host", "db.internal")
            .with_param("user", "app");
        mysql_metadata()
            .validate(&config)
            .expect("the database is optional");
    }
}
