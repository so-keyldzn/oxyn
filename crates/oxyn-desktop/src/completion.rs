//! The catalog cache as the editor's completion reads it.
//!
//! Reads the executor-owned cache and nothing else, like [`crate::catalog`]: it
//! never asks a driver, and it is built on the blocking pool, off the UI thread
//! ([I-05](../../../CLAUDE.md#i-05)). What the cache does not hold yet is read
//! by the bus, in bounded passes, not here
//! ([ADR-0057](../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md)).
//!
//! # What it keeps apart
//!
//! * **Not asked, none, and cut.** A namespace never listed is `listed: false`,
//!   a relation never described is [`CompletionFields::NotLoaded`], and what a
//!   bound left out is counted in [`CompletionTruncation`]: an empty list never
//!   stands for any of them.
//! * **A name and the way to write it.** Every name travels beside the form to
//!   insert, composed by [`insertion_form`] — the one quoting of Oxyn — so that
//!   the interface never composes an identifier of its own
//!   ([I-10](../../../CLAUDE.md#i-10)).
//! * **A name and what it shows.** A name that hides text — a control character,
//!   a line separator, a direction control — is not suggested: quoted, the SQL
//!   stays intact, but an editor would show text other than the one that runs.
//!   It is counted as left out, like what a bound cuts.

use oxyn_catalog::{
    CatalogCache, CatalogPath, CatalogScope, Freshness, NamespaceRef, RelationRef, ServerInfo,
    hides_text, insertion_form,
};
use oxyn_core::query::SqlDialect;

use crate::ipc::completion::{
    CompletionCatalog, CompletionField, CompletionFields, CompletionFolding, CompletionNamespace,
    CompletionRelation, CompletionResolution, CompletionTruncation,
};

pub mod plan;

/// How much of the cache one snapshot may carry.
///
/// The cache bounds objects, not bytes, and a name can be long: the snapshot
/// crosses IPC as one message the webview parses on its main thread, so its
/// size is bounded here, in counts **and** in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionBounds {
    /// Relations carried, all namespaces together.
    pub relations: usize,
    /// Columns carried, all relations together.
    pub fields: usize,
    /// Bytes of text carried — names, insertion forms, kinds and types — plus a
    /// fixed overhead per entry for what the message wraps around them.
    pub bytes: usize,
    /// Longest name carried, in bytes. A longer one is left out.
    pub name_bytes: usize,
}

/// The bounds a snapshot is built with.
///
/// Measured on 2026-10-21 (the figures are in PERFORMANCE): a message of one
/// MiB costs about 3 ms to parse on V8 (Node 22.23.2), the engine the
/// measurement could reach — the macOS webview runs another one, so this is an
/// order of magnitude, and the webview's own measurement is the fourth pull
/// request's. 3,000 relations at about 175 bytes and 6,000 columns at about 80
/// make 1.0 MiB with ordinary names; the byte budget is what holds the same
/// size when the names are not ordinary. A name is capped at 256 bytes: that is
/// the longest a MySQL identifier gets — 64 characters, four bytes each at most
/// — and four times PostgreSQL's 63 bytes, so no name of those two engines is
/// left out for its length.
pub const PRODUCT_BOUNDS: CompletionBounds = CompletionBounds {
    relations: 3_000,
    fields: 6_000,
    bytes: 1_048_576,
    name_bytes: 256,
};

/// What a relation's entry weighs beyond its text: the keys and punctuation of
/// its JSON, measured at about 130 bytes.
const RELATION_OVERHEAD: usize = 128;

/// The same for a column, measured at about 40 bytes.
const FIELD_OVERHEAD: usize = 40;

/// The same for a namespace, measured at about 90 bytes.
const NAMESPACE_OVERHEAD: usize = 96;

