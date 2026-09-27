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

        let mut normalisees: Vec<String> = extensions
            .into_iter()
            .map(|nom| nom.trim().to_ascii_lowercase())
            .filter(|nom| !nom.is_empty())
            .collect();
        normalisees.sort_unstable();
        normalisees.dedup();

        Self {
            flavor,
            banner: banner.to_owned(),
            server_version: server_version.to_owned(),
            major_version: parse_major(server_version),
            extensions: normalisees,
        }
    }

    /// Is this extension installed?
    #[must_use]
    pub fn has_extension(&self, name: &str) -> bool {
        let cherche = name.to_ascii_lowercase();
        self.extensions.contains(&cherche)
    }

    /// The product name to display: `PostgreSQL`, `PostgreSQL + TimescaleDB`…
    ///
    /// The extensions that change capabilities are named, the others are not:
    /// the user needs to know why a surface exists, not to read the list of what
    /// is installed.
    #[must_use]
    pub fn product(&self) -> String {
        let mut nom = self.flavor.as_str().to_owned();
        let mut marques = Vec::new();
        if self.has_extension(EXT_TIMESCALEDB) {
            marques.push("TimescaleDB");
        }
        if self.has_extension(EXT_VECTOR) {
            marques.push("pgvector");
        }
        if self.has_extension(EXT_CITUS) {
            marques.push("Citus");
        }
        if self.has_extension(EXT_POSTGIS) {
            marques.push("PostGIS");
        }
        if !marques.is_empty() {
            nom.push_str(" + ");
            nom.push_str(&marques.join(", "));
        }
        nom
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
        let mut capacites = base_capabilities();
        if self.major_version.is_none_or(|version| version < 12) {
            capacites.remove(Capabilities::OBJECT_DEFINITION);
        }

        if self.flavor == PostgresFlavor::Redshift {
            capacites.remove(redshift_missing());
        }

        if self.has_extension(EXT_TIMESCALEDB) {
            capacites.insert(Capabilities::TIME_SERIES);
        }
        if self.has_extension(EXT_VECTOR) {
            capacites.insert(Capabilities::VECTOR_SEARCH);
        }

        capacites
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
    let tete: String = server_version
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    tete.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNIERE_PG: &str =
        "PostgreSQL 17.2 on aarch64-apple-darwin, compiled by Apple clang 16.0.0, 64-bit";
    const BANNIERE_REDSHIFT: &str = "PostgreSQL 8.0.2 on i686-pc-linux-gnu, compiled by GCC gcc (GCC) 3.4.2, Redshift 1.0.75008";

    #[test]
    fn a_bare_postgres_declares_the_base() {
        let variante = PostgresVariant::detect(BANNIERE_PG, "17.2", Vec::new());
        assert_eq!(variante.flavor, PostgresFlavor::Postgres);
        assert_eq!(variante.major_version, Some(17));
        assert_eq!(variante.dialect(), SqlDialect::Postgres);
        assert_eq!(variante.product(), "PostgreSQL");
        assert_eq!(variante.capabilities(), base_capabilities());
    }

    #[test]
    fn the_base_does_not_promise_what_the_driver_does_not_do() {
        // "Not knowing how is an acceptable answer; letting the user believe
        // otherwise is not": the session declares neither transactions, nor
        // multiple statements, nor COPY, because it does not implement them.
        let socle = base_capabilities();
        for absente in [
            Capabilities::TRANSACTIONS,
            Capabilities::SAVEPOINTS,
            Capabilities::MULTIPLE_STATEMENTS,
            Capabilities::NAMED_CURSORS,
            Capabilities::BULK_LOAD,
        ] {
            assert!(!socle.contains(absente), "{absente} must not be declared");
        }
    }

    #[test]
    fn server_side_cancel_is_declared_because_it_exists() {
        // `pg_cancel_backend`: without this flag, the "Cancel" button cannot
        // claim to cut the query (DRIVER-CONTRACT §2).
        assert!(base_capabilities().contains(Capabilities::SERVER_SIDE_CANCEL));
        assert!(base_capabilities().contains(Capabilities::STREAMING));
    }

    #[test]
    fn pgvector_opens_vector_search_on_the_session_that_has_it() {
        // Two databases of the same server can differ: that is the whole point
        // of the "capabilities per session" model (ADR-0003).
        let sans = PostgresVariant::detect(BANNIERE_PG, "17.2", Vec::new());
        let avec = PostgresVariant::detect(BANNIERE_PG, "17.2", vec!["vector".to_owned()]);

        assert!(!sans.capabilities().contains(Capabilities::VECTOR_SEARCH));
        assert!(avec.capabilities().contains(Capabilities::VECTOR_SEARCH));
        assert_eq!(avec.product(), "PostgreSQL + pgvector");
    }

    #[test]
    fn timescaledb_opens_time_series() {
        let variante = PostgresVariant::detect(
            BANNIERE_PG,
            "16.4",
            vec!["TimescaleDB".to_owned(), "plpgsql".to_owned()],
        );
        assert!(variante.has_extension(EXT_TIMESCALEDB), "case-insensitive");
        assert!(variante.capabilities().contains(Capabilities::TIME_SERIES));
        assert_eq!(variante.product(), "PostgreSQL + TimescaleDB");
    }

    #[test]
    fn citus_is_named_without_changing_capabilities() {
        // Citus distributes tables; it adds no surface Oxyn can use today.
        // Saying so is more honest than inventing a flag.
        let variante = PostgresVariant::detect(BANNIERE_PG, "17.2", vec!["citus".to_owned()]);
        assert_eq!(variante.capabilities(), base_capabilities());
        assert_eq!(variante.product(), "PostgreSQL + Citus");
    }

    #[test]
    fn redshift_is_recognized_by_its_banner_and_loses_what_it_lacks() {
        let variante = PostgresVariant::detect(BANNIERE_REDSHIFT, "8.0.2", Vec::new());
        assert_eq!(variante.flavor, PostgresFlavor::Redshift);
        assert_eq!(variante.dialect(), SqlDialect::Redshift);
        assert_eq!(variante.product(), "Amazon Redshift");

        let capacites = variante.capabilities();
        assert!(
            !capacites.contains(Capabilities::EXPLAIN_ANALYZE),
            "Redshift does not execute the plan it explains"
        );
        assert!(!capacites.contains(Capabilities::TRIGGERS));
        assert!(!capacites.contains(Capabilities::CONSTRAINTS));
        assert!(!capacites.contains(Capabilities::FULL_TEXT_SEARCH));

        // What it has, it keeps.
        assert!(capacites.contains(Capabilities::SERVER_SIDE_CANCEL));
        assert!(capacites.contains(Capabilities::SQL));
        assert!(capacites.contains(Capabilities::TABLES));
        // `ORDER BY`, `WHERE` and `LIMIT … OFFSET` are common grammar: what
        // Redshift truncates is the type catalog, not selection. Removing them
        // would deprive the user of controls that work.
        assert!(capacites.contains(Capabilities::PREVIEW_SORT));
        assert!(capacites.contains(Capabilities::PREVIEW_FILTER));
        // `TRUNCATE` exists, but commits the transaction and ignores foreign
        // keys: the review must promise neither rollback nor refusal.
        assert!(capacites.contains(Capabilities::TRUNCATE));
        assert!(!capacites.contains(Capabilities::TRANSACTIONAL_DDL));
        assert!(!capacites.contains(Capabilities::RESTRICT_DEPENDENTS));
    }

    #[test]
    fn an_extension_does_not_cancel_an_absence_of_the_variant() {
        // The order of the computation matters: pgvector on Redshift does not
        // bring `EXPLAIN ANALYZE` back.
        let variante = PostgresVariant::detect(
            BANNIERE_REDSHIFT,
            "8.0.2",
            vec!["vector".to_owned(), "timescaledb".to_owned()],
        );
        let capacites = variante.capabilities();
        assert!(capacites.contains(Capabilities::VECTOR_SEARCH));
        assert!(!capacites.contains(Capabilities::EXPLAIN_ANALYZE));
    }

    #[test]
    fn the_major_version_is_read_on_real_forms() {
        assert_eq!(parse_major("17.2"), Some(17));
        assert_eq!(parse_major("16.4 (Debian 16.4-1.pgdg120+1)"), Some(16));
        assert_eq!(parse_major("9.6.24"), Some(9));
        assert_eq!(parse_major(" 15beta1"), Some(15));
        assert_eq!(parse_major("inconnue"), None, "nothing is guessed");
        assert_eq!(parse_major(""), None);
    }

    #[test]
    fn extensions_are_normalized_and_deduplicated() {
        let variante = PostgresVariant::detect(
            BANNIERE_PG,
            "17.2",
            vec![
                "Vector".to_owned(),
                " vector ".to_owned(),
                String::new(),
                "PostGIS".to_owned(),
            ],
        );
        assert_eq!(variante.extensions, ["postgis", "vector"]);
    }

    #[test]
    fn the_driver_ceiling_covers_every_session() {
        // The driver announces at best; the session is authoritative. No
        // session must be able to declare a capability the driver does not
        // announce, otherwise a surface would appear without ever having been
        // planned.
        let plafond = driver_capabilities();
        for variante in [
            PostgresVariant::detect(BANNIERE_PG, "17.2", Vec::new()),
            PostgresVariant::detect(BANNIERE_PG, "17.2", vec!["vector".to_owned()]),
            PostgresVariant::detect(BANNIERE_PG, "16.4", vec!["timescaledb".to_owned()]),
            PostgresVariant::detect(BANNIERE_REDSHIFT, "8.0.2", Vec::new()),
        ] {
            let session = variante.capabilities();
            assert!(
                plafond.contains(session),
                "{} declares {} outside the ceiling",
                variante.product(),
                session.difference(plafond)
            );
        }
    }
}
