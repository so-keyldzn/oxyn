//! What crosses the IPC boundary for the object view: relation metadata and
//! the shape of a preview.
//!
//! Two directions, two rules. What the front **sends** is a draft: the preview
//! shape is rebuilt here into [`PreviewShape`], checked against the catalog, and
//! refused rather than trimmed when it asks for something the product must not
//! compose ([ADR-0020](../../../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
//! What it **receives** is narrower than the catalog model: names stay data,
//! addressed by segments, and nothing here is SQL the front could run
//! ([I-10](../../../../CLAUDE.md#i-10)).

use std::fmt;

use oxyn_catalog::model::{Constraint, ForeignKey, IncomingForeignKey, Index, Relation};
use oxyn_catalog::{DefinitionSource, Freshness, MatchKind, RelationDefinition, SearchHit};
use oxyn_core::{PreviewShape, PreviewSort, SqlDialect};

use crate::ipc::consoles::ParameterKind;
use serde::{Deserialize, Serialize};

use crate::ipc::{CatalogAddress, IpcError, RelationDetail};

/// Rows one preview reads, the bound UX-SPEC sets.
pub const PREVIEW_ROWS: u32 = 200;

/// Sort keys one preview may carry. A composed order longer than this is not a
/// preview any more; it is a query, and the console writes those.
pub const MAX_SORT_KEYS: usize = 16;

/// The largest predicate accepted, in bytes. A preview filter is a condition,
/// not a document: past this size it is refused, never cut.
pub const MAX_PREDICATE_BYTES: usize = 64 * 1024;

/// One sort key as the front asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSortDraft {
    /// A column name exactly as the catalog spells it; never an expression.
    pub column: String,
    pub descending: bool,
}

/// The shape the front asks a preview for.
///
/// **No `Debug` derive**: the predicate is SQL the user wrote, and it may hold
/// a literal they would not want in a log; the manual implementation renders
/// its length only.
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewShapeDraft {
    #[serde(default)]
    pub sort: Vec<PreviewSortDraft>,
    #[serde(default)]
    pub predicate: Option<String>,
    #[serde(default)]
    pub offset: u64,
}

impl fmt::Debug for PreviewShapeDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewShapeDraft")
            .field("sort", &self.sort.len())
            .field("predicate_bytes", &self.predicate.as_ref().map(String::len))
            .field("offset", &self.offset)
            .finish()
    }
}

impl PreviewShapeDraft {
    /// Rebuilds the domain shape, refusing what no preview may ask for.
    ///
    /// `relation` is the catalog's description when it has been read. A sort
    /// column it does not declare is refused here, before anything is composed;
    /// an undescribed relation leaves that check to the driver, which reads the
    /// columns itself.
    ///
    /// A page other than the first is refused unless the order is total
    /// ([ADR-0028](../../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md),
    /// [ADR-0068](../../../../docs/adr/0068-a-preview-comes-in-key-order.md)):
    /// the front never offers one, and a script that asks anyway gets the same
    /// answer the interface would have given.
    ///
    /// # Errors
    /// Too many or duplicated sort keys, an empty or undeclared column, an
    /// oversized predicate, or a page without a total order.
    pub fn into_shape(self, relation: Option<&Relation>) -> Result<PreviewShape, IpcError> {
        if self.sort.len() > MAX_SORT_KEYS {
            return Err(IpcError::invalid(format!(
                "A preview can be sorted on {MAX_SORT_KEYS} columns at most"
            )));
        }
        let mut sort: Vec<PreviewSort> = Vec::with_capacity(self.sort.len());
        for key in self.sort {
            if key.column.is_empty() {
                return Err(IpcError::invalid("A sort key needs a column"));
            }
            if sort.iter().any(|existing| existing.column == key.column) {
                return Err(IpcError::invalid(format!(
                    "The column `{}` appears twice in the sort",
                    key.column
                )));
            }
            if let Some(relation) = relation
                && !relation.fields.iter().any(|field| field.name == key.column)
            {
                return Err(IpcError::invalid(format!(
                    "Cannot sort this preview on `{}`: the relation does not declare that column",
                    key.column
                )));
            }
            sort.push(PreviewSort {
                column: key.column,
                descending: key.descending,
            });
        }
        if self
            .predicate
            .as_ref()
            .is_some_and(|text| text.len() > MAX_PREDICATE_BYTES)
        {
            return Err(IpcError::invalid(format!(
                "This filter is longer than {} KiB; write it in a console instead",
                MAX_PREDICATE_BYTES / 1024
            )));
        }
        let shape = PreviewShape {
            sort,
            predicate: self.predicate,
            offset: self.offset,
            columns: None,
        };
        if shape.offset > 0 {
            // Asked of the pagination itself, so that what the interface is
            // offered and what a script may ask for cannot drift apart: a page
            // of an order nobody can name is what ADR-0028 refuses, and the
            // order a relation comes in by default is the one ADR-0068 names.
            match pagination(&shape, relation, 0) {
                Pagination::Unknown => {
                    return Err(IpcError::invalid(
                        "A page other than the first needs the relation to be described: \
                         its key is not known yet",
                    ));
                }
                Pagination::NeedsOrder => {
                    return Err(IpcError::invalid(
                        "A page other than the first needs a sort: an OFFSET over an \
                         uncertain order shows a row twice and hides another",
                    ));
                }
                Pagination::NoUniqueKey => {
                    return Err(IpcError::invalid(
                        "This preview stays on its first page: no unique key is known for \
                         this relation, so no order can be made total",
                    ));
                }
                Pagination::Ready { .. } => {}
            }
            if !shape.offset.is_multiple_of(u64::from(PREVIEW_ROWS)) {
                return Err(IpcError::invalid(
                    "A preview page starts on a multiple of its page size",
                ));
            }
        }
        Ok(shape)
    }
}

