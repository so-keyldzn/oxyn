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
const PALIER_ABSENT: &str = "";

const MAX_DEFINITIONS: usize = 16;
const MAX_DEFINITION_BYTES: usize = 16 * 1024 * 1024;

/// The node key matching an optional level name.
fn cle(nom: Option<&str>) -> &str {
    nom.unwrap_or(PALIER_ABSENT)
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
fn depuis_cle(valeur: &str) -> Option<String> {
    if valeur.is_empty() {
        None
    } else {
        Some(valeur.to_owned())
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
                Ok(limite) => now.signed_duration_since(*instant) > limite,
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
        let Some(prefixe) = self.path() else {
            return true;
        };
        let Some(cible) = other.path() else {
            return false;
        };
        if other.level() < self.level() {
            return false;
        }
        cible.starts_with(prefixe)
    }
}

impl std::fmt::Display for CatalogScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.path() {
            None => f.write_str("server"),
            Some(chemin) => write!(f, "{} {chemin}", self.level()),
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

    /// Records the list of catalogs.
    ///
    /// Catalogs missing from the list are **removed**: a fresh listing is
    /// authoritative on its level. Those that remain keep their subtree, hence
    /// their freshness.
    pub fn set_catalogs(&mut self, catalogs: Vec<CatalogRef>) {
        let mut ancien = std::mem::take(&mut self.catalogs.value);
        let mut nouveau = IndexMap::with_capacity(catalogs.len());
        for info in catalogs {
            let cle_noeud = info.name().to_owned();
            // `swap_remove` and not `shift_remove`: the original map is thrown
            // away, and an ordered removal would cost one walk per element —
            // quadratic on a server with a thousand databases.
            let noeud = match ancien.swap_remove(&cle_noeud) {
                Some(mut existant) => {
                    existant.info = Some(info);
                    existant
                }
                None => CatalogNode {
                    info: Some(info),
                    namespaces: Cached::default(),
                },
            };
            nouveau.insert(cle_noeud, noeud);
        }
        self.catalogs.set(nouveau);
    }

    /// Records the namespaces of a catalog, or of the server when `catalog` is
    /// `None`.
    ///
    /// The catalog is created if missing. Each reference is reattached to the
    /// given parent: the path of a [`NamespaceRef`] and its position in the
    /// tree therefore cannot diverge.
    pub fn set_namespaces(&mut self, catalog: Option<&str>, namespaces: Vec<NamespaceRef>) {
        let parent = CatalogPath::from_validated(catalog.map(str::to_owned), None, None);
        let noeud = self.catalog_node_mut(catalog);
        let mut ancien = std::mem::take(&mut noeud.namespaces.value);
        let mut nouveau = IndexMap::with_capacity(namespaces.len());
        for mut info in namespaces {
            info.reparent(parent.clone());
            let cle_noeud = info.name().to_owned();
            let enfant = match ancien.swap_remove(&cle_noeud) {
                Some(mut existant) => {
                    existant.info = Some(info);
                    existant
                }
                None => NamespaceNode {
                    info: Some(info),
                    relations: Cached::default(),
                },
            };
            nouveau.insert(cle_noeud, enfant);
        }
        noeud.namespaces.set(nouveau);
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
        let noeud = self.namespace_node_mut(namespace);
        let mut ancien = std::mem::take(&mut noeud.relations.value);
        let mut nouveau = IndexMap::with_capacity(relations.len());
        for mut summary in relations {
            summary.reparent(parent.clone());
            let cle_noeud = summary.name().to_owned();
            let enfant = match ancien.swap_remove(&cle_noeud) {
                Some(mut existant) => {
                    existant.summary = summary;
                    existant
                }
                None => RelationNode::new(summary),
            };
            nouveau.insert(cle_noeud, enfant);
        }
        noeud.relations.set(nouveau);
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
        let noeud = self.relation_node_or_create(path, kind)?;
        noeud.detail.set(Some(relation));
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

    /// The known catalogs, in the order the server gave them.
    ///
    /// The "missing level" node is not among them: a source without a catalog
    /// returns an empty iterator, which is the truth.
    pub fn catalogs(&self) -> impl Iterator<Item = &CatalogRef> + '_ {
        self.catalogs
            .value
            .values()
            .filter_map(|noeud| noeud.info.as_ref())
    }

    /// The namespaces of a catalog. Empty iterator if the catalog is unknown —
    /// the absence of information is not an error here, it is the normal state
    /// of a lazy tree.
    pub fn namespaces(&self, catalog: Option<&str>) -> impl Iterator<Item = &NamespaceRef> + '_ {
        self.catalogs
            .value
            .get(cle(catalog))
            .map(|noeud| noeud.namespaces.value.values())
            .into_iter()
            .flatten()
            .filter_map(|noeud| noeud.info.as_ref())
    }

    /// The relations of a namespace. Empty iterator if the namespace is
    /// unknown.
    pub fn relations(&self, namespace: &CatalogPath) -> impl Iterator<Item = &RelationRef> + '_ {
        self.namespace_node(namespace)
            .map(|noeud| noeud.relations.value.values())
            .into_iter()
            .flatten()
            .map(|noeud| &noeud.summary)
    }

    /// The summary of a relation.
    #[must_use]
    pub fn relation_summary(&self, path: &CatalogPath) -> Option<&RelationRef> {
        self.relation_node(path).map(|noeud| &noeud.summary)
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
            .flat_map(|espace| espace.relations.value.values())
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
            CatalogScope::Catalog(chemin) => self
                .catalogs
                .value
                .get(cle(chemin.catalog()))
                .map_or(Freshness::Never, |noeud| noeud.namespaces.freshness),
            CatalogScope::Namespace(chemin) => self
                .namespace_node(chemin)
                .map_or(Freshness::Never, |noeud| noeud.relations.freshness),
            CatalogScope::Relation(chemin) => self
                .relation_node(chemin)
                .map_or(Freshness::Never, |noeud| noeud.detail.freshness),
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

        let mut perimes = Vec::new();
        for (cle_catalogue, catalogue) in &self.catalogs.value {
            let chemin_catalogue =
                CatalogPath::from_validated(depuis_cle(cle_catalogue), None, None);
            if catalogue.namespaces.freshness.is_stale_at(now, ttl) {
                perimes.push(CatalogScope::Catalog(chemin_catalogue));
                continue;
            }
            for (cle_espace, espace) in &catalogue.namespaces.value {
                if espace.relations.freshness.is_stale_at(now, ttl) {
                    perimes.push(CatalogScope::Namespace(CatalogPath::from_validated(
                        depuis_cle(cle_catalogue),
                        depuis_cle(cle_espace),
                        None,
                    )));
                    continue;
                }
                for relation in espace.relations.value.values() {
                    if relation.is_stale_at(now, ttl) {
                        perimes.push(CatalogScope::Relation(relation.summary.path()));
                    } else {
                        if relation.constraints.freshness.is_stale_at(now, ttl) {
                            perimes.push(CatalogScope::Constraints(relation.summary.path()));
                        }
                        if relation.incoming_keys.freshness.is_stale_at(now, ttl) {
                            perimes
                                .push(CatalogScope::IncomingForeignKeys(relation.summary.path()));
                        }
                        if relation.definition.freshness.is_stale_at(now, ttl) {
                            perimes.push(CatalogScope::Definition(relation.summary.path()));
                        }
                    }
                }
            }
        }
        perimes
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
                self.catalogs.freshness.invalidate();
                for catalogue in self.catalogs.value.values_mut() {
                    Self::invalidate_catalog(catalogue);
                }
            }
            CatalogScope::Catalog(chemin) => {
                if let Some(catalogue) = self.catalogs.value.get_mut(cle(chemin.catalog())) {
                    Self::invalidate_catalog(catalogue);
                }
            }
            CatalogScope::Namespace(chemin) => {
                if let Some(espace) = self.namespace_node_mut_opt(chemin) {
                    Self::invalidate_namespace(espace);
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
            CatalogScope::Relation(chemin) => {
                if let Some(relation) = self.relation_node_mut_opt(chemin) {
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
            CatalogScope::Catalog(chemin) => {
                self.catalogs.value.shift_remove(cle(chemin.catalog()));
                self.catalogs.freshness.invalidate();
            }
            CatalogScope::Namespace(chemin) => {
                if let Some(catalogue) = self.catalogs.value.get_mut(cle(chemin.catalog())) {
                    catalogue
                        .namespaces
                        .value
                        .shift_remove(cle(chemin.namespace()));
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
            CatalogScope::Relation(chemin) => {
                let Some(nom_relation) = chemin.relation().map(str::to_owned) else {
                    return;
                };
                if let Some(espace) = self.namespace_node_mut_opt(chemin) {
                    espace.relations.value.shift_remove(nom_relation.as_str());
                    espace.relations.freshness.invalidate();
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
                let key = cle(path.catalog());
                if self.catalogs.freshness == Freshness::Never {
                    self.catalogs.value.shift_remove(key);
                } else if let Some(catalog) = self.catalogs.value.get_mut(key) {
                    catalog.namespaces = Cached::default();
                }
            }
            CatalogScope::Namespace(path) => {
                let Some(catalog) = self.catalogs.value.get_mut(cle(path.catalog())) else {
                    return;
                };
                let key = cle(path.namespace());
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
        for espace in catalogue.namespaces.value.values_mut() {
            Self::invalidate_namespace(espace);
        }
    }

    fn invalidate_namespace(espace: &mut NamespaceNode) {
        espace.relations.freshness.invalidate();
        for relation in espace.relations.value.values_mut() {
            relation.invalidate();
        }
    }

    fn namespace_node(&self, path: &CatalogPath) -> Option<&NamespaceNode> {
        self.catalogs
            .value
            .get(cle(path.catalog()))?
            .namespaces
            .value
            .get(cle(path.namespace()))
    }

    fn namespace_node_mut_opt(&mut self, path: &CatalogPath) -> Option<&mut NamespaceNode> {
        self.catalogs
            .value
            .get_mut(cle(path.catalog()))?
            .namespaces
            .value
            .get_mut(cle(path.namespace()))
    }

    fn relation_node(&self, path: &CatalogPath) -> Option<&RelationNode> {
        let nom_relation = path.relation()?;
        self.namespace_node(path)?.relations.value.get(nom_relation)
    }

    fn relation_node_mut_opt(&mut self, path: &CatalogPath) -> Option<&mut RelationNode> {
        let nom_relation = path.relation()?.to_owned();
        self.namespace_node_mut_opt(path)?
            .relations
            .value
            .get_mut(nom_relation.as_str())
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
        let info = catalog.map(|nom| CatalogRef::validated(nom.to_owned()));
        self.catalogs
            .value
            .entry(cle(catalog).to_owned())
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
            .map(|nom| NamespaceRef::validated(parent, nom.to_owned()));
        let cle_noeud = cle(path.namespace()).to_owned();
        self.catalog_node_mut(path.catalog())
            .namespaces
            .value
            .entry(cle_noeud)
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
        let Some(nom_relation) = path.relation().map(str::to_owned) else {
            return Err(CacheError::NotARelation);
        };
        let parent = CatalogPath::from_validated(
            path.catalog().map(str::to_owned),
            path.namespace().map(str::to_owned),
            None,
        );
        let resume = RelationRef::validated(parent, nom_relation.clone(), kind);
        Ok(self
            .namespace_node_mut(path)
            .relations
            .value
            .entry(nom_relation)
            .or_insert_with(|| RelationNode::new(resume)))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::Capabilities;

    use super::*;
    use crate::DefinitionSource;
    use crate::model::{ConstraintKind, Field, LogicalType};

    const HEURE: Duration = Duration::from_secs(3600);

    fn chemin(catalogue: Option<&str>, espace: Option<&str>, relation: &str) -> CatalogPath {
        CatalogPath::for_relation(catalogue, espace, relation).expect("valid path")
    }

    fn espace_public() -> CatalogPath {
        CatalogPath::for_namespace(Some("caisse"), "public").expect("valid path")
    }

    fn cache_postgres() -> CatalogCache {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "PostgreSQL",
            "17.2",
            Capabilities::SQL | Capabilities::SCHEMAS,
        ));
        cache.set_catalogs(vec![CatalogRef::new("caisse").expect("valid")]);
        cache.set_namespaces(
            Some("caisse"),
            vec![
                NamespaceRef::new(CatalogPath::for_catalog("caisse").expect("valid"), "public")
                    .expect("valid"),
            ],
        );
        let espace = espace_public();
        cache
            .set_relations(
                &espace,
                vec![
                    RelationRef::new(espace.clone(), "clients", RelationKind::Table)
                        .expect("valid"),
                    RelationRef::new(espace.clone(), "commandes", RelationKind::Table)
                        .expect("valid"),
                ],
            )
            .expect("a namespace is indeed a namespace");
        cache
    }

    #[test]
    fn eviction_drops_contents_but_keeps_the_node_its_parent_listed() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("relation");
        cache.set_indexes(&table, vec![]).expect("indexes");
        cache.set_constraints(&table, vec![]).expect("constraints");

        cache.evict(&CatalogScope::Relation(table.clone()));
        assert!(cache.relation_summary(&table).is_some());
        assert!(cache.relation(&table).is_none());
        assert!(cache.indexes(&table).is_none(), "unread, never empty");
        assert!(cache.constraints(&table).is_some(), "own scope, kept");
        assert_eq!(
            cache.freshness(&CatalogScope::Relation(table.clone())),
            Freshness::Never
        );

        let espace = espace_public();
        cache.evict(&CatalogScope::Namespace(espace.clone()));
        assert_eq!(cache.relations(&espace).count(), 0);
        assert_eq!(cache.namespaces(Some("caisse")).count(), 1);
        assert_eq!(
            cache.freshness(&CatalogScope::Namespace(espace)),
            Freshness::Never
        );

        let catalogue = CatalogPath::for_catalog("caisse").expect("valid");
        cache.evict(&CatalogScope::Catalog(catalogue));
        assert_eq!(cache.namespaces(Some("caisse")).count(), 0);
        assert_eq!(cache.catalogs().count(), 1);
        assert!(cache.server_info().is_some());
    }

    #[test]
    fn eviction_removes_a_relation_no_listing_carries() {
        let mut cache = CatalogCache::new();
        let direct = chemin(None, Some("main"), "opened_from_a_tab");
        let narrow = chemin(None, Some("main"), "with_constraints");
        for path in [&direct, &narrow] {
            cache
                .set_relation(path, Relation::new("t", RelationKind::Table))
                .expect("relation");
        }
        cache.set_constraints(&narrow, vec![]).expect("constraints");

        cache.evict(&CatalogScope::Relation(direct.clone()));
        cache.evict(&CatalogScope::Relation(narrow.clone()));
        assert!(cache.relation_summary(&direct).is_none());
        assert!(
            cache.relation_summary(&narrow).is_some(),
            "still carries its constraints"
        );

        cache.evict(&CatalogScope::Constraints(narrow.clone()));
        cache.evict(&CatalogScope::Relation(narrow.clone()));
        assert!(cache.relation_summary(&narrow).is_none());
    }

    #[test]
    fn incoming_key_cache_is_independent_from_outgoing_keys_and_constraints() {
        let path = CatalogPath::for_relation(None, Some("main"), "parent").expect("path");
        let mut cache = CatalogCache::new();
        assert!(cache.set_incoming_foreign_keys(&path, vec![]).is_err());
        cache
            .set_relation(&path, Relation::new("parent", RelationKind::Table))
            .expect("relation");
        cache.set_foreign_keys(&path, vec![]).expect("outgoing");
        cache.set_constraints(&path, vec![]).expect("constraints");
        assert!(cache.incoming_foreign_keys(&path).is_none());
        cache
            .set_incoming_foreign_keys(&path, vec![])
            .expect("empty incoming");
        let scope = CatalogScope::IncomingForeignKeys(path.clone());
        cache.invalidate(&scope);
        assert_eq!(cache.freshness(&scope), Freshness::Invalidated);
        assert!(matches!(
            cache.freshness(&CatalogScope::Constraints(path.clone())),
            Freshness::Fetched(_)
        ));
        assert!(!scope.contains(&CatalogScope::Constraints(path.clone())));
        assert!(CatalogScope::Relation(path.clone()).contains(&scope));
        cache.forget(&scope);
        assert!(cache.incoming_foreign_keys(&path).is_none());
        assert!(cache.foreign_keys(&path).is_some());
    }

    #[test]
    fn legacy_constraint_json_does_not_invent_a_validation_status() {
        let constraint: Constraint = serde_json::from_str(r#"{"name":"key","kind":"primary_key","fields":["id"],"expression":"PRIMARY KEY (id)"}"#).expect("legacy constraint");
        assert_eq!(constraint.validated, None);
    }

    #[test]
    fn constraints_preserve_unknown_empty_and_invalidated_states() {
        let path = CatalogPath::for_relation(None, Some("main"), "odd\"; table").expect("path");
        let mut cache = CatalogCache::new();
        assert!(cache.set_constraints(&path, vec![]).is_err());
        cache
            .set_relation(&path, Relation::new("odd\"; table", RelationKind::Table))
            .expect("relation");
        assert_eq!(cache.constraints(&path), None);
        let legacy = serde_json::to_string(&cache).expect("serialize");
        let mut json: serde_json::Value = serde_json::from_str(&legacy).expect("json");
        fn remove_constraints(value: &mut serde_json::Value) {
            if let Some(object) = value.as_object_mut() {
                object.remove("constraints");
                for value in object.values_mut() {
                    remove_constraints(value);
                }
            }
        }
        remove_constraints(&mut json);
        let restored: CatalogCache =
            serde_json::from_value(json).expect("old caches remain readable");
        assert_eq!(restored.constraints(&path), None);
        cache.set_constraints(&path, vec![]).expect("empty");
        assert_eq!(cache.constraints(&path), Some([].as_slice()));
        let scope = CatalogScope::Constraints(path.clone());
        cache.invalidate(&scope);
        assert_eq!(cache.freshness(&scope), Freshness::Invalidated);
        assert_eq!(cache.constraints(&path), Some([].as_slice()));
        assert!(CatalogScope::Relation(path.clone()).contains(&scope));
        assert!(!scope.contains(&CatalogScope::Relation(path.clone())));
        cache.forget(&scope);
        assert!(cache.relation(&path).is_some());
        assert_eq!(cache.constraints(&path), None);
    }

    #[test]
    fn a_new_cache_is_empty_and_makes_nothing_stale() {
        let cache = CatalogCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.server_info(), None);
        assert!(
            cache.stale(HEURE).is_empty(),
            "a level never read is absent, not stale"
        );
    }

    #[test]
    fn the_tree_fills_level_by_level() {
        let cache = cache_postgres();
        assert_eq!(cache.catalogs().count(), 1);
        assert_eq!(cache.namespaces(Some("caisse")).count(), 1);
        assert_eq!(cache.relations(&espace_public()).count(), 2);
        assert_eq!(cache.relation_count(), 2);
        assert_eq!(
            cache.server_info().map(ToString::to_string).as_deref(),
            Some("PostgreSQL 17.2")
        );
    }

    #[test]
    fn querying_an_unknown_node_returns_empty_not_an_error() {
        let cache = cache_postgres();
        assert_eq!(cache.namespaces(Some("inexistant")).count(), 0);
        let ailleurs = CatalogPath::for_namespace(Some("caisse"), "inexistant").expect("valid");
        assert_eq!(cache.relations(&ailleurs).count(), 0);
        assert!(
            cache
                .relation(&chemin(Some("caisse"), Some("public"), "absente"))
                .is_none()
        );
    }

    #[test]
    fn a_partial_insertion_creates_its_parents() {
        // The search case: a table is described without its schema being
        // listed.
        let mut cache = CatalogCache::new();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("the path names a relation");

        assert!(cache.relation(&table).is_some());
        assert_eq!(cache.relation_count(), 1);
        assert!(
            cache.stale(HEURE).is_empty(),
            "the parents created on the way were never read: they are not stale"
        );
    }

    #[test]
    fn a_relation_created_on_the_way_keeps_its_kind() {
        // The kind comes from the description, never from a default: a MongoDB
        // collection displayed as a table would be a wrong interface surface.
        let mut cache = CatalogCache::new();
        let collection = chemin(None, Some("boutique"), "commandes");
        cache
            .set_relation(
                &collection,
                Relation::new("commandes", RelationKind::Collection),
            )
            .expect("valid");
        assert_eq!(
            cache.relation_summary(&collection).map(|r| r.kind),
            Some(RelationKind::Collection)
        );
    }

    #[test]
    fn a_path_at_the_wrong_level_is_a_calling_bug() {
        let mut cache = CatalogCache::new();
        let err = cache
            .set_relation(
                &espace_public(),
                Relation::new("clients", RelationKind::Table),
            )
            .expect_err("a namespace is not a relation");
        assert_eq!(err, CacheError::NotARelation);

        let table = chemin(Some("caisse"), Some("public"), "clients");
        assert_eq!(
            cache
                .set_relations(&table, Vec::new())
                .expect_err("a relation is not a namespace"),
            CacheError::NotANamespace
        );
    }

    #[test]
    fn attaching_indexes_to_an_unknown_relation_is_refused() {
        // Creating it here would force guessing its kind.
        let mut cache = CatalogCache::new();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        assert_eq!(
            cache
                .set_indexes(&table, Vec::new())
                .expect_err("relation never seen"),
            CacheError::UnknownRelation
        );
    }

    #[test]
    fn relisting_keeps_the_details_of_remaining_relations() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(
                &table,
                Relation::new("clients", RelationKind::Table).with_fields(vec![Field::new(
                    "id",
                    0,
                    LogicalType::INT64,
                    "int8",
                )]),
            )
            .expect("valid");

        let espace = espace_public();
        cache
            .set_relations(
                &espace,
                vec![
                    RelationRef::new(espace.clone(), "clients", RelationKind::Table)
                        .expect("valid"),
                ],
            )
            .expect("valid");

        assert!(
            cache.relation(&table).is_some(),
            "relisting a schema must not force redescribing every open table"
        );
        assert!(
            cache
                .relation(&chemin(Some("caisse"), Some("public"), "commandes"))
                .is_none(),
            "a relation missing from the fresh listing disappears"
        );
        assert_eq!(cache.relation_count(), 1);
    }

    #[test]
    fn missing_levels_do_not_get_confused() {
        // MySQL has no catalog level. The "missing level" node must not
        // collide with a really named catalog.
        let mut cache = CatalogCache::new();
        let espace_mysql = CatalogPath::for_namespace(None, "caisse").expect("valid");
        cache
            .set_relations(
                &espace_mysql,
                vec![
                    RelationRef::new(espace_mysql.clone(), "clients", RelationKind::Table)
                        .expect("valid"),
                ],
            )
            .expect("valid");

        assert_eq!(cache.relations(&espace_mysql).count(), 1);
        let sous_catalogue = CatalogPath::for_namespace(Some("caisse"), "caisse").expect("valid");
        assert_eq!(cache.relations(&sous_catalogue).count(), 0);
        assert_eq!(
            cache.catalogs().count(),
            0,
            "a missing level is not displayed as an anonymous catalog"
        );
    }

    #[test]
    fn an_intermediate_hole_can_be_navigated() {
        // Neo4j: catalog + relation, without a namespace.
        let mut cache = CatalogCache::new();
        let base = CatalogPath::for_catalog("graphe").expect("valid");
        cache
            .set_relations(
                &base,
                vec![
                    RelationRef::new(base.clone(), "Personne", RelationKind::NodeLabel)
                        .expect("valid"),
                ],
            )
            .expect("a catalog is an acceptable parent");

        let label = chemin(Some("graphe"), None, "Personne");
        assert!(cache.relation_summary(&label).is_some());
        assert_eq!(cache.relations(&base).count(), 1);
        assert_eq!(
            cache.relation_summary(&label).map(RelationRef::path),
            Some(label)
        );
    }

    #[test]
    fn what_was_not_read_differs_from_what_is_empty() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        assert!(
            cache.indexes(&table).is_none(),
            "\"not read\" is not \"no index\""
        );

        cache.set_indexes(&table, Vec::new()).expect("valid");
        let index = cache.indexes(&table).expect("the indexes were read");
        assert!(index.is_empty(), "an empty slice does say \"no index\"");
    }

    #[test]
    fn staleness_follows_the_lifetime() {
        let cache = cache_postgres();
        assert!(
            cache.stale(HEURE).is_empty(),
            "the listings were just written"
        );
        let perimes = cache.stale_at(Utc::now() + TimeDelta::hours(2), HEURE);
        assert_eq!(
            perimes,
            vec![CatalogScope::Server],
            "the returned scope is minimal"
        );
    }

    #[test]
    fn staleness_returns_a_minimal_set() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("valid");

        cache.invalidate(&CatalogScope::Relation(table.clone()));
        assert_eq!(
            cache.stale(HEURE),
            vec![CatalogScope::Relation(table)],
            "only the invalidated relation is to be reread"
        );

        // Invalidating the namespace above absorbs the relation: refreshing the
        // parent refreshes the child.
        let espace = espace_public();
        cache.invalidate(&CatalogScope::Namespace(espace.clone()));
        assert_eq!(cache.stale(HEURE), vec![CatalogScope::Namespace(espace)]);
    }

    #[test]
    fn an_invalidation_does_not_lose_the_data() {
        // The tree stays readable during the reread: emptying it at the first
        // ALTER TABLE would make it flicker.
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("valid");

        cache.invalidate(&CatalogScope::Relation(table.clone()));
        assert!(cache.relation(&table).is_some(), "the data stays readable");
        assert_eq!(
            cache.freshness(&CatalogScope::Relation(table.clone())),
            Freshness::Invalidated
        );
        assert!(cache.stale(HEURE).contains(&CatalogScope::Relation(table)));
    }

    #[test]
    fn an_invalidation_descends_into_the_subtree() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("valid");

        cache.invalidate_all();
        assert_eq!(
            cache.freshness(&CatalogScope::Relation(table)),
            Freshness::Invalidated,
            "the server invalidation reaches the leaves"
        );
        assert_eq!(cache.stale(HEURE), vec![CatalogScope::Server]);
    }

    #[test]
    fn invalidating_what_was_never_read_does_not_make_it_appear() {
        let mut cache = cache_postgres();
        // The relation is in the listing, but was never described.
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache.invalidate(&CatalogScope::Relation(table));
        assert!(
            cache.stale(HEURE).is_empty(),
            "invalidating a description never requested does not put it to work"
        );
    }

    #[test]
    fn forgetting_invalidates_the_parent_listing() {
        // After a DROP TABLE, the schema's listing became wrong: leaving it
        // fresh would make the table reappear at the next refresh.
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache.forget(&CatalogScope::Relation(table.clone()));

        assert_eq!(cache.relation_summary(&table), None);
        assert_eq!(cache.relation_count(), 1);
        let espace = espace_public();
        assert_eq!(
            cache.freshness(&CatalogScope::Namespace(espace.clone())),
            Freshness::Invalidated
        );
        assert_eq!(cache.stale(HEURE), vec![CatalogScope::Namespace(espace)]);
    }

    #[test]
    fn forgetting_the_server_empties_the_cache() {
        let mut cache = cache_postgres();
        cache.forget(&CatalogScope::Server);
        assert!(cache.is_empty());
    }

    #[test]
    fn a_scope_covers_its_descendants() {
        let table = chemin(Some("caisse"), Some("public"), "clients");
        let espace = espace_public();
        let catalogue = CatalogPath::for_catalog("caisse").expect("valid");

        assert!(CatalogScope::Server.contains(&CatalogScope::Relation(table.clone())));
        assert!(
            CatalogScope::Catalog(catalogue.clone())
                .contains(&CatalogScope::Namespace(espace.clone()))
        );
        assert!(
            CatalogScope::Namespace(espace.clone())
                .contains(&CatalogScope::Relation(table.clone()))
        );

        assert!(!CatalogScope::Relation(table).contains(&CatalogScope::Namespace(espace)));
        assert!(!CatalogScope::Catalog(catalogue).contains(&CatalogScope::Server));
    }

    #[test]
    fn a_scope_does_not_cover_a_sibling() {
        let a = CatalogScope::Namespace(espace_public());
        let b = CatalogScope::Relation(chemin(Some("caisse"), Some("archives"), "clients"));
        assert!(!a.contains(&b));
    }

    #[test]
    fn the_scope_is_deduced_from_the_path() {
        assert_eq!(
            CatalogScope::of(&CatalogPath::empty()),
            CatalogScope::Server
        );
        assert_eq!(
            CatalogScope::of(&chemin(Some("c"), Some("n"), "r")).level(),
            CatalogLevel::Relation
        );
    }

    #[test]
    fn a_clock_going_backwards_makes_nothing_stale() {
        let cache = cache_postgres();
        assert!(
            cache
                .stale_at(Utc::now() - TimeDelta::hours(48), HEURE)
                .is_empty(),
            "an instant before the read does not make the node stale"
        );
    }

    #[test]
    fn the_rendering_of_a_scope_names_the_level() {
        let scope = CatalogScope::Relation(chemin(Some("caisse"), Some("public"), "clients"));
        assert_eq!(scope.to_string(), "relation caisse.public.clients");
        assert_eq!(CatalogScope::Server.to_string(), "server");
    }

    fn definition_fixture(sql_len: usize, notes: Vec<String>) -> RelationDefinition {
        RelationDefinition {
            sql: "x".repeat(sql_len),
            source: DefinitionSource::Stored,
            notes,
        }
    }

    #[test]
    fn bounded_definition_eviction_preserves_relation_details() {
        let mut cache = CatalogCache::new();
        let paths: Vec<_> = (0..17)
            .map(|index| chemin(None, Some("main"), &format!("relation_{index}")))
            .collect();
        for path in &paths[..16] {
            cache
                .set_relation(
                    path,
                    Relation::new(path.relation().unwrap_or_default(), RelationKind::Table),
                )
                .expect("relation");
            cache
                .set_definition(path, definition_fixture(1, vec![]))
                .expect("definition");
        }

        cache
            .set_definition(&paths[0], definition_fixture(2, vec!["refreshed".into()]))
            .expect("refresh");
        cache
            .set_relation(
                &paths[16],
                Relation::new(
                    paths[16].relation().unwrap_or_default(),
                    RelationKind::Table,
                ),
            )
            .expect("relation");
        cache
            .set_definition(&paths[16], definition_fixture(1, vec![]))
            .expect("definition");

        assert!(cache.definition(&paths[0]).is_some());
        assert!(cache.definition(&paths[1]).is_none());
        assert!(cache.definition(paths.last().expect("last")).is_some());
        assert_eq!(
            paths
                .iter()
                .filter(|path| cache.definition(path).is_some())
                .count(),
            MAX_DEFINITIONS
        );

        let retained = paths.last().expect("last").clone();
        cache
            .set_indexes(&retained, vec![Index::new("idx", vec!["id".into()])])
            .expect("index");
        cache
            .set_constraints(
                &retained,
                vec![Constraint::new(
                    "pk",
                    ConstraintKind::PrimaryKey,
                    vec!["id".into()],
                )],
            )
            .expect("constraint");
        assert_eq!(cache.definition(&paths[0]).expect("refreshed").sql.len(), 2);
        assert_eq!(cache.indexes(&retained).expect("index").len(), 1);
        assert_eq!(cache.constraints(&retained).expect("constraint").len(), 1);
    }

    #[test]
    fn definition_size_limit_counts_sql_and_notes() {
        let mut cache = CatalogCache::new();
        let paths: Vec<_> = (0..16)
            .map(|index| chemin(None, Some("main"), &format!("large_{index}")))
            .collect();
        for path in &paths {
            cache
                .set_relation(
                    path,
                    Relation::new(path.relation().unwrap_or_default(), RelationKind::Table),
                )
                .expect("relation");
            cache
                .set_definition(path, definition_fixture(1_048_576, vec![]))
                .expect("definition");
        }
        let extra = chemin(None, Some("main"), "with_notes");
        cache
            .set_relation(&extra, Relation::new("with_notes", RelationKind::Table))
            .expect("relation");
        cache
            .set_definition(&extra, definition_fixture(1, vec!["n".repeat(65_536)]))
            .expect("definition");

        assert!(cache.definition(&paths[0]).is_none());
        assert!(cache.definition(&extra).is_some());
        assert_eq!(
            paths
                .iter()
                .filter(|path| cache.definition(path).is_some())
                .count(),
            MAX_DEFINITIONS - 1
        );
    }

    #[test]
    fn invalidated_definitions_remain_evictable_and_bounded() {
        let mut cache = CatalogCache::new();
        let paths: Vec<_> = (0..16)
            .map(|index| chemin(None, Some("main"), &format!("invalidated_{index}")))
            .collect();
        for path in &paths {
            cache
                .set_relation(
                    path,
                    Relation::new(path.relation().unwrap_or_default(), RelationKind::Table),
                )
                .expect("relation");
            cache
                .set_definition(path, definition_fixture(1_048_576, vec![]))
                .expect("definition");
            cache.invalidate(&CatalogScope::Definition(path.clone()));
        }
        let extra = chemin(None, Some("main"), "after_invalidation");
        cache
            .set_relation(
                &extra,
                Relation::new("after_invalidation", RelationKind::Table),
            )
            .expect("relation");
        cache
            .set_definition(&extra, definition_fixture(1, vec!["n".repeat(65_536)]))
            .expect("definition");

        let definitions: Vec<_> = paths
            .iter()
            .chain(std::iter::once(&extra))
            .filter_map(|path| cache.definition(path))
            .collect();
        assert!(definitions.len() <= MAX_DEFINITIONS);
        assert!(
            definitions
                .iter()
                .map(|definition| definition_bytes(definition))
                .sum::<usize>()
                <= MAX_DEFINITION_BYTES
        );
        assert!(cache.definition(&extra).is_some());
    }
}
