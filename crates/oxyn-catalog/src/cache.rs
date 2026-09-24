//! Le cache d'introspection : un arbre en mémoire, alimentable par morceaux.
//!
//! C'est ce qui rend l'exploration hors ligne possible, et ce qui rend le
//! workspace IA viable : le contexte d'un agent se construit à partir du
//! catalogue local, pas d'un aller-retour serveur à chaque question
//! (ARCHITECTURE §6).
//!
//! # Trois décisions qui gouvernent ce module
//!
//! **Un palier jamais lu n'est pas périmé : il est absent.**
//! [`CatalogCache::stale`] ne rend que des nœuds lus au moins une fois. Sans
//! cette règle, un rafraîchissement de fond décrirait les 20 000 relations d'un
//! schéma que personne n'a ouvertes — exactement ce que la paresse du
//! [`CatalogProvider`](crate::provider::CatalogProvider) évite.
//!
//! **Invalider n'est pas oublier.** [`CatalogCache::invalidate`] marque un
//! sous-arbre à relire mais **garde ses données** : l'exploration hors ligne est
//! une fonctionnalité, et vider l'arborescence au premier `ALTER TABLE` la
//! ferait clignoter. [`CatalogCache::forget`] existe pour ce qui a réellement
//! disparu.
//!
//! **L'horloge est celle du mur.** Le cache est persisté par `oxyn-store` et
//! survit au redémarrage ; un `Instant` monotone ne se sérialise pas. Un recul
//! de l'horloge rend donc un nœud « pas encore périmé » plutôt que périmé — le
//! sens prudent, puisque l'autre inviterait à réintrospecter en boucle.
//!
//! # Concurrence
//!
//! [`CatalogCache`] n'a pas de verrou interne : le partage est le choix de
//! l'appelant, et [`SharedCatalog`] en donne la forme habituelle — le thread
//! d'interface lit, la tâche de rafraîchissement écrit. La section critique se
//! limite à des métadonnées en mémoire, jamais à une entrée-sortie (I-05).

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

/// Clé de nœud d'un palier absent.
///
/// La chaîne vide ne peut pas nommer un objet — `crate::path::validate_segment`
/// la refuse —, donc elle n'entre jamais en collision avec un nom réel. C'est ce
/// qui permet à MySQL (« pas de palier catalogue ») et à une base PostgreSQL
/// réellement nommée d'occuper le même arbre sans se marcher dessus.
const PALIER_ABSENT: &str = "";

const MAX_DEFINITIONS: usize = 16;
const MAX_DEFINITION_BYTES: usize = 16 * 1024 * 1024;

/// La clé de nœud correspondant à un nom de palier éventuel.
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

/// Le nom de palier correspondant à une clé de nœud.
fn depuis_cle(valeur: &str) -> Option<String> {
    if valeur.is_empty() {
        None
    } else {
        Some(valeur.to_owned())
    }
}

/// Un partage habituel du cache : lecture par l'interface, écriture par la
/// tâche de rafraîchissement.
pub type SharedCatalog = Arc<RwLock<CatalogCache>>;

/// Le cache d'une connexion, remis à qui a obtenu le droit de le lire.
///
/// Existe pour traverser les rapports d'exécution — qui se comparent et se
/// journalisent — sans copier le cache : deux poignées sont égales quand elles
/// désignent **le même** cache, et le `Debug` ne dit rien de son contenu, qui
/// porte les noms d'objets de la base de l'utilisateur.
#[derive(Clone)]
pub struct CatalogHandle(SharedCatalog);

impl CatalogHandle {
    /// Enveloppe un cache partagé.
    #[must_use]
    pub const fn new(catalog: SharedCatalog) -> Self {
        Self(catalog)
    }

