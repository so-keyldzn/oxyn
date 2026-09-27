//! What a session can do (ADR-0003).
//!
//! Oxyn has no leveling abstraction: Redis has no schema, Neo4j no tables,
//! Elasticsearch no SQL. A common denominator would reduce each database to
//! its poorest expression. Instead, each session **declares** what it can do,
//! and the interface as well as the agents comply with it.
//!
//! Two consequences that are easy to miss:
//!
//! * capabilities are evaluated **per session**, not per driver. The server's
//!   version, its extensions and the connected account's rights change what is
//!   available: the same PostgreSQL driver talks to a version 12 database
//!   without `MERGE` and to a version 17 database that has it;
//! * **simulate nothing.** A driver that emulates transactions with a chain of
//!   queries lets the user believe a `ROLLBACK` undid their write. Not knowing
//!   how is an acceptable answer; pretending is not.
//!
//! This type lives in `oxyn-core` and not in `oxyn-driver` so that the
//! interface and the agents can read capabilities without depending on a
//! driver implementation.

use std::fmt;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::error::{OxynError, Result};
use crate::query::QueryLanguage;

bitflags! {
    /// Capabilities declared by a session (or, by default, by a driver).
    ///
    /// A missing flag means "I do not know how", never "I will pretend".
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Capabilities: u64 {
        // ── Catalog introspection ──────────────────────────────────────────
        /// The source exposes named schemas.
        const SCHEMAS              = 1 << 0;
        /// The source exposes tables or collections.
        const TABLES               = 1 << 1;
        /// The source exposes views.
        const VIEWS                = 1 << 2;
        /// The source exposes materialized views.
        const MATERIALIZED_VIEWS   = 1 << 3;
        /// Indexes can be introspected.
        const INDEXES              = 1 << 4;
        /// Constraints (uniqueness, check) can be introspected.
        const CONSTRAINTS          = 1 << 5;
        /// Foreign keys can be introspected — that is what makes it possible
        /// to build a relationship diagram without guessing it.
        const FOREIGN_KEYS         = 1 << 6;
        /// Stored functions and procedures can be introspected.
        const ROUTINES             = 1 << 7;
        /// Triggers can be introspected.
        const TRIGGERS             = 1 << 8;
        /// Sequences can be introspected.
        const SEQUENCES            = 1 << 9;
        /// User-defined types can be introspected.
        const USER_TYPES           = 1 << 10;
        /// Object comments are readable.
        const COMMENTS             = 1 << 11;
        /// Rights and roles can be introspected.
        const PERMISSIONS          = 1 << 12;
        /// A row count estimate is available without counting.
        const ROW_COUNT_ESTIMATE   = 1 << 13;
        /// Foreign keys declared by other relations can be found from their target.
        const INCOMING_FOREIGN_KEYS = 1 << 14;
        /// Native object creation statements can be read for inspection.
        const OBJECT_DEFINITION     = 1 << 15;

        // ── Execution ───────────────────────────────────────────────────────
        /// Real transactions, with a `ROLLBACK` that really undoes.
        const TRANSACTIONS         = 1 << 16;
        /// Savepoints inside a transaction.
        const SAVEPOINTS           = 1 << 17;
        /// Prepared statements with server-side bound parameters.
        const PREPARED_STATEMENTS  = 1 << 18;
        /// Named cursors, to go through a result without materializing it.
        const NAMED_CURSORS        = 1 << 19;
        /// Several statements in a single submission.
        const MULTIPLE_STATEMENTS  = 1 << 20;
        /// Cancellation reaches the query **on the server side**
        /// (`pg_cancel_backend`, `KILL QUERY`, `sqlite3_interrupt`). Without
        /// this flag, a "Cancel" button cannot claim to stop the query
        /// (DRIVER-CONTRACT §2).
        const SERVER_SIDE_CANCEL   = 1 << 21;
        /// Results arrive as a stream, without full materialization.
        const STREAMING            = 1 << 22;
        /// The number of rows affected by a write is reliable.
        const AFFECTED_ROWS        = 1 << 23;
        /// `EXPLAIN` or equivalent.
        const EXPLAIN              = 1 << 24;
        /// `EXPLAIN ANALYZE`: the plan is **executed**. It is not a harmless
        /// read on a write statement.
        const EXPLAIN_ANALYZE      = 1 << 25;
        /// The session accepts DDL.
        const DDL                  = 1 << 26;
        /// The session accepts data writes.
        const DML                  = 1 << 27;
        /// The session accepts rights management.
        const GRANT_REVOKE         = 1 << 28;
        /// Dedicated bulk loading (`COPY`, `LOAD DATA`).
        const BULK_LOAD            = 1 << 29;
        /// The server can enforce a read-only session — a much stronger
        /// guarantee than client-side filtering.
        const READ_ONLY_SESSION    = 1 << 30;
        /// The session can declare where it resolves unqualified names, and
        /// report what the server actually kept. An engine without this
        /// capability shows no context selector: its schemas are qualified in
        /// the SQL ([ADR-0019](../../docs/adr/0019-contexte-de-session.md)).
        const SESSION_CONTEXT      = 1 << 31;
        // Bits 16 to 31 of execution are taken; these two continue beyond the
        // languages rather than dislodge one. A flag's number is serialized:
        // reassigning it would change the meaning of a capability already
        // written somewhere.
        /// The source can **order** a preview read. Absent, the row order stays
        /// the one the engine chooses, and no sort control is offered
        /// ([ADR-0020](../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
        const PREVIEW_SORT         = 1 << 42;
        /// The source can **restrict** a preview read to rows that satisfy a
        /// condition. Separate from sorting: an engine can know how to order
        /// without knowing how to filter, and the reverse.
        const PREVIEW_FILTER       = 1 << 43;
        // What the review of a `DROP`, a `TRUNCATE` or a rename must say about
        // what goes ([ADR-0042](../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
        // None is declared without the driver's integration test that proves
        // it against the engine.
        /// The session accepts the `TRUNCATE` statement.
        const TRUNCATE             = 1 << 44;
        /// Every accepted DDL statement, `TRUNCATE` included, applies entirely
        /// or not at all, and obeys the surrounding transaction. Redshift does
        /// not have it: its `TRUNCATE` commits the transaction.
        const TRANSACTIONAL_DDL    = 1 << 45;
        /// Without `CASCADE`, the engine refuses `DROP` and `TRUNCATE` as long
        /// as another object depends on it. SQLite does not have it: a view or
        /// child rows do not prevent a `DROP TABLE` there.
        const RESTRICT_DEPENDENTS  = 1 << 46;

        // ── Accepted languages ──────────────────────────────────────────────
        /// SQL.
        const SQL                  = 1 << 32;
        /// Cypher.
        const CYPHER               = 1 << 33;
        /// Gremlin.
        const GREMLIN              = 1 << 34;
        /// MongoDB document language.
        const MONGO_QUERY          = 1 << 35;
        /// Redis commands.
        const REDIS_COMMAND        = 1 << 36;
        /// Elasticsearch / OpenSearch Query DSL.
        const SEARCH_DSL           = 1 << 37;
        /// CQL.
        const CQL                  = 1 << 38;
        /// PartiQL.
        const PARTIQL              = 1 << 39;
        /// InfluxQL.
        const INFLUXQL             = 1 << 40;
        /// Flux.
        const FLUX                 = 1 << 41;

        // ── Data model ──────────────────────────────────────────────────────
        /// Relational model.
        const RELATIONAL           = 1 << 48;
        /// Document model.
        const DOCUMENT             = 1 << 49;
        /// Key-value model.
        const KEY_VALUE            = 1 << 50;
        /// Graph model.
        const GRAPH                = 1 << 51;
        /// Time series.
        const TIME_SERIES          = 1 << 52;
        /// The source imposes no schema.
        const SCHEMALESS           = 1 << 53;
        /// The exposed schema is **inferred by sampling**, hence fallible. A
        /// field absent from the sample may exist further on
        /// (DRIVER-CONTRACT §3).
        const INFERRED_SCHEMA      = 1 << 54;
        /// Vector search.
        const VECTOR_SEARCH        = 1 << 55;
        /// Full-text search.
        const FULL_TEXT_SEARCH     = 1 << 56;
    }
}

