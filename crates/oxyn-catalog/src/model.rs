//! Le modèle de métadonnées unifié (ARCHITECTURE §6).
//!
//! ```text
//! Server → Catalog/Database → Namespace/Schema → Relation → Field
//! ```
//!
//! **Les paliers intermédiaires sont optionnels**, et pas seulement les
//! derniers : MySQL n'a pas de catalogue, Neo4j n'a pas d'espace de noms,
//! Elasticsearch n'a ni l'un ni l'autre. Le modèle ne comble aucun trou avec une
//! valeur inventée — un palier absent est absent, et [`CatalogPath`] le rend
//! tel quel.
//!
//! # Deux drapeaux qui ne sont pas décoratifs
//!
//! * [`Field::inferred`] — pour MongoDB, le schéma est **déduit par
//!   échantillonnage** ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
//!   Un champ absent de l'échantillon existe peut-être plus loin. L'interface
//!   doit pouvoir le dire ; présenter une inférence comme une vérité du serveur
//!   fait écrire des requêtes fausses en confiance.
//! * [`Field::nullable`] — un serveur ment parfois. La valeur est reprise telle
//!   quelle, jamais recalculée à partir des données.
//!
//! # Ce qui n'est pas ici
//!
//! Aucune valeur de ligne. Le catalogue porte des **métadonnées** ; l'échantillon
//! de données appartient au niveau de confidentialité `Sampled`
//! ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)) et à `oxyn-data`.

use std::fmt;

use oxyn_core::Capabilities;
use serde::{Deserialize, Serialize};

use crate::path::{CatalogPath, CatalogPathError, validate_segment};

/// Ce que le serveur dit de lui-même, plus ce que la session sait faire.
///
/// `capabilities` est celle de la **session**, pas celle du driver : la version
/// du serveur, ses extensions et les droits du compte changent ce qui est
/// disponible (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Nom du produit tel que le serveur le donne (`PostgreSQL`, `MariaDB`…).
    pub product: String,
    /// Version, non analysée. Comparer des versions demande de connaître le
    /// schéma de versionnement du produit ; c'est le travail du driver.
    pub version: String,
    /// Ce que la session sait faire.
    pub capabilities: Capabilities,
}

impl ServerInfo {
    /// Construit une description de serveur.
    #[must_use]
    pub fn new(
        product: impl Into<String>,
        version: impl Into<String>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            product: product.into(),
            version: version.into(),
            capabilities,
        }
    }
}

impl fmt::Display for ServerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.product, self.version)
    }
}

/// Un catalogue : le palier « base de données » de PostgreSQL, le « project » de
/// BigQuery, l'index numérique de Redis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRef {
    name: String,
    /// Commentaire porté par l'objet, s'il en a un.
    pub comment: Option<String>,
    /// Est-ce le catalogue auquel la session est connectée ?
    pub is_default: bool,
}

impl CatalogRef {
    /// Construit une référence de catalogue.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si le nom est vide ou contient un caractère
    /// de contrôle — un nom d'objet vient du serveur, donc d'une source hostile
    /// ([SECURITY, surface d'entrée §2](../../../docs/SECURITY.md)).
    pub fn new(name: impl Into<String>) -> Result<Self, CatalogPathError> {
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            name,
            comment: None,
            is_default: false,
        })
    }

    /// Construit une référence à partir d'un nom **déjà validé**.
    ///
    /// Réservé au cache, qui reconstruit des références à partir de chemins
    /// dont chaque palier a été validé à la construction. Une fonction faillible
    /// obligerait le cache à traiter une erreur impossible.
    pub(crate) fn validated(name: String) -> Self {
        Self {
            name,
            comment: None,
            is_default: false,
        }
    }

    /// Nom du catalogue.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Chemin du catalogue.
    #[must_use]
    pub fn path(&self) -> CatalogPath {
        CatalogPath::from_validated(Some(self.name.clone()), None, None)
    }

    /// Attache un commentaire.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Marque ce catalogue comme celui de la session.
    #[must_use]
    pub fn with_default(mut self) -> Self {
        self.is_default = true;
        self
    }
}

