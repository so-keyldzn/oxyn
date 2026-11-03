//! The server's variant, and what it changes in the capabilities.
//!
//! One driver per **protocol**, not per product ([ADR-0003]): Redshift,
//! TimescaleDB, pgvector and Citus all speak the PostgreSQL protocol and all go
//! through this crate. What sets them apart is not a separate implementation,
//! it is a different set of [`Capabilities`] — and that is precisely the use the
//! "capabilities per session" model exists for.
//!
//! Detection happens **at connection time**, once, from `version()` and
//! `pg_extension`. It is not guessed from the configuration: two connections to
//! the same host can target two databases of which only one has `pgvector`
//! installed.
//!
//! # What detection does not do
//!
//! It **never** changes execution behavior — no query rewriting, no silent
//! workaround. It only declares what the session can do, leaving it to the
//! interface and the agents to comply. An "Execution plan" panel does not exist
//! in front of a Redshift that lacks `EXPLAIN ANALYZE`; it is not greyed out
//! for no reason.
//!
//! [ADR-0003]: ../../../docs/adr/0003-driver-capabilities.md

use oxyn_core::{Capabilities, SqlDialect};

/// Name of the TimescaleDB extension in `pg_extension`.
pub const EXT_TIMESCALEDB: &str = "timescaledb";
/// Name of the pgvector extension in `pg_extension` — `vector`, not `pgvector`.
pub const EXT_VECTOR: &str = "vector";
/// Name of the Citus extension.
pub const EXT_CITUS: &str = "citus";
/// Name of the PostGIS extension.
pub const EXT_POSTGIS: &str = "postgis";

/// The product behind the protocol.
///
/// Deliberately short: a variant exists here only if it **changes
/// capabilities**. A product that behaves like PostgreSQL does not need to be
/// named, otherwise this enumeration would become the list of products that
/// splitting by protocol precisely seeks to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PostgresFlavor {
    /// PostgreSQL, or a product that differs from it only by its extensions.
    #[default]
    Postgres,
    /// Amazon Redshift: same protocol, truncated grammar and catalog.
    Redshift,
}

impl PostgresFlavor {
    /// Stable name, for auditing and display.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Redshift => "Amazon Redshift",
        }
    }

    /// The SQL dialect to announce to `oxyn-query`.
    ///
    /// Redshift speaks PostgreSQL's protocol without accepting its grammar:
    /// that is exactly why [`SqlDialect`] has a distinct value while there is no
    /// distinct driver crate.
    #[must_use]
    pub const fn dialect(&self) -> SqlDialect {
        match self {
            Self::Postgres => SqlDialect::Postgres,
            Self::Redshift => SqlDialect::Redshift,
        }
    }
}

impl std::fmt::Display for PostgresFlavor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a session learned from its server at connection time.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PostgresVariant {
    /// The product recognized in the banner.
    pub flavor: PostgresFlavor,
    /// `version()`, as is. Used for diagnostics; never parse it anywhere but
    /// here.
    pub banner: String,
    /// `current_setting('server_version')`: `17.2`, `16.4 (Debian …)`.
    pub server_version: String,
    /// The major version number, when it can be read.
    pub major_version: Option<u32>,
    /// The extensions installed **in the current database**, lowercase.
    ///
    /// Empty if `pg_extension` is not readable: that is the case of Redshift,
    /// and of an account with restricted rights. An empty list says "I found
    /// nothing", not "there is nothing".
    pub extensions: Vec<String>,
}

