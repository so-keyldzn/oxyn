//! What the session learned from its server, and what it can therefore do.
//!
//! MySQL and MariaDB speak one protocol and share this crate (ADR-0003). What
//! tells them apart is read at connection time — the version banner — and
//! becomes data here, not a second decoder.

use oxyn_core::{Capabilities, QueryLanguage, SqlDialect};

/// The product behind the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MysqlFlavor {
    /// Oracle's MySQL.
    MySql,
    /// MariaDB. It prepares every statement MySQL refuses to prepare, and
    /// sends `VECTOR` and `JSON` under ordinary string types (ADR-0050).
    MariaDb,
}

impl MysqlFlavor {
    /// The product name, as shown to the user.
    #[must_use]
    pub const fn product(self) -> &'static str {
        match self {
            Self::MySql => "MySQL",
            Self::MariaDb => "MariaDB",
        }
    }
}

/// The server a session talks to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MysqlVariant {
    /// MySQL or MariaDB.
    pub flavor: MysqlFlavor,
    /// The version as the server announces it (`8.4.11`, `11.8.9-MariaDB`).
    pub server_version: String,
    /// Could a second connection of the same account open? Server-side
    /// cancellation depends on it (ADR-0050 §7).
    pub can_kill: bool,
}

impl MysqlVariant {
    /// Reads the flavor from the `VERSION()` banner.
    ///
    /// MariaDB's banner carries `MariaDB` (`11.8.9-MariaDB-ubu2404`); nothing
    /// else is guessed.
    #[must_use]
    pub fn detect(banner: &str, can_kill: bool) -> Self {
        let flavor = if banner.to_ascii_lowercase().contains("mariadb") {
            MysqlFlavor::MariaDb
        } else {
            MysqlFlavor::MySql
        };
        Self {
            flavor,
            server_version: banner.to_owned(),
            can_kill,
        }
    }

    /// The product name.
    #[must_use]
    pub const fn product(&self) -> &'static str {
        self.flavor.product()
    }

    /// What this session can do.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        let mut capabilities = base_capabilities();
        if self.can_kill {
            capabilities.insert(Capabilities::SERVER_SIDE_CANCEL);
        }
        capabilities
    }
}

/// What every MySQL or MariaDB session declares.
///
/// Absent on purpose:
///
/// * `MULTIPLE_STATEMENTS` — ADR-0050 §5: a text is proven one statement by
///   the server's prepare, and a second result set from anything but `CALL`
///   is an error;
/// * `TRANSACTIONAL_DDL` — MySQL commits implicitly before and after DDL;
/// * `RESTRICT_DEPENDENTS` — a view does not stop a `DROP TABLE`;
/// * `SAVEPOINTS`, `EXPLAIN_ANALYZE`, `BULK_LOAD`, `VECTOR_SEARCH` — nothing in
///   the driver exercises them yet, and a flag is only declared once a test
///   holds it (ADR-0042);
/// * `SERVER_SIDE_CANCEL` — added per session, see
///   [`MysqlVariant::capabilities`].
#[must_use]
pub fn base_capabilities() -> Capabilities {
    Capabilities::SQL
        | Capabilities::RELATIONAL
        | Capabilities::SCHEMAS
        | Capabilities::TABLES
        | Capabilities::VIEWS
        | Capabilities::INDEXES
        | Capabilities::FOREIGN_KEYS
        | Capabilities::INCOMING_FOREIGN_KEYS
        | Capabilities::COMMENTS
        | Capabilities::ROW_COUNT_ESTIMATE
        | Capabilities::OBJECT_DEFINITION
        | Capabilities::TRANSACTIONS
        | Capabilities::PREPARED_STATEMENTS
        | Capabilities::STREAMING
        | Capabilities::AFFECTED_ROWS
        | Capabilities::EXPLAIN
        | Capabilities::DDL
        | Capabilities::DML
        | Capabilities::TRUNCATE
        | Capabilities::READ_ONLY_SESSION
        | Capabilities::SESSION_CONTEXT
        | Capabilities::PREVIEW_SORT
        | Capabilities::PREVIEW_FILTER
}

/// The driver's ceiling: the base plus what a session may add.
#[must_use]
pub fn driver_capabilities() -> Capabilities {
    base_capabilities() | Capabilities::SERVER_SIDE_CANCEL
}

/// The language every session accepts.
pub const LANGUAGE: QueryLanguage = QueryLanguage::Sql(SqlDialect::MySql);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mariadb_is_read_from_its_banner() {
        assert_eq!(
            MysqlVariant::detect("11.8.9-MariaDB-ubu2404", true).flavor,
            MysqlFlavor::MariaDb
        );
        assert_eq!(
            MysqlVariant::detect("8.4.11", true).flavor,
            MysqlFlavor::MySql
        );
    }

    #[test]
    fn server_side_cancel_depends_on_the_second_connection() {
        // ADR-0050 §7: declared only if the second connection opened.
        assert!(
            MysqlVariant::detect("9.7.2", true)
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL)
        );
        assert!(
            !MysqlVariant::detect("9.7.2", false)
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL)
        );
    }

    #[test]
    fn several_statements_are_never_declared() {
        // ADR-0050 §5.
        assert!(!driver_capabilities().contains(Capabilities::MULTIPLE_STATEMENTS));
        assert!(!driver_capabilities().contains(Capabilities::TRANSACTIONAL_DDL));
        assert!(driver_capabilities().supports_language(LANGUAGE));
    }
}