    /// Le cache désigné. La lecture prend le verrou du cache : ne pas la
    /// garder au-delà d'un rendu.
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

/// Ce qu'une opération sur le cache peut refuser.
///
/// Ces refus dénoncent un **bug d'appel**, pas une panne : passer un chemin
/// d'espace de noms là où une relation est attendue, ou attacher des index à une
/// relation dont le cache n'a jamais entendu parler. D'où la conversion vers
/// [`OxynError::Internal`](oxyn_core::OxynError::Internal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// Le chemin ne désigne pas une relation.
    #[error("the path does not name a relation")]
    NotARelation,
    /// Le chemin descend plus bas que le palier espace de noms.
    #[error("the path does not name a namespace")]
    NotANamespace,
    /// La relation visée n'est pas dans le cache.
    ///
    /// Attacher des index à une relation inconnue en inventerait une, avec une
    /// nature devinée. Lister ou décrire la relation d'abord.
    #[error("the relation is not in the cache")]
    UnknownRelation,
}

impl From<CacheError> for oxyn_core::OxynError {
    fn from(err: CacheError) -> Self {
        Self::Internal(err.to_string())
    }
}

/// L'état de fraîcheur d'un nœud du cache.
///
/// Trois états, et pas deux : « jamais lu » et « lu puis invalidé » se
/// ressemblent — ni l'un ni l'autre n'a de donnée à jour — mais appellent des
/// conduites opposées. Le premier attend qu'on le demande ; le second doit être
/// relu tout de suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Jamais lu. **N'est pas périmé** : il est absent, et son chargement est
    /// une expansion paresseuse déclenchée par l'utilisateur.
    #[default]
    Never,
    /// Lu à cet instant.
    Fetched(DateTime<Utc>),
    /// Lu, puis invalidé — typiquement après un DDL émis depuis Oxyn. Périmé
    /// quel que soit le délai.
    Invalidated,
}

impl Freshness {
    /// Marque une lecture qui vient d'avoir lieu.
    #[must_use]
    pub fn now() -> Self {
        Self::Fetched(Utc::now())
    }

    /// Ce nœud a-t-il déjà été lu ?
    #[must_use]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Never)
    }

    /// Instant de la dernière lecture, s'il y en a eu une.
    #[must_use]
    pub const fn fetched_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Fetched(instant) => Some(*instant),
            Self::Never | Self::Invalidated => None,
        }
    }

    /// Ce nœud doit-il être relu, à l'instant `now` et pour une durée de vie
    /// `ttl` ?
    #[must_use]
    pub fn is_stale_at(&self, now: DateTime<Utc>, ttl: Duration) -> bool {
        match self {
            Self::Never => false,
            Self::Invalidated => true,
            Self::Fetched(instant) => match TimeDelta::from_std(ttl) {
                Ok(limite) => now.signed_duration_since(*instant) > limite,
                // Une durée de vie hors des bornes de `chrono` (plus de ~584
                // millénaires) ne périme rien. Le sens inverse ferait
                // réintrospecter en boucle sur une valeur aberrante.
                Err(_) => false,
            },
        }
    }

    /// Passe à [`Invalidated`](Self::Invalidated), sauf si le nœud n'a jamais
    /// été lu — invalider ce qui n'a jamais existé le ferait apparaître dans
    /// [`CatalogCache::stale`] sans que personne ne l'ait demandé.
    fn invalidate(&mut self) {
        if self.is_known() {
            *self = Self::Invalidated;
        }
    }
}

/// Une valeur du cache et sa fraîcheur.
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
    /// Remplace la valeur et la marque lue maintenant.
    fn set(&mut self, value: T) {
        self.value = value;
        self.freshness = Freshness::now();
    }
}

/// Un catalogue et ses espaces de noms.
///
/// `info` vaut `None` pour le nœud qui porte un **palier absent** : MySQL n'a
/// pas de catalogue, et lui en inventer un au nom vide le ferait apparaître dans
/// l'arborescence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CatalogNode {
    info: Option<CatalogRef>,
    namespaces: Cached<IndexMap<String, NamespaceNode>>,
}

