//! What the server and the account allow, read once at connection time
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! Two facts change what a session can do, and both belong to the account,
//! not to the driver:
//!
//! * the account's `readonly` setting. At 1, the server refuses every write
//!   **and** every setting a request carries, the driver's own included; at
//!   2, it refuses writes and lets settings through, but never a lower
//!   `readonly`. Neither level can be undone by a request;
//! * whether the account may read `system.processes`, which `KILL QUERY`
//!   needs even to stop the account's own statement (observed on 26.3,
//!   RESEARCH-NOTES). Without it, `SERVER_SIDE_CANCEL` is not declared.

use oxyn_core::{Capabilities, QueryLanguage, SqlDialect};

/// The language every session declares.
pub const LANGUAGE: QueryLanguage = QueryLanguage::Sql(SqlDialect::ClickHouse);

/// What the server said about itself and the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFacts {
    /// `version()`.
    pub version: String,
    /// `timezone()`: the zone instants without one are shown in.
    pub zone: Option<String>,
    /// The account's `readonly` level, 0 to 2.
    pub readonly: u8,
    /// Whether `KILL QUERY … TEST` succeeded for this account.
    pub can_kill: bool,
    /// Whether a 200 carries `X-ClickHouse-Exception-Tag`, which marks an
    /// error raised after rows left.
    pub marks_exceptions: bool,
}

impl ServerFacts {
    /// What a session on this server and account can do.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        let mut capabilities = base_capabilities();
        if self.readonly > 0 {
            // The account cannot write, whatever a request asks.
            capabilities.remove(Capabilities::DML | Capabilities::DDL);
        }
        if self.can_kill {
            capabilities.insert(Capabilities::SERVER_SIDE_CANCEL);
        }
        capabilities
    }

    /// Whether a request may carry settings at all.
    #[must_use]
    pub const fn takes_settings(&self) -> bool {
        self.readonly != 1
    }

    /// Whether every request must ask the server to answer only once the
    /// statement ended (`wait_end_of_query = 1`): a server that marks no
    /// error in its body then sends the error's status instead of rows. The
    /// result is buffered on the server, not in Oxyn.
    #[must_use]
    pub const fn waits_end_of_query(&self) -> bool {
        !self.marks_exceptions
    }

    /// Whether the driver can tell this server's rows from its errors: the
    /// server marks them, or the account may ask it to wait.
    #[must_use]
    pub const fn reads_errors_apart(&self) -> bool {
        self.marks_exceptions || self.takes_settings()
    }

    /// Whether a request may lower itself to `readonly = 1`.
    #[must_use]
    pub const fn can_bound_reads(&self) -> bool {
        self.readonly == 0
    }
}

/// What every session can do, before the account is known.
///
/// `PREPARED_STATEMENTS` is declared for what it promises the caller: the
/// server binds the values to the statement's `{name:Type}` placeholders,
/// never the text. Nothing is prepared ahead: each run sends its statement.
///
/// Not declared, and why: `TRANSACTIONS` (no transaction in the general
/// case), `MULTIPLE_STATEMENTS` (the server refuses them over HTTP), `AFFECTED_ROWS`
/// (a mutation reports none: it runs after its statement returns),
/// `FOREIGN_KEYS` (ClickHouse has none), `RESTRICT_DEPENDENTS` (no
/// `CASCADE`: a materialized view outlives the table it reads).
/// `TRUNCATE TABLE` is declared: 26.3 runs it on a `MergeTree` table
/// (RESEARCH-NOTES).
#[must_use]
pub const fn base_capabilities() -> Capabilities {
    Capabilities::SQL
        .union(Capabilities::RELATIONAL)
        .union(Capabilities::SCHEMAS)
        .union(Capabilities::TABLES)
        .union(Capabilities::VIEWS)
        .union(Capabilities::MATERIALIZED_VIEWS)
        .union(Capabilities::INDEXES)
        .union(Capabilities::COMMENTS)
        .union(Capabilities::ROW_COUNT_ESTIMATE)
        .union(Capabilities::OBJECT_DEFINITION)
        .union(Capabilities::STREAMING)
        .union(Capabilities::EXPLAIN)
        .union(Capabilities::DDL)
        .union(Capabilities::DML)
        .union(Capabilities::READ_ONLY_SESSION)
        .union(Capabilities::SESSION_CONTEXT)
        .union(Capabilities::PREVIEW_SORT)
        .union(Capabilities::PREVIEW_FILTER)
        .union(Capabilities::TRUNCATE)
        .union(Capabilities::PREPARED_STATEMENTS)
}

/// The ceiling a session may reach: an account that may kill.
#[must_use]
pub const fn driver_capabilities() -> Capabilities {
    base_capabilities().union(Capabilities::SERVER_SIDE_CANCEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(readonly: u8, can_kill: bool) -> ServerFacts {
        ServerFacts {
            version: "26.3.59.10".into(),
            zone: Some("UTC".into()),
            readonly,
            can_kill,
            marks_exceptions: true,
        }
    }

    #[test]
    fn a_server_that_marks_no_error_is_asked_to_wait_or_refused() {
        let unmarked = |readonly| ServerFacts {
            marks_exceptions: false,
            ..facts(readonly, true)
        };
        assert!(unmarked(0).waits_end_of_query() && unmarked(0).reads_errors_apart());
        assert!(unmarked(2).reads_errors_apart());
        assert!(!unmarked(1).reads_errors_apart(), "no setting can be asked");
        assert!(!facts(1, true).waits_end_of_query());
    }

    #[test]
    fn a_read_only_account_declares_no_write() {
        for level in [1, 2] {
            let capabilities = facts(level, false).capabilities();
            assert!(!capabilities.intersects(Capabilities::DML | Capabilities::DDL));
            assert!(capabilities.contains(Capabilities::READ_ONLY_SESSION));
        }
        assert!(facts(0, false).capabilities().contains(Capabilities::DML));
    }

    #[test]
    fn server_side_cancel_is_declared_only_when_the_account_may_kill() {
        assert!(
            !facts(0, false)
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL)
        );
        assert!(
            facts(0, true)
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL)
        );
    }

    #[test]
    fn nothing_is_simulated() {
        for flag in [
            Capabilities::TRANSACTIONS,
            Capabilities::MULTIPLE_STATEMENTS,
            Capabilities::AFFECTED_ROWS,
            Capabilities::FOREIGN_KEYS,
            Capabilities::TRANSACTIONAL_DDL,
        ] {
            assert!(!driver_capabilities().contains(flag), "{flag:?}");
        }
        assert!(driver_capabilities().supports_language(LANGUAGE));
        // The server binds the console's values.
        assert!(driver_capabilities().contains(Capabilities::PREPARED_STATEMENTS));
    }

    #[test]
    fn settings_follow_the_account_level() {
        assert!(facts(0, true).takes_settings());
        assert!(facts(0, true).can_bound_reads());
        assert!(!facts(1, true).takes_settings());
        assert!(facts(2, true).takes_settings());
        assert!(!facts(2, true).can_bound_reads());
    }
}
