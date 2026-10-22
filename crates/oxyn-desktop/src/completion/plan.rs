//! What the editor's pass reads next
//! ([ADR-0057](../../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md) § 2).
//!
//! The cache is empty where the user needs it first: a console edited before
//! the tree was expanded would complete nothing. The editor therefore asks the
//! host, after a pause in typing and never on a keystroke, to read what the
//! statement under the cursor needs and the cache lacks — and nothing more:
//!
//! 1. the server level, then the **resolution path**;
//! 2. the relations of each namespace of that path, and of each namespace the
//!    statement qualifies with — at most [`MAX_LISTINGS`];
//! 3. the columns of each relation the statement names — at most
//!    [`MAX_DESCRIPTIONS`].
//!
//! Nothing here reads: [`next_read`] looks at the cache and answers with the one
//! read to make, so that what a pass decides is testable without a server, and
//! the loop that submits it (`backend::completion_load`) stays about
//! the bus.
//!
//! # Names are resolved here, in Rust
//!
//! The editor sends the names as it lexed them — `users`, `public.orders`,
//! `"Order Lines"` — and the host decides which relation each one is, with the
//! resolution path the server reported, so that a name the cache cannot place
//! costs no query and **cannot steer a read toward an arbitrary object**. Only
//! relations the cache has *listed* are described, and the scope of every read
//! is built from a path the cache itself holds: a name typed in the editor is
//! compared, never composed ([I-10](../../../../CLAUDE.md#i-10)).

use std::collections::HashSet;
use std::time::Duration;

use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, Freshness};

use super::{Space, spaces};
use crate::ipc::completion::CompletionNamePart;

/// The most namespaces one pass lists the relations of.
///
/// Product bound, the assistant's: a working database has a handful of
/// schemas, one per tenant can have hundreds, each listing a round trip. The
/// next pass continues with those left, since they are still unread.
pub const MAX_LISTINGS: usize = 32;

/// The most relations one pass describes.
///
/// Product bound. A statement names a few relations, and a pass is repeated at
/// every pause in typing; describing is three queries a relation (columns,
/// indexes, foreign keys), so sixteen of them on a remote server at 50 ms of
/// round trip are under three seconds — the arithmetic of the assistant's own
/// bound (`catalog_fill`), which is not re-measured here: it needs a server.
pub const MAX_DESCRIPTIONS: usize = 16;

/// The longest a pass may take, reads included.
///
/// Product bound, the assistant's: past it, the completion goes on with what
/// is loaded, and the next pass continues. The editor never waits for it.
pub const DEADLINE: Duration = Duration::from_secs(5);

/// One read of a pass.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Read {
    /// A level of the tree — the command its expansion submits.
    Scope(CatalogScope),
    /// The namespaces an unqualified name resolves in.
    ResolutionPath,
}

/// What a pass has spent of its bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spent {
    pub listings: usize,
    pub descriptions: usize,
}

/// Read, and not invalidated since. A level never read, or read before a DDL
/// Oxyn ran, is to read.
const fn fetched(freshness: Freshness) -> bool {
    matches!(freshness, Freshness::Fetched(_))
}

/// Whether a name typed in the editor is `name`: a quoted part exactly, an
/// unquoted one without case, as the servers Oxyn speaks to fold it — the rule
/// of the editor's own matching (`sameName`).
fn same(part: &CompletionNamePart, name: &str) -> bool {
    if part.quoted {
        part.text == name
    } else {
        part.text.to_lowercase() == name.to_lowercase()
    }
}

/// The next read the editor's pass should make, or `None` when it is done.
///
/// `tried` is what this pass already submitted — read or failed, never asked
/// twice — and `spent` what it has used of [`MAX_LISTINGS`] and
/// [`MAX_DESCRIPTIONS`].
#[must_use]
pub fn next_read(
    cache: &CatalogCache,
    names: &[Vec<CompletionNamePart>],
    tried: &HashSet<Read>,
    spent: Spent,
) -> Option<Read> {
    // The server first: without the session's capabilities, the cache cannot
    // tell which levels exist.
    let server = Read::Scope(CatalogScope::Server);
    let server_read =
        cache.server_info().is_some() && fetched(cache.freshness(&CatalogScope::Server));
    if !server_read && !tried.contains(&server) {
        return Some(server);
    }
    // Then the path, which says which namespaces to list. A source that cannot
    // tell refuses, and the cache stays on « unknown »: asked once per pass.
    let path_read = cache.resolution_path().is_some() && fetched(cache.resolution_path_freshness());
    if !path_read && !tried.contains(&Read::ResolutionPath) {
        return Some(Read::ResolutionPath);
    }
    let spaces = spaces(cache);
    if spent.listings < MAX_LISTINGS
        && let Some(scope) = next_listing(cache, &spaces, names, tried)
    {
        return Some(Read::Scope(scope));
    }
    if spent.descriptions < MAX_DESCRIPTIONS
        && let Some(scope) = next_description(cache, &spaces, names, tried)
    {
        return Some(Read::Scope(scope));
    }
    None
}