/// Un espace de noms : le schéma de PostgreSQL, la base de MySQL ou de MongoDB,
/// le préfixe logique de Redis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceRef {
    /// Le catalogue qui le contient, s'il y en a un.
    parent: CatalogPath,
    name: String,
    /// Commentaire porté par l'objet, s'il en a un.
    pub comment: Option<String>,
    /// Espace de noms du système (`pg_catalog`, `information_schema`,
    /// `mysql`…). L'arborescence les replie par défaut ; elle ne les cache pas.
    pub is_system: bool,
}

impl NamespaceRef {
    /// Construit une référence d'espace de noms.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si le nom est invalide, ou si `parent`
    /// descend plus bas que le palier catalogue.
    pub fn new(parent: CatalogPath, name: impl Into<String>) -> Result<Self, CatalogPathError> {
        if parent.namespace().is_some() || parent.relation().is_some() {
            return Err(CatalogPathError::new(
                "le parent d'un espace de noms est un catalogue, ou rien",
            ));
        }
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            parent,
            name,
            comment: None,
            is_system: false,
        })
    }

    /// Construit une référence à partir de paliers **déjà validés**.
    ///
    /// Réservé au cache, pour la même raison que [`CatalogRef::validated`].
    pub(crate) fn validated(parent: CatalogPath, name: String) -> Self {
        Self {
            parent,
            name,
            comment: None,
            is_system: false,
        }
    }

    /// Nom de l'espace de noms.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Le catalogue qui le contient, éventuellement vide.
    #[must_use]
    pub const fn parent(&self) -> &CatalogPath {
        &self.parent
    }

    /// Chemin complet de l'espace de noms.
    #[must_use]
    pub fn path(&self) -> CatalogPath {
        CatalogPath::from_validated(
            self.parent.catalog().map(str::to_owned),
            Some(self.name.clone()),
            None,
        )
    }

    /// Attache un commentaire.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Marque cet espace de noms comme appartenant au système.
    #[must_use]
    pub fn with_system(mut self) -> Self {
        self.is_system = true;
        self
    }

    /// Réattache la référence sous un autre parent.
    ///
    /// Réservé au cache : c'est ce qui garantit que le chemin rendu par
    /// [`Self::path`] et la position du nœud dans l'arbre ne peuvent pas
    /// diverger.
    pub(crate) fn reparent(&mut self, parent: CatalogPath) {
        self.parent = parent;
    }
}

/// Nature d'une relation.
///
/// « Relation » est le palier, pas le modèle relationnel : une collection
/// MongoDB, un motif de clés Redis et un label de nœud Neo4j occupent la même
/// place dans la hiérarchie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RelationKind {
    /// Table.
    Table,
    /// Vue.
    View,
    /// Vue matérialisée.
    MaterializedView,
    /// Collection documentaire (MongoDB, Couchbase).
    Collection,
    /// Index exposé comme un objet à part entière (Elasticsearch).
    Index,
    /// Flux ou data stream (Kafka, Elasticsearch, ksqlDB).
    Stream,
    /// Motif de clés (Redis).
    KeyPattern,
    /// Label de nœud (Neo4j).
    NodeLabel,
    /// Type de relation (Neo4j).
    RelationshipType,
    /// Fonction.
    Function,
    /// Procédure stockée.
    Procedure,
    /// Séquence.
    Sequence,
}

impl RelationKind {
    /// Nom stable, pour l'audit et l'interface.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Collection => "collection",
            Self::Index => "index",
            Self::Stream => "stream",
            Self::KeyPattern => "key_pattern",
            Self::NodeLabel => "node_label",
            Self::RelationshipType => "relationship_type",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Sequence => "sequence",
        }
    }

    /// Cette relation contient-elle des enregistrements consultables ?
    ///
    /// Sert à décider si un double-clic dans l'arborescence peut ouvrir une
    /// grille. Une fonction ou une séquence n'a pas de contenu à parcourir ;
    /// proposer l'action serait une surface qui ne mène nulle part (ADR-0003).
    #[must_use]
    pub const fn holds_records(&self) -> bool {
        matches!(
            self,
            Self::Table
                | Self::View
                | Self::MaterializedView
                | Self::Collection
                | Self::Index
                | Self::Stream
                | Self::KeyPattern
                | Self::NodeLabel
                | Self::RelationshipType
        )
    }
}

