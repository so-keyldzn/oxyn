//! The introspection cache: an in-memory tree, fed piece by piece.
//!
//! It is what makes exploration possible without a server round trip, and
//! what makes the AI workspace viable: an agent's context is built from the
//! local catalog, not from a server round trip for every question
//! (ARCHITECTURE §6).
//!
//! # Three decisions that govern this module
//!
//! **A level never read is not stale: it is absent.**
//! [`CatalogCache::stale`] only returns nodes read at least once. Without this
//! rule, a background refresh would describe the 20,000 relations of a schema
//! nobody opened — exactly what the laziness of the
//! [`CatalogProvider`](crate::provider::CatalogProvider) avoids.
//!
//! **Invalidating is not forgetting.** [`CatalogCache::invalidate`] marks a
//! subtree for rereading but **keeps its data**: the tree stays readable
//! during the refresh or after it fails, and emptying the tree at the first
//! `ALTER TABLE` would make it flicker. [`CatalogCache::forget`] exists for what
//! has really disappeared.
//!
//! **The clock is the wall clock.** The cache is serializable, and a monotonic
//! `Instant` does not serialize. A clock going backwards therefore makes a node
//! "not stale yet" rather than stale — the cautious direction, since the other
//! would invite re-introspecting in a loop.
//!
//! # Concurrency
//!
//! [`CatalogCache`] has no internal lock: sharing is the caller's choice, and
//! [`SharedCatalog`] gives its usual form — the interface thread reads, the
//! refresh task writes. The critical section is limited to in-memory metadata,
//! never to I/O (I-05).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use indexmap::IndexMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::RelationDefinition;
use crate::model::{
    CatalogRef, Constraint, ForeignKey, IncomingForeignKey, Index, NamespaceRef, Relation,
    RelationKind, RelationRef, ServerInfo,
};
use crate::path::{CatalogLevel, CatalogPath};

mod listing;

/// Node key of a missing level.
///
/// The empty string cannot name an object — `crate::path::validate_segment`
/// refuses it —, so it never collides with a real name. That is what lets
/// MySQL ("no catalog level") and a really named PostgreSQL database occupy the
/// same tree without stepping on each other.
const MISSING_TIER: &str = "";

const MAX_DEFINITIONS: usize = 16;
const MAX_DEFINITION_BYTES: usize = 16 * 1024 * 1024;

/// The node key matching an optional level name.
fn map_key(ident: Option<&str>) -> &str {
    ident.unwrap_or(MISSING_TIER)
}

fn definition_bytes(definition: &RelationDefinition) -> usize {
    definition
        .notes
        .iter()
        .fold(definition.sql.len(), |total, note| {
            total.saturating_add(note.len())
        })
}

/// The level name matching a node key.
fn from_key(raw_value: &str) -> Option<String> {
    if raw_value.is_empty() {
        None
    } else {
        Some(raw_value.to_owned())
    }
}

/// The usual sharing of the cache: read by the interface, written by the
/// refresh task.
pub type SharedCatalog = Arc<RwLock<CatalogCache>>;

/// The cache of a connection, handed to whoever got the right to read it.
///
/// Exists to go through execution reports — which are compared and logged —
/// without copying the cache: two handles are equal when they designate **the
/// same** cache, and `Debug` says nothing of its content, which carries the
/// object names of the user's database.
#[derive(Clone)]
pub struct CatalogHandle(SharedCatalog);

impl CatalogHandle {
    /// Wraps a shared cache.
    #[must_use]
    pub const fn new(catalog: SharedCatalog) -> Self {
        Self(catalog)
    }

    /// The designated cache. Reading takes the cache's lock: do not hold it
    /// beyond a render.
    #[must_use]
    pub const fn catalog(&self) -> &SharedCatalog {
        &self.0
    }
}

impl PartialEq for CatalogHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CatalogHandle {}

impl std::fmt::Debug for CatalogHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogHandle").finish_non_exhaustive()
    }
}

/// What an operation on the cache can refuse.
///
/// These refusals denounce a **calling bug**, not a failure: passing a
/// namespace path where a relation is expected, or attaching indexes to a
/// relation the cache never heard of. Hence the conversion to
/// [`OxynError::Internal`](oxyn_core::OxynError::Internal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// The path does not designate a relation.
    #[error("the path does not name a relation")]
    NotARelation,
    /// The path goes lower than the namespace level.
    #[error("the path does not name a namespace")]
    NotANamespace,
    /// The target relation is not in the cache.
    ///
    /// Attaching indexes to an unknown relation would invent one, with a
    /// guessed kind. List or describe the relation first.
    #[error("the relation is not in the cache")]
    UnknownRelation,
}

impl From<CacheError> for oxyn_core::OxynError {
    fn from(err: CacheError) -> Self {
        Self::Internal(err.to_string())
    }
}

