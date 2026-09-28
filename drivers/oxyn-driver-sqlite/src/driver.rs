//! The driver: what SQLite says about itself, and opening a session.
//!
//! # A single connection field
//!
//! SQLite has no host, no port, no user, no password: it has a **file**. The form
//! therefore carries a single field, `path`, of kind [`FieldKind::Path`], and the
//! special value [`SqliteDriver::MEMORY`] opens an in-memory database.
//!
//! A second "in-memory database" checkbox would have been more explicit on one
//! side and wrong on the other: two ways of saying the same thing always end up
//! contradicting each other — what to do with a checked box and a filled path?
//! The single field has no such state.
//!
//! # The capabilities, and what makes them vary from one session to another
//!
//! The driver declares a **ceiling**; what is authoritative is
//! [`oxyn_driver::Session::capabilities`], evaluated after opening (ADR-0003).
//! For SQLite, one thing really varies: the file may be opened read-only —
//! because the user marked the connection so, or because the file system
//! imposes it. The session then declares `READ_ONLY_SESSION` and **removes**
//! `DDL` and `DML`: it is not a client-side filter, the engine is what will
//! refuse.
//!
//! # What is not declared, and why
//!
//! | Capability | Why it is absent |
//! |---|---|
//! | `SERVER_SIDE_CANCEL` | SQLite has no server; the interruption is local (see [`session`](crate::session)) |
//! | `COMMENTS` | SQLite has no `COMMENT ON` |
//! | `PERMISSIONS`, `GRANT_REVOKE` | SQLite has no permission model |
//! | `ROW_COUNT_ESTIMATE` | no estimate without `COUNT(*)`, which scans |
//! | `ROUTINES`, `SEQUENCES`, `USER_TYPES`, `MATERIALIZED_VIEWS` | SQLite has none |
//! | `TRIGGERS` | SQLite has them, but `CatalogProvider` has no method to return them yet: declaring a capability without a surface would be a promise |
//! | `SAVEPOINTS` | SQLite has them, but no trait exposes them yet |
//! | `EXPLAIN_ANALYZE` | SQLite has `EXPLAIN QUERY PLAN`, which does not execute |
//! | `BULK_LOAD` | no `COPY` |
//! | `FULL_TEXT_SEARCH` | FTS5 depends on the compile options of the linked engine; declaring it without checking would be faking it |
//!
//! `TRIGGERS`, `SAVEPOINTS` and `FULL_TEXT_SEARCH` are absences of **surface**,
//! not of engine.
//! <!-- TODO(2026-09-10): expose triggers/savepoints and detect FTS5 via compile_options. -->

use std::path::PathBuf;

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};
use rusqlite::Connection;

use crate::error::{self, Effect};
use crate::options::BatchLimits;
use crate::session::SqliteSession;
use crate::worker::{self, OpenSpec, OpenTarget};

/// The SQLite driver: embedded, synchronous, without network.
#[derive(Debug)]
pub struct SqliteDriver {
    metadata: DriverMetadata,
    limits: BatchLimits,
}

impl SqliteDriver {
    /// The value of `path` that opens an **in-memory** database.
    ///
    /// It is private to the connection and disappears when it closes. It is
    /// SQLite's own convention, not an Oxyn invention.
    pub const MEMORY: &'static str = ":memory:";

    /// The key of the only connection field.
    pub const PATH: &'static str = "path";