/// What the page controls of a preview may offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Pagination {
    /// The relation has not been described, so its key is not known. Not the
    /// same as having no key: nothing is said, and a page is not offered.
    Unknown,
    /// A key is known, but this relation does not come in key order by itself
    /// — a virtual table, a warehouse table — and nothing was sorted: a « next »
    /// page would be the second page of an order the user never saw. Sorting
    /// is what makes pages possible, at the cost of reading it all.
    NeedsOrder,
    /// The relation is described and holds no unique key: no order can be made
    /// total, and sorting would not change that.
    NoUniqueKey,
    /// Each direction is offered only when it leads somewhere.
    #[serde(rename_all = "camelCase")]
    Ready {
        previous: bool,
        next: bool,
        /// One-based position of the first row of this page in the order.
        first_row: u64,
        /// The key columns, when the order in force is the one the relation
        /// comes in by default; `None` when the user's sort is in force.
        ordered_by: Option<Vec<String>>,
    },
}

/// Which page controls the rows on screen allow.
///
/// Checked against the shape **the rows came from**, never the one being
/// typed: two pages only fail to overlap when both were composed from the same
/// total order. `relation` is `None` when the relation has not been described,
/// which is not the same as having no key — and neither lets a page be
/// offered.
///
/// The order is total when a unique key is known and either the user sorted —
/// the driver completes the sort with the key — or the relation comes in key
/// order by itself ([`Relation::previews_in_key_order`], ADR-0068). A driver
/// that does not vouch for its default order is not assumed to have applied
/// one.
#[must_use]
pub fn pagination(applied: &PreviewShape, relation: Option<&Relation>, rows: u64) -> Pagination {
    let Some(relation) = relation else {
        return Pagination::Unknown;
    };
    let key = relation.primary_key();
    if key.is_empty() {
        return Pagination::NoUniqueKey;
    }
    let by_default = applied.sort.is_empty();
    if by_default && !relation.previews_in_key_order() {
        return Pagination::NeedsOrder;
    }
    Pagination::Ready {
        previous: applied.offset > 0,
        // A short page is the last one: « next » there would spend a read to
        // show nothing.
        next: rows >= u64::from(PREVIEW_ROWS),
        first_row: applied.offset.saturating_add(1),
        ordered_by: by_default.then(|| key.iter().map(|field| field.name.clone()).collect()),
    }
}