/// The freshness state of a cache node.
///
/// Three states, not two: "never read" and "read then invalidated" look alike
/// — neither has up-to-date data — but call for opposite conduct. The first
/// waits to be asked for; the second must be reread right away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Never read. **Is not stale**: it is absent, and loading it is a lazy
    /// expansion triggered by the user.
    #[default]
    Never,
    /// Read at this instant.
    Fetched(DateTime<Utc>),
    /// Read, then invalidated — typically after a DDL issued from Oxyn. Stale
    /// whatever the delay.
    Invalidated,
}

impl Freshness {
    /// Marks a read that just happened.
    #[must_use]
    pub fn now() -> Self {
        Self::Fetched(Utc::now())
    }

    /// Has this node already been read?
    #[must_use]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Never)
    }

    /// Instant of the last read, if there was one.
    #[must_use]
    pub const fn fetched_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Fetched(instant) => Some(*instant),
            Self::Never | Self::Invalidated => None,
        }
    }

    /// Must this node be reread, at instant `now` and for a lifetime `ttl`?
    #[must_use]
    pub fn is_stale_at(&self, now: DateTime<Utc>, ttl: Duration) -> bool {
        match self {
            Self::Never => false,
            Self::Invalidated => true,
            Self::Fetched(instant) => match TimeDelta::from_std(ttl) {
                Ok(limit) => now.signed_duration_since(*instant) > limit,
                // A lifetime beyond `chrono`'s bounds (more than ~584
                // millennia) makes nothing stale. The other direction would
                // re-introspect in a loop on an aberrant value.
                Err(_) => false,
            },
        }
    }

    /// Goes to [`Invalidated`](Self::Invalidated), unless the node was never
    /// read — invalidating what never existed would make it appear in
    /// [`CatalogCache::stale`] without anyone asking for it.
    fn invalidate(&mut self) {
        if self.is_known() {
            *self = Self::Invalidated;
        }
    }
}

/// A cache value and its freshness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Cached<T> {
    freshness: Freshness,
    value: T,
}

impl<T: Default> Default for Cached<T> {
    fn default() -> Self {
        Self {
            freshness: Freshness::Never,
            value: T::default(),
        }
    }
}

impl<T> Cached<T> {
    /// Replaces the value and marks it read now.
    fn set(&mut self, value: T) {
        self.value = value;
        self.freshness = Freshness::now();
    }
}

/// A catalog and its namespaces.
///
/// `info` is `None` for the node that carries a **missing level**: MySQL has
/// no catalog, and inventing one with an empty name would make it appear in
/// the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CatalogNode {
    info: Option<CatalogRef>,
    namespaces: Cached<IndexMap<String, NamespaceNode>>,
}

/// A namespace and its relations. `info` follows the same rule as
/// [`CatalogNode::info`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct NamespaceNode {
    info: Option<NamespaceRef>,
    relations: Cached<IndexMap<String, RelationNode>>,
}

/// A relation: its summary, and the three details requested separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RelationNode {
    summary: RelationRef,
    detail: Cached<Option<Relation>>,
    indexes: Cached<Option<Vec<Index>>>,
    foreign_keys: Cached<Option<Vec<ForeignKey>>>,
    #[serde(default)]
    constraints: Cached<Option<Vec<Constraint>>>,
    #[serde(default)]
    incoming_keys: Cached<Option<Vec<IncomingForeignKey>>>,
    #[serde(default)]
    definition: Cached<Option<RelationDefinition>>,
}

impl RelationNode {
    fn new(summary: RelationRef) -> Self {
        Self {
            summary,
            detail: Cached::default(),
            indexes: Cached::default(),
            foreign_keys: Cached::default(),
            constraints: Cached::default(),
            incoming_keys: Cached::default(),
            definition: Cached::default(),
        }
    }

    fn invalidate(&mut self) {
        self.detail.freshness.invalidate();
        self.indexes.freshness.invalidate();
        self.foreign_keys.freshness.invalidate();
        self.constraints.freshness.invalidate();
        self.incoming_keys.freshness.invalidate();
        self.definition.freshness.invalidate();
    }

    fn is_stale_at(&self, now: DateTime<Utc>, ttl: Duration) -> bool {
        self.detail.freshness.is_stale_at(now, ttl)
            || self.indexes.freshness.is_stale_at(now, ttl)
            || self.foreign_keys.freshness.is_stale_at(now, ttl)
    }
}

/// The subtree targeted by an invalidation or a refresh.
///
/// A scope designates a node **and everything below**: refreshing
/// [`Server`](Self::Server) rereads the whole tree, refreshing
/// [`Relation`](Self::Relation) rereads only one table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogScope {
    /// The whole server.
    Server,
    /// A catalog and its content.
    Catalog(CatalogPath),
    /// A namespace and its relations.
    Namespace(CatalogPath),
    /// A relation: its description, its indexes, its foreign keys.
    Relation(CatalogPath),
    /// Only the constraints of a relation.
    Constraints(CatalogPath),
    /// Only the keys referencing this relation.
    IncomingForeignKeys(CatalogPath),
    /// Only the creation statements of a relation.
    Definition(CatalogPath),
}

