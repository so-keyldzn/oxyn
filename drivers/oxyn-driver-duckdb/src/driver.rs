//! The driver: what DuckDB says about itself, and opening a session.
//!
//! # Two connection fields
//!
//! DuckDB has a **file**, no host, no user, no password. [`DuckDbDriver::PATH`]
//! names it — `:memory:` opens a private in-memory database — and the generic
//! read-only switch of the connection opens it read-only **in the engine**.
//! [`DuckDbDriver::FILE_ACCESS`] is the one directory SQL may read and write
//! files in (`read_csv`, `read_parquet`, `COPY … TO`, `ATTACH`); empty, SQL
//! reaches no file but the database (`crate::open`).
//!
//! # Capabilities, per session
//!
//! What varies is the access mode: a read-only session declares
//! `READ_ONLY_SESSION` and loses `DDL`, `DML`, `TRUNCATE` and
//! `TRANSACTIONAL_DDL` — the engine refuses them, not a client filter.
//!
//! | Not declared | Why |
//! |---|---|
//! | `SERVER_SIDE_CANCEL` | the engine runs in Oxyn's process; the interruption is local ([`crate::session`]) |
//! | `PERMISSIONS`, `GRANT_REVOKE` | DuckDB has no users and no privileges |
//! | `MATERIALIZED_VIEWS`, `TRIGGERS` | DuckDB has neither |
//! | `ROUTINES`, `SEQUENCES`, `USER_TYPES` | DuckDB has macros, sequences and types, but `CatalogProvider` has no method to list them yet: a capability without a surface would be a promise |
//! | `SAVEPOINTS` | DuckDB has no savepoints |
//! | `NAMED_CURSORS` | no `DECLARE … CURSOR` |
//! | `RESTRICT_DEPENDENTS` | DuckDB does not track a view's dependency on its tables: a view does not stop a `DROP TABLE` |
//! | `BULK_LOAD` | `COPY … FROM` exists, but nothing in Oxyn drives it yet |

use std::path::PathBuf;

use async_trait::async_trait;
use duckdb::Connection;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};

use crate::open::{OpenSpec, OpenTarget};
use crate::options::BatchLimits;
use crate::session::{self, DuckDbSession};
use crate::worker;

/// The embedded DuckDB driver.
#[derive(Debug)]
pub struct DuckDbDriver {
    metadata: DriverMetadata,
    limits: BatchLimits,
}

impl DuckDbDriver {
    /// The key of the database file field.
    pub const PATH: &'static str = "path";
    /// The value of [`Self::PATH`] that opens a private in-memory database —
    /// DuckDB's own spelling.
    pub const MEMORY: &'static str = ":memory:";
    /// The key of the directory SQL may read and write files in.
    pub const FILE_ACCESS: &'static str = "file_access";

    /// The driver, with the default batch bounds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: metadata(),
            limits: BatchLimits::default(),
        }
    }

    /// Replaces the bounds of an Arrow batch.
    #[must_use]
    pub fn with_batch_limits(mut self, limits: BatchLimits) -> Self {
        self.limits = limits;
        self
    }

    /// What a writable session offers; a read-only one removes from it.
    #[must_use]
    pub fn ceiling() -> Capabilities {
        Capabilities::SQL
            | Capabilities::RELATIONAL
            // Introspection, as `DuckDbCatalog` returns it.
            | Capabilities::SCHEMAS
            | Capabilities::TABLES
            | Capabilities::VIEWS
            | Capabilities::INDEXES
            | Capabilities::CONSTRAINTS
            | Capabilities::FOREIGN_KEYS
            | Capabilities::INCOMING_FOREIGN_KEYS
            | Capabilities::OBJECT_DEFINITION
            | Capabilities::COMMENTS
            | Capabilities::ROW_COUNT_ESTIMATE
            // Execution.
            | Capabilities::TRANSACTIONS
            | Capabilities::PREPARED_STATEMENTS
            | Capabilities::STREAMING
            | Capabilities::MULTIPLE_STATEMENTS
            | Capabilities::AFFECTED_ROWS
            | Capabilities::EXPLAIN
            | Capabilities::EXPLAIN_ANALYZE
            | Capabilities::DDL
            | Capabilities::DML
            | Capabilities::TRUNCATE
            // `BEGIN; DROP TABLE t; ROLLBACK;` gives the table back: proven by
            // the integration tests.
            | Capabilities::TRANSACTIONAL_DDL
            | Capabilities::SESSION_CONTEXT
            | Capabilities::PREVIEW_SORT
            | Capabilities::PREVIEW_FILTER
            | Capabilities::READ_ONLY_SESSION
    }

    /// The capabilities of a session opened with this access mode.
    #[must_use]
    pub fn session_capabilities(read_only: bool) -> Capabilities {
        let mut capabilities = Self::ceiling().difference(Capabilities::READ_ONLY_SESSION);
        if read_only {
            capabilities.remove(
                Capabilities::DDL
                    | Capabilities::DML
                    | Capabilities::TRUNCATE
                    | Capabilities::TRANSACTIONAL_DDL,
            );
            capabilities.insert(Capabilities::READ_ONLY_SESSION);
        }
        capabilities
    }
}