/// Un espace de noms et ses relations. `info` suit la même règle que
/// [`CatalogNode::info`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct NamespaceNode {
    info: Option<NamespaceRef>,
    relations: Cached<IndexMap<String, RelationNode>>,
}

/// Une relation : son résumé, et les trois détails qui se demandent séparément.
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

/// Le sous-arbre visé par une invalidation ou un rafraîchissement.
///
/// Un scope désigne un nœud **et tout ce qui est dessous** : rafraîchir
/// [`Server`](Self::Server) relit l'arbre entier, rafraîchir
/// [`Relation`](Self::Relation) ne relit qu'une table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogScope {
    /// Tout le serveur.
    Server,
    /// Un catalogue et son contenu.
    Catalog(CatalogPath),
    /// Un espace de noms et ses relations.
    Namespace(CatalogPath),
    /// Une relation : sa description, ses index, ses clés étrangères.
    Relation(CatalogPath),
    /// Only the constraints of a relation.
    Constraints(CatalogPath),
    /// Only the keys referencing this relation.
    IncomingForeignKeys(CatalogPath),
    /// Only the creation statements of a relation.
    Definition(CatalogPath),
}

impl CatalogScope {
    /// Le scope correspondant au palier le plus profond d'un chemin.
    ///
    /// Ne peut pas se tromper de variante, contrairement à une construction
    /// directe.
    #[must_use]
    pub fn of(path: &CatalogPath) -> Self {
        match path.level() {
            CatalogLevel::Server => Self::Server,
            CatalogLevel::Catalog => Self::Catalog(path.clone()),
            CatalogLevel::Namespace => Self::Namespace(path.clone()),
            CatalogLevel::Relation => Self::Relation(path.clone()),
        }
    }

    /// Le chemin visé, hors [`Server`](Self::Server) qui n'en a pas.
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

    /// Le palier visé.
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

    /// Ce scope couvre-t-il `other` ?
    ///
    /// Sert à ne pas rafraîchir deux fois : rafraîchir un espace de noms couvre
    /// déjà chacune de ses relations. Un scope se couvre lui-même.
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

/// L'arbre de métadonnées d'**une** connexion.
///
/// Alimentable par morceaux : décrire une relation ne demande pas d'avoir
/// d'abord listé son espace de noms. Les paliers manquants sont créés au
/// passage, avec une fraîcheur [`Freshness::Never`] — ils sont là pour porter
/// leur enfant, ils ne prétendent pas avoir été lus.
///
/// Sérialisable de bout en bout : c'est `oxyn-store` qui le persiste, et le
/// format reste lisible sans Oxyn (I-11).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogCache {
    server: Cached<Option<ServerInfo>>,
    catalogs: Cached<IndexMap<String, CatalogNode>>,
}

impl CatalogCache {
    /// Un cache vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ── Écriture ────────────────────────────────────────────────────────────

    /// Enregistre l'identité du serveur et les capacités de la session.
    pub fn set_server_info(&mut self, info: ServerInfo) {
        self.server.set(Some(info));
    }