/// How fresh one facet of a relation is.
///
/// Three states, as the cache keeps them: never read waits to be asked for,
/// read has a date, and invalidated — after a DDL sent from Oxyn — is stale
/// whatever its age.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum FacetFreshness {
    Never,
    #[serde(rename_all = "camelCase")]
    Fetched {
        /// RFC 3339, in UTC: the front formats it, it never guesses a zone.
        fetched_at: String,
    },
    Invalidated,
}

impl From<Freshness> for FacetFreshness {
    fn from(freshness: Freshness) -> Self {
        match freshness {
            Freshness::Fetched(instant) => Self::Fetched {
                fetched_at: instant.to_rfc3339(),
            },
            Freshness::Invalidated => Self::Invalidated,
            // `Never`, and any state added later: claiming a read that cannot
            // be dated would be worse than asking for one.
            _ => Self::Never,
        }
    }
}

/// One lazily loaded part of a relation, and how fresh it is.
///
/// `value` is `None` both when it was never read and when the source reported
/// nothing; `freshness` tells them apart. An empty list is a read that found
/// none.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facet<T> {
    pub freshness: FacetFreshness,
    pub value: Option<T>,
}

/// A constraint as the Constraints tab lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintRow {
    /// Empty when the engine exposes no constraint name.
    pub name: String,
    /// `primary_key`, `unique`, `check`, `foreign_key`, `exclusion`, …
    pub kind: String,
    pub fields: Vec<String>,
    /// The engine's rendering; shown and copied, never executed.
    pub expression: Option<String>,
    /// Whether existing rows were validated, when the engine says.
    pub validated: Option<bool>,
}

impl From<&Constraint> for ConstraintRow {
    fn from(constraint: &Constraint) -> Self {
        Self {
            name: constraint.name.clone(),
            kind: constraint.kind.as_str().to_owned(),
            fields: constraint.fields.clone(),
            expression: constraint.expression.clone(),
            validated: constraint.validated,
        }
    }
}

/// An index of the relation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRow {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
    pub method: Option<String>,
    pub predicate: Option<String>,
}

impl From<&Index> for IndexRow {
    fn from(index: &Index) -> Self {
        Self {
            name: index.name.clone(),
            fields: index.fields.clone(),
            unique: index.unique,
            method: index.method.clone(),
            predicate: index.predicate.clone(),
        }
    }
}

/// A foreign key declared by this relation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyRow {
    pub name: String,
    pub fields: Vec<String>,
    /// The referenced relation, addressed by segments so it can be opened.
    pub references: CatalogAddress,
    pub referenced_fields: Vec<String>,
    pub on_delete: String,
    /// Whether deleting a referenced row changes rows here too (cascade, set null, set default).
    pub cascades: bool,
    /// Both column lists have the same, non-zero length.
    pub well_formed: bool,
}

impl From<&ForeignKey> for ForeignKeyRow {
    fn from(key: &ForeignKey) -> Self {
        Self {
            name: key.name.clone(),
            fields: key.fields.clone(),
            references: CatalogAddress::of(&key.references.relation),
            referenced_fields: key.references.fields.clone(),
            on_delete: key.on_delete.as_str().to_owned(),
            cascades: key.on_delete.propagates_delete(),
            well_formed: key.is_well_formed(),
        }
    }
}

/// A foreign key another relation declares towards this one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingKeyRow {
    /// The referencing relation, addressed by segments so it can be opened.
    pub source: CatalogAddress,
    pub key: ForeignKeyRow,
    /// Whether one referenced value matches at most one source row; `None`
    /// when the engine does not report it.
    pub source_unique: Option<bool>,
}

impl From<&IncomingForeignKey> for IncomingKeyRow {
    fn from(incoming: &IncomingForeignKey) -> Self {
        Self {
            source: CatalogAddress::of(&incoming.source),
            key: ForeignKeyRow::from(&incoming.key),
            source_unique: incoming.source_unique,
        }
    }
}

/// Creation statements, shown read-only and never sent back to be run.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionView {
    pub sql: String,
    /// `stored` or `reconstructed`: provenance stays visible (ADR-0018).
    pub source: String,
    pub notes: Vec<String>,
}

// Not derived, as `RelationDefinition` does not: a definition can be a
// megabyte, and a `{view:?}` in a log line would carry all of it.
impl fmt::Debug for DefinitionView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefinitionView")
            .field("sql_bytes", &self.sql.len())
            .field("source", &self.source)
            .field("notes", &self.notes.len())
            .finish()
    }
}