    /// The driver, with the default batch bounds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: metadata(),
            limits: BatchLimits::new(),
        }
    }

    /// Replaces the bounds of an Arrow batch.
    #[must_use]
    pub fn with_batch_limits(mut self, limits: BatchLimits) -> Self {
        self.limits = limits;
        self
    }

    /// What the driver can offer **at best**, before any opening.
    ///
    /// Defined outside the trait to be readable without an instance: it is the
    /// ceiling from which a session removes what its database does not allow.
    #[must_use]
    pub fn ceiling() -> Capabilities {
        Capabilities::SQL
            | Capabilities::RELATIONAL
            // Introspection, as `SqliteCatalog` actually returns it.
            | Capabilities::SCHEMAS
            | Capabilities::TABLES
            | Capabilities::VIEWS
            | Capabilities::INDEXES
            | Capabilities::CONSTRAINTS
            | Capabilities::FOREIGN_KEYS
            | Capabilities::INCOMING_FOREIGN_KEYS
            | Capabilities::OBJECT_DEFINITION
            // Execution.
            | Capabilities::TRANSACTIONS
            | Capabilities::PREPARED_STATEMENTS
            | Capabilities::STREAMING
            | Capabilities::MULTIPLE_STATEMENTS
            | Capabilities::AFFECTED_ROWS
            | Capabilities::EXPLAIN
            | Capabilities::DDL
            // `BEGIN; DROP TABLE t; ROLLBACK;` gives the table back; proven
            // by `ddl_tests`. Neither `TRUNCATE`, which SQLite does not
            // have, nor `RESTRICT_DEPENDENTS`: a view or child rows do not
            // stop a `DROP TABLE` here (ADR-0042).
            | Capabilities::TRANSACTIONAL_DDL
            | Capabilities::DML
            | Capabilities::READ_ONLY_SESSION
            // Preview: `ORDER BY` on quoted columns, a predicate written by the
            // user, `LIMIT … OFFSET` (ADR-0020). These two survive a read-only
            // session: they only read.
            | Capabilities::PREVIEW_SORT
            | Capabilities::PREVIEW_FILTER
    }

    /// Reads both file flags and the engine's query-only state after opening.
    /// Every request retains this restriction even if the caller asks for writable limits.
    #[must_use]
    fn session_capabilities(read_only: bool) -> Capabilities {
        let mut capabilities = Self::ceiling().difference(Capabilities::READ_ONLY_SESSION);
        if read_only {
            capabilities.remove(Capabilities::DDL | Capabilities::DML);
            capabilities.insert(Capabilities::READ_ONLY_SESSION);
        }
        capabilities
    }
}

impl Default for SqliteDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// The connection form and the driver's identity.
fn metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::sqlite(), "SQLite", DriverFamily::Relational)
        // No `default_port`: an embedded database listens nowhere.
        .with_field(
            // Shown as is in the interface, which is in English (CLAUDE.md,
            // « Langue ») like every message a user reads.
            ConnectionField::new(SqliteDriver::PATH, "Database file", FieldKind::Path)
                .required()
                .with_help(
                    "Path to the SQLite file. `:memory:` opens an in-memory database, \
                     shared by the sessions of this connection and lost when the last one closes.",
                ),
        )
}

#[async_trait]
impl Driver for SqliteDriver {
    fn id(&self) -> DriverId {
        self.metadata.id.clone()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        Self::ceiling()
    }

    /// Opens a database and returns a session.
    ///
    /// `credentials` is **ignored**, and that is correct: SQLite authenticates
    /// nobody. The parameter is not read, not logged, not kept.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the configuration does not suit this driver or if
    /// `path` is missing, [`OxynError::Connection`] if the database does not open,
    /// [`OxynError::Cancelled`] if the token fires during opening.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        // Checks that the configuration targets this driver, that `path` is
        // set, and above all that no secret was persisted with it.
        self.metadata.validate(config)?;
        // SQLite has no credentials. Not reading them is the only thing to do;
        // logging them would be the worst.
        let _ = credentials;

