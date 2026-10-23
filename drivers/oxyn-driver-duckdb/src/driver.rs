//! Driver metadata and safe database opening.

use std::path::PathBuf;

use async_trait::async_trait;
use duckdb::{AccessMode, Config, Connection};
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::error;
use crate::options::BatchLimits;
use crate::session::DuckDbSession;

/// An embedded DuckDB driver.
#[derive(Debug)]
pub struct DuckDbDriver {
    metadata: DriverMetadata,
    limits: BatchLimits,
}

impl DuckDbDriver {
    /// The configured database file, or [`Self::MEMORY`].
    pub const PATH: &'static str = "path";
    /// DuckDB's in-memory database spelling.
    pub const MEMORY: &'static str = ":memory:";

    /// Builds the driver with bounded batches.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: metadata(),
            limits: BatchLimits::default(),
        }
    }

    /// Changes result-batch bounds, primarily for tests.
    #[must_use]
    pub fn with_batch_limits(mut self, limits: BatchLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Features implemented by every writable DuckDB session.
    #[must_use]
    pub fn ceiling() -> Capabilities {
        Capabilities::SQL
            | Capabilities::RELATIONAL
            | Capabilities::SCHEMAS
            | Capabilities::TABLES
            | Capabilities::VIEWS
            | Capabilities::INDEXES
            | Capabilities::TRANSACTIONS
            | Capabilities::PREPARED_STATEMENTS
            | Capabilities::STREAMING
            | Capabilities::AFFECTED_ROWS
            | Capabilities::DDL
            | Capabilities::DML
            | Capabilities::READ_ONLY_SESSION
    }
}

impl Default for DuckDbDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::duckdb(), "DuckDB", DriverFamily::Analytical).with_field(
        ConnectionField::new(DuckDbDriver::PATH, "Database file", FieldKind::Path)
            .required()
            .with_help("Path to the DuckDB file. `:memory:` opens a private in-memory database."),
    )
}

fn config(read_only: bool) -> Result<Config> {
    let access_mode = if read_only {
        AccessMode::ReadOnly
    } else {
        AccessMode::ReadWrite
    };
    Config::default()
        .access_mode(access_mode)
        .map_err(error::read)?
        .with("enable_external_access", "false")
        .map_err(error::read)?
        .with("autoload_known_extensions", "false")
        .map_err(error::read)?
        .with("autoinstall_known_extensions", "false")
        .map_err(error::read)
}

#[async_trait]
impl Driver for DuckDbDriver {
    fn id(&self) -> DriverId {
        self.metadata.id.clone()
    }
    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }
    fn capabilities(&self) -> Capabilities {
        Self::ceiling()
    }

    async fn connect(
        &self,
        connection: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        self.metadata.validate(connection)?;
        let _ = credentials;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let path = connection
            .params
            .get(Self::PATH)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OxynError::Config("driver `duckdb`: parameter `path` is required".to_owned())
            })?;
        let config = config(connection.read_only)?;
        let target = if path == Self::MEMORY {
            None
        } else {
            Some(PathBuf::from(path))
        };
        let opened = tokio::task::spawn_blocking(move || match target {
            Some(path) => Connection::open_with_flags(path, config),
            None => Connection::open_in_memory_with_flags(config),
        })
        .await
        .map_err(|_| OxynError::Connection("DuckDB opening task ended unexpectedly".to_owned()))?
        .map_err(error::read)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let mut capabilities = Self::ceiling();
        if connection.read_only {
            capabilities.remove(Capabilities::DDL | Capabilities::DML);
        }
        Ok(Box::new(DuckDbSession::new(
            opened,
            capabilities,
            self.limits,
        )))
    }
}