impl Capabilities {
    /// Mask of all language flags.
    ///
    /// Defined outside the macro so as not to appear as a composite flag in
    /// [`Capabilities::iter_names`] — the [`fmt::Display`] rendering would stay
    /// exact, but less precise.
    pub const LANGUAGES: Self = Self::from_bits_retain(
        Self::SQL.bits()
            | Self::CYPHER.bits()
            | Self::GREMLIN.bits()
            | Self::MONGO_QUERY.bits()
            | Self::REDIS_COMMAND.bits()
            | Self::SEARCH_DSL.bits()
            | Self::CQL.bits()
            | Self::PARTIQL.bits()
            | Self::INFLUXQL.bits()
            | Self::FLUX.bits(),
    );

    /// The flag matching a query language.
    #[must_use]
    pub const fn for_language(language: QueryLanguage) -> Self {
        match language {
            QueryLanguage::Sql(_) => Self::SQL,
            QueryLanguage::Cypher => Self::CYPHER,
            QueryLanguage::Gremlin => Self::GREMLIN,
            QueryLanguage::MongoQuery => Self::MONGO_QUERY,
            QueryLanguage::RedisCommand => Self::REDIS_COMMAND,
            QueryLanguage::SearchDsl => Self::SEARCH_DSL,
            QueryLanguage::Cql => Self::CQL,
            QueryLanguage::PartiQl => Self::PARTIQL,
            QueryLanguage::InfluxQl => Self::INFLUXQL,
            QueryLanguage::Flux => Self::FLUX,
        }
    }

