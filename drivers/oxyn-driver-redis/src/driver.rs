//! The driver: what it says about itself, and how it opens a session.
//!
//! # One driver per protocol
//!
//! Redis and Valkey speak RESP: one crate, and what differs between them is
//! read from the server when a session opens — its name and version from
//! `HELLO`, its commands from `COMMAND`
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # Opening, in four steps
//!
//! 1. the configuration becomes a [`ConnectSpec`], which refuses a weak TLS
//!    mode outside a local database and has no talkative `Debug`;
//! 2. the console connection opens: `HELLO 3`, authentication, name,
//!    database; a server in cluster or sentinel mode is refused there;
//! 3. `COMMAND` gives the table the session classifies commands with;
//! 4. a second connection checks that `CLIENT UNBLOCK` is allowed to this
//!    account: if it is, the session declares server-side cancellation.

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::options::{ConnectSpec, DEFAULT_PORT, TlsMode};
use crate::session::{RedisSession, base_capabilities};

/// The identifier of the driver.
pub const DRIVER_ID: &str = "redis";

/// The Redis and Valkey driver.
#[derive(Debug, Clone)]
pub struct RedisDriver {
    metadata: DriverMetadata,
}

impl RedisDriver {
    /// Builds the driver and its connection form.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: redis_metadata(),
        }
    }
}

impl Default for RedisDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// The driver's identifier.
#[must_use]
pub fn driver_id() -> DriverId {
    DriverId::redis()
}

/// The connection form.
///
/// The user is optional: without one, `AUTH` names Redis's `default` user.
/// The password has no default: it would be written in clear in the binary.
#[must_use]
pub fn redis_metadata() -> DriverMetadata {
    DriverMetadata::new(driver_id(), "Redis", DriverFamily::KeyValue)
        .with_default_port(DEFAULT_PORT)
        .with_alias("Valkey")
        .with_fields([
            ConnectionField::new("host", "Host", FieldKind::Text)
                .required()
                .with_default("localhost"),
            ConnectionField::new("port", "Port", FieldKind::Number).with_default(DEFAULT_PORT.to_string()),
            ConnectionField::new("database", "Database number", FieldKind::Number)
                .with_default("0")
                .with_help("The database SELECT opens on; the tree lists the others."),
            ConnectionField::new("user", "User", FieldKind::Text)
                .with_help("An ACL user. Empty: Redis's `default` user."),
            ConnectionField::new("password", "Password", FieldKind::Password)
                .with_help("Stored in the system keyring."),
            ConnectionField::new(
                "tls",
                "TLS mode",
                FieldKind::Choice(TlsMode::ALL.iter().map(|mode| mode.name().to_owned()).collect()),
            )
            .with_default(TlsMode::VerifyFull.name()),
            ConnectionField::new("ca_file", "CA certificate", FieldKind::Path)
                .with_help("A PEM file of the authority that signed the server's certificate. Empty: the public roots."),
            ConnectionField::new("socket", "Unix socket", FieldKind::Path)
                .with_help("Instead of a host: a socket on this machine. TLS does not apply."),
        ])
}

#[async_trait]
impl Driver for RedisDriver {
    fn id(&self) -> DriverId {
        driver_id()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// The ceiling: what a session may declare. The session's own list is
    /// authoritative.
    fn capabilities(&self) -> Capabilities {
        base_capabilities() | Capabilities::SERVER_SIDE_CANCEL
    }

    /// Opens a session.
    ///
    /// # Errors
    /// [`OxynError::Config`] for an unusable configuration;
    /// [`OxynError::Authentication`] when the server refuses the credentials;
    /// [`OxynError::Connection`] when it is unreachable, speaks no RESP3, or
    /// runs in cluster or sentinel mode; [`OxynError::Cancelled`].
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let session = RedisSession::open(Arc::new(spec), config.read_only, cancel).await?;
        Ok(Box::new(session))
    }
}

/// Refuses a server this driver does not speak to correctly.
///
/// A cluster answers `MOVED` for every key on another node, and following it
/// silently would send the user's command to a server they did not name; a
/// sentinel holds no data.
///
/// # Errors
/// [`OxynError::NotSupported`] naming the mode.
pub fn require_standalone(mode: &str) -> Result<()> {
    match mode {
        "standalone" | "" => Ok(()),
        "cluster" => Err(OxynError::NotSupported {
            capability:
                "Redis Cluster: this connection reaches one node of a cluster, and Oxyn does \
                         not follow MOVED to the others yet; connect to a standalone server"
                    .to_owned(),
        }),
        "sentinel" => Err(OxynError::NotSupported {
            capability: "Redis Sentinel: this address is a sentinel, which holds no data; connect \
                         to the primary it monitors"
                .to_owned(),
        }),
        other => Err(OxynError::NotSupported {
            capability: format!("server mode `{other}`"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn the_driver_declares_itself_consistently() {
        let driver = RedisDriver::new();
        assert_eq!(driver.id(), driver.metadata().id);
        assert_eq!(driver.id().as_str(), DRIVER_ID);
        driver.metadata().check().expect("consistent form");
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(RedisDriver::new()))
            .expect("registration");
        assert!(registry.contains(&DriverId::redis()));
        assert!(driver.capabilities().contains(Capabilities::REDIS_COMMAND));
        assert!(!driver.capabilities().contains(Capabilities::SQL));
    }

    #[test]
    fn the_password_is_the_only_secret_and_has_no_default() {
        let metadata = redis_metadata();
        let secrets: Vec<&str> = metadata
            .secret_fields()
            .map(|field| field.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);
        assert_eq!(metadata.family, DriverFamily::KeyValue);
        assert_eq!(metadata.aliases, ["Valkey"]);
    }

    #[test]
    fn only_a_standalone_server_is_accepted() {
        assert!(require_standalone("standalone").is_ok());
        for mode in ["cluster", "sentinel", "other"] {
            let error = require_standalone(mode).expect_err("refused");
            assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
        }
    }
}