impl PostgresVariant {
    /// Recognizes the variant from the server's banner and its extensions.
    ///
    /// `extensions` is normalized to lowercase and sorted: the comparison
    /// becomes stable, and so does the diagnostic rendering.
    #[must_use]
    pub fn detect(banner: &str, server_version: &str, extensions: Vec<String>) -> Self {
        let flavor = if banner.to_ascii_lowercase().contains("redshift") {
            PostgresFlavor::Redshift
        } else {
            PostgresFlavor::Postgres
        };

        let mut normalized: Vec<String> = extensions
            .into_iter()
            .map(|name| name.trim().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .collect();
        normalized.sort_unstable();
        normalized.dedup();

        Self {
            flavor,
            banner: banner.to_owned(),
            server_version: server_version.to_owned(),
            major_version: parse_major(server_version),
            extensions: normalized,
        }
    }

    /// Is this extension installed?
    #[must_use]
    pub fn has_extension(&self, name: &str) -> bool {
        let needle = name.to_ascii_lowercase();
        self.extensions.contains(&needle)
    }

    /// The product name to display: `PostgreSQL`, `PostgreSQL + TimescaleDB`…
    ///
    /// The extensions that change capabilities are named, the others are not:
    /// the user needs to know why a surface exists, not to read the list of what
    /// is installed.
    #[must_use]
    pub fn product(&self) -> String {
        let mut name = self.flavor.as_str().to_owned();
        let mut marks = Vec::new();
        if self.has_extension(EXT_TIMESCALEDB) {
            marks.push("TimescaleDB");
        }
        if self.has_extension(EXT_VECTOR) {
            marks.push("pgvector");
        }
        if self.has_extension(EXT_CITUS) {
            marks.push("Citus");
        }
        if self.has_extension(EXT_POSTGIS) {
            marks.push("PostGIS");
        }
        if !marks.is_empty() {
            name.push_str(" + ");
            name.push_str(&marks.join(", "));
        }
        name
    }

    /// The SQL dialect of this session.
    #[must_use]
    pub const fn dialect(&self) -> SqlDialect {
        self.flavor.dialect()
    }

    /// The capabilities of this session.
    ///
    /// Starts from the PostgreSQL base ([`base_capabilities`]), removes what
    /// the variant lacks, adds what the extensions bring. The order matters: an
    /// extension never has to re-enable what the variant removed.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        let mut capabilities = base_capabilities();
        if self.major_version.is_none_or(|version| version < 12) {
            capabilities.remove(Capabilities::OBJECT_DEFINITION);
        }

        if self.flavor == PostgresFlavor::Redshift {
            capabilities.remove(redshift_missing());
        }

        if self.has_extension(EXT_TIMESCALEDB) {
            capabilities.insert(Capabilities::TIME_SERIES);
        }
        if self.has_extension(EXT_VECTOR) {
            capabilities.insert(Capabilities::VECTOR_SEARCH);
        }

        capabilities
    }
}