    /// Requires the presence of all the requested capabilities.
    ///
    /// # Errors
    /// Returns [`OxynError::NotSupported`] naming the **missing** flags, and
    /// those alone: a message that repeats everything requested helps nobody
    /// understand what blocks.
    pub fn require(&self, cap: Capabilities) -> Result<()> {
        let manquantes = cap.difference(*self);
        if manquantes.is_empty() {
            return Ok(());
        }
        Err(OxynError::NotSupported {
            capability: manquantes.to_string(),
        })
    }

    /// Does the session accept this query language?
    #[must_use]
    pub fn supports_language(&self, language: QueryLanguage) -> bool {
        self.contains(Self::for_language(language))
    }

    /// Requires the language to be accepted.
    ///
    /// # Errors
    /// Returns [`OxynError::NotSupported`] if the language is not declared.
    pub fn require_language(&self, language: QueryLanguage) -> Result<()> {
        self.require(Self::for_language(language))
    }
}

impl fmt::Display for Capabilities {
    /// Lists the active capabilities, separated by ` | `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("(none)");
        }
        let mut premier = true;
        for (nom, _) in self.iter_names() {
            if !premier {
                f.write_str(" | ")?;
            }
            f.write_str(nom)?;
            premier = false;
        }
        // An unnamed bit can only come from `from_bits_retain` on a value read
        // back from a file written by a newer version.
        let nommees = Self::all().intersection(*self);
        let inconnues = self.difference(nommees);
        if !inconnues.is_empty() {
            if !premier {
                f.write_str(" | ")?;
            }
            write!(f, "(unknown: {:#x})", inconnues.bits())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::SqlDialect;

    #[test]
    fn require_succeeds_when_everything_is_present() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS | Capabilities::STREAMING;
        assert!(caps.require(Capabilities::SQL).is_ok());
        assert!(
            caps.require(Capabilities::SQL | Capabilities::TRANSACTIONS)
                .is_ok()
        );
        assert!(caps.require(Capabilities::empty()).is_ok());
    }

    #[test]
    fn require_names_only_what_is_missing() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS;
        let err = caps
            .require(Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL)
            .expect_err("SERVER_SIDE_CANCEL is missing");

        let OxynError::NotSupported { capability } = &err else {
            panic!("wrong error variant: {err:?}");
        };
        assert_eq!(capability.as_str(), "SERVER_SIDE_CANCEL");
        assert!(
            !capability.contains("SQL"),
            "the message must name what is missing, not what is requested"
        );
    }

    #[test]
    fn require_names_every_missing_capability() {
        let caps = Capabilities::SQL;
        let err = caps
            .require(Capabilities::TRANSACTIONS | Capabilities::SAVEPOINTS)
            .expect_err("both are missing");
        let message = err.to_string();
        assert!(message.contains("TRANSACTIONS"), "{message}");
        assert!(message.contains("SAVEPOINTS"), "{message}");
    }

    #[test]
    fn display_lists_the_active_capabilities() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS;
        let rendu = caps.to_string();
        assert!(rendu.contains("SQL"), "{rendu}");
        assert!(rendu.contains("TRANSACTIONS"), "{rendu}");
        assert!(rendu.contains(" | "), "{rendu}");
        assert_eq!(Capabilities::empty().to_string(), "(none)");
    }

    #[test]
    fn an_undeclared_language_is_refused() {
        // A key-value driver does not speak SQL: we refuse, we do not translate.
        let redis = Capabilities::REDIS_COMMAND | Capabilities::KEY_VALUE;
        assert!(redis.supports_language(QueryLanguage::RedisCommand));
        assert!(!redis.supports_language(QueryLanguage::SQL));

        let err = redis
            .require_language(QueryLanguage::Sql(SqlDialect::Postgres))
            .expect_err("SQL is not declared");
        assert!(err.to_string().contains("SQL"));
        assert!(err.is_user_error());
    }

    #[test]
    fn the_dialect_does_not_change_the_language_flag() {
        assert_eq!(
            Capabilities::for_language(QueryLanguage::Sql(SqlDialect::Postgres)),
            Capabilities::for_language(QueryLanguage::Sql(SqlDialect::Sqlite)),
            "the dialect is a grammar nuance, not a distinct capability"
        );
    }

    #[test]
    fn the_language_mask_covers_every_language() {
        for langage in [
            QueryLanguage::SQL,
            QueryLanguage::Cypher,
            QueryLanguage::Gremlin,
            QueryLanguage::MongoQuery,
            QueryLanguage::RedisCommand,
            QueryLanguage::SearchDsl,
            QueryLanguage::Cql,
            QueryLanguage::PartiQl,
            QueryLanguage::InfluxQl,
            QueryLanguage::Flux,
        ] {
            let drapeau = Capabilities::for_language(langage);
            assert!(
                Capabilities::LANGUAGES.contains(drapeau),
                "{langage} missing from the LANGUAGES mask"
            );
        }
    }

    #[test]
    fn flags_do_not_overlap() {
        let mut vus = 0_u64;
        for (nom, drapeau) in Capabilities::all().iter_names() {
            let bits = drapeau.bits();
            assert_eq!(bits.count_ones(), 1, "{nom} is not a simple flag");
            assert_eq!(vus & bits, 0, "{nom} reuses a bit already taken");
            vus |= bits;
        }
    }

    #[test]
    fn json_round_trip() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS | Capabilities::INDEXES;
        let json = serde_json::to_string(&caps).expect("serialization");
        let relu: Capabilities = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(caps, relu);
    }

    /// Two capabilities that share a bit are the same capability.
    ///
    /// Nothing reports it at runtime: `bitflags` merges two constants of the
    /// same value, so `iter_names` only returns one and a test built on it would
    /// pass. A PostgreSQL session declaring it can sort a preview would also
    /// announce that it speaks Cypher, and a capability-driven interface would
    /// draw the wrong surface.
    ///
    /// The declaration is therefore read **in the source**, the only place
    /// where both numbers still exist side by side.
    #[test]
    fn no_flag_shares_its_bit_with_another() {
        let source = include_str!("capabilities.rs");
        let mut vus: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        for ligne in source.lines() {
            let Some((gauche, droite)) = ligne.split_once("= 1 << ") else {
                continue;
            };
            let Some(nom) = gauche.trim().strip_prefix("const ") else {
                continue;
            };
            let Some(numero) = droite.split(';').next() else {
                continue;
            };
            let Ok(bit) = numero.trim().parse::<u32>() else {
                continue;
            };
            let nom = nom.trim().to_owned();
            if let Some(precedent) = vus.insert(bit, nom.clone()) {
                panic!("{nom} and {precedent} share bit {bit}");
            }
        }
        assert!(
            vus.len() >= 50,
            "reading the source found only {} flags: the format changed \
             and this test no longer checks anything",
            vus.len()
        );
    }
}
