//! The driver: what it says about itself, and how it opens a session.
//!
//! # Opening, in three requests
//!
//! 1. the configuration becomes a [`ConnectSpec`], which refuses a weak TLS
//!    mode outside a local database and has no talkative `Debug`;
//! 2. `SELECT version(), timezone(), getSetting('readonly')` proves the
//!    account, and says what the server is and what the account may do;
//! 3. `KILL QUERY … TEST` on an id no statement has tells whether the account
//!    may kill its own statements: `SERVER_SIDE_CANCEL` is declared only then.

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::fetch::{counts, fetch, texts};
use crate::http::{Call, Http};
use crate::options::{ConnectSpec, HTTPS_PORT, TlsMode};
use crate::session::ClickhouseSession;
use crate::variant::{ServerFacts, driver_capabilities};

/// What the server is, and what the account may do. A literal.
const SQL_PROBE: &str = "SELECT version(), timezone(), toUInt64(getSetting('readonly'))";

/// Whether the account may kill: `TEST` stops nothing, it checks the right.
const SQL_KILL_PROBE: &str = "KILL QUERY WHERE query_id = {id:String} TEST";

/// The ClickHouse driver.
#[derive(Debug, Clone)]
pub struct ClickhouseDriver {
    metadata: DriverMetadata,
}

impl ClickhouseDriver {
    /// Builds the driver and its connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: clickhouse_metadata(),
        }
    }
}

impl Default for ClickhouseDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// ClickHouse's connection form.
///
/// The port has no default of its own: left empty, it follows the TLS mode —
/// 8443 for HTTPS, 8123 for plain HTTP. The database is optional: a
/// connection opens on the server, and its databases are the tree's first
/// level. The password has no default: it would be written in clear in the
/// binary.
#[must_use]
pub fn clickhouse_metadata() -> DriverMetadata {
    DriverMetadata::new(
        DriverId::clickhouse(),
        "ClickHouse",
        DriverFamily::Analytical,
    )
    .with_default_port(HTTPS_PORT)
    .with_fields([
        ConnectionField::new("host", "Host", FieldKind::Text)
            .required()
            .with_default("localhost"),
        ConnectionField::new("port", "Port", FieldKind::Number)
            .with_help("HTTP interface port. Empty: 8443 with TLS, 8123 without."),
        ConnectionField::new("database", "Database", FieldKind::Text)
            .with_help("Empty: the account's default database."),
        ConnectionField::new("user", "User", FieldKind::Text)
            .required()
            .with_default("default"),
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
impl Driver for ClickhouseDriver {
    fn id(&self) -> DriverId {
        DriverId::clickhouse()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// The ceiling: what a session may declare. The session's own list is
    /// authoritative.
    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Proves the account and reads what the session can do.
    ///
    /// # Errors
    /// [`oxyn_core::OxynError::Config`] for an incomplete configuration or a
    /// TLS mode refused outside a local database;
    /// [`oxyn_core::OxynError::Authentication`] if the server refuses the
    /// account; [`oxyn_core::OxynError::Connection`] if it is unreachable;
    /// [`oxyn_core::OxynError::Cancelled`] if `cancel` fires.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let database = spec.database().map(str::to_owned);
        let http = Arc::new(Http::new(spec)?);
        let driver = self.id();

        let probe = Call {
            text: SQL_PROBE.to_owned(),
            query_id: uuid::Uuid::new_v4().to_string(),
            database,
            ..Call::default()
        };
        let batches = fetch(&http, &driver, &probe, false, cancel).await?;
        let version = texts(&batches, 0)?.into_iter().flatten().next();
        let zone = texts(&batches, 1)?.into_iter().flatten().next();
        let readonly = counts(&batches, 2)?.into_iter().flatten().next();
        let (Some(version), Some(readonly)) = (version, readonly) else {
            return Err(oxyn_core::OxynError::Connection(
                "the server did not say its version".to_owned(),
            ));
        };

        let kill_probe = Call {
            text: SQL_KILL_PROBE.to_owned(),
            query_id: uuid::Uuid::new_v4().to_string(),
            params: vec![("id", uuid::Uuid::new_v4().to_string())],
            ..Call::default()
        };
        let can_kill = match fetch(&http, &driver, &kill_probe, false, cancel).await {
            Ok(_) => true,
            Err(oxyn_core::OxynError::Cancelled) => return Err(oxyn_core::OxynError::Cancelled),
            Err(_) => false,
        };

        let facts = ServerFacts {
            version,
            zone,
            readonly: u8::try_from(readonly.min(2)).unwrap_or(2),
            can_kill,
        };
        tracing::debug!(
            target: "oxyn::driver::clickhouse",
            version = %facts.version,
            readonly = facts.readonly,
            server_side_cancel = facts.can_kill,
            "session opened"
        );
        Ok(Box::new(ClickhouseSession::new(http, driver, facts)))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn the_driver_declares_itself_consistently() {
        let driver = ClickhouseDriver::new();
        assert_eq!(driver.id(), driver.metadata().id);
        driver.metadata().check().expect("consistent form");
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(ClickhouseDriver::new()))
            .expect("registration");
        assert!(registry.contains(&DriverId::clickhouse()));
    }

    #[test]
    fn the_form_is_the_contract_the_interface_builds_on() {
        let metadata = clickhouse_metadata();
        assert_eq!(metadata.id.as_str(), "clickhouse");
        assert_eq!(metadata.display_name, "ClickHouse");
        assert_eq!(metadata.family, DriverFamily::Analytical);
        assert_eq!(metadata.default_port, Some(8443));
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
            "disable".to_owned(),
        ]);
        assert_eq!(
            fields,
            [
                ("host", &FieldKind::Text, true, Some("localhost")),
                ("port", &FieldKind::Number, false, None),
                ("database", &FieldKind::Text, false, None),
                ("user", &FieldKind::Text, true, Some("default")),
                ("password", &FieldKind::Password, false, None),
                ("sslmode", &choice, false, Some("verify-full")),
            ]
        );
    }

    #[test]
    fn the_password_is_the_only_secret_and_has_no_default() {
        let metadata = clickhouse_metadata();
        let secrets: Vec<&str> = metadata
            .secret_fields()
            .map(|field| field.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);
    }
}