    /// Enregistre la liste des catalogues.
    ///
    /// Les catalogues absents de la liste sont **retirés** : un listing frais
    /// fait autorité sur son palier. Ceux qui subsistent gardent leur
    /// sous-arbre, donc leur fraîcheur.
    pub fn set_catalogs(&mut self, catalogs: Vec<CatalogRef>) {
        let mut ancien = std::mem::take(&mut self.catalogs.value);
        let mut nouveau = IndexMap::with_capacity(catalogs.len());
        for info in catalogs {
            let cle_noeud = info.name().to_owned();
            // `swap_remove` et non `shift_remove` : la carte d'origine est
            // jetée, et un retrait ordonné coûterait un parcours par élément —
            // quadratique sur un serveur à mille bases.
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

    /// Enregistre les espaces de noms d'un catalogue, ou du serveur quand
    /// `catalog` vaut `None`.
    ///
    /// Le catalogue est créé s'il manque. Chaque référence est réattachée au
    /// parent donné : le chemin d'un [`NamespaceRef`] et sa position dans
    /// l'arbre ne peuvent donc pas diverger.
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

    /// Enregistre les relations d'un espace de noms.
    ///
    /// Les détails déjà connus des relations qui subsistent sont conservés :
    /// relister un schéma ne doit pas obliger à redécrire chaque table ouverte.
    ///
    /// # Erreurs
    /// [`CacheError::NotANamespace`] si `namespace` désigne une relation.
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

    /// Enregistre la description complète d'une relation.
    ///
    /// Les paliers manquants sont créés, et la relation elle-même si le listing
    /// n'a pas encore eu lieu : décrire une table trouvée par la recherche ne
    /// doit pas exiger d'avoir d'abord listé son schéma. La nature du nœud créé
    /// vient de [`Relation::kind`], jamais d'une valeur par défaut.
    ///
    /// Les types imbriqués au-delà de
    /// [`MAX_TYPE_DEPTH`](crate::nesting::MAX_TYPE_DEPTH) sont coupés : un
    /// document inféré sans fin ferait déborder la pile au premier `Drop`.
    ///
    /// # Erreurs
    /// [`CacheError::NotARelation`] si `path` ne nomme pas de relation.
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

    /// Enregistre les index d'une relation.
    ///
    /// # Erreurs
    /// [`CacheError::NotARelation`] si `path` ne nomme pas de relation,
    /// [`CacheError::UnknownRelation`] si la relation n'a été ni listée ni
    /// décrite : la créer ici obligerait à deviner sa nature.
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

    /// Enregistre les clés étrangères d'une relation.
    ///
    /// # Erreurs
    /// Les mêmes que [`Self::set_indexes`].
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

    // ── Lecture ─────────────────────────────────────────────────────────────

    /// L'identité du serveur, si elle a été lue.
    #[must_use]
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server.value.as_ref()
    }

    /// Les catalogues connus, dans l'ordre où le serveur les a donnés.
    ///
    /// Le nœud « palier absent » n'y figure pas : une source sans catalogue rend
    /// un itérateur vide, ce qui est la vérité.
    pub fn catalogs(&self) -> impl Iterator<Item = &CatalogRef> + '_ {
        self.catalogs
            .value
            .values()
            .filter_map(|noeud| noeud.info.as_ref())
    }

