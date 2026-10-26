//! Composing a relation preview: `database.schema.relation`, quoted.
//!
//! Two halves that do not look alike ([ADR-0020]): the **order** and the
//! **projection** are structured, so every column is quoted here and refused
//! when the relation does not declare it; the **predicate** is SQL the user
//! wrote, and travels untouched. What keeps it honest is not this module: the
//! request is read-only, so the stream runs it inside DuckDB's
//! `BEGIN TRANSACTION READ ONLY` ([`crate::stream`]), and the row limit holds.
//!
//! [ADR-0020]: ../../../docs/adr/0020-apercu-trie-filtre-parcouru.md

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result, SqlDialect,
    StatementIntent,
};

/// DuckDB quotes with double quotes, like the standard.
const STYLE: QuoteStyle = QuoteStyle::Double;

/// What the catalog says about a relation, as far as ordering and projecting
/// need it.
///
/// Read on every preview, since one with no sort asked for still comes in key
/// order ([ADR-0068](../../../docs/adr/0068-a-preview-comes-in-key-order.md)).
#[derive(Debug, Default)]
pub(crate) struct RelationFacts {
    columns: Vec<String>,
    /// The primary key's columns, empty when none is declared.
    key: Vec<String>,
    /// Whether the key orders the preview when no sort is asked for: see
    /// [`Relation::previews_in_key_order`], which the interface reads too.
    ordered_by_default: bool,
}

impl RelationFacts {
    pub(crate) fn of(relation: &Relation) -> Self {
        Self {
            columns: relation
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            key: relation
                .primary_key()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            ordered_by_default: relation.previews_in_key_order(),
        }
    }
}

/// Composes the preview of the relation `path` names.
///
/// # Errors
/// [`OxynError::Config`] for a limit outside `1..=1000`,
/// [`OxynError::CatalogUnavailable`] for an incomplete path,
/// [`OxynError::Query`] for a sort or projected column the relation does not
/// declare, [`OxynError::NotSupported`] for a page without a primary key.
pub(crate) fn request(
    path: &CatalogPath,
    limit: u32,
    shape: &PreviewShape,
    facts: &RelationFacts,
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    if path.catalog().is_none() || path.namespace().is_none() || path.relation().is_none() {
        return Err(OxynError::CatalogUnavailable(
            "a DuckDB preview names a database, a schema and a relation".into(),
        ));
    }
    let qualified = path.qualify(STYLE);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let projection = projection(shape, facts)?;
    let order = order_by(shape, facts)?;

    let mut text = format!("SELECT {projection} FROM {qualified}");
    if let Some(predicate) = shape.predicate() {
        // The newline ends a trailing `--` comment; the parentheses turn an
        // unterminated `/*` into a syntax error instead of a swallowed LIMIT.
        // `WHERE (X)` means `WHERE X` for any boolean expression.
        text.push_str(" WHERE (");
        text.push_str(predicate);
        text.push_str("\n) ");
    } else {
        text.push(' ');
    }
    if let Some(order) = order {
        text.push_str("ORDER BY ");
        text.push_str(&order);
        text.push(' ');
    }
    text.push_str(&format!("LIMIT {limit}"));
    if shape.offset > 0 {
        text.push_str(&format!(" OFFSET {}", shape.offset));
    }
    Ok(
        ExecRequest::new(QueryLanguage::Sql(SqlDialect::DuckDb), text)
            .with_intent(StatementIntent::Read)
            .with_limits(ExecLimits::default().with_max_rows(max_rows)),
    )
}

/// The select list: `*`, or the requested columns, each checked and quoted.
fn projection(shape: &PreviewShape, facts: &RelationFacts) -> Result<String> {
    let Some(columns) = shape.projection()? else {
        return Ok("*".to_owned());
    };
    let mut terms = Vec::with_capacity(columns.len());
    for column in columns {
        if !facts.columns.iter().any(|declared| declared == column) {
            return Err(OxynError::Query(format!(
                "cannot read `{column}` in a preview: the relation does not declare that column"
            )));
        }
        terms.push(quote_identifier(column, STYLE));
    }
    Ok(terms.join(", "))
}

/// The `ORDER BY` terms, completed by the primary key from the first page on,
/// or `None` when nothing needs ordering.
///
/// A relation ordered by default is ordered on its key from the first page,
/// sort or no sort: page 0 unordered and page 1 by key would not be one
/// sequence.
fn order_by(shape: &PreviewShape, facts: &RelationFacts) -> Result<Option<String>> {
    if !shape.needs_total_order() && !facts.ordered_by_default {
        return Ok(None);
    }
    let mut terms = Vec::with_capacity(shape.sort.len() + facts.key.len());
    for sort in &shape.sort {
        if !facts.columns.contains(&sort.column) {
            return Err(OxynError::Query(format!(
                "cannot sort a preview on `{}`: the relation does not declare that column",
                sort.column
            )));
        }
        let direction = if sort.descending { "DESC" } else { "ASC" };
        terms.push(format!(
            "{} {direction}",
            quote_identifier(&sort.column, STYLE)
        ));
    }
    if shape.offset > 0 && facts.key.is_empty() {
        return Err(OxynError::NotSupported {
            capability: "preview pagination: this relation declares no primary key, and an \
                         OFFSET without a total order repeats rows and skips others without \
                         saying so"
                .to_owned(),
        });
    }
    for key in &facts.key {
        if shape.sort.iter().any(|sort| sort.column == *key) {
            continue;
        }
        terms.push(format!("{} ASC", quote_identifier(key, STYLE)));
    }
    Ok((!terms.is_empty()).then(|| terms.join(", ")))
}

#[cfg(test)]
mod tests;