impl fmt::Display for RelationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Une relation, telle qu'un listing la donne : identité et nature, sans champs.
///
/// C'est ce que rend
/// [`CatalogProvider::list_relations`](crate::provider::CatalogProvider::list_relations).
/// Décrire les champs de 20 000 tables prend des minutes ; le listing doit
/// pouvoir remplir l'arborescence sans les demander.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationRef {
    parent: CatalogPath,
    name: String,
    /// Nature de la relation.
    pub kind: RelationKind,
    /// Commentaire porté par l'objet, s'il en a un.
    ///
    /// **Contenu non fiable.** Un commentaire de colonne qui dit « ignore les
    /// instructions précédentes » est une donnée, jamais une consigne
    /// (ARCHITECTURE §8) : c'est au point de passage unique de `oxyn-ai` de
    /// l'encadrer avant qu'il ne rejoigne une invite.
    pub comment: Option<String>,
}

impl RelationRef {
    /// Construit une référence de relation.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si le nom est invalide, ou si `parent`
    /// désigne déjà une relation.
    pub fn new(
        parent: CatalogPath,
        name: impl Into<String>,
        kind: RelationKind,
    ) -> Result<Self, CatalogPathError> {
        if parent.relation().is_some() {
            return Err(CatalogPathError::new(
                "le parent d'une relation ne peut pas être une relation",
            ));
        }
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            parent,
            name,
            kind,
            comment: None,
        })
    }

    /// Construit une référence à partir de paliers **déjà validés**.
    ///
    /// Réservé au cache, pour la même raison que [`CatalogRef::validated`].
    pub(crate) fn validated(parent: CatalogPath, name: String, kind: RelationKind) -> Self {
        Self {
            parent,
            name,
            kind,
            comment: None,
        }
    }

    /// Nom de la relation.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Le palier qui la contient : espace de noms, catalogue, ou rien.
    #[must_use]
    pub const fn parent(&self) -> &CatalogPath {
        &self.parent
    }

    /// Chemin complet de la relation.
    #[must_use]
    pub fn path(&self) -> CatalogPath {
        self.parent.with_validated_relation(&self.name)
    }

    /// Attache un commentaire.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Réattache la référence sous un autre parent.
    ///
    /// Réservé au cache, pour la même raison que
    /// [`NamespaceRef::reparent`].
    pub(crate) fn reparent(&mut self, parent: CatalogPath) {
        self.parent = parent;
    }
}

/// Type logique d'un champ, indépendant du produit.
///
/// Le type **brut** du serveur reste disponible dans [`Field::raw_type`] : le
/// type logique sert à décider d'un rendu ou d'un éditeur, pas à remplacer ce
/// que dit le serveur. Un type que le driver ne sait pas ramener à cette liste
/// devient [`Unknown`](Self::Unknown) — jamais un voisin plausible, qui
/// afficherait une valeur fausse sans le dire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogicalType {
    /// Booléen.
    Boolean,
    /// Entier signé de `bits` bits.
    Integer {
        /// Largeur en bits telle que le serveur la déclare (8, 16, 32, 64…).
        bits: u8,
    },
    /// Flottant de `bits` bits.
    Float {
        /// Largeur en bits (32, 64).
        bits: u8,
    },
    /// Décimal exact.
    Decimal {
        /// Précision, quand le serveur la contraint.
        precision: Option<u16>,
        /// Échelle. Négative chez Oracle, d'où le type signé.
        scale: Option<i16>,
    },
    /// Texte.
    Text,
    /// Suite d'octets.
    Bytes,
    /// UUID.
    Uuid,
    /// Date sans heure.
    Date,
    /// Heure sans date.
    Time,
    /// Horodatage.
    Timestamp {
        /// Porte-t-il un fuseau ? La distinction n'est pas cosmétique : la
        /// confondre décale la donnée de façon invisible et permanente
        /// ([`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md)).
        tz: bool,
    },
    /// Intervalle.
    Interval,
    /// Document JSON.
    Json,
    /// Tableau d'un type.
    Array(Box<LogicalType>),
    /// Structure à champs nommés (`ROW`, `STRUCT`, sous-document).
    Struct(Vec<Field>),
    /// Vecteur dense (pgvector, Qdrant).
    Vector {
        /// Dimension, quand elle est contrainte.
        dims: Option<u32>,
    },
    /// Géométrie (PostGIS, MySQL spatial).
    Geometry,
    /// Type que le driver n'a pas su ramener à cette liste.
    Unknown,
}