    /// Les espaces de noms d'un catalogue. Itérateur vide si le catalogue est
    /// inconnu — l'absence d'information n'est pas une erreur ici, c'est l'état
    /// normal d'un arbre paresseux.
    pub fn namespaces(&self, catalog: Option<&str>) -> impl Iterator<Item = &NamespaceRef> + '_ {
        self.catalogs
            .value
            .get(cle(catalog))
            .map(|noeud| noeud.namespaces.value.values())
            .into_iter()
            .flatten()
            .filter_map(|noeud| noeud.info.as_ref())
    }

    /// Les relations d'un espace de noms. Itérateur vide si l'espace de noms est
    /// inconnu.
    pub fn relations(&self, namespace: &CatalogPath) -> impl Iterator<Item = &RelationRef> + '_ {
        self.namespace_node(namespace)
            .map(|noeud| noeud.relations.value.values())
            .into_iter()
            .flatten()
            .map(|noeud| &noeud.summary)
    }

    /// Le résumé d'une relation.
    #[must_use]
    pub fn relation_summary(&self, path: &CatalogPath) -> Option<&RelationRef> {
        self.relation_node(path).map(|noeud| &noeud.summary)
    }

    /// La description d'une relation, si elle a été demandée.
    #[must_use]
    pub fn relation(&self, path: &CatalogPath) -> Option<&Relation> {
        self.relation_node(path)?.detail.value.as_ref()
    }

    /// Les index d'une relation, s'ils ont été demandés.
    ///
    /// `None` signifie « pas lu », pas « aucun index » : c'est le tranchant du
    /// modèle de capacités, et la tranche vide dit bien, elle, « aucun ».
    #[must_use]
    pub fn indexes(&self, path: &CatalogPath) -> Option<&[Index]> {
        self.relation_node(path)?.indexes.value.as_deref()
    }

    /// Les clés étrangères d'une relation, si elles ont été demandées. Même
    /// distinction que pour [`Self::indexes`].
    #[must_use]
    pub fn foreign_keys(&self, path: &CatalogPath) -> Option<&[ForeignKey]> {
        self.relation_node(path)?.foreign_keys.value.as_deref()
    }

    /// Toutes les relations connues, avec leur description quand elle a été
    /// demandée.
    ///
    /// C'est le parcours qu'emploie [`fn@crate::search`]. Aucun chemin n'est
    /// construit ici : chaque [`RelationRef`] porte déjà le sien, et allouer
    /// trois chaînes par relation à chaque frappe dans une barre de recherche
    /// serait un défaut de conception, pas une optimisation à faire plus tard.
    pub fn iter_relations(&self) -> impl Iterator<Item = (&RelationRef, Option<&Relation>)> + '_ {
        self.catalogs
            .value
            .values()
            .flat_map(|catalogue| catalogue.namespaces.value.values())
            .flat_map(|espace| espace.relations.value.values())
            .map(|relation| (&relation.summary, relation.detail.value.as_ref()))
    }

    /// Nombre de relations connues, tous paliers confondus.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.iter_relations().count()
    }

    /// Le cache ne contient-il rien du tout ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.server.value.is_none() && self.catalogs.value.is_empty()
    }

    /// La fraîcheur d'un sous-arbre, telle que son nœud racine la porte.
    ///
    /// Rend [`Freshness::Never`] pour un nœud inconnu : ne pas connaître un nœud
    /// et ne l'avoir jamais lu sont la même chose.
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

    // ── Péremption ──────────────────────────────────────────────────────────

    /// Les sous-arbres à relire, pour une durée de vie donnée.
    ///
    /// Rend un ensemble **minimal** : aucun scope rendu n'en couvre un autre,
    /// puisque rafraîchir un parent rafraîchit ses enfants. Un rafraîchissement
    /// de fond peut donc parcourir la liste sans dédoublonner.
    ///
    /// Un nœud jamais lu n'y figure pas : il est absent, pas périmé. Voir la
    /// documentation du module.
    #[must_use]
    pub fn stale(&self, ttl: Duration) -> Vec<CatalogScope> {
        self.stale_at(Utc::now(), ttl)
    }

    /// [`Self::stale`], avec l'instant courant fourni. Rend les tests
    /// déterministes ; c'est la seule raison de son existence.
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

    /// Marque un sous-arbre à relire, **sans effacer ses données**.
    ///
    /// À appeler immédiatement après tout DDL émis depuis Oxyn (ARCHITECTURE
    /// §6). Le scope à viser est celui de l'objet touché : un `ALTER TABLE`
    /// invalide la relation ; un `CREATE TABLE` ou un `DROP TABLE` invalide
    /// l'**espace de noms**, parce que c'est son listing qui vient de devenir
    /// faux.
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

    /// Marque tout le cache à relire.
    pub fn invalidate_all(&mut self) {
        self.invalidate(&CatalogScope::Server);
    }

    /// Retire un sous-arbre du cache.
    ///
    /// Pour ce qui a réellement disparu — un `DROP` dont Oxyn est l'auteur. Le
    /// listing du parent est invalidé au passage : il vient de devenir faux, et
    /// le laisser frais ferait réapparaître l'objet au prochain
    /// rafraîchissement.
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

    // ── Navigation interne ──────────────────────────────────────────────────

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

    /// Le nœud d'une relation **déjà connue**.
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

    /// Le nœud de catalogue, créé s'il manque.
    ///
    /// Le nœud créé porte une fraîcheur [`Freshness::Never`] : il existe pour
    /// porter un enfant, il ne prétend pas avoir été listé. Son `info` reste
    /// `None` quand le palier est absent.
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

    /// Le nœud d'espace de noms, avec ses parents, créés s'ils manquent.
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

    /// Le nœud de relation, avec ses parents, créés s'ils manquent.
    ///
    /// `kind` ne sert qu'à la création : le résumé d'une relation déjà listée
    /// fait autorité sur sa nature.
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
        CatalogPath::for_relation(catalogue, espace, relation).expect("chemin valide")
    }

    fn espace_public() -> CatalogPath {
        CatalogPath::for_namespace(Some("caisse"), "public").expect("chemin valide")
    }

    fn cache_postgres() -> CatalogCache {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "PostgreSQL",
            "17.2",
            Capabilities::SQL | Capabilities::SCHEMAS,
        ));
        cache.set_catalogs(vec![CatalogRef::new("caisse").expect("valide")]);
        cache.set_namespaces(
            Some("caisse"),
            vec![
                NamespaceRef::new(
                    CatalogPath::for_catalog("caisse").expect("valide"),
                    "public",
                )
                .expect("valide"),
            ],
        );
        let espace = espace_public();
        cache
            .set_relations(
                &espace,
                vec![
                    RelationRef::new(espace.clone(), "clients", RelationKind::Table)
                        .expect("valide"),
                    RelationRef::new(espace.clone(), "commandes", RelationKind::Table)
                        .expect("valide"),
                ],
            )
            .expect("un espace de noms est bien un espace de noms");
        cache
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
    fn un_cache_neuf_est_vide_et_ne_perime_rien() {
        let cache = CatalogCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.server_info(), None);
        assert!(
            cache.stale(HEURE).is_empty(),
            "un palier jamais lu est absent, pas périmé"
        );
    }

    #[test]
    fn l_arbre_se_remplit_palier_par_palier() {
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
    fn interroger_un_noeud_inconnu_rend_du_vide_pas_une_erreur() {
        let cache = cache_postgres();
        assert_eq!(cache.namespaces(Some("inexistant")).count(), 0);
        let ailleurs = CatalogPath::for_namespace(Some("caisse"), "inexistant").expect("valide");
        assert_eq!(cache.relations(&ailleurs).count(), 0);
        assert!(
            cache
                .relation(&chemin(Some("caisse"), Some("public"), "absente"))
                .is_none()
        );
    }

    #[test]
    fn une_insertion_partielle_cree_ses_parents() {
        // Le cas de la recherche : on décrit une table sans avoir listé son
        // schéma.
        let mut cache = CatalogCache::new();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("le chemin nomme une relation");

        assert!(cache.relation(&table).is_some());
        assert_eq!(cache.relation_count(), 1);
        assert!(
            cache.stale(HEURE).is_empty(),
            "les parents créés au passage n'ont jamais été lus : ils ne sont pas périmés"
        );
    }

    #[test]
    fn une_relation_creee_au_passage_garde_sa_nature() {
        // La nature vient de la description, jamais d'un défaut : une collection
        // MongoDB affichée comme une table serait une surface d'interface fausse.
        let mut cache = CatalogCache::new();
        let collection = chemin(None, Some("boutique"), "commandes");
        cache
            .set_relation(
                &collection,
                Relation::new("commandes", RelationKind::Collection),
            )
            .expect("valide");
        assert_eq!(
            cache.relation_summary(&collection).map(|r| r.kind),
            Some(RelationKind::Collection)
        );
    }

    #[test]
    fn un_chemin_du_mauvais_palier_est_un_bug_d_appel() {
        let mut cache = CatalogCache::new();
        let err = cache
            .set_relation(
                &espace_public(),
                Relation::new("clients", RelationKind::Table),
            )
            .expect_err("un espace de noms n'est pas une relation");
        assert_eq!(err, CacheError::NotARelation);

        let table = chemin(Some("caisse"), Some("public"), "clients");
        assert_eq!(
            cache
                .set_relations(&table, Vec::new())
                .expect_err("une relation n'est pas un espace de noms"),
            CacheError::NotANamespace
        );
    }

    #[test]
    fn attacher_des_index_a_une_relation_inconnue_est_refuse() {
        // La créer ici obligerait à deviner sa nature.
        let mut cache = CatalogCache::new();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        assert_eq!(
            cache
                .set_indexes(&table, Vec::new())
                .expect_err("relation jamais vue"),
            CacheError::UnknownRelation
        );
    }

    #[test]
    fn relister_conserve_les_details_des_relations_qui_subsistent() {
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
            .expect("valide");

        let espace = espace_public();
        cache
            .set_relations(
                &espace,
                vec![
                    RelationRef::new(espace.clone(), "clients", RelationKind::Table)
                        .expect("valide"),
                ],
            )
            .expect("valide");

        assert!(
            cache.relation(&table).is_some(),
            "relister un schéma ne doit pas obliger à redécrire chaque table ouverte"
        );
        assert!(
            cache
                .relation(&chemin(Some("caisse"), Some("public"), "commandes"))
                .is_none(),
            "une relation absente du listing frais disparaît"
        );
        assert_eq!(cache.relation_count(), 1);
    }

    #[test]
    fn les_paliers_absents_ne_se_confondent_pas() {
        // MySQL n'a pas de palier catalogue. Le nœud « palier absent » ne doit
        // pas entrer en collision avec un catalogue réellement nommé.
        let mut cache = CatalogCache::new();
        let espace_mysql = CatalogPath::for_namespace(None, "caisse").expect("valide");
        cache
            .set_relations(
                &espace_mysql,
                vec![
                    RelationRef::new(espace_mysql.clone(), "clients", RelationKind::Table)
                        .expect("valide"),
                ],
            )
            .expect("valide");

        assert_eq!(cache.relations(&espace_mysql).count(), 1);
        let sous_catalogue = CatalogPath::for_namespace(Some("caisse"), "caisse").expect("valide");
        assert_eq!(cache.relations(&sous_catalogue).count(), 0);
        assert_eq!(
            cache.catalogs().count(),
            0,
            "un palier absent ne s'affiche pas comme un catalogue anonyme"
        );
    }

    #[test]
    fn un_trou_intermediaire_se_navigue() {
        // Neo4j : catalogue + relation, sans espace de noms.
        let mut cache = CatalogCache::new();
        let base = CatalogPath::for_catalog("graphe").expect("valide");
        cache
            .set_relations(
                &base,
                vec![
                    RelationRef::new(base.clone(), "Personne", RelationKind::NodeLabel)
                        .expect("valide"),
                ],
            )
            .expect("un catalogue est un parent acceptable");

        let label = chemin(Some("graphe"), None, "Personne");
        assert!(cache.relation_summary(&label).is_some());
        assert_eq!(cache.relations(&base).count(), 1);
        assert_eq!(
            cache.relation_summary(&label).map(RelationRef::path),
            Some(label)
        );
    }

    #[test]
    fn ce_qui_n_a_pas_ete_lu_se_distingue_de_ce_qui_est_vide() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        assert!(
            cache.indexes(&table).is_none(),
            "« pas lu » n'est pas « aucun index »"
        );

        cache.set_indexes(&table, Vec::new()).expect("valide");
        let index = cache.indexes(&table).expect("les index ont été lus");
        assert!(
            index.is_empty(),
            "une tranche vide dit bien « aucun index »"
        );
    }

    #[test]
    fn la_peremption_suit_la_duree_de_vie() {
        let cache = cache_postgres();
        assert!(
            cache.stale(HEURE).is_empty(),
            "les listings viennent d'être écrits"
        );
        let perimes = cache.stale_at(Utc::now() + TimeDelta::hours(2), HEURE);
        assert_eq!(
            perimes,
            vec![CatalogScope::Server],
            "le scope rendu est minimal"
        );
    }

    #[test]
    fn la_peremption_rend_un_ensemble_minimal() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("valide");

        cache.invalidate(&CatalogScope::Relation(table.clone()));
        assert_eq!(
            cache.stale(HEURE),
            vec![CatalogScope::Relation(table)],
            "seule la relation invalidée est à relire"
        );

        // Invalider l'espace de noms au-dessus absorbe la relation : rafraîchir
        // le parent rafraîchit l'enfant.
        let espace = espace_public();
        cache.invalidate(&CatalogScope::Namespace(espace.clone()));
        assert_eq!(cache.stale(HEURE), vec![CatalogScope::Namespace(espace)]);
    }

    #[test]
    fn une_invalidation_ne_perd_pas_les_donnees() {
        // L'exploration hors ligne est une fonctionnalité : vider l'arbre au
        // premier ALTER TABLE la ferait clignoter.
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("valide");

        cache.invalidate(&CatalogScope::Relation(table.clone()));
        assert!(cache.relation(&table).is_some(), "la donnée reste lisible");
        assert_eq!(
            cache.freshness(&CatalogScope::Relation(table.clone())),
            Freshness::Invalidated
        );
        assert!(cache.stale(HEURE).contains(&CatalogScope::Relation(table)));
    }

    #[test]
    fn une_invalidation_descend_dans_le_sous_arbre() {
        let mut cache = cache_postgres();
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache
            .set_relation(&table, Relation::new("clients", RelationKind::Table))
            .expect("valide");

        cache.invalidate_all();
        assert_eq!(
            cache.freshness(&CatalogScope::Relation(table)),
            Freshness::Invalidated,
            "l'invalidation du serveur atteint les feuilles"
        );
        assert_eq!(cache.stale(HEURE), vec![CatalogScope::Server]);
    }

    #[test]
    fn invalider_ce_qui_n_a_jamais_ete_lu_ne_le_fait_pas_apparaitre() {
        let mut cache = cache_postgres();
        // La relation figure au listing, mais n'a jamais été décrite.
        let table = chemin(Some("caisse"), Some("public"), "clients");
        cache.invalidate(&CatalogScope::Relation(table));
        assert!(
            cache.stale(HEURE).is_empty(),
            "invalider une description jamais demandée ne la met pas au travail"
        );
    }

    #[test]
    fn oublier_invalide_le_listing_du_parent() {
        // Après un DROP TABLE, le listing du schéma est devenu faux : le laisser
        // frais ferait réapparaître la table au prochain rafraîchissement.
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
    fn oublier_le_serveur_vide_le_cache() {
        let mut cache = cache_postgres();
        cache.forget(&CatalogScope::Server);
        assert!(cache.is_empty());
    }

    #[test]
    fn un_scope_couvre_ses_descendants() {
        let table = chemin(Some("caisse"), Some("public"), "clients");
        let espace = espace_public();
        let catalogue = CatalogPath::for_catalog("caisse").expect("valide");

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
    fn un_scope_ne_couvre_pas_un_frere() {
        let a = CatalogScope::Namespace(espace_public());
        let b = CatalogScope::Relation(chemin(Some("caisse"), Some("archives"), "clients"));
        assert!(!a.contains(&b));
    }

    #[test]
    fn le_scope_se_deduit_du_chemin() {
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
    fn une_horloge_qui_recule_ne_perime_rien() {
        let cache = cache_postgres();
        assert!(
            cache
                .stale_at(Utc::now() - TimeDelta::hours(48), HEURE)
                .is_empty(),
            "un instant antérieur à la lecture ne rend pas le nœud périmé"
        );
    }

    #[test]
    fn le_rendu_d_un_scope_nomme_le_palier() {
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