        let path = config
            .params
            .get(Self::PATH)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OxynError::Config(format!(
                    "driver `sqlite`: parameter `{}` is required",
                    Self::PATH
                ))
            })?;

        let target = if path == Self::MEMORY {
            OpenTarget::Memory(config.id)
        } else {
            OpenTarget::File(PathBuf::from(path))
        };
        let spec = OpenSpec {
            target,
            // A connection marked read-only by the user is opened read-only **by
            // the engine**. It is a far stronger guarantee than client-side
            // filtering — and it does not replace the `PolicyGate`, it doubles it.
            read_only: config.read_only,
        };

        let (worker, thread) = worker::spawn(spec, cancel).await?;

        // What the database really allows, asked of the engine rather than
        // inferred from the configuration (ADR-0003).
        let read_only = worker
            .call(cancel, |connection: &Connection| {
                let file_read_only = connection
                    .is_readonly(crate::catalog::MAIN)
                    .map_err(|err| error::engine(err, Effect::ReadOnly))?;
                let query_only: bool = connection
                    .pragma_query_value(None, "query_only", |row| row.get(0))
                    .map_err(|err| error::engine(err, Effect::ReadOnly))?;
                Ok(file_read_only || query_only)
            })
            .await?;

        Ok(Box::new(SqliteSession::new(
            worker,
            thread,
            Self::session_capabilities(read_only),
            self.limits,
        )))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::Environment;

    use super::*;

    #[test]
    fn the_driver_declaration_is_consistent() {
        // What `DriverRegistry::register` checks at registration.
        let driver = SqliteDriver::new();
        driver.metadata().check().expect("consistent declaration");
        assert_eq!(driver.id(), driver.metadata().id);
        assert_eq!(driver.metadata().default_port, None, "nothing listens");
    }

    #[test]
    fn the_form_fits_in_one_path_field() {
        let driver = SqliteDriver::new();
        let fields = &driver.metadata().connection_fields;
        assert_eq!(fields.len(), 1);
        let only_field = fields.first().expect("one field");
        assert_eq!(only_field.key, SqliteDriver::PATH);
        assert_eq!(only_field.kind, FieldKind::Path);
        assert!(only_field.required);
        assert!(
            only_field
                .help
                .as_deref()
                .is_some_and(|text| text.contains(SqliteDriver::MEMORY)),
            "the in-memory option must be stated somewhere"
        );
        assert_eq!(
            driver.metadata().secret_fields().count(),
            0,
            "SQLite authenticates nobody"
        );
    }

    #[test]
    fn server_side_cancel_is_never_declared() {
        // SQLite has no server: `sqlite3_interrupt` is a local interruption, and
        // announcing it as a server cancellation would lie about what the
        // "Cancel" button guarantees.
        assert!(
            !SqliteDriver::ceiling().contains(Capabilities::SERVER_SIDE_CANCEL),
            "{}",
            SqliteDriver::ceiling()
        );
        for read_only in [false, true] {
            assert!(
                !SqliteDriver::session_capabilities(read_only)
                    .contains(Capabilities::SERVER_SIDE_CANCEL)
            );
        }
    }

    #[test]
    fn nothing_is_declared_without_a_matching_surface() {
        let declared_ceiling = SqliteDriver::ceiling();
        for unsupported in [
            Capabilities::COMMENTS,
            Capabilities::PERMISSIONS,
            Capabilities::GRANT_REVOKE,
            Capabilities::ROW_COUNT_ESTIMATE,
            Capabilities::ROUTINES,
            Capabilities::SEQUENCES,
            Capabilities::MATERIALIZED_VIEWS,
            Capabilities::EXPLAIN_ANALYZE,
            Capabilities::BULK_LOAD,
            Capabilities::FULL_TEXT_SEARCH,
            Capabilities::TRIGGERS,
            Capabilities::SAVEPOINTS,
        ] {
            assert!(
                !declared_ceiling.contains(unsupported),
                "capability declared without a surface: {unsupported}"
            );
        }
    }

    #[test]
    fn a_read_only_session_removes_writing() {
        let writing = SqliteDriver::session_capabilities(false);
        assert!(writing.contains(Capabilities::DDL | Capabilities::DML));
        assert!(!writing.contains(Capabilities::READ_ONLY_SESSION));

        let read_caps = SqliteDriver::session_capabilities(true);
        assert!(read_caps.contains(Capabilities::READ_ONLY_SESSION));
        assert!(
            !read_caps.contains(Capabilities::DDL),
            "a session the engine refuses to write must not say otherwise"
        );
        assert!(!read_caps.contains(Capabilities::DML));
        assert!(
            read_caps.contains(Capabilities::SQL | Capabilities::TRANSACTIONS),
            "a read transaction stays possible"
        );
    }

    #[test]
    fn a_configuration_carrying_a_secret_is_refused() {
        // I-03, made checkable: the workspace file committed by mistake.
        let driver = SqliteDriver::new();
        let config = ConnectionConfig::new("workshop", DriverId::sqlite())
            .with_param(SqliteDriver::PATH, "/tmp/workshop.sqlite")
            .with_param("password", "hunter2");
        let err = driver
            .metadata()
            .validate(&config)
            .expect_err("expected refusal");
        assert!(!err.to_string().contains("hunter2"), "{err}");
    }

    #[test]
    fn a_missing_path_is_refused_before_any_opening() {
        let driver = SqliteDriver::new();
        let config = ConnectionConfig::new("workshop", DriverId::sqlite())
            .with_environment(Environment::Local);
        let err = driver
            .metadata()
            .validate(&config)
            .expect_err("`path` is required");
        assert!(err.to_string().contains(SqliteDriver::PATH), "{err}");
    }
}