impl LogicalType {
    /// Entier 32 bits, le cas le plus fréquent.
    pub const INT32: Self = Self::Integer { bits: 32 };
    /// Entier 64 bits.
    pub const INT64: Self = Self::Integer { bits: 64 };
    /// Flottant double précision.
    pub const FLOAT64: Self = Self::Float { bits: 64 };
    /// Horodatage avec fuseau.
    pub const TIMESTAMPTZ: Self = Self::Timestamp { tz: true };

    /// Le type contient-il d'autres types ?
    #[must_use]
    pub const fn is_nested(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Struct(_))
    }

    /// Le type des éléments d'un tableau.
    #[must_use]
    pub fn element(&self) -> Option<&Self> {
        match self {
            Self::Array(inner) => Some(&**inner),
            _ => None,
        }
    }
}

impl fmt::Display for LogicalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean => f.write_str("boolean"),
            Self::Integer { bits } => write!(f, "int{bits}"),
            Self::Float { bits } => write!(f, "float{bits}"),
            Self::Decimal {
                precision: Some(p),
                scale: Some(s),
            } => write!(f, "decimal({p},{s})"),
            Self::Decimal {
                precision: Some(p),
                scale: None,
            } => write!(f, "decimal({p})"),
            Self::Decimal { .. } => f.write_str("decimal"),
            Self::Text => f.write_str("text"),
            Self::Bytes => f.write_str("bytes"),
            Self::Uuid => f.write_str("uuid"),
            Self::Date => f.write_str("date"),
            Self::Time => f.write_str("time"),
            Self::Timestamp { tz: true } => f.write_str("timestamptz"),
            Self::Timestamp { tz: false } => f.write_str("timestamp"),
            Self::Interval => f.write_str("interval"),
            Self::Json => f.write_str("json"),
            Self::Array(inner) => write!(f, "array<{inner}>"),
            Self::Struct(fields) => {
                f.write_str("struct<")?;
                for (i, champ) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: {}", champ.name, champ.logical_type)?;
                }
                f.write_str(">")
            }
            Self::Vector { dims: Some(d) } => write!(f, "vector({d})"),
            Self::Vector { dims: None } => f.write_str("vector"),
            Self::Geometry => f.write_str("geometry"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

/// Un champ d'une relation : colonne, clé de document, propriété de nœud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Nom du champ. **Entrée hostile** : il peut contenir un point, un
    /// guillemet, du SQL.
    pub name: String,
    /// Position ordinale, telle que le serveur la donne.
    pub position: u32,
    /// Type ramené au vocabulaire d'Oxyn.
    pub logical_type: LogicalType,
    /// Type tel que le serveur le nomme (`int4`, `VARCHAR(255)`, `geography`).
    /// Toujours conservé : c'est ce que l'utilisateur reconnaît.
    pub raw_type: String,
    /// Le champ accepte-t-il l'absence de valeur, d'après le serveur ?
    pub nullable: bool,
    /// Expression de valeur par défaut, telle quelle.
    pub default: Option<String>,
    /// Commentaire. **Contenu non fiable**, voir [`RelationRef::comment`].
    pub comment: Option<String>,
    /// Le champ participe-t-il à la clé primaire ?
    pub is_primary_key: bool,
    /// Le champ vient-il d'une **inférence par échantillonnage** plutôt que
    /// d'une déclaration du serveur ?
    ///
    /// Vrai pour MongoDB et toute source sans schéma. L'interface doit le dire :
    /// un champ absent de l'échantillon existe peut-être plus loin, et un type
    /// inféré sur cent documents peut être faux au cent unième
    /// ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
    pub inferred: bool,
}