fn unread(cache: &CatalogCache, scope: &CatalogScope, tried: &HashSet<Read>) -> bool {
    !tried.contains(&Read::Scope(scope.clone())) && !fetched(cache.freshness(scope))
}

/// The next level to list: the schemas of the database, then the relations of
/// the namespaces the path names, then those the statement qualifies with.
fn next_listing(
    cache: &CatalogCache,
    spaces: &[Space],
    names: &[Vec<CompletionNamePart>],
    tried: &HashSet<Read>,
) -> Option<CatalogScope> {
    let path = cache.resolution_path().unwrap_or_default();
    let on_path = |scope: &CatalogScope| match scope {
        CatalogScope::Namespace(namespace) => match namespace.namespace() {
            // A source without a schema level: its relations are the only
            // level there is.
            None => true,
            Some(name) => path.iter().any(|entry| entry == name),
        },
        _ => true,
    };
    cache
        .listing_scopes()
        .into_iter()
        .filter(on_path)
        .chain(qualified_namespaces(spaces, names))
        .find(|scope| unread(cache, scope, tried))
}

/// The namespaces the statement writes before a dot — system ones too, which
/// the tree folds: typing `pg_catalog.` asks for them.
///
/// A name of one part has no qualifier: `orders` is a relation, even when a
/// schema is called `orders` too.
fn qualified_namespaces<'a>(
    spaces: &'a [Space],
    names: &'a [Vec<CompletionNamePart>],
) -> impl Iterator<Item = CatalogScope> + 'a {
    names
        .iter()
        .filter(|chain| chain.len() >= 2)
        .filter_map(|chain| chain.first())
        .flat_map(move |first| {
            spaces
                .iter()
                .filter(move |space| space.name.as_deref().is_some_and(|name| same(first, name)))
                .map(|space| CatalogScope::Namespace(space.path.clone()))
        })
}

/// The next named relation whose columns are not read.
fn next_description(
    cache: &CatalogCache,
    spaces: &[Space],
    names: &[Vec<CompletionNamePart>],
    tried: &HashSet<Read>,
) -> Option<CatalogScope> {
    names.iter().find_map(|chain| {
        let scope = CatalogScope::Relation(resolve(cache, spaces, chain)?);
        unread(cache, &scope, tried).then_some(scope)
    })
}

/// The relation `chain` names, among those the cache has listed — or `None`
/// when it names none, or when the cache cannot say which.
fn resolve(
    cache: &CatalogCache,
    spaces: &[Space],
    chain: &[CompletionNamePart],
) -> Option<CatalogPath> {
    match chain {
        [namespace, relation] => spaces
            .iter()
            .filter(|space| {
                space
                    .name
                    .as_deref()
                    .is_some_and(|name| same(namespace, name))
            })
            .find_map(|space| relation_in(cache, space, relation)),
        [relation] => resolve_bare(cache, spaces, relation),
        // `database.schema.table` is not resolved: one connection sees one
        // database, and the editor does not complete across them.
        _ => None,
    }
}

fn relation_in(
    cache: &CatalogCache,
    space: &Space,
    relation: &CompletionNamePart,
) -> Option<CatalogPath> {
    cache
        .relations(&space.path)
        .find(|candidate| same(relation, candidate.name()))
        .map(oxyn_catalog::RelationRef::path)
}

/// An unqualified name: the first namespace of the resolution path that holds
/// it — if every one before it was read, so that none can shadow it.
fn resolve_bare(
    cache: &CatalogCache,
    spaces: &[Space],
    relation: &CompletionNamePart,
) -> Option<CatalogPath> {
    if let Some(root) = spaces.iter().find(|space| space.name.is_none()) {
        return relation_in(cache, root, relation);
    }
    let mut every_earlier_read = true;
    for entry in cache.resolution_path()? {
        let space = spaces
            .iter()
            .find(|space| space.name.as_deref() == Some(entry))?;
        if let Some(found) = relation_in(cache, space, relation) {
            return every_earlier_read.then_some(found);
        }
        let listed = cache.freshness(&CatalogScope::Namespace(space.path.clone()));
        every_earlier_read &= fetched(listed);
    }
    None
}

#[cfg(test)]
mod tests;
