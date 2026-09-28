//! Oxyn's unified metadata model, its cache and its search.
//!
//! Introspecting a database is **expensive**: minutes on a schema with 20,000
//! objects (ARCHITECTURE §6). This crate is what lets it be paid only once —
//! and what makes possible the two things that follow from it: **exploration
//! without a server round trip**, and **an agent's context** built from the
//! local catalog rather than from a server round trip for every question.
//!
//! # What it contains
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`model`] | the five-level hierarchy, its relations and its fields | ARCHITECTURE §6 |
//! | [`path`] | the qualified path, and identifier quoting | DRIVER-CONTRACT §6, I-10 |
//! | [`literal`] | the SQL string literal composed from a value | I-10 |
//! | [`provider`] | what a session can tell about its structure | DRIVER-CONTRACT §5 |
//! | [`cache`] | the in-memory tree, its freshness, its invalidation | ARCHITECTURE §6 |
//! | [`mod@search`] | the lexical selection of relevant relations | ARCHITECTURE §7.4 |
//!
//! # The four choices that govern this crate
//!
//! **Levels are optional, and not only the last ones.** MySQL has no catalog,
//! Neo4j no namespace, Elasticsearch neither. Nothing is filled with an
//! invented value: a missing level is missing, and [`CatalogPath`] returns it
//! as is — including on the round trip through disk.
//!
//! **An object name is hostile input.** A table named
//! `"users"; DROP TABLE audit; --` is legal in PostgreSQL.
//! [`CatalogPath::qualify`] is the only path through which an identifier joins
//! a query composed by Oxyn (I-10), and [`fn@search`] ranks names without ever
//! composing a prompt.
//!
//! **An empty list and "I don't know" are two different answers.** A
//! [`CatalogProvider`] that cannot introspect indexes refuses; it does not
//! return an empty list, which would assert there are none
//! ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)). The cache keeps
//! the distinction: [`CatalogCache::indexes`] returns `None` for "not read" and
//! an empty slice for "none".
//!
//! **A level never read is not stale.** [`CatalogCache::stale`] only claims
//! what has already been read at least once — otherwise a background refresh
//! would describe the 20,000 relations nobody opened.
//!
//! # Example
//!
//! ```
//! use std::time::Duration;
//!
//! use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, QuoteStyle};
//! use oxyn_catalog::model::{Relation, RelationKind};
//!
//! let mut cache = CatalogCache::new();
//!
//! // A partial insertion: a table is described without its schema being listed.
//! let table = CatalogPath::for_relation(Some("sales"), Some("public"), "clients")?;
//! cache.set_relation(&table, Relation::new("clients", RelationKind::Table))?;
//!
//! // The SQL composed by Oxyn always quotes its identifiers.
//! assert_eq!(table.qualify(QuoteStyle::Double), r#""sales"."public"."clients""#);
//!
//! // After a DDL, the subtree is marked for rereading — without losing what it knows.
//! cache.invalidate(&CatalogScope::Relation(table.clone()));
//! assert!(cache.relation(&table).is_some());
//! assert_eq!(cache.stale(Duration::from_secs(3600)), vec![CatalogScope::Relation(table)]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod cache;
pub mod definition;
pub mod literal;
pub mod model;
pub mod nesting;
pub mod path;
pub mod provider;
pub mod search;

pub use cache::{CacheError, CatalogCache, CatalogHandle, CatalogScope, Freshness, SharedCatalog};
pub use definition::{DefinitionSource, RelationDefinition};
pub use literal::{LiteralError, check_identifier, push_string_literal};
pub use model::{
    CatalogRef, Constraint, ConstraintKind, Field, ForeignKey, ForeignKeyTarget,
    IncomingForeignKey, Index, LogicalType, NamespaceRef, ReferentialAction, Relation,
    RelationKind, RelationRef, ServerInfo,
};
pub use path::{CatalogLevel, CatalogPath, CatalogPathError, QuoteStyle, quote_identifier};
pub use provider::CatalogProvider;
pub use search::{MatchKind, SearchHit, SearchOptions, search};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxyn_core::Capabilities;

    use crate::model::{Field, LogicalType, Relation, RelationKind, RelationRef, ServerInfo};
    use crate::{CatalogCache, CatalogPath, CatalogScope, SearchOptions, search};

    /// The full path of the crate, on the one scenario that brings them all
    /// into play: list, describe, search, undergo a DDL, reread what is stale.
    #[test]
    fn the_full_catalog_path() {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "PostgreSQL",
            "17.2",
            Capabilities::SQL | Capabilities::SCHEMAS | Capabilities::INDEXES,
        ));

        let space = CatalogPath::for_namespace(Some("sales"), "public").expect("valid path");
        cache
            .set_relations(
                &space,
                vec![
                    RelationRef::new(space.clone(), "orders", RelationKind::Table)
                        .expect("valid name"),
                    RelationRef::new(space.clone(), "clients", RelationKind::Table)
                        .expect("valid name"),
                ],
            )
            .expect("a namespace");

        let orders = space.with_relation("orders").expect("valid path");
        cache
            .set_relation(
                &orders,
                Relation::new("orders", RelationKind::Table).with_fields(vec![
                    Field::new("id", 0, LogicalType::INT64, "int8").primary_key(),
                    Field::new("client_id", 1, LogicalType::INT64, "int8").not_null(),
                ]),
            )
            .expect("the path names a relation");

        // The search finds the table by its name, and the other by its field.
        let results = search(&cache, "client", &SearchOptions::default());
        let names: Vec<Option<&str>> = results.iter().map(|hit| hit.path.relation()).collect();
        assert_eq!(names, [Some("clients"), Some("orders")]);

        // An ALTER TABLE issued from Oxyn: the subtree is to be reread, but it
        // stays readable in the meantime.
        cache.invalidate(&CatalogScope::Relation(orders.clone()));
        assert!(cache.relation(&orders).is_some());
        assert_eq!(
            cache.stale(Duration::from_secs(3600)),
            vec![CatalogScope::Relation(orders)]
        );
    }
}