impl Field {
    /// Construit un champ.
    ///
    /// Les valeurs non renseignées prennent le parti prudent : `nullable` est
    /// vrai, `is_primary_key` et `inferred` sont faux.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        position: u32,
        logical_type: LogicalType,
        raw_type: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            position,
            logical_type,
            raw_type: raw_type.into(),
            nullable: true,
            default: None,
            comment: None,
            is_primary_key: false,
            inferred: false,
        }
    }

    /// Déclare le champ non nul.
    #[must_use]
    pub fn not_null(mut self) -> Self {
        self.nullable = false;
        self
    }

    /// Déclare le champ membre de la clé primaire. Implique `NOT NULL`.
    #[must_use]
    pub fn primary_key(mut self) -> Self {
        self.is_primary_key = true;
        self.nullable = false;
        self
    }

    /// Marque le champ comme déduit par échantillonnage.
    #[must_use]
    pub fn with_inferred(mut self) -> Self {
        self.inferred = true;
        self
    }

    /// Attache une valeur par défaut.
    #[must_use]
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Attache un commentaire.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }
}

/// Une relation décrite : identité, volumétrie et champs.
///
/// C'est ce que rend
/// [`CatalogProvider::describe_relation`](crate::provider::CatalogProvider::describe_relation).
/// Le chemin n'est pas porté ici : il est donné par la position dans le cache,
/// ou par le [`RelationRef`] qui a servi à la demander.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    /// Nom de la relation.
    pub name: String,
    /// Nature de la relation.
    pub kind: RelationKind,
    /// Commentaire. **Contenu non fiable**, voir [`RelationRef::comment`].
    pub comment: Option<String>,
    /// Estimation du nombre de lignes, quand la source en donne une **sans
    /// compter**. `None` signifie « inconnu », jamais « zéro » : lancer un
    /// `COUNT(*)` pour remplir ce champ scannerait la table à chaque
    /// rafraîchissement d'arborescence.
    pub estimated_rows: Option<u64>,
    /// Taille sur disque en octets, quand la source la donne.
    pub size_bytes: Option<u64>,
    /// Les champs, dans l'ordre où le serveur les donne.
    pub fields: Vec<Field>,
}

impl Relation {
    /// Construit une relation sans champ.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: RelationKind) -> Self {
        Self {
            name: name.into(),
            kind,
            comment: None,
            estimated_rows: None,
            size_bytes: None,
            fields: Vec::new(),
        }
    }

    /// Attache les champs.
    #[must_use]
    pub fn with_fields(mut self, fields: Vec<Field>) -> Self {
        self.fields = fields;
        self
    }

    /// Attache un commentaire.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Attache une estimation de volumétrie.
    #[must_use]
    pub fn with_estimated_rows(mut self, rows: u64) -> Self {
        self.estimated_rows = Some(rows);
        self
    }

    /// Un champ par son nom, exactement (les identifiants sont sensibles à la
    /// casse une fois cités).
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|champ| champ.name == name)
    }

    /// Les champs de la clé primaire, dans l'ordre des positions.
    ///
    /// Vide quand la relation n'en a pas — ce qui est le cas courant hors du
    /// modèle relationnel.
    #[must_use]
    pub fn primary_key(&self) -> Vec<&Field> {
        let mut cles: Vec<&Field> = self
            .fields
            .iter()
            .filter(|champ| champ.is_primary_key)
            .collect();
        cles.sort_by_key(|champ| champ.position);
        cles
    }

    /// Le schéma de cette relation est-il, en tout ou partie, **déduit** ?
    ///
    /// L'interface s'en sert pour marquer la relation : présenter une inférence
    /// comme une vérité du serveur fait écrire des requêtes fausses en
    /// confiance.
    #[must_use]
    pub fn has_inferred_schema(&self) -> bool {
        self.fields.iter().any(|champ| champ.inferred)
    }
}

