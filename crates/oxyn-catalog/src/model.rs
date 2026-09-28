//! The unified metadata model (ARCHITECTURE §6).
//!
//! ```text
//! Server → Catalog/Database → Namespace/Schema → Relation → Field
//! ```
//!
//! **Intermediate levels are optional**, and not only the last ones: MySQL
//! has no catalog, Neo4j has no namespace, Elasticsearch has neither. The
//! model fills no hole with an invented value — a missing level is missing,
//! and [`CatalogPath`] returns it as is.
//!
//! # Two flags that are not decorative
//!
//! * [`Field::inferred`] — for MongoDB, the schema is **deduced by sampling**
//!   ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)). A field
//!   missing from the sample may exist further on. The interface must be able
//!   to say so; presenting an inference as a server truth makes people write
//!   wrong queries with confidence.
//! * [`Field::nullable`] — a server sometimes lies. The value is taken as is,
//!   never recomputed from the data.
//!
//! # What is not here
//!
//! No row value. The catalog carries **metadata**; the data sample belongs to
//! the `Sampled` privacy tier
//! ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)) and to `oxyn-data`.

use std::fmt;

use oxyn_core::Capabilities;
use serde::{Deserialize, Serialize};

use crate::path::{CatalogPath, CatalogPathError, validate_segment};

/// What the server says about itself, plus what the session can do.
///
/// `capabilities` is the **session**'s, not the driver's: the server version,
/// its extensions and the account's rights change what is available
/// (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Product name as the server gives it (`PostgreSQL`, `MariaDB`…).
    pub product: String,
    /// Version, unparsed. Comparing versions requires knowing the product's
    /// versioning scheme; that is the driver's job.
    pub version: String,
    /// What the session can do.
    pub capabilities: Capabilities,
}

impl ServerInfo {
    /// Builds a server description.
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

/// A catalog: PostgreSQL's "database" level, BigQuery's "project", Redis's
/// numeric index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRef {
    name: String,
    /// Comment carried by the object, if it has one.
    pub comment: Option<String>,
    /// Is it the catalog the session is connected to?
    pub is_default: bool,
}

impl CatalogRef {
    /// Builds a catalog reference.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if the name is empty or contains a control
    /// character — an object name comes from the server, hence from a hostile
    /// source ([SECURITY, input surface §2](../../../docs/SECURITY.md)).
    pub fn new(name: impl Into<String>) -> Result<Self, CatalogPathError> {
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            name,
            comment: None,
            is_default: false,
        })
    }

    /// Builds a reference from an **already validated** name.
    ///
    /// Reserved to the cache, which rebuilds references from paths whose every
    /// level was validated at construction. A fallible function would force
    /// the cache to handle an impossible error.
    pub(crate) fn validated(name: String) -> Self {
        Self {
            name,
            comment: None,
            is_default: false,
        }
    }

    /// Name of the catalog.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Path of the catalog.
    #[must_use]
    pub fn path(&self) -> CatalogPath {
        CatalogPath::from_validated(Some(self.name.clone()), None, None)
    }

    /// Attaches a comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Marks this catalog as the session's.
    #[must_use]
    pub fn with_default(mut self) -> Self {
        self.is_default = true;
        self
    }
}

/// A namespace: PostgreSQL's schema, MySQL's or MongoDB's database, Redis's
/// logical prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceRef {
    /// The catalog that contains it, if there is one.
    parent: CatalogPath,
    name: String,
    /// Comment carried by the object, if it has one.
    pub comment: Option<String>,
    /// System namespace (`pg_catalog`, `information_schema`, `mysql`…). The
    /// tree folds them by default; it does not hide them.
    pub is_system: bool,
}

