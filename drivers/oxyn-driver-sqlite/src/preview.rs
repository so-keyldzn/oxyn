//! Pure SQL composition using SQLite attached databases, without an extra schema.
//!
//! Two halves that do not look alike ([ADR-0020]): the **order** and the
//! **projection** are structured, so every column is quoted here and refused
//! when the relation does not declare it; the **predicate** is SQL the user wrote, and travels through
//! untouched — neither parsed nor rewritten. What keeps the second one honest
//! is not this module: the statement stays read-only for the engine itself
//! (`sqlite3_stmt_readonly`, see [`crate::stream`]) and the row limit still
//! applies.
//!
//! [ADR-0020]: ../../../docs/adr/0020-apercu-trie-filtre-parcouru.md

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result, SqlDialect,
    StatementIntent,
};

/// SQLite quotes with double quotes, like the standard.
const STYLE: QuoteStyle = QuoteStyle::Double;

/// What the catalog says about a relation, as far as ordering and projecting
/// need it.
///
/// Read from the engine on **every** preview, since a preview with no sort
/// asked for still comes in key order
/// ([ADR-0068](../../../docs/adr/0068-a-preview-comes-in-key-order.md)). The
/// default value stands for a relation that could not be described and whose
/// read will say why — a module that is not loaded, for one.
#[derive(Debug, Default)]
pub(crate) struct RelationFacts {
    /// Column names, as the catalog spells them.
    columns: Vec<String>,
    /// The columns of a known unique key, empty when none is declared.
    ///
    /// SQLite's implicit `rowid` is **not** used here: views and `WITHOUT
    /// ROWID` tables have none, and paging on a key that only sometimes exists
    /// is the silent failure this whole path avoids.
    key: Vec<String>,
    /// Whether the key orders the preview when no sort is asked for. A virtual
    /// table keeps its key for the sorts the user asks for, and is not
    /// ordered otherwise: see [`Relation::previews_in_key_order`].
    ordered_by_default: bool,
}

impl RelationFacts {
    /// Keeps from a described relation what an `ORDER BY` needs, and nothing
    /// else.
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

/// The attached database a preview path designates.
///
/// The current SQLite catalog exposes attached databases as namespaces. An
/// explicit catalog is accepted too, but a conflicting level is never silently
/// discarded. The same answer is used to compose the statement and to describe
/// the relation it reads: [`crate::catalog::SqliteCatalog`] itself only looks
/// at the namespace, so describing the path as given would look in `main`.
///
/// # Errors
/// [`OxynError::CatalogUnavailable`] when the catalog and namespace levels name
/// two different databases.
pub(crate) fn database_of(path: &CatalogPath) -> Result<&str> {
    match (path.catalog(), path.namespace()) {
        (Some(catalog), Some(namespace)) if catalog != namespace => Err(
            OxynError::CatalogUnavailable("SQLite preview has conflicting database levels".into()),
        ),
        (Some(database), _) | (None, Some(database)) => Ok(database),
        (None, None) => Ok(crate::catalog::MAIN),
    }
}

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
    let database = database_of(path)?;
    let relation = path
        .relation()
        .ok_or_else(|| OxynError::CatalogUnavailable("preview path must name a relation".into()))?;
    let qualified =
        CatalogPath::for_relation(Some(database), None, relation)?.qualify_sql(SqlDialect::Sqlite);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let projection = projection(shape, facts)?;
    let order = order_by(shape, facts)?;

    let mut text = format!("SELECT {projection} FROM {qualified}");
    if let Some(predicate) = shape.predicate() {
        // Two guards around a fragment the driver does not parse, and each one
        // catches what the other lets through.
        //
        // The newline ends a trailing `--` comment, which would otherwise
        // swallow the ORDER BY and the LIMIT that follow.
        //
        // The parentheses catch the unterminated `/*`, which no newline ends:
        // SQLite then reads to the end of the text and never finds the closing
        // one, so it refuses the statement instead of running an unbounded scan
        // (verified on 2026-09-10). `WHERE (X)` means exactly `WHERE X` for any
        // boolean expression, so no legitimate predicate changes meaning.
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
        ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text)
            .with_intent(StatementIntent::Read)
            .with_limits(ExecLimits::default().with_max_rows(max_rows)),
    )
}

/// The select list: `*`, or the requested columns, each quoted.
///
/// # Erreurs
/// Those of [`PreviewShape::projection`], and [`OxynError::Query`] when a name
/// is not a column of the relation — the same permanent refusal as an unknown
/// sort column, before the engine sees it.
fn projection(shape: &PreviewShape, facts: &RelationFacts) -> Result<String> {
    let Some(columns) = shape.projection()? else {
        return Ok("*".to_owned());
    };
    let mut terms = Vec::with_capacity(columns.len());
    for column in columns {
        if !facts.columns.iter().any(|declared| declared == column) {
            return Err(unknown_projected(column));
        }
        terms.push(quote_identifier(column, STYLE));
    }
    Ok(terms.join(", "))
}

fn unknown_projected(column: &str) -> OxynError {
    OxynError::Query(format!(
        "cannot read `{column}` in a preview: the relation does not declare that column"
    ))
}

/// The `ORDER BY` terms, or `None` when nothing needs ordering.
///
/// The order does **not** depend on the page: the unique key completes the
/// requested sort even on the first page. Ordering page 0 by `name` alone and
/// page 1 by `name, id` would show a tied row twice and hide another, exactly
/// at the boundary nobody inspects. For the same reason a relation ordered by
/// default is ordered on its key from the first page, sort or no sort.
///
/// # Erreurs
/// [`OxynError::Query`] when a sort names a column the relation does not
/// declare — sent to the engine, it would be rejected far from the column that
/// caused it; retrying would not make the column appear — and [`OxynError::NotSupported`] when a page is asked
/// for without any unique key to make the order total.
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
            capability: "preview pagination: this relation declares no unique key, and an \
                         OFFSET without a total order repeats rows and skips others without \
                         saying so"
                .to_owned(),
        });
    }
    for key in &facts.key {
        // A column already sorted on makes the order total on its own; adding it
        // twice would only lengthen the clause.
        if shape.sort.iter().any(|sort| sort.column == *key) {
            continue;
        }
        terms.push(format!("{} ASC", quote_identifier(key, STYLE)));
    }
    Ok((!terms.is_empty()).then(|| terms.join(", ")))
}

#[cfg(test)]
mod tests;