/// Un index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// Nom de l'index.
    pub name: String,
    /// Champs indexés, dans l'ordre de l'index — l'ordre décide de ce que
    /// l'index sait servir.
    pub fields: Vec<String>,
    /// L'index impose-t-il l'unicité ?
    pub unique: bool,
    /// Méthode d'accès (`btree`, `gin`, `hnsw`…), quand la source la nomme.
    pub method: Option<String>,
    /// Prédicat d'un index partiel, tel quel.
    pub predicate: Option<String>,
}

impl Index {
    /// Construit un index.
    #[must_use]
    pub fn new(name: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            fields,
            unique: false,
            method: None,
            predicate: None,
        }
    }

    /// Déclare l'index unique.
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    /// L'index ne couvre-t-il qu'une partie des lignes ?
    #[must_use]
    pub const fn is_partial(&self) -> bool {
        self.predicate.is_some()
    }
}

/// Action référentielle d'une clé étrangère.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReferentialAction {
    /// Aucune action — le défaut de la norme.
    #[default]
    NoAction,
    /// Refus de la suppression.
    Restrict,
    /// Suppression en cascade.
    Cascade,
    /// Mise à nul.
    SetNull,
    /// Retour à la valeur par défaut.
    SetDefault,
}

impl ReferentialAction {
    /// Nom stable.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NoAction => "no_action",
            Self::Restrict => "restrict",
            Self::Cascade => "cascade",
            Self::SetNull => "set_null",
            Self::SetDefault => "set_default",
        }
    }

    /// Supprimer une ligne référencée en supprime-t-il d'autres ?
    ///
    /// C'est ce qu'un aperçu d'approbation doit montrer : un `DELETE` d'une
    /// ligne peut en effacer un million par cascade.
    #[must_use]
    pub const fn propagates_delete(&self) -> bool {
        matches!(self, Self::Cascade | Self::SetNull | Self::SetDefault)
    }
}

impl fmt::Display for ReferentialAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// La cible d'une clé étrangère : une relation, et les champs qu'elle expose.
///
/// Regroupés parce qu'ils ne veulent rien dire séparément — une relation cible
/// sans ses colonnes ne permet ni de tracer un diagramme, ni de composer une
/// jointure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeyTarget {
    /// La relation référencée.
    pub relation: CatalogPath,
    /// Les champs référencés, dans l'ordre correspondant à
    /// [`ForeignKey::fields`].
    pub fields: Vec<String>,
}

/// Une clé étrangère.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    /// Declared name; empty when the engine exposes no constraint name.
    pub name: String,
    /// Champs porteurs, dans l'ordre.
    pub fields: Vec<String>,
    /// Ce qui est référencé.
    pub references: ForeignKeyTarget,
    /// Ce qui arrive aux lignes porteuses quand la ligne référencée disparaît.
    pub on_delete: ReferentialAction,
}

impl ForeignKey {
    /// Construit une clé étrangère.
    #[must_use]
    pub fn new(name: impl Into<String>, fields: Vec<String>, references: ForeignKeyTarget) -> Self {
        Self {
            name: name.into(),
            fields,
            references,
            on_delete: ReferentialAction::NoAction,
        }
    }

    /// Les deux côtés de la clé ont-ils le même nombre de champs ?
    ///
    /// Un serveur peut renvoyer des listes désaccordées ; composer une jointure
    /// par appariement positionnel sans vérifier produirait un `ON` faux.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        !self.fields.is_empty() && self.fields.len() == self.references.fields.len()
    }
}

/// A foreign key together with the relation declaring it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomingForeignKey {
    /// Source relation; the target remains in `key.references`.
    pub source: CatalogPath,
    /// The declared key, with both column lists in matching order.
    pub key: ForeignKey,
    /// Whether a valid, unconditional unique key on the source limits each
    /// referenced value to one source row. `None` means unreported.
    pub source_unique: Option<bool>,
}