/// Entries of a resolution path the snapshot carries. A `search_path` of PostgreSQL
/// is a handful of schemas; a path longer than this is not one the completion
/// can reason about, and one entry dropped would let a later namespace pass for
/// the first match.
const MAX_PATH_ENTRIES: usize = 64;

/// A level of the hierarchy that holds relations.
struct Space {
    /// `None` when the relations live at the root of a source with no
    /// namespace level.
    name: Option<String>,
    path: CatalogPath,
    system: bool,
}

/// What is left of the bounds while a snapshot is built, and what they cut.
struct Budget {
    relations: usize,
    fields: usize,
    bytes: usize,
    name_bytes: usize,
    namespaces_left_out: u64,
    relations_left_out: u64,
    fields_left_out: u64,
}

impl Budget {
    /// May this name be carried: it shows what it is, and it is not too long.
    fn admits(&self, name: &str) -> bool {
        name.len() <= self.name_bytes && !hides_text(name)
    }
}

/// The dialect a snapshot is composed for.
///
/// The connection's driver says `postgres` for Redshift too, and the driver id
/// is all the host keeps: only the server's own name for itself tells them
/// apart. Redshift reserves words PostgreSQL does not (`timestamp`, `top`,
/// `open`…), so a Redshift server read as PostgreSQL would be sent reserved
/// words bare. Anything not recognized stays the driver's dialect.
#[must_use]
pub fn dialect_of_server(driver: SqlDialect, server: Option<&ServerInfo>) -> SqlDialect {
    let redshift =
        server.is_some_and(|server| server.product.to_ascii_lowercase().contains("redshift"));
    if driver == SqlDialect::Postgres && redshift {
        SqlDialect::Redshift
    } else {
        driver
    }
}

/// How the dialect compares a name written without quotes with the stored one.
///
/// PostgreSQL folds it to lowercase and compares exactly, so a schema holding
/// both `"Users"` and `users` has one of them for `users`. Redshift is
/// PostgreSQL's in this. The others compare without regard to case, or depend
/// on a server setting the cache does not hold.
#[must_use]
pub const fn folding_of(dialect: SqlDialect) -> CompletionFolding {
    match dialect {
        SqlDialect::Postgres | SqlDialect::Redshift => CompletionFolding::Lower,
        _ => CompletionFolding::Insensitive,
    }
}

/// The snapshot of a connection's cache.
///
/// Namespaces that are not system come first in the budget's order, so a bound
/// cuts `pg_catalog` before it cuts `public`; the answer keeps the cache's own
/// order.
#[must_use]
pub fn snapshot(
    cache: &CatalogCache,
    dialect: SqlDialect,
    bounds: CompletionBounds,
) -> CompletionCatalog {
    let spaces = spaces(cache);
    let mut budget = Budget {
        relations: bounds.relations,
        fields: bounds.fields,
        bytes: bounds.bytes,
        name_bytes: bounds.name_bytes,
        namespaces_left_out: 0,
        relations_left_out: 0,
        fields_left_out: 0,
    };

    let mut priority: Vec<usize> = (0..spaces.len()).collect();
    // Stable: the cache's order holds inside each group.
    priority.sort_by_key(|&index| spaces.get(index).is_some_and(|space| space.system));
    let mut built: Vec<Option<CompletionNamespace>> = spaces.iter().map(|_| None).collect();
    for index in priority {
        let (Some(space), Some(slot)) = (spaces.get(index), built.get_mut(index)) else {
            continue;
        };
        let insert = space
            .name
            .as_deref()
            .map(|name| insertion_form(name, dialect));
        let cost = space.name.as_deref().map_or(0, |name| {
            NAMESPACE_OVERHEAD + name.len() + insert.as_ref().map_or(0, String::len)
        });
        let carried =
            space.name.as_deref().is_none_or(|name| budget.admits(name)) && cost <= budget.bytes;
        if !carried {
            // Its relations cannot be qualified by a name that is not carried,
            // and the namespace itself is counted: a namespace nobody carries
            // may be the one an unqualified name resolves in.
            budget.namespaces_left_out = budget.namespaces_left_out.saturating_add(1);
            let count = cache.relations(&space.path).count();
            budget.relations_left_out = budget
                .relations_left_out
                .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
            continue;
        }
        budget.bytes -= cost;
        *slot = Some(namespace(cache, space, insert, dialect, &mut budget));
    }

    let cut = budget.namespaces_left_out > 0
        || budget.relations_left_out > 0
        || budget.fields_left_out > 0;
    CompletionCatalog {
        namespaces: built.into_iter().flatten().collect(),
        resolution: resolution(cache, &budget),
        folding: folding_of(dialect),
        truncated: cut.then_some(CompletionTruncation {
            namespaces_left_out: budget.namespaces_left_out,
            relations_left_out: budget.relations_left_out,
            fields_left_out: budget.fields_left_out,
        }),
    }
}