impl From<&RelationDefinition> for DefinitionView {
    fn from(definition: &RelationDefinition) -> Self {
        Self {
            sql: definition.sql.clone(),
            source: match definition.source {
                DefinitionSource::Stored => "stored",
                DefinitionSource::Reconstructed => "reconstructed",
                // A provenance added later is not presented as either known
                // one: ADR-0018 asks that it stay visible, not guessed.
                _ => "unknown",
            }
            .to_owned(),
            notes: definition.notes.clone(),
        }
    }
}

/// Everything the object view knows about one relation, from one cache read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationFacets {
    pub address: CatalogAddress,
    /// `table`, `view`, … from the tree summary; `None` when the relation is
    /// not in the cache at all.
    pub kind: Option<String>,
    pub holds_records: bool,
    /// The name as the session's dialect quotes it, for copying. Composed by
    /// the backend from the segments, never by joining strings in the front.
    pub qualified_name: String,
    pub detail: Facet<RelationDetail>,
    /// Indexes and outgoing keys come with the relation read; `None` when the
    /// source does not report them, which is not the same as « none ».
    pub indexes: Option<Vec<IndexRow>>,
    pub foreign_keys: Option<Vec<ForeignKeyRow>>,
    pub constraints: Facet<Vec<ConstraintRow>>,
    pub incoming_keys: Facet<Vec<IncomingKeyRow>>,
    pub definition: Facet<DefinitionView>,
    /// Whether a primary key is declared; `None` until the relation is read.
    pub unique_key: Option<bool>,
}

/// The parts of a relation a tab loads on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelationFacet {
    /// Columns, comments, size, indexes and outgoing keys.
    Detail,
    Constraints,
    IncomingKeys,
    Definition,
}

/// The texts the catalog's « Copy as » entries compose for a relation.
///
/// Composed in Rust, quoted by the dialect, and copied — never run
/// ([I-10](../../../../CLAUDE.md#i-10)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectSqlForm {
    /// The qualified name, as the object view shows it.
    QuotedName,
    /// `SELECT *` over the relation.
    SelectAll,
    /// `INSERT` naming every column the catalog has read, with placeholders.
    InsertTemplate,
}

/// A relation the catalog search found among what is already loaded.
///
/// The search never introspects: an object whose level was never expanded is
/// not found, and the front says so.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchHit {
    pub address: CatalogAddress,
    pub name: String,
    pub kind: String,
    pub holds_records: bool,
    /// `relationName`, `fieldName` or `comment`: what made it match.
    pub matched: String,
    pub matched_fields: Vec<String>,
}

impl CatalogSearchHit {
    #[must_use]
    pub fn of(hit: &SearchHit) -> Self {
        Self {
            address: CatalogAddress::of(&hit.path),
            name: hit.path.relation().unwrap_or_default().to_owned(),
            kind: hit.kind.as_str().to_owned(),
            holds_records: hit.kind.holds_records(),
            matched: match hit.matched {
                MatchKind::RelationName => "relationName",
                MatchKind::FieldName => "fieldName",
                MatchKind::Comment => "comment",
                _ => "other",
            }
            .to_owned(),
            matched_fields: hit.matched_fields.clone(),
        }
    }
}

/// What a view showing this connection should read again, and why
/// ([ADR-0022](../../../../docs/adr/0022-rafraichissement-automatique.md)).
///
/// Carries no object name: the classifier does not name tables, and the
/// refresh is scoped to the connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RefreshSignal {
    /// A DDL succeeded: the catalog of this connection is invalidated.
    #[serde(rename_all = "camelCase")]
    CatalogInvalidated { connection: String },
    /// A catalog read of this connection finished and published its result —
    /// even a read that brought nothing new, since the event does not say. The
    /// cache may hold other content than before: a level read, another evicted
    /// under the budget. It is a prompt to read the snapshot again, never a
    /// statement that something changed, and it starts no load: the editor's
    /// completion only re-reads what the cache holds
    /// ([ADR-0057](../../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md)).
    ///
    /// Not [`Self::CatalogInvalidated`], which only a finished DDL emits: a
    /// plain load changes what the cache holds just as surely, and nothing
    /// else tells the webview which connection it concerns.
    #[serde(rename_all = "camelCase")]
    CatalogChanged { connection: String },
    /// A write or a DDL succeeded: the **visible** preview is read again, with
    /// the shape in force. A DDL counts: `ALTER`, `TRUNCATE` or `DROP COLUMN`
    /// change the rows a preview shows as surely as an `UPDATE`.
    #[serde(rename_all = "camelCase")]
    RowsChanged { connection: String },
    /// A successful execution, whoever ran it, is written to the query
    /// history: an open library reads it again.
    #[serde(rename_all = "camelCase")]
    HistoryRecorded { connection: String },
    /// The front missed events: nobody knows what changed, so everything
    /// visible is read again.
    Lagged,
}