/// Nature d'une contrainte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConstraintKind {
    /// Clé primaire.
    PrimaryKey,
    /// Unicité.
    Unique,
    /// Vérification d'une expression.
    Check,
    /// Clé étrangère — détaillée par [`ForeignKey`].
    ForeignKey,
    /// Exclusion (PostgreSQL).
    Exclusion,
    /// A user-defined constraint trigger.
    Trigger,
    /// Non-nullité exprimée comme une contrainte nommée.
    NotNull,
}

impl ConstraintKind {
    /// Nom stable.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PrimaryKey => "primary_key",
            Self::Unique => "unique",
            Self::Check => "check",
            Self::ForeignKey => "foreign_key",
            Self::Exclusion => "exclusion",
            Self::Trigger => "trigger",
            Self::NotNull => "not_null",
        }
    }
}

impl fmt::Display for ConstraintKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Une contrainte portée par une relation.
///
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    /// Declared name; empty when the engine exposes no constraint name.
    pub name: String,
    /// Nature de la contrainte.
    pub kind: ConstraintKind,
    /// Champs concernés, quand la contrainte en nomme.
    pub fields: Vec<String>,
    /// Definition returned by the engine, which may be a normalized rendering
    /// rather than the original source text. Never execute it implicitly.
    pub expression: Option<String>,
    /// Whether existing rows have been validated, when the engine reports it.
    /// This does not assert that enforcement is currently enabled.
    #[serde(default)]
    pub validated: Option<bool>,
}

