//! What a Turso session declares it can do
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! Every session of this driver is the same: the server, the account and the
//! token change nothing the driver can see without a statement of its own, so
//! the declaration is a constant. What is **not** declared is as much the
//! point as what is:
//!
//! | Not declared | Why |
//! |---|---|
//! | `TRANSACTIONS`, `SAVEPOINTS` | every statement runs on a stream of its own that ends with it; a transaction would need a held `baton`, which Turso closes after 10 s of idleness and 5 s of transaction |
//! | `MULTIPLE_STATEMENTS` | the `execute` request takes one statement |
//! | `SERVER_SIDE_CANCEL` | Hrana over HTTP has no interrupt call; a stop drops the request |
//! | `READ_ONLY_SESSION` | the server is not asked to enforce a read-only bound, so a read is bounded by classification alone (SECURITY, connection marking) |
//! | `STREAMING` | the answer is one JSON document, read whole under a budget |
//! | `SESSION_CONTEXT` | there is one database per URL, and no `USE` |
//! | `CONSTRAINTS`, `INCOMING_FOREIGN_KEYS`, `OBJECT_DEFINITION`, `TRUNCATE` | not read yet; ADR-0069 lists them |

use oxyn_core::{Capabilities, QueryLanguage, SqlDialect};

/// The language every session declares: SQLite's, since libSQL is its fork.
pub const LANGUAGE: QueryLanguage = QueryLanguage::Sql(SqlDialect::Sqlite);

/// What every Turso session can do.
///
/// `PREPARED_STATEMENTS` is declared for what it promises the caller: the
/// server binds the values to the statement's placeholders, never the text.
/// Nothing is prepared ahead.
#[must_use]
pub const fn capabilities() -> Capabilities {
    Capabilities::SQL
        .union(Capabilities::RELATIONAL)
        .union(Capabilities::SCHEMAS)
        .union(Capabilities::TABLES)
        .union(Capabilities::VIEWS)
        .union(Capabilities::INDEXES)
        .union(Capabilities::FOREIGN_KEYS)
        .union(Capabilities::PREPARED_STATEMENTS)
        .union(Capabilities::AFFECTED_ROWS)
        .union(Capabilities::EXPLAIN)
        .union(Capabilities::DDL)
        .union(Capabilities::DML)
        .union(Capabilities::PREVIEW_SORT)
        .union(Capabilities::PREVIEW_FILTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_simulated() {
        for flag in [
            Capabilities::TRANSACTIONS,
            Capabilities::SAVEPOINTS,
            Capabilities::MULTIPLE_STATEMENTS,
            Capabilities::SERVER_SIDE_CANCEL,
            Capabilities::READ_ONLY_SESSION,
            Capabilities::STREAMING,
            Capabilities::SESSION_CONTEXT,
            Capabilities::TRANSACTIONAL_DDL,
        ] {
            assert!(!capabilities().contains(flag), "{flag:?}");
        }
    }

    #[test]
    fn the_session_speaks_sqlite_sql_and_binds_values_server_side() {
        assert!(capabilities().supports_language(LANGUAGE));
        assert!(capabilities().contains(Capabilities::PREPARED_STATEMENTS));
        assert!(capabilities().contains(Capabilities::DML | Capabilities::DDL));
    }
}