impl NamespaceRef {
    /// Builds a namespace reference.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if the name is invalid, or if `parent`
    /// goes lower than the catalog level.
    pub fn new(parent: CatalogPath, name: impl Into<String>) -> Result<Self, CatalogPathError> {
        if parent.namespace().is_some() || parent.relation().is_some() {
            return Err(CatalogPathError::new(
                "the parent of a namespace is a catalog, or nothing",
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

    /// Builds a reference from **already validated** levels.
    ///
    /// Reserved to the cache, for the same reason as [`CatalogRef::validated`].
    pub(crate) fn validated(parent: CatalogPath, name: String) -> Self {
        Self {
            parent,
            name,
            comment: None,
            is_system: false,
        }
    }

    /// Name of the namespace.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The catalog that contains it, possibly empty.
    #[must_use]
    pub const fn parent(&self) -> &CatalogPath {
        &self.parent
    }

    /// Full path of the namespace.
    #[must_use]
    pub fn path(&self) -> CatalogPath {
        CatalogPath::from_validated(
            self.parent.catalog().map(str::to_owned),
            Some(self.name.clone()),
            None,
        )
    }

    /// Attaches a comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Marks this namespace as belonging to the system.
    #[must_use]
    pub fn with_system(mut self) -> Self {
        self.is_system = true;
        self
    }

    /// Reattaches the reference under another parent.
    ///
    /// Reserved to the cache: it is what guarantees that the path returned by
    /// [`Self::path`] and the node's position in the tree cannot diverge.
    pub(crate) fn reparent(&mut self, parent: CatalogPath) {
        self.parent = parent;
    }
}

/// Kind of a relation.
///
/// "Relation" is the level, not the relational model: a MongoDB collection, a
/// Redis key pattern and a Neo4j node label take the same place in the
/// hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RelationKind {
    /// Table.
    Table,
    /// View.
    View,
    /// Materialized view.
    MaterializedView,
    /// Document collection (MongoDB, Couchbase).
    Collection,
    /// Index exposed as a full object (Elasticsearch).
    Index,
    /// Stream or data stream (Kafka, Elasticsearch, ksqlDB).
    Stream,
    /// Key pattern (Redis).
    KeyPattern,
    /// Node label (Neo4j).
    NodeLabel,
    /// Relationship type (Neo4j).
    RelationshipType,
    /// Function.
    Function,
    /// Stored procedure.
    Procedure,
    /// Sequence.
    Sequence,
}

impl RelationKind {
    /// Stable name, for the audit and the interface.
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

    /// Does this relation contain browsable records?
    ///
    /// Serves to decide whether a double-click in the tree can open a grid. A
    /// function or a sequence has no content to browse; offering the action
    /// would be a surface that leads nowhere (ADR-0003).
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

/// A relation, as a listing gives it: identity and kind, without fields.
///
/// It is what
/// [`CatalogProvider::list_relations`](crate::provider::CatalogProvider::list_relations)
/// returns. Describing the fields of 20,000 tables takes minutes; the listing
/// must be able to fill the tree without requesting them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationRef {
    parent: CatalogPath,
    name: String,
    /// Kind of the relation.
    pub kind: RelationKind,
    /// Comment carried by the object, if it has one.
    ///
    /// **Untrusted content.** A column comment that says "ignore the previous
    /// instructions" is data, never an instruction (ARCHITECTURE §8): it is up
    /// to the single gateway of `oxyn-ai` to frame it before it joins a prompt.
    pub comment: Option<String>,
}

impl RelationRef {
    /// Builds a relation reference.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if the name is invalid, or if `parent`
    /// already designates a relation.
    pub fn new(
        parent: CatalogPath,
        name: impl Into<String>,
        kind: RelationKind,
    ) -> Result<Self, CatalogPathError> {
        if parent.relation().is_some() {
            return Err(CatalogPathError::new(
                "the parent of a relation cannot be a relation",
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

    /// Builds a reference from **already validated** levels.
    ///
    /// Reserved to the cache, for the same reason as [`CatalogRef::validated`].
    pub(crate) fn validated(parent: CatalogPath, name: String, kind: RelationKind) -> Self {
        Self {
            parent,
            name,
            kind,
            comment: None,
        }
    }

    /// Name of the relation.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The level that contains it: namespace, catalog, or nothing.
    #[must_use]
    pub const fn parent(&self) -> &CatalogPath {
        &self.parent
    }

    /// Full path of the relation.
    #[must_use]
    pub fn path(&self) -> CatalogPath {
        self.parent.with_validated_relation(&self.name)
    }

    /// Attaches a comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Reattaches the reference under another parent.
    ///
    /// Reserved to the cache, for the same reason as
    /// [`NamespaceRef::reparent`].
    pub(crate) fn reparent(&mut self, parent: CatalogPath) {
        self.parent = parent;
    }
}

/// Logical type of a field, independent of the product.
///
/// The server's **raw** type stays available in [`Field::raw_type`]: the
/// logical type serves to choose a rendering or an editor, not to replace what
/// the server says. A type the driver cannot map to this list becomes
/// [`Unknown`](Self::Unknown) — never a plausible neighbor, which would display
/// a wrong value without saying so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogicalType {
    /// Boolean.
    Boolean,
    /// Signed integer of `bits` bits.
    Integer {
        /// Width in bits as the server declares it (8, 16, 32, 64…).
        bits: u8,
    },
    /// Float of `bits` bits.
    Float {
        /// Width in bits (32, 64).
        bits: u8,
    },
    /// Exact decimal.
    Decimal {
        /// Precision, when the server constrains it.
        precision: Option<u16>,
        /// Scale. Negative on Oracle, hence the signed type.
        scale: Option<i16>,
    },
    /// Text.
    Text,
    /// Byte sequence.
    Bytes,
    /// UUID.
    Uuid,
    /// Date without time.
    Date,
    /// Time without date.
    Time,
    /// Timestamp.
    Timestamp {
        /// Does it carry a time zone? The distinction is not cosmetic:
        /// confusing it shifts the data invisibly and permanently
        /// ([`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md)).
        tz: bool,
    },
    /// Interval.
    Interval,
    /// JSON document.
    Json,
    /// Array of a type.
    Array(Box<LogicalType>),
    /// Structure with named fields (`ROW`, `STRUCT`, subdocument).
    Struct(Vec<Field>),
    /// Dense vector (pgvector, Qdrant).
    Vector {
        /// Dimension, when constrained.
        dims: Option<u32>,
    },
    /// Geometry (PostGIS, MySQL spatial).
    Geometry,
    /// Type the driver could not map to this list.
    Unknown,
}

impl LogicalType {
    /// 32-bit integer, the most frequent case.
    pub const INT32: Self = Self::Integer { bits: 32 };
    /// 64-bit integer.
    pub const INT64: Self = Self::Integer { bits: 64 };
    /// Double-precision float.
    pub const FLOAT64: Self = Self::Float { bits: 64 };
    /// Timestamp with time zone.
    pub const TIMESTAMPTZ: Self = Self::Timestamp { tz: true };

    /// Does the type contain other types?
    #[must_use]
    pub const fn is_nested(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Struct(_))
    }

    /// The type of an array's elements.
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
                for (i, col) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: {}", col.name, col.logical_type)?;
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

/// A field of a relation: column, document key, node property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Name of the field. **Hostile input**: it can contain a dot, a double
    /// quote, SQL.
    pub name: String,
    /// Ordinal position, as the server gives it.
    pub position: u32,
    /// Type mapped to Oxyn's vocabulary.
    pub logical_type: LogicalType,
    /// Type as the server names it (`int4`, `VARCHAR(255)`, `geography`).
    /// Always kept: it is what the user recognizes.
    pub raw_type: String,
    /// Does the field accept the absence of a value, according to the server?
    pub nullable: bool,
    /// Default value expression, as is.
    pub default: Option<String>,
    /// Comment. **Untrusted content**, see [`RelationRef::comment`].
    pub comment: Option<String>,
    /// Does the field take part in the primary key?
    pub is_primary_key: bool,
    /// Does the field come from an **inference by sampling** rather than from
    /// a declaration of the server?
    ///
    /// True for MongoDB and any schemaless source. The interface must say so:
    /// a field missing from the sample may exist further on, and a type
    /// inferred over a hundred documents can be wrong at the hundred and first
    /// ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
    pub inferred: bool,
}

impl Field {
    /// Builds a field.
    ///
    /// Unspecified values take the cautious side: `nullable` is true,
    /// `is_primary_key` and `inferred` are false.
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

    /// Declares the field non-null.
    #[must_use]
    pub fn not_null(mut self) -> Self {
        self.nullable = false;
        self
    }

    /// Declares the field a member of the primary key. Implies `NOT NULL`.
    #[must_use]
    pub fn primary_key(mut self) -> Self {
        self.is_primary_key = true;
        self.nullable = false;
        self
    }

    /// Marks the field as deduced by sampling.
    #[must_use]
    pub fn with_inferred(mut self) -> Self {
        self.inferred = true;
        self
    }

    /// Attaches a default value.
    #[must_use]
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Attaches a comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }
}

/// A described relation: identity, volume and fields.
///
/// It is what
/// [`CatalogProvider::describe_relation`](crate::provider::CatalogProvider::describe_relation)
/// returns. The path is not carried here: it is given by the position in the
/// cache, or by the [`RelationRef`] used to request it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    /// Name of the relation.
    pub name: String,
    /// Kind of the relation.
    pub kind: RelationKind,
    /// Comment. **Untrusted content**, see [`RelationRef::comment`].
    pub comment: Option<String>,
    /// Estimate of the number of rows, when the source gives one **without
    /// counting**. `None` means "unknown", never "zero": running a `COUNT(*)`
    /// to fill this field would scan the table on every tree refresh.
    pub estimated_rows: Option<u64>,
    /// Size on disk in bytes, when the source gives it.
    pub size_bytes: Option<u64>,
    /// The fields, in the order the server gives them.
    pub fields: Vec<Field>,
}

impl Relation {
    /// Builds a relation without fields.
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

    /// Attaches the fields.
    #[must_use]
    pub fn with_fields(mut self, fields: Vec<Field>) -> Self {
        self.fields = fields;
        self
    }

    /// Attaches a comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Attaches a volume estimate.
    #[must_use]
    pub fn with_estimated_rows(mut self, rows: u64) -> Self {
        self.estimated_rows = Some(rows);
        self
    }

    /// A field by its name, exactly (identifiers are case-sensitive once
    /// quoted).
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|col| col.name == name)
    }

    /// The primary key fields, in position order.
    ///
    /// Empty when the relation has none — which is the common case outside the
    /// relational model.
    #[must_use]
    pub fn primary_key(&self) -> Vec<&Field> {
        let mut keys: Vec<&Field> = self
            .fields
            .iter()
            .filter(|col| col.is_primary_key)
            .collect();
        keys.sort_by_key(|col| col.position);
        keys
    }

    /// Is the schema of this relation, in whole or in part, **deduced**?
    ///
    /// The interface uses it to mark the relation: presenting an inference as
    /// a server truth makes people write wrong queries with confidence.
    #[must_use]
    pub fn has_inferred_schema(&self) -> bool {
        self.fields.iter().any(|col| col.inferred)
    }
}

/// An index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// Name of the index.
    pub name: String,
    /// Indexed fields, in index order — the order decides what the index can
    /// serve.
    pub fields: Vec<String>,
    /// Does the index enforce uniqueness?
    pub unique: bool,
    /// Access method (`btree`, `gin`, `hnsw`…), when the source names it.
    pub method: Option<String>,
    /// Predicate of a partial index, as is.
    pub predicate: Option<String>,
}