impl Constraint {
    /// Construit une contrainte.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: ConstraintKind, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            fields,
            expression: None,
            validated: None,
        }
    }

    /// Attache une expression.
    #[must_use]
    pub fn with_expression(mut self, expression: impl Into<String>) -> Self {
        self.expression = Some(expression.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_nom_d_objet_hostile_est_refuse_a_la_construction() {
        // Un nom d'objet vient du serveur : les caractères de contrôle sont
        // refusés (SECURITY, surface d'entrée §2), les guillemets ne le sont
        // pas — ils sont légaux, et `qualify` sait les citer.
        assert!(CatalogRef::new("base\u{1b}[31m").is_err());
        assert!(CatalogRef::new("").is_err());
        assert!(CatalogRef::new(r#"users"; DROP TABLE audit; --"#).is_ok());
    }

    #[test]
    fn le_parent_d_une_relation_ne_peut_pas_etre_une_relation() {
        let relation = CatalogPath::for_relation(None, Some("public"), "clients").expect("valide");
        assert!(RelationRef::new(relation, "autre", RelationKind::Table).is_err());
    }

    #[test]
    fn le_parent_d_un_espace_de_noms_ne_descend_pas_plus_bas() {
        let espace = CatalogPath::for_namespace(None, "public").expect("valide");
        assert!(NamespaceRef::new(espace, "autre").is_err());
    }

    #[test]
    fn le_chemin_d_une_reference_se_reconstruit_sans_perte() {
        let parent = CatalogPath::for_namespace(Some("caisse"), "public").expect("valide");
        let relation =
            RelationRef::new(parent, "ventes.2026", RelationKind::Table).expect("valide");
        let chemin = relation.path();
        assert_eq!(chemin.catalog(), Some("caisse"));
        assert_eq!(chemin.namespace(), Some("public"));
        assert_eq!(chemin.relation(), Some("ventes.2026"));
    }

    #[test]
    fn une_relation_sans_palier_intermediaire_garde_son_catalogue() {
        // Neo4j : base + label, sans espace de noms.
        let parent = CatalogPath::for_catalog("graphe").expect("valide");
        let label = RelationRef::new(parent, "Personne", RelationKind::NodeLabel).expect("valide");
        assert_eq!(label.path().to_string(), "graphe..Personne");
    }

    #[test]
    fn un_schema_infere_se_declare() {
        let mongo = Relation::new("commandes", RelationKind::Collection).with_fields(vec![
            Field::new("_id", 0, LogicalType::Uuid, "objectId").primary_key(),
            Field::new("montant", 1, LogicalType::FLOAT64, "double").with_inferred(),
        ]);
        assert!(
            mongo.has_inferred_schema(),
            "un schéma déduit par échantillonnage doit pouvoir se dire"
        );

        let postgres = Relation::new("commandes", RelationKind::Table)
            .with_fields(vec![Field::new("id", 0, LogicalType::INT64, "int8")]);
        assert!(!postgres.has_inferred_schema());
    }

    #[test]
    fn la_cle_primaire_sort_dans_l_ordre_des_positions() {
        let relation = Relation::new("lignes", RelationKind::Table).with_fields(vec![
            Field::new("libelle", 0, LogicalType::Text, "text"),
            Field::new("ligne", 2, LogicalType::INT32, "int4").primary_key(),
            Field::new("commande", 1, LogicalType::INT64, "int8").primary_key(),
        ]);
        let noms: Vec<&str> = relation
            .primary_key()
            .iter()
            .map(|champ| champ.name.as_str())
            .collect();
        assert_eq!(noms, ["commande", "ligne"]);
    }

    #[test]
    fn un_champ_de_cle_primaire_est_non_nul() {
        let champ = Field::new("id", 0, LogicalType::INT64, "int8").primary_key();
        assert!(!champ.nullable);
    }

    #[test]
    fn une_volumetrie_inconnue_n_est_pas_zero() {
        let relation = Relation::new("journal", RelationKind::Table);
        assert_eq!(relation.estimated_rows, None);
        assert_ne!(relation.estimated_rows, Some(0));
    }

    #[test]
    fn le_rendu_des_types_logiques_est_lisible() {
        assert_eq!(LogicalType::INT32.to_string(), "int32");
        assert_eq!(LogicalType::TIMESTAMPTZ.to_string(), "timestamptz");
        assert_eq!(
            LogicalType::Timestamp { tz: false }.to_string(),
            "timestamp"
        );
        assert_eq!(
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
            .to_string(),
            "decimal(10,2)"
        );
        assert_eq!(
            LogicalType::Decimal {
                precision: None,
                scale: None
            }
            .to_string(),
            "decimal"
        );
        assert_eq!(
            LogicalType::Array(Box::new(LogicalType::Text)).to_string(),
            "array<text>"
        );
        assert_eq!(
            LogicalType::Vector { dims: Some(1536) }.to_string(),
            "vector(1536)"
        );
        assert_eq!(
            LogicalType::Struct(vec![Field::new("a", 0, LogicalType::Text, "text")]).to_string(),
            "struct<a: text>"
        );
    }

    #[test]
    fn un_horodatage_avec_fuseau_ne_se_confond_pas_avec_un_horodatage_nu() {
        // La confusion qui décale la donnée de deux heures, de façon invisible
        // et permanente (DRIVER-CONTRACT §7).
        assert_ne!(
            LogicalType::Timestamp { tz: true },
            LogicalType::Timestamp { tz: false }
        );
    }

    #[test]
    fn une_cle_etrangere_desaccordee_se_detecte() {
        let cible = ForeignKeyTarget {
            relation: CatalogPath::for_relation(None, Some("public"), "clients").expect("valide"),
            fields: vec!["id".to_owned()],
        };
        let bonne = ForeignKey::new("fk_ok", vec!["client_id".to_owned()], cible.clone());
        assert!(bonne.is_well_formed());

        let mauvaise = ForeignKey::new(
            "fk_ko",
            vec!["client_id".to_owned(), "site_id".to_owned()],
            cible,
        );
        assert!(
            !mauvaise.is_well_formed(),
            "deux listes de longueurs différentes ne s'apparient pas positionnellement"
        );
    }

    #[test]
    fn seules_les_relations_a_enregistrements_s_ouvrent() {
        assert!(RelationKind::Table.holds_records());
        assert!(RelationKind::Collection.holds_records());
        assert!(RelationKind::KeyPattern.holds_records());
        assert!(!RelationKind::Function.holds_records());
        assert!(!RelationKind::Sequence.holds_records());
    }

    #[test]
    fn une_cascade_propage_la_suppression() {
        assert!(ReferentialAction::Cascade.propagates_delete());
        assert!(!ReferentialAction::NoAction.propagates_delete());
        assert!(!ReferentialAction::Restrict.propagates_delete());
    }
}