impl CatalogScope {
    /// The scope matching the deepest level of a path.
    ///
    /// Cannot pick the wrong variant, unlike a direct construction.
    #[must_use]
    pub fn of(path: &CatalogPath) -> Self {
        match path.level() {
            CatalogLevel::Server => Self::Server,
            CatalogLevel::Catalog => Self::Catalog(path.clone()),
            CatalogLevel::Namespace => Self::Namespace(path.clone()),
            CatalogLevel::Relation => Self::Relation(path.clone()),
        }
    }

    /// The targeted path, except for [`Server`](Self::Server) which has none.
    #[must_use]
    pub const fn path(&self) -> Option<&CatalogPath> {
        match self {
            Self::Server => None,
            Self::Catalog(p)
            | Self::Namespace(p)
            | Self::Relation(p)
            | Self::Constraints(p)
            | Self::IncomingForeignKeys(p)
            | Self::Definition(p) => Some(p),
        }
    }

    /// The targeted level.
    #[must_use]
    pub const fn level(&self) -> CatalogLevel {
        match self {
            Self::Server => CatalogLevel::Server,
            Self::Catalog(_) => CatalogLevel::Catalog,
            Self::Namespace(_) => CatalogLevel::Namespace,
            Self::Relation(_)
            | Self::Constraints(_)
            | Self::IncomingForeignKeys(_)
            | Self::Definition(_) => CatalogLevel::Relation,
        }
    }

    /// Does this scope cover `other`?
    ///
    /// Serves to avoid refreshing twice: refreshing a namespace already covers
    /// each of its relations. A scope covers itself.
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        if matches!(
            self,
            Self::Constraints(_) | Self::IncomingForeignKeys(_) | Self::Definition(_)
        ) {
            return self == other;
        }
        let Some(prefix) = self.path() else {
            return true;
        };
        let Some(target) = other.path() else {
            return false;
        };
        if other.level() < self.level() {
            return false;
        }
        target.starts_with(prefix)
    }
}

impl std::fmt::Display for CatalogScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.path() {
            None => f.write_str("server"),
            Some(item_path) => write!(f, "{} {item_path}", self.level()),
        }
    }
}

/// The metadata tree of **one** connection.
///
/// Fed piece by piece: describing a relation does not require listing its
/// namespace first. Missing levels are created on the way, with a
/// [`Freshness::Never`] freshness — they are there to carry their child, they
/// do not claim to have been read.
///
/// Serializable end to end, in a format readable without Oxyn (I-11). It is
/// not persisted: a future persistence will go through an ADR
/// (ARCHITECTURE §6).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogCache {
    server: Cached<Option<ServerInfo>>,
    catalogs: Cached<IndexMap<String, CatalogNode>>,
    /// The namespaces an unqualified name resolves in, first match first.
    ///
    /// `None` is "never read" **or** "the source cannot say": both leave the
    /// completion on qualified names, and neither is an empty path, which
    /// would assert that nothing resolves.
    #[serde(default)]
    resolution_path: Cached<Option<Vec<String>>>,
}