/// Where an unqualified name resolves, when the snapshot can carry it whole.
///
/// A path is an ordered list whose first match wins, so a path with an entry
/// dropped — too many, one that hides text, one too long — would let a later
/// namespace pass for the first. Such a path crosses as unknown, which is the
/// answer that suggests nothing unqualified.
fn resolution(cache: &CatalogCache, budget: &Budget) -> CompletionResolution {
    let Some(path) = cache.resolution_path() else {
        return CompletionResolution::Unknown;
    };
    if path.len() > MAX_PATH_ENTRIES || !path.iter().all(|entry| budget.admits(entry)) {
        return CompletionResolution::Unknown;
    }
    CompletionResolution::Known {
        namespaces: path.to_vec(),
        stale: cache.resolution_path_freshness() == Freshness::Invalidated,
    }
}

/// Every level that holds relations, catalogs flattened away.
fn spaces(cache: &CatalogCache) -> Vec<Space> {
    let catalogs: Vec<&str> = cache.catalogs().map(|catalog| catalog.name()).collect();
    if catalogs.is_empty() {
        return spaces_under(cache, None);
    }
    catalogs
        .into_iter()
        .flat_map(|catalog| spaces_under(cache, Some(catalog)))
        .collect()
}

fn spaces_under(cache: &CatalogCache, catalog: Option<&str>) -> Vec<Space> {
    let listed: Vec<&NamespaceRef> = cache.namespaces(catalog).collect();
    if listed.is_empty() {
        // Neither catalog nor namespace: the relations live at the root.
        let root = match catalog {
            None => CatalogPath::empty(),
            Some(name) => match CatalogPath::for_catalog(name) {
                Ok(path) => path,
                // A name `CatalogPath` refuses came from the server: skip the
                // subtree rather than panic (I-09).
                Err(_) => return Vec::new(),
            },
        };
        return vec![Space {
            name: None,
            path: root,
            system: false,
        }];
    }
    listed
        .into_iter()
        .map(|space| Space {
            name: Some(space.name().to_owned()),
            path: space.path(),
            system: space.is_system,
        })
        .collect()
}

fn namespace(
    cache: &CatalogCache,
    space: &Space,
    insert: Option<String>,
    dialect: SqlDialect,
    budget: &mut Budget,
) -> CompletionNamespace {
    let freshness = cache.freshness(&CatalogScope::of(&space.path));
    let stale = freshness == Freshness::Invalidated;
    let mut relations = Vec::new();
    for relation in cache.relations(&space.path) {
        if budget.relations == 0 {
            // Nothing more can be carried: count, without composing an entry
            // the bound would throw away (a flat 50,000-relation schema).
            budget.relations_left_out = budget.relations_left_out.saturating_add(1);
            continue;
        }
        let entry = budget
            .admits(relation.name())
            .then(|| relation_entry(cache, space, relation, dialect, stale))
            .flatten();
        let Some(mut entry) = entry else {
            budget.relations_left_out = budget.relations_left_out.saturating_add(1);
            continue;
        };
        let cost = RELATION_OVERHEAD
            + entry.name.len()
            + entry.insert.len()
            + entry.qualified.len()
            + entry.kind.len();
        if budget.relations == 0 || cost > budget.bytes {
            budget.relations_left_out = budget.relations_left_out.saturating_add(1);
            continue;
        }
        budget.relations -= 1;
        budget.bytes -= cost;
        entry.fields = fields(cache, &relation.path(), dialect, budget);
        relations.push(entry);
    }
    CompletionNamespace {
        insert,
        name: space.name.clone(),
        system: space.system,
        stale,
        listed: freshness.is_known(),
        relations,
    }
}