impl RefreshSignal {
    /// The signals an execution event carries, possibly none.
    ///
    /// Nothing after a failure or a cancellation: a read that follows an
    /// error hides the error. A statement classified as a read stales no
    /// preview, but its history entry is still news to the library.
    #[must_use]
    pub fn of(event: &oxyn_exec::ExecEvent) -> Vec<Self> {
        let Some(connection) = event.connection.map(|id| id.to_string()) else {
            return Vec::new();
        };
        match &event.event {
            oxyn_core::Event::Completed { intent, .. } => match intent {
                oxyn_core::StatementIntent::Read => Vec::new(),
                oxyn_core::StatementIntent::Ddl => vec![
                    Self::CatalogInvalidated {
                        connection: connection.clone(),
                    },
                    Self::RowsChanged { connection },
                ],
                // Write, Grant, Unknown and whatever comes later: the price of
                // being wrong the other way is one bounded 200-row read.
                _ => vec![Self::RowsChanged { connection }],
            },
            oxyn_core::Event::HistoryRecorded => vec![Self::HistoryRecorded { connection }],
            oxyn_core::Event::CatalogUpdated => vec![Self::CatalogChanged { connection }],
            _ => Vec::new(),
        }
    }
}

/// The placeholder a bound value takes in this dialect, 1-based.
///
/// A related-row query is a **template**: the value comes bound, never
/// concatenated into the text ([I-10](../../../../CLAUDE.md#i-10)). Written
/// with the dialect's own syntax, so that the statement parses as it stands —
/// an empty `=` would not.
#[must_use]
pub fn parameter_placeholder(dialect: SqlDialect, position: usize) -> String {
    match dialect {
        SqlDialect::Postgres | SqlDialect::Redshift | SqlDialect::DuckDb => {
            format!("${position}")
        }
        SqlDialect::Oracle => format!(":{position}"),
        SqlDialect::SqlServer => format!("@p{position}"),
        // MySQL, SQLite, ClickHouse, Snowflake, BigQuery and plain SQL all
        // take positional `?`; an unknown dialect gets the same, which is
        // what most drivers accept.
        _ => "?".to_owned(),
    }
}

/// A value the console should bind, ready for its Parameters panel.
///
/// **No `Debug` derive**: `text` is a value read from the user's table, and a
/// `{query:?}` added later would print it
/// ([I-03](../../../../CLAUDE.md#i-03)).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundValue {
    #[serde(rename = "type")]
    pub kind: ParameterKind,
    pub text: String,
}

impl fmt::Debug for BoundValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundValue")
            .field("kind", &self.kind)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

/// The row a related-row query takes its values from.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedRowsSource {
    #[serde(deserialize_with = "crate::ipc::metadata::result_id")]
    pub result: oxyn_core::ResultId,
    pub row: usize,
}

/// Parses the result id the front sends as a string.
fn result_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<oxyn_core::ResultId, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.parse().map_err(serde::de::Error::custom)
}

/// The related-row query a console opens: text, and the values to bind.
///
/// `parameters` is empty when no row was given, or when a key value could not
/// be typed: the console then shows an unbound parameter rather than a value
/// nobody chose.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedRowsQuery {
    pub sql: String,
    pub parameters: Vec<BoundValue>,
    /// The statement has a parameter left for the user to fill.
    pub needs_values: bool,
}

#[cfg(test)]
mod tests;