impl CatalogCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ── Writing ─────────────────────────────────────────────────────────────

    /// Records the server identity and the session capabilities.
    pub fn set_server_info(&mut self, info: ServerInfo) {
        self.server.set(Some(info));
    }

    /// Records the namespaces an unqualified name resolves in, in priority
    /// order ([ADR-0057](../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md)).
    ///
    /// The names are the server's words: data, never composed into SQL here.
    pub fn set_resolution_path(&mut self, path: Vec<String>) {
        self.resolution_path.set(Some(path));
    }

    /// Records the list of catalogs.
    ///
    /// Catalogs missing from the list are **removed**: a fresh listing is
    /// authoritative on its level. Those that remain keep their subtree, hence
    /// their freshness.
    pub fn set_catalogs(&mut self, catalogs: Vec<CatalogRef>) {
        let mut previous = std::mem::take(&mut self.catalogs.value);
        let mut fresh = IndexMap::with_capacity(catalogs.len());
        for info in catalogs {
            let node_key = info.name().to_owned();
            // `swap_remove` and not `shift_remove`: the original map is thrown
            // away, and an ordered removal would cost one walk per element —
            // quadratic on a server with a thousand databases.
            let tree_node = match previous.swap_remove(&node_key) {
                Some(mut existing) => {
                    existing.info = Some(info);
                    existing
                }
                None => CatalogNode {
                    info: Some(info),
                    namespaces: Cached::default(),
                },
            };
            fresh.insert(node_key, tree_node);
        }
        self.catalogs.set(fresh);
    }

    /// Records the namespaces of a catalog, or of the server when `catalog` is
    /// `None`.
    ///
    /// The catalog is created if missing. Each reference is reattached to the
    /// given parent: the path of a [`NamespaceRef`] and its position in the
    /// tree therefore cannot diverge.
    pub fn set_namespaces(&mut self, catalog: Option<&str>, namespaces: Vec<NamespaceRef>) {
        let parent = CatalogPath::from_validated(catalog.map(str::to_owned), None, None);
        let tree_node = self.catalog_node_mut(catalog);
        let mut previous = std::mem::take(&mut tree_node.namespaces.value);
        let mut fresh = IndexMap::with_capacity(namespaces.len());
        for mut info in namespaces {
            info.reparent(parent.clone());
            let node_key = info.name().to_owned();
            let child = match previous.swap_remove(&node_key) {
                Some(mut existing) => {
                    existing.info = Some(info);
                    existing
                }
                None => NamespaceNode {
                    info: Some(info),
                    relations: Cached::default(),
                },
            };
            fresh.insert(node_key, child);
        }
        tree_node.namespaces.set(fresh);
    }

    /// Records the relations of a namespace.
    ///
    /// The details already known of the relations that remain are kept:
    /// relisting a schema must not force redescribing every open table.
    ///
    /// # Errors
    /// [`CacheError::NotANamespace`] if `namespace` designates a relation.
    pub fn set_relations(
        &mut self,
        namespace: &CatalogPath,
        relations: Vec<RelationRef>,
    ) -> Result<(), CacheError> {
        if namespace.relation().is_some() {
            return Err(CacheError::NotANamespace);
        }
        let parent = namespace.clone();
        let tree_node = self.namespace_node_mut(namespace);
        let mut previous = std::mem::take(&mut tree_node.relations.value);
        let mut fresh = IndexMap::with_capacity(relations.len());
        for mut summary in relations {
            summary.reparent(parent.clone());
            let node_key = summary.name().to_owned();
            let child = match previous.swap_remove(&node_key) {
                Some(mut existing) => {
                    existing.summary = summary;
                    existing
                }
                None => RelationNode::new(summary),
            };
            fresh.insert(node_key, child);
        }
        tree_node.relations.set(fresh);
        Ok(())
    }

    /// Records the full description of a relation.
    ///
    /// Missing levels are created, and the relation itself if the listing has
    /// not happened yet: describing a table found by the search must not require
    /// listing its schema first. The kind of the created node comes from
    /// [`Relation::kind`], never from a default value.
    ///
    /// Types nested beyond [`MAX_TYPE_DEPTH`](crate::nesting::MAX_TYPE_DEPTH)
    /// are cut: an endlessly inferred document would overflow the stack at the
    /// first `Drop`.
    ///
    /// # Errors
    /// [`CacheError::NotARelation`] if `path` does not name a relation.
    pub fn set_relation(
        &mut self,
        path: &CatalogPath,
        mut relation: Relation,
    ) -> Result<(), CacheError> {
        crate::nesting::bound(&mut relation.fields);
        let kind = relation.kind;
        let tree_node = self.relation_node_or_create(path, kind)?;
        tree_node.detail.set(Some(relation));
        Ok(())
    }

    /// Records the indexes of a relation.
    ///
    /// # Errors
    /// [`CacheError::NotARelation`] if `path` does not name a relation,
    /// [`CacheError::UnknownRelation`] if the relation was neither listed nor
    /// described: creating it here would force guessing its kind.
    pub fn set_indexes(
        &mut self,
        path: &CatalogPath,
        indexes: Vec<Index>,
    ) -> Result<(), CacheError> {
        self.relation_node_existing_mut(path)?
            .indexes
            .set(Some(indexes));
        Ok(())
    }

    /// Records the foreign keys of a relation.
    ///
    /// # Errors
    /// The same as [`Self::set_indexes`].
    pub fn set_foreign_keys(
        &mut self,
        path: &CatalogPath,
        keys: Vec<ForeignKey>,
    ) -> Result<(), CacheError> {
        self.relation_node_existing_mut(path)?
            .foreign_keys
            .set(Some(keys));
        Ok(())
    }

    /// Publishes a successful constraint read, preserving unread versus empty.
    /// Returns the same path errors as [`Self::set_indexes`].
    pub fn set_constraints(
        &mut self,
        path: &CatalogPath,
        constraints: Vec<Constraint>,
    ) -> Result<(), CacheError> {
        self.relation_node_existing_mut(path)?
            .constraints
            .set(Some(constraints));
        Ok(())
    }

    /// Constraints previously read for this relation; `None` means unread.
    #[must_use]
    pub fn constraints(&self, path: &CatalogPath) -> Option<&[Constraint]> {
        self.relation_node(path)?.constraints.value.as_deref()
    }

    /// Publishes all keys referencing a known relation. Path errors match
    /// [`Self::set_indexes`]; absence and a successful empty read stay distinct.
    pub fn set_incoming_foreign_keys(
        &mut self,
        path: &CatalogPath,
        keys: Vec<IncomingForeignKey>,
    ) -> Result<(), CacheError> {
        self.relation_node_existing_mut(path)?
            .incoming_keys
            .set(Some(keys));
        Ok(())
    }

    /// Previously read incoming keys; `None` means unreported or unread.
    #[must_use]
    pub fn incoming_foreign_keys(&self, path: &CatalogPath) -> Option<&[IncomingForeignKey]> {
        self.relation_node(path)?.incoming_keys.value.as_deref()
    }

    /// Stores a definition for a known relation. Path errors match
    /// [`Self::set_indexes`]; the bus validates payload bounds before publication.
    pub fn set_definition(
        &mut self,
        path: &CatalogPath,
        definition: RelationDefinition,
    ) -> Result<(), CacheError> {
        self.relation_node_existing_mut(path)?
            .definition
            .set(Some(definition));
        self.evict_definitions(path);
        Ok(())
    }

    /// Last successful definition read, or `None` when unread.
    #[must_use]
    pub fn definition(&self, path: &CatalogPath) -> Option<&RelationDefinition> {
        self.relation_node(path)?.definition.value.as_ref()
    }

    /// Evicts the oldest definitions until the bounded DDL cache is within its
    /// construction limits. The just-published definition is always retained.
    fn evict_definitions(&mut self, protected: &CatalogPath) {
        loop {
            let mut candidates = Vec::new();
            let mut count = 0;
            let mut bytes: usize = 0;
            for catalog in self.catalogs.value.values() {
                for namespace in catalog.namespaces.value.values() {
                    for relation in namespace.relations.value.values() {
                        let Some(definition) = relation.definition.value.as_ref() else {
                            continue;
                        };
                        count += 1;
                        bytes = bytes.saturating_add(definition_bytes(definition));
                        candidates.push((
                            relation
                                .summary
                                .parent()
                                .with_validated_relation(relation.summary.name()),
                            relation.definition.freshness.fetched_at(),
                        ));
                    }
                }
            }
            if count <= MAX_DEFINITIONS && bytes <= MAX_DEFINITION_BYTES {
                return;
            }
            let Some((oldest, _)) = candidates
                .into_iter()
                .filter(|(path, _)| path != protected)
                .min_by_key(|(_, fetched_at)| *fetched_at)
            else {
                return;
            };
            if let Some(relation) = self.relation_node_mut_opt(&oldest) {
                relation.definition = Cached::default();
            } else {
                return;
            }
        }
    }

    // ── Reading ─────────────────────────────────────────────────────────────

    /// The server identity, if it was read.
    #[must_use]
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server.value.as_ref()
    }

    /// The namespaces an unqualified name resolves in, first match first.
    ///
    /// `None` when it was never read or when the source cannot tell — not an
    /// empty slice, which would say that no namespace resolves anything.
    /// After an invalidation the last value stays readable, flagged by
    /// [`Self::resolution_path_freshness`]: a path that was right a moment ago
    /// serves better than none while it is read again.
    #[must_use]
    pub fn resolution_path(&self) -> Option<&[String]> {
        self.resolution_path.value.as_deref()
    }

    /// Freshness of [`Self::resolution_path`].
    #[must_use]
    pub fn resolution_path_freshness(&self) -> Freshness {
        self.resolution_path.freshness
    }

    /// The known catalogs, in the order the server gave them.
    ///
    /// The "missing level" node is not among them: a source without a catalog
    /// returns an empty iterator, which is the truth.
    pub fn catalogs(&self) -> impl Iterator<Item = &CatalogRef> + '_ {
        self.catalogs
            .value
            .values()
            .filter_map(|tree_node| tree_node.info.as_ref())
    }

    /// The namespaces of a catalog. Empty iterator if the catalog is unknown —
    /// the absence of information is not an error here, it is the normal state
    /// of a lazy tree.
    pub fn namespaces(&self, catalog: Option<&str>) -> impl Iterator<Item = &NamespaceRef> + '_ {
        self.catalogs
            .value
            .get(map_key(catalog))
            .map(|tree_node| tree_node.namespaces.value.values())
            .into_iter()
            .flatten()
            .filter_map(|tree_node| tree_node.info.as_ref())
    }

    /// The relations of a namespace. Empty iterator if the namespace is
    /// unknown.
    pub fn relations(&self, namespace: &CatalogPath) -> impl Iterator<Item = &RelationRef> + '_ {
        self.namespace_node(namespace)
            .map(|tree_node| tree_node.relations.value.values())
            .into_iter()
            .flatten()
            .map(|tree_node| &tree_node.summary)
    }

    /// The summary of a relation.
    #[must_use]
    pub fn relation_summary(&self, path: &CatalogPath) -> Option<&RelationRef> {
        self.relation_node(path).map(|tree_node| &tree_node.summary)
    }

    /// The description of a relation, if it was requested.
    #[must_use]
    pub fn relation(&self, path: &CatalogPath) -> Option<&Relation> {
        self.relation_node(path)?.detail.value.as_ref()
    }

    /// The indexes of a relation, if they were requested.
    ///
    /// `None` means "not read", not "no index": it is the cutting edge of the
    /// capability model, and the empty slice, for its part, does say "none".
    #[must_use]
    pub fn indexes(&self, path: &CatalogPath) -> Option<&[Index]> {
        self.relation_node(path)?.indexes.value.as_deref()
    }

    /// The foreign keys of a relation, if they were requested. Same
    /// distinction as for [`Self::indexes`].
    #[must_use]
    pub fn foreign_keys(&self, path: &CatalogPath) -> Option<&[ForeignKey]> {
        self.relation_node(path)?.foreign_keys.value.as_deref()
    }

    /// Every known relation, with its description when it was requested.
    ///
    /// It is the walk [`fn@crate::search`] uses. No path is built here: each
    /// [`RelationRef`] already carries its own, and allocating three strings
    /// per relation on every keystroke in a search bar would be a design
    /// defect, not an optimization to do later.
    pub fn iter_relations(&self) -> impl Iterator<Item = (&RelationRef, Option<&Relation>)> + '_ {
        self.catalogs
            .value
            .values()
            .flat_map(|catalogue| catalogue.namespaces.value.values())
            .flat_map(|space| space.relations.value.values())
            .map(|relation| (&relation.summary, relation.detail.value.as_ref()))
    }

    /// Number of known relations, all levels together.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.iter_relations().count()
    }

    /// Does the cache contain nothing at all?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.server.value.is_none() && self.catalogs.value.is_empty()
    }

    /// The freshness of a subtree, as its root node carries it.
    ///
    /// Returns [`Freshness::Never`] for an unknown node: not knowing a node and
    /// never having read it are the same thing.
    #[must_use]
    pub fn freshness(&self, scope: &CatalogScope) -> Freshness {
        match scope {
            CatalogScope::Server => self.catalogs.freshness,
            CatalogScope::Catalog(item_path) => self
                .catalogs
                .value
                .get(map_key(item_path.catalog()))
                .map_or(Freshness::Never, |tree_node| tree_node.namespaces.freshness),
            CatalogScope::Namespace(item_path) => self
                .namespace_node(item_path)
                .map_or(Freshness::Never, |tree_node| tree_node.relations.freshness),
            CatalogScope::Relation(item_path) => self
                .relation_node(item_path)
                .map_or(Freshness::Never, |tree_node| tree_node.detail.freshness),
            CatalogScope::Constraints(path) => self
                .relation_node(path)
                .map_or(Freshness::Never, |node| node.constraints.freshness),
            CatalogScope::IncomingForeignKeys(path) => self
                .relation_node(path)
                .map_or(Freshness::Never, |node| node.incoming_keys.freshness),
            CatalogScope::Definition(path) => self
                .relation_node(path)
                .map_or(Freshness::Never, |node| node.definition.freshness),
        }
    }

    // ── Staleness ───────────────────────────────────────────────────────────

    /// The subtrees to reread, for a given lifetime.
    ///
    /// Returns a **minimal** set: no returned scope covers another, since
    /// refreshing a parent refreshes its children. A background refresh can
    /// therefore walk the list without deduplicating.
    ///
    /// A node never read is not in it: it is absent, not stale. See the module
    /// documentation.
    #[must_use]
    pub fn stale(&self, ttl: Duration) -> Vec<CatalogScope> {
        self.stale_at(Utc::now(), ttl)
    }

    /// [`Self::stale`], with the current instant supplied. Makes tests
    /// deterministic; it is the only reason it exists.
    #[must_use]
    pub fn stale_at(&self, now: DateTime<Utc>, ttl: Duration) -> Vec<CatalogScope> {
        if self.server.freshness.is_stale_at(now, ttl)
            || self.catalogs.freshness.is_stale_at(now, ttl)
        {
            return vec![CatalogScope::Server];
        }

        let mut stale_ones = Vec::new();
        for (catalog_key, catalogue) in &self.catalogs.value {
            let catalog_path = CatalogPath::from_validated(from_key(catalog_key), None, None);
            if catalogue.namespaces.freshness.is_stale_at(now, ttl) {
                stale_ones.push(CatalogScope::Catalog(catalog_path));
                continue;
            }
            for (space_key, space) in &catalogue.namespaces.value {
                if space.relations.freshness.is_stale_at(now, ttl) {
                    stale_ones.push(CatalogScope::Namespace(CatalogPath::from_validated(
                        from_key(catalog_key),
                        from_key(space_key),
                        None,
                    )));
                    continue;
                }
                for relation in space.relations.value.values() {
                    if relation.is_stale_at(now, ttl) {
                        stale_ones.push(CatalogScope::Relation(relation.summary.path()));
                    } else {
                        if relation.constraints.freshness.is_stale_at(now, ttl) {
                            stale_ones.push(CatalogScope::Constraints(relation.summary.path()));
                        }
                        if relation.incoming_keys.freshness.is_stale_at(now, ttl) {
                            stale_ones
                                .push(CatalogScope::IncomingForeignKeys(relation.summary.path()));
                        }
                        if relation.definition.freshness.is_stale_at(now, ttl) {
                            stale_ones.push(CatalogScope::Definition(relation.summary.path()));
                        }
                    }
                }
            }
        }
        stale_ones
    }

    // ── Invalidation ────────────────────────────────────────────────────────

    /// Marks a subtree for rereading, **without erasing its data**.
    ///
    /// To call immediately after any DDL issued from Oxyn (ARCHITECTURE §6).
    /// The scope to target is that of the touched object: an `ALTER TABLE`
    /// invalidates the relation; a `CREATE TABLE` or a `DROP TABLE` invalidates
    /// the **namespace**, because it is its listing that just became wrong.
    pub fn invalidate(&mut self, scope: &CatalogScope) {
        tracing::debug!(scope = %scope, "catalog cache invalidated");
        match scope {
            CatalogScope::Server => {
                self.server.freshness.invalidate();
                // A `CREATE SCHEMA` or a `DROP SCHEMA` changes which entries of
                // the path exist.
                self.resolution_path.freshness.invalidate();
                self.catalogs.freshness.invalidate();
                for catalogue in self.catalogs.value.values_mut() {
                    Self::invalidate_catalog(catalogue);
                }
            }
            CatalogScope::Catalog(item_path) => {
                if let Some(catalogue) = self.catalogs.value.get_mut(map_key(item_path.catalog())) {
                    Self::invalidate_catalog(catalogue);
                }
            }
            CatalogScope::Namespace(item_path) => {
                if let Some(space) = self.namespace_node_mut_opt(item_path) {
                    Self::invalidate_namespace(space);
                }
            }
            CatalogScope::Constraints(path) => {
                if let Some(node) = self.relation_node_mut_opt(path) {
                    node.constraints.freshness.invalidate();
                }
            }
            CatalogScope::IncomingForeignKeys(path) => {
                if let Some(node) = self.relation_node_mut_opt(path) {
                    node.incoming_keys.freshness.invalidate();
                }
            }
            CatalogScope::Definition(path) => {
                if let Some(node) = self.relation_node_mut_opt(path) {
                    node.definition.freshness.invalidate();
                }
            }
            CatalogScope::Relation(item_path) => {
                if let Some(relation) = self.relation_node_mut_opt(item_path) {
                    relation.invalidate();
                }
            }
        }
    }

    /// Marks the whole cache for rereading.
    pub fn invalidate_all(&mut self) {
        self.invalidate(&CatalogScope::Server);
    }

    /// Removes a subtree from the cache.
    ///
    /// For what has really disappeared — a `DROP` Oxyn authored. The parent's
    /// listing is invalidated on the way: it just became wrong, and leaving it
    /// fresh would make the object reappear at the next refresh.
    pub fn forget(&mut self, scope: &CatalogScope) {
        tracing::debug!(scope = %scope, "catalog cache entry removed");
        match scope {
            CatalogScope::Server => *self = Self::new(),
            CatalogScope::Catalog(item_path) => {
                self.catalogs
                    .value
                    .shift_remove(map_key(item_path.catalog()));
                self.catalogs.freshness.invalidate();
            }
            CatalogScope::Namespace(item_path) => {
                if let Some(catalogue) = self.catalogs.value.get_mut(map_key(item_path.catalog())) {
                    catalogue
                        .namespaces
                        .value
                        .shift_remove(map_key(item_path.namespace()));
                    catalogue.namespaces.freshness.invalidate();
                }
            }
            CatalogScope::Constraints(path) => {
                if let Some(node) = self.relation_node_mut_opt(path) {
                    node.constraints = Cached::default();
                }
            }
            CatalogScope::IncomingForeignKeys(path) => {
                if let Some(node) = self.relation_node_mut_opt(path) {
                    node.incoming_keys = Cached::default();
                }
            }
            CatalogScope::Definition(path) => {
                if let Some(node) = self.relation_node_mut_opt(path) {
                    node.definition = Cached::default();
                }
            }
            CatalogScope::Relation(item_path) => {
                let Some(relation_name) = item_path.relation().map(str::to_owned) else {
                    return;
                };
                if let Some(space) = self.namespace_node_mut_opt(item_path) {
                    space.relations.value.shift_remove(relation_name.as_str());
                    space.relations.freshness.invalidate();
                }
            }
        }
    }

    /// Drops what was read *under* a node, to bound memory.
    ///
    /// Unlike [`Self::forget`], the node itself stays where its parent's
    /// listing put it: the object still exists, only its contents are no
    /// longer held. The dropped levels read as never fetched, never as empty.
    /// A node that no listing carries — created on the way by a direct
    /// description — goes once empty: nothing would count it any more.
    /// [`CatalogScope::Server`] empties the whole cache. An unknown node is a
    /// no-op.
    pub fn evict(&mut self, scope: &CatalogScope) {
        tracing::debug!(scope = %scope, "catalog cache entry evicted");
        match scope {
            CatalogScope::Server => *self = Self::new(),
            CatalogScope::Catalog(path) => {
                let key = map_key(path.catalog());
                if self.catalogs.freshness == Freshness::Never {
                    self.catalogs.value.shift_remove(key);
                } else if let Some(catalog) = self.catalogs.value.get_mut(key) {
                    catalog.namespaces = Cached::default();
                }
            }
            CatalogScope::Namespace(path) => {
                let Some(catalog) = self.catalogs.value.get_mut(map_key(path.catalog())) else {
                    return;
                };
                let key = map_key(path.namespace());
                if catalog.namespaces.freshness == Freshness::Never {
                    catalog.namespaces.value.shift_remove(key);
                } else if let Some(namespace) = catalog.namespaces.value.get_mut(key) {
                    namespace.relations = Cached::default();
                }
            }
            CatalogScope::Relation(path) => {
                let Some(name) = path.relation() else {
                    return;
                };
                let Some(namespace) = self.namespace_node_mut_opt(path) else {
                    return;
                };
                let listed = namespace.relations.freshness != Freshness::Never;
                let Some(node) = namespace.relations.value.get_mut(name) else {
                    return;
                };
                node.detail = Cached::default();
                node.indexes = Cached::default();
                node.foreign_keys = Cached::default();
                let empty = node.constraints.value.is_none()
                    && node.incoming_keys.value.is_none()
                    && node.definition.value.is_none();
                if !listed && empty {
                    namespace.relations.value.shift_remove(name);
                }
            }
            CatalogScope::Constraints(_)
            | CatalogScope::IncomingForeignKeys(_)
            | CatalogScope::Definition(_) => self.forget(scope),
        }
    }

    // ── Internal navigation ─────────────────────────────────────────────────

    fn invalidate_catalog(catalogue: &mut CatalogNode) {
        catalogue.namespaces.freshness.invalidate();
        for space in catalogue.namespaces.value.values_mut() {
            Self::invalidate_namespace(space);
        }
    }

    fn invalidate_namespace(space: &mut NamespaceNode) {
        space.relations.freshness.invalidate();
        for relation in space.relations.value.values_mut() {
            relation.invalidate();
        }
    }

    fn namespace_node(&self, path: &CatalogPath) -> Option<&NamespaceNode> {
        self.catalogs
            .value
            .get(map_key(path.catalog()))?
            .namespaces
            .value
            .get(map_key(path.namespace()))
    }

    fn namespace_node_mut_opt(&mut self, path: &CatalogPath) -> Option<&mut NamespaceNode> {
        self.catalogs
            .value
            .get_mut(map_key(path.catalog()))?
            .namespaces
            .value
            .get_mut(map_key(path.namespace()))
    }

    fn relation_node(&self, path: &CatalogPath) -> Option<&RelationNode> {
        let relation_name = path.relation()?;
        self.namespace_node(path)?
            .relations
            .value
            .get(relation_name)
    }

    fn relation_node_mut_opt(&mut self, path: &CatalogPath) -> Option<&mut RelationNode> {
        let relation_name = path.relation()?.to_owned();
        self.namespace_node_mut_opt(path)?
            .relations
            .value
            .get_mut(relation_name.as_str())
    }

    /// The node of an **already known** relation.
    fn relation_node_existing_mut(
        &mut self,
        path: &CatalogPath,
    ) -> Result<&mut RelationNode, CacheError> {
        if path.relation().is_none() {
            return Err(CacheError::NotARelation);
        }
        self.relation_node_mut_opt(path)
            .ok_or(CacheError::UnknownRelation)
    }

    /// The catalog node, created if missing.
    ///
    /// The created node carries a [`Freshness::Never`] freshness: it exists to
    /// carry a child, it does not claim to have been listed. Its `info` stays
    /// `None` when the level is missing.
    fn catalog_node_mut(&mut self, catalog: Option<&str>) -> &mut CatalogNode {
        let info = catalog.map(|ident| CatalogRef::validated(ident.to_owned()));
        self.catalogs
            .value
            .entry(map_key(catalog).to_owned())
            .or_insert_with(|| CatalogNode {
                info,
                namespaces: Cached::default(),
            })
    }

    /// The namespace node, with its parents, created if missing.
    fn namespace_node_mut(&mut self, path: &CatalogPath) -> &mut NamespaceNode {
        let parent = CatalogPath::from_validated(path.catalog().map(str::to_owned), None, None);
        let info = path
            .namespace()
            .map(|ident| NamespaceRef::validated(parent, ident.to_owned()));
        let node_key = map_key(path.namespace()).to_owned();
        self.catalog_node_mut(path.catalog())
            .namespaces
            .value
            .entry(node_key)
            .or_insert_with(|| NamespaceNode {
                info,
                relations: Cached::default(),
            })
    }

    /// The relation node, with its parents, created if missing.
    ///
    /// `kind` only serves at creation: the summary of an already listed
    /// relation is authoritative on its kind.
    fn relation_node_or_create(
        &mut self,
        path: &CatalogPath,
        kind: RelationKind,
    ) -> Result<&mut RelationNode, CacheError> {
        let Some(relation_name) = path.relation().map(str::to_owned) else {
            return Err(CacheError::NotARelation);
        };
        let parent = CatalogPath::from_validated(
            path.catalog().map(str::to_owned),
            path.namespace().map(str::to_owned),
            None,
        );
        let rel_summary = RelationRef::validated(parent, relation_name.clone(), kind);
        Ok(self
            .namespace_node_mut(path)
            .relations
            .value
            .entry(relation_name)
            .or_insert_with(|| RelationNode::new(rel_summary)))
    }
}

#[cfg(test)]
mod tests;