/// The entry of a relation, its columns still unread.
fn relation_entry(
    cache: &CatalogCache,
    space: &Space,
    relation: &RelationRef,
    dialect: SqlDialect,
    listing_stale: bool,
) -> Option<CompletionRelation> {
    let path = relation.path();
    // A name the model refuses came from the server: leave it out (I-09).
    let qualified = match space.name.as_deref() {
        Some(namespace) => CatalogPath::for_relation(None, Some(namespace), relation.name())
            .ok()?
            .insertion_sql(dialect),
        None => insertion_form(relation.name(), dialect),
    };
    Some(CompletionRelation {
        insert: insertion_form(relation.name(), dialect),
        qualified,
        kind: relation.kind.as_str().to_owned(),
        // The name itself may be gone when the listing is stale.
        stale: listing_stale || cache.freshness(&CatalogScope::of(&path)) == Freshness::Invalidated,
        virtual_table: relation.virtual_table.is_some(),
        shadow: relation.shadow_of.is_some(),
        fields: CompletionFields::NotLoaded,
        name: relation.name().to_owned(),
    })
}

/// The columns of a described relation, or why they are not all carried.
///
/// A column whose name hides text or is too long is left out and counted, and
/// the others are carried. A relation whose columns do not fit the budget is
/// carried with none, as [`CompletionFields::Omitted`]: half a table's columns
/// would pass for the table.
fn fields(
    cache: &CatalogCache,
    path: &CatalogPath,
    dialect: SqlDialect,
    budget: &mut Budget,
) -> CompletionFields {
    let Some(described) = cache.relation(path) else {
        return CompletionFields::NotLoaded;
    };
    let mut items = Vec::with_capacity(described.fields.len());
    let mut cost = 0_usize;
    let mut refused = 0_u64;
    for field in &described.fields {
        if !budget.admits(&field.name) {
            refused = refused.saturating_add(1);
            continue;
        }
        let item = CompletionField {
            insert: insertion_form(&field.name, dialect),
            name: field.name.clone(),
            // Display only, and from the server like the name: a type that
            // hides text is shown as none rather than as other text.
            data_type: if hides_text(&field.raw_type) {
                String::new()
            } else {
                shortened(&field.raw_type, budget.name_bytes)
            },
        };
        cost = cost.saturating_add(
            FIELD_OVERHEAD + item.name.len() + item.insert.len() + item.data_type.len(),
        );
        items.push(item);
    }
    if items.len() > budget.fields || cost > budget.bytes {
        let count = u64::try_from(described.fields.len()).unwrap_or(u64::MAX);
        budget.fields_left_out = budget.fields_left_out.saturating_add(count);
        return CompletionFields::Omitted { count };
    }
    budget.fields -= items.len();
    budget.bytes -= cost;
    budget.fields_left_out = budget.fields_left_out.saturating_add(refused);
    CompletionFields::Loaded { items }
}

/// A type as the server spells it, cut to `limit` bytes at a character
/// boundary with an ellipsis when it is longer — a MySQL `enum(…)` can be.
/// Display only: the type is never inserted.
fn shortened(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut cut = text.get(..end).unwrap_or_default().to_owned();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests;