impl Default for DuckDbDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// The connection form and the driver's identity.
fn metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::duckdb(), "DuckDB", DriverFamily::Analytical)
        .with_field(
            ConnectionField::new(DuckDbDriver::PATH, "Database file", FieldKind::Path)
                .required()
                .with_help(
                    "Path to the DuckDB file, created if it does not exist. `:memory:` opens an \
                     in-memory database, shared by the sessions of this connection and lost when \
                     the last one closes. A file is held by one process: close it in other tools \
                     first, or open it read-only there and here.",
                ),
        )
        .with_field(
            ConnectionField::new(
                DuckDbDriver::FILE_ACCESS,
                "Folder SQL may read and write",
                FieldKind::Path,
            )
            .with_help(
                "Lets read_csv, read_parquet, read_json, COPY … TO and ATTACH reach files in this \
                 folder, and nowhere else. Empty: SQL reaches no file but the database. Network \
                 access and extension downloads stay off either way.",
            ),
        )
}

/// The file access directory, when one is set. That it exists is checked on
/// the carrier thread ([`crate::open`]): a `stat` on a stale network mount
/// can block for long, and never on the runtime (I-05).
fn file_access(config: &ConnectionConfig) -> Option<PathBuf> {
    config
        .params
        .get(DuckDbDriver::FILE_ACCESS)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Whether DuckDB would open `path` under another spelling: it replaces a
/// leading `~` with the home directory and strips a `file:` URL prefix
/// (`FileSystem::ExpandPath`, `src/common/file_system.cpp`, v1.5.6). The
/// registry would then key one file twice and open two engines on it
/// (`crate::open`); expanding `~` here would mean reading the environment,
/// which a driver does not.
pub(crate) fn rewritten_by_the_engine(path: &str) -> bool {
    path.starts_with('~')
        || path
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
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

    /// Opens the database on a carrier thread and returns a session.
    ///
    /// `credentials` is ignored: DuckDB authenticates nobody.
    ///
    /// # Errors
    /// [`OxynError::Config`] for a missing path or folder, a permanent driver
    /// error when the file is locked by another process, damaged, or open here
    /// with other settings, [`OxynError::Connection`] otherwise,
    /// [`OxynError::Cancelled`] if the token fires before opening.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        self.metadata.validate(config)?;
        let _ = credentials;
        let path = config
            .params
            .get(Self::PATH)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OxynError::Config(format!(
                    "driver `duckdb`: parameter `{}` is required",
                    Self::PATH
                ))
            })?;
        if rewritten_by_the_engine(path) {
            return Err(OxynError::Config(format!(
                "driver `duckdb`: parameter `{}` starts with `~` or `file:`, which DuckDB \
                 rewrites itself; write the full path of the file",
                Self::PATH
            )));
        }
        let target = if path == Self::MEMORY {
            OpenTarget::Memory(config.id)
        } else {
            OpenTarget::File(PathBuf::from(path))
        };
        let spec = OpenSpec {
            target,
            read_only: config.read_only,
            file_access: file_access(config),
        };
        let (worker, thread) = worker::spawn(spec, cancel).await?;
        let (origin, version) = worker
            .call(cancel, |connection: &Connection| {
                let origin = session::current_context(connection)?;
                let version = crate::query::read(connection, "SELECT version()", &[])?
                    .rows()
                    .next()
                    .and_then(|row| row.text(0))
                    .unwrap_or_default();
                Ok((origin, version))
            })
            .await?;
        Ok(Box::new(DuckDbSession::new(
            worker,
            thread,
            Self::session_capabilities(config.read_only),
            self.limits,
            origin,
            version,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_only_session_loses_what_writes() {
        let capabilities = DuckDbDriver::session_capabilities(true);
        assert!(capabilities.contains(Capabilities::READ_ONLY_SESSION));
        for writes in [
            Capabilities::DDL,
            Capabilities::DML,
            Capabilities::TRUNCATE,
            Capabilities::TRANSACTIONAL_DDL,
        ] {
            assert!(!capabilities.contains(writes), "{writes:?}");
        }
        let writable = DuckDbDriver::session_capabilities(false);
        assert!(!writable.contains(Capabilities::READ_ONLY_SESSION));
        assert!(writable.contains(Capabilities::DML));
    }

    #[test]
    fn a_path_the_engine_would_respell_is_recognized() {
        for respelled in [
            "~/sales.duckdb",
            "~",
            "file:///tmp/a.duckdb",
            "FILE:/tmp/a.duckdb",
        ] {
            assert!(rewritten_by_the_engine(respelled), "{respelled}");
        }
        for plain in [
            "/tmp/a.duckdb",
            "a~b.duckdb",
            "files/a.duckdb",
            ":memory:",
            "é",
        ] {
            assert!(!rewritten_by_the_engine(plain), "{plain}");
        }
    }

    #[test]
    fn nothing_is_claimed_that_the_engine_does_not_have() {
        let ceiling = DuckDbDriver::ceiling();
        for absent in [
            Capabilities::SERVER_SIDE_CANCEL,
            Capabilities::GRANT_REVOKE,
            Capabilities::PERMISSIONS,
            Capabilities::SAVEPOINTS,
            Capabilities::RESTRICT_DEPENDENTS,
        ] {
            assert!(!ceiling.contains(absent), "{absent:?}");
        }
    }
}