/// What a PostgreSQL session can do, before any variant.
///
/// # Four deliberate absences
///
/// **`MULTIPLE_STATEMENTS`**: this driver uses only the extended protocol,
/// which prepares **one** statement per submission. Splitting a batch belongs
/// to `oxyn-query`. Declaring the capability would force switching to the
/// simple protocol, where the schema is known only after the first row — hence
/// giving up the first display under 100 ms.
///
/// **`TRANSACTIONS`** and **`SAVEPOINTS`**: a transaction lives on **one**
/// connection, yet an Oxyn session relies on a pool and borrows one per
/// execution. Declaring the capability without pinning a connection would let
/// the user believe a `ROLLBACK` cancelled their write, which the contract
/// explicitly forbids. The transaction *internal* to a read-only execution does
/// exist: it is [`Capabilities::READ_ONLY_SESSION`].
///
/// **`NAMED_CURSORS`** and **`BULK_LOAD`**: `DECLARE`/`FETCH` and `COPY` are not
/// implemented. The stream goes through the extended protocol's portal, which
/// is enough for [`Capabilities::STREAMING`].
///
// TODO(phase 1): pin a connection per session to open TRANSACTIONS and
// SAVEPOINTS, and implement `COPY` for BULK_LOAD. Unblocks: data editing with
// DML preview (IMPLEMENTATION-PLAN, phase 1).
// Trap when pinning: `oxyn-exec` bounds every production read to read-only
// (issue #11). Inside an already open transaction, `BEGIN READ ONLY` is only a
// warning, and the closing `ROLLBACK` would cancel the user's transaction. The
// bound must leave the transaction intact there — lead to try: `SAVEPOINT` then
// `SET TRANSACTION READ ONLY` —, otherwise the execution is refused
// (DRIVER-CONTRACT §5).
#[must_use]
pub fn base_capabilities() -> Capabilities {
    Capabilities::SCHEMAS
        | Capabilities::TABLES
        | Capabilities::VIEWS
        | Capabilities::MATERIALIZED_VIEWS
        | Capabilities::INDEXES
        | Capabilities::CONSTRAINTS
        | Capabilities::FOREIGN_KEYS
        | Capabilities::INCOMING_FOREIGN_KEYS
        | Capabilities::OBJECT_DEFINITION
        | Capabilities::ROUTINES
        | Capabilities::TRIGGERS
        | Capabilities::SEQUENCES
        | Capabilities::USER_TYPES
        | Capabilities::COMMENTS
        | Capabilities::PERMISSIONS
        | Capabilities::ROW_COUNT_ESTIMATE
        | Capabilities::PREPARED_STATEMENTS
        | Capabilities::SERVER_SIDE_CANCEL
        | Capabilities::STREAMING
        | Capabilities::AFFECTED_ROWS
        | Capabilities::EXPLAIN
        | Capabilities::EXPLAIN_ANALYZE
        | Capabilities::DDL
        | Capabilities::DML
        | Capabilities::GRANT_REVOKE
        | Capabilities::READ_ONLY_SESSION
        | Capabilities::SESSION_CONTEXT
        // `ORDER BY` and `WHERE` on a preview: the driver composes them, quotes
        // the sort columns and passes the predicate through as is (ADR-0020).
        | Capabilities::PREVIEW_SORT
        | Capabilities::PREVIEW_FILTER
        // Each proven against the server by `ddl_tests` (ADR-0042).
        | Capabilities::TRUNCATE
        | Capabilities::TRANSACTIONAL_DDL
        | Capabilities::RESTRICT_DEPENDENTS
        | Capabilities::SQL
        | Capabilities::RELATIONAL
        | Capabilities::FULL_TEXT_SEARCH
}

/// What Redshift lacks, despite the common protocol.
///
/// * no materialized views exposed through `relkind = 'm'`;
/// * neither triggers, nor sequences, nor user-defined types;
/// * `EXPLAIN` exists, `EXPLAIN ANALYZE` does not — the plan is never executed;
/// * no `tsvector`, hence no native full-text search;
/// * `TRUNCATE` commits the surrounding transaction, and ignores foreign keys:
///   neither transactional DDL, nor refusal on dependents
///   ([ADR-0042](../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
#[must_use]
fn redshift_missing() -> Capabilities {
    // Constraint introspection uses PostgreSQL catalog functions unavailable here.
    Capabilities::CONSTRAINTS
        | Capabilities::INCOMING_FOREIGN_KEYS
        | Capabilities::OBJECT_DEFINITION
        | Capabilities::MATERIALIZED_VIEWS
        | Capabilities::TRIGGERS
        | Capabilities::SEQUENCES
        | Capabilities::USER_TYPES
        | Capabilities::EXPLAIN_ANALYZE
        | Capabilities::FULL_TEXT_SEARCH
        | Capabilities::TRANSACTIONAL_DDL
        | Capabilities::RESTRICT_DEPENDENTS
}

/// The capabilities the **driver** announces before any connection.
///
/// An indicative ceiling, not a promise: what is authoritative is
/// [`PostgresVariant::capabilities`], evaluated once the session is open. It
/// therefore holds the union of what a session can offer at best.
#[must_use]
pub fn driver_capabilities() -> Capabilities {
    base_capabilities() | Capabilities::TIME_SERIES | Capabilities::VECTOR_SEARCH
}

/// The major version number of a `server_version` string.
///
/// PostgreSQL writes `17.2`, `16.4 (Debian 16.4-1)`, sometimes `9.6.24` — and
/// Redshift announces `8.0.2`. Only the first number is read, and nothing is
/// assumed about the rest.
fn parse_major(server_version: &str) -> Option<u32> {
    let head: String = server_version
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    head.parse().ok()
}

#[cfg(test)]
mod tests;