impl Index {
    /// Builds an index.
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

    /// Declares the index unique.
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    /// Does the index cover only part of the rows?
    #[must_use]
    pub const fn is_partial(&self) -> bool {
        self.predicate.is_some()
    }
}

/// Referential action of a foreign key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReferentialAction {
    /// No action — the standard's default.
    #[default]
    NoAction,
    /// Refusal of the deletion.
    Restrict,
    /// Cascading deletion.
    Cascade,
    /// Set to null.
    SetNull,
    /// Back to the default value.
    SetDefault,
}

impl ReferentialAction {
    /// Stable name.
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

    /// Does deleting a referenced row delete others?
    ///
    /// It is what an approval preview must show: a `DELETE` of one row can
    /// erase a million through a cascade.
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

/// The target of a foreign key: a relation, and the fields it exposes.
///
/// Grouped because they mean nothing separately — a target relation without
/// its columns allows neither drawing a diagram nor composing a join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeyTarget {
    /// The referenced relation.
    pub relation: CatalogPath,
    /// The referenced fields, in the order matching [`ForeignKey::fields`].
    pub fields: Vec<String>,
}

/// A foreign key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    /// Declared name; empty when the engine exposes no constraint name.
    pub name: String,
    /// Carrying fields, in order.
    pub fields: Vec<String>,
    /// What is referenced.
    pub references: ForeignKeyTarget,
    /// What happens to the carrying rows when the referenced row disappears.
    pub on_delete: ReferentialAction,
}

impl ForeignKey {
    /// Builds a foreign key.
    #[must_use]
    pub fn new(name: impl Into<String>, fields: Vec<String>, references: ForeignKeyTarget) -> Self {
        Self {
            name: name.into(),
            fields,
            references,
            on_delete: ReferentialAction::NoAction,
        }
    }

    /// Do both sides of the key have the same number of fields?
    ///
    /// A server may return mismatched lists; composing a join by positional
    /// pairing without checking would produce a wrong `ON`.
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

/// Kind of a constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConstraintKind {
    /// Primary key.
    PrimaryKey,
    /// Uniqueness.
    Unique,
    /// Check of an expression.
    Check,
    /// Foreign key — detailed by [`ForeignKey`].
    ForeignKey,
    /// Exclusion (PostgreSQL).
    Exclusion,
    /// A user-defined constraint trigger.
    Trigger,
    /// Non-nullability expressed as a named constraint.
    NotNull,
}

impl ConstraintKind {
    /// Stable name.
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

/// A constraint carried by a relation.
///
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    /// Declared name; empty when the engine exposes no constraint name.
    pub name: String,
    /// Kind of the constraint.
    pub kind: ConstraintKind,
    /// Fields concerned, when the constraint names some.
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
    /// Builds a constraint.
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

    /// Attaches an expression.
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
    fn a_hostile_object_name_is_refused_at_construction() {
        // An object name comes from the server: control characters are refused
        // (SECURITY, input surface §2), double quotes are not — they are legal,
        // and `qualify` knows how to quote them.
        assert!(CatalogRef::new("base\u{1b}[31m").is_err());
        assert!(CatalogRef::new("").is_err());
        assert!(CatalogRef::new(r#"users"; DROP TABLE audit; --"#).is_ok());
    }

    #[test]
    fn the_parent_of_a_relation_cannot_be_a_relation() {
        let relation = CatalogPath::for_relation(None, Some("public"), "clients").expect("valid");
        assert!(RelationRef::new(relation, "other", RelationKind::Table).is_err());
    }

    #[test]
    fn the_parent_of_a_namespace_does_not_go_lower() {
        let space = CatalogPath::for_namespace(None, "public").expect("valid");
        assert!(NamespaceRef::new(space, "other").is_err());
    }

    #[test]
    fn the_path_of_a_reference_rebuilds_without_loss() {
        let parent = CatalogPath::for_namespace(Some("sales"), "public").expect("valid");
        let relation = RelationRef::new(parent, "ventes.2026", RelationKind::Table).expect("valid");
        let item_path = relation.path();
        assert_eq!(item_path.catalog(), Some("sales"));
        assert_eq!(item_path.namespace(), Some("public"));
        assert_eq!(item_path.relation(), Some("ventes.2026"));
    }

    #[test]
    fn a_relation_without_intermediate_level_keeps_its_catalog() {
        // Neo4j: database + label, without a namespace.
        let parent = CatalogPath::for_catalog("graph").expect("valid");
        let label = RelationRef::new(parent, "Person", RelationKind::NodeLabel).expect("valid");
        assert_eq!(label.path().to_string(), "graph..Person");
    }

    #[test]
    fn an_inferred_schema_declares_itself() {
        let mongo = Relation::new("orders", RelationKind::Collection).with_fields(vec![
            Field::new("_id", 0, LogicalType::Uuid, "objectId").primary_key(),
            Field::new("amount", 1, LogicalType::FLOAT64, "double").with_inferred(),
        ]);
        assert!(
            mongo.has_inferred_schema(),
            "a schema deduced by sampling must be able to say so"
        );

        let postgres = Relation::new("orders", RelationKind::Table).with_fields(vec![Field::new(
            "id",
            0,
            LogicalType::INT64,
            "int8",
        )]);
        assert!(!postgres.has_inferred_schema());
    }

    #[test]
    fn the_primary_key_comes_out_in_position_order() {
        let relation = Relation::new("lines", RelationKind::Table).with_fields(vec![
            Field::new("caption", 0, LogicalType::Text, "text"),
            Field::new("line", 2, LogicalType::INT32, "int4").primary_key(),
            Field::new("order", 1, LogicalType::INT64, "int8").primary_key(),
        ]);
        let names: Vec<&str> = relation
            .primary_key()
            .iter()
            .map(|col| col.name.as_str())
            .collect();
        assert_eq!(names, ["order", "line"]);
    }

    #[test]
    fn a_primary_key_field_is_non_null() {
        let col = Field::new("id", 0, LogicalType::INT64, "int8").primary_key();
        assert!(!col.nullable);
    }

    #[test]
    fn an_unknown_volume_is_not_zero() {
        let relation = Relation::new("journal", RelationKind::Table);
        assert_eq!(relation.estimated_rows, None);
        assert_ne!(relation.estimated_rows, Some(0));
    }

    #[test]
    fn logical_types_render_readably() {
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
    fn a_timestamp_with_time_zone_is_not_confused_with_a_bare_timestamp() {
        // The confusion that shifts the data by two hours, invisibly and
        // permanently (DRIVER-CONTRACT §7).
        assert_ne!(
            LogicalType::Timestamp { tz: true },
            LogicalType::Timestamp { tz: false }
        );
    }

    #[test]
    fn a_mismatched_foreign_key_is_detected() {
        let target = ForeignKeyTarget {
            relation: CatalogPath::for_relation(None, Some("public"), "clients").expect("valid"),
            fields: vec!["id".to_owned()],
        };
        let good = ForeignKey::new("fk_ok", vec!["client_id".to_owned()], target.clone());
        assert!(good.is_well_formed());

        let bad = ForeignKey::new(
            "fk_ko",
            vec!["client_id".to_owned(), "site_id".to_owned()],
            target,
        );
        assert!(
            !bad.is_well_formed(),
            "two lists of different lengths do not pair positionally"
        );
    }

    #[test]
    fn only_relations_with_records_open() {
        assert!(RelationKind::Table.holds_records());
        assert!(RelationKind::Collection.holds_records());
        assert!(RelationKind::KeyPattern.holds_records());
        assert!(!RelationKind::Function.holds_records());
        assert!(!RelationKind::Sequence.holds_records());
    }

    #[test]
    fn a_cascade_propagates_the_deletion() {
        assert!(ReferentialAction::Cascade.propagates_delete());
        assert!(!ReferentialAction::NoAction.propagates_delete());
        assert!(!ReferentialAction::Restrict.propagates_delete());
    }
}
