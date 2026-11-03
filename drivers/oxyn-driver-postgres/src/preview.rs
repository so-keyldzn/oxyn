//! Pure SQL composition for relation previews.
//!
//! Two halves that do not look alike ([ADR-0020]): the **order** and the
//! **projection** are structured, so every column is quoted here and refused
//! when the relation does not declare it; the **predicate** is SQL the user wrote, and travels through
//! untouched — neither parsed nor rewritten.
//!
//! [ADR-0020]: ../../../docs/adr/0020-apercu-trie-filtre-parcouru.md

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result, SqlDialect,
    StatementIntent,
};

/// The typed column list of a relation, with what ordering it needs: one round
/// trip carries the whole preview's metadata on PostgreSQL.
///
/// Per column: its name, whether it is read as text, and its rank in the
/// primary key (`NULL` outside it). Repeated on every row: the relation's
/// `relkind` and `relhassubclass`, from which
/// [`crate::catalog::previews_in_key_order`] decides whether a preview comes in
/// key order — the same decision `describe_relation` declares, so the order and
/// the page the interface offers agree.
///
/// The rank is the column's place **in the index**, not in the table: ordering
/// a key declared `(b, a)` as `a, b` cannot use the index, and reads and sorts
/// the whole table to return 200 rows (seen on 17.11, 2026-10-26).
///
/// Array elements and domain bases are resolved without asking SQLx to decode
/// their type metadata. UNION also stops cycles in malformed catalogs.
pub(crate) const SQL_COLUMNS: &str = "\
WITH RECURSIVE target AS ( \
    SELECT c.oid, c.relkind, c.relhassubclass \
    FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
    WHERE n.nspname = $1 AND c.relname = $2 \
), column_types(attnum, attname, type_oid) AS ( \
    SELECT a.attnum, a.attname, a.atttypid \
    FROM pg_catalog.pg_attribute a JOIN target ON target.oid = a.attrelid \
    WHERE a.attnum > 0 AND NOT a.attisdropped \
    UNION \
    SELECT c.attnum, c.attname, \
           CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE t.typelem END \
    FROM column_types c JOIN pg_catalog.pg_type t ON t.oid = c.type_oid \
    WHERE t.typtype = 'd' OR (t.typcategory = 'A' AND t.typelem <> 0) \
), key_columns(attnum, rank) AS ( \
    SELECT k.attnum, k.rank \
    FROM pg_catalog.pg_index i JOIN target ON target.oid = i.indrelid \
    CROSS JOIN LATERAL pg_catalog.unnest(i.indkey) WITH ORDINALITY AS k(attnum, rank) \
    WHERE i.indisprimary \
) \
SELECT c.attname::text, \
       bool_or(t.typsend = 0 OR t.typcategory = 'Z' OR \
         (n.nspname = 'pg_catalog' AND t.typname IN \
           ('regproc', 'regprocedure', 'regoper', 'regoperator', 'regclass', \
            'regtype', 'regconfig', 'regdictionary', 'regnamespace', \
            'regrole', 'regcollation', 'int2vector', 'oidvector'))), \
       min(k.rank), \
       (SELECT target.relkind::text FROM target), \
       (SELECT target.relhassubclass FROM target) \
FROM column_types c JOIN pg_catalog.pg_type t ON t.oid = c.type_oid \
JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
LEFT JOIN key_columns k ON k.attnum = c.attnum \
GROUP BY c.attnum, c.attname ORDER BY c.attnum";

/// What the catalog says about a relation, as far as ordering — and, on
/// Redshift, projecting — needs it.
///
/// On PostgreSQL it comes with the typed column list, in the same round trip,
/// on every preview: one with no sort asked for still comes in key order
/// ([ADR-0068](../../../docs/adr/0068-a-preview-comes-in-key-order.md)). On
/// Redshift it is read only when the shape depends on it, and never orders a
/// preview by default. An empty [`Self::columns`] means « nothing was read »:
/// on Redshift, a shape that asks for neither order nor projection.
#[derive(Debug, Default)]
pub(crate) struct RelationFacts {
    /// Column names, as the catalog spells them.
    columns: Vec<String>,
    /// The columns of a known unique key, empty when none is declared.
    key: Vec<String>,
    /// Whether the key orders the preview when no sort is asked for: see
    /// [`Relation::previews_in_key_order`].
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

    /// The facts read with the typed column list.
    ///
    /// `key` is in index order. `key_order` is the driver's word that this
    /// relation's previews come in key order; without a key it orders nothing,
    /// exactly as [`Relation::previews_in_key_order`] decides.
    pub(crate) fn typed(columns: Vec<String>, key: Vec<String>, key_order: bool) -> Self {
        let ordered_by_default = key_order && !key.is_empty();
        Self {
            columns,
            key,
            ordered_by_default,
        }
    }
}

pub(crate) fn request(
    database: &str,
    dialect: SqlDialect,
    path: &CatalogPath,
    limit: u32,
    shape: &PreviewShape,
    facts: &RelationFacts,
) -> Result<ExecRequest> {
    request_with_columns(database, dialect, path, limit, &[], shape, facts)
}

pub(crate) fn request_with_columns(
    database: &str,
    dialect: SqlDialect,
    path: &CatalogPath,
    limit: u32,
    columns: &[(String, bool)],
    shape: &PreviewShape,
    facts: &RelationFacts,
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    if path.catalog().is_some_and(|catalog| catalog != database) {
        return Err(OxynError::CatalogUnavailable(
            "preview catalog differs from the connected database".into(),
        ));
    }
    let namespace = path.namespace().ok_or_else(|| {
        OxynError::CatalogUnavailable("PostgreSQL preview requires a schema".into())
    })?;
    let relation = path
        .relation()
        .ok_or_else(|| OxynError::CatalogUnavailable("preview path must name a relation".into()))?;
    // PostgreSQL resolves only schema + relation within the connected database.
    let qualified =
        CatalogPath::for_relation(None, Some(namespace), relation)?.qualify_sql(dialect);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let style = QuoteStyle::for_dialect(dialect);
    let projection = select_list(columns, shape, facts, style)?;
    let order = order_by(shape, facts, style)?;

    let mut text = format!("SELECT {projection} FROM {qualified}");
    if let Some(predicate) = shape.predicate() {
        // Two guards around a fragment the driver does not parse, and each one
        // catches what the other lets through.
        //
        // The newline ends a trailing `--` comment, which would otherwise
        // swallow the ORDER BY and the LIMIT that follow.
        //
        // The parentheses catch the unterminated `/*`, which no newline ends.
        // PostgreSQL already refuses that one on its own, but the guard belongs
        // here too: SQLite does not, and a preview that silently drops its LIMIT
        // on one engine must not depend on the other engine being stricter.
        // `WHERE (X)` means exactly `WHERE X` for any boolean expression, so no
        // legitimate predicate changes meaning.
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

    Ok(ExecRequest::new(QueryLanguage::Sql(dialect), text)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

/// The select list: every column or the requested ones, each quoted, and cast
/// to text when `columns` says the type has no binary form.
///
/// `columns` is the typed list read from the server, empty when none was read
/// (Redshift, or a caller that needs no cast): a requested name is then
/// checked against `facts` instead.
///
/// # Errors
/// Those of [`PreviewShape::projection`], and [`OxynError::Query`] when a name
/// is not a column of the relation — the same permanent refusal as an unknown
/// sort column, before the server sees it.
fn select_list(
    columns: &[(String, bool)],
    shape: &PreviewShape,
    facts: &RelationFacts,
    style: QuoteStyle,
) -> Result<String> {
    let render = |name: &str, as_text: bool| {
        let name = quote_identifier(name, style);
        if as_text {
            format!("{name}::pg_catalog.text AS {name}")
        } else {
            name
        }
    };
    let Some(requested) = shape.projection()? else {
        if columns.is_empty() {
            return Ok("*".to_owned());
        }
        return Ok(columns
            .iter()
            .map(|(name, as_text)| render(name, *as_text))
            .collect::<Vec<_>>()
            .join(", "));
    };
    let mut terms = Vec::with_capacity(requested.len());
    for name in requested {
        let as_text = if columns.is_empty() {
            facts
                .columns
                .iter()
                .any(|declared| declared == name)
                .then_some(false)
        } else {
            columns
                .iter()
                .find(|(declared, _)| declared == name)
                .map(|(_, as_text)| *as_text)
        };
        let Some(as_text) = as_text else {
            return Err(OxynError::Query(format!(
                "cannot read `{name}` in a preview: the relation does not declare that column"
            )));
        };
        terms.push(render(name, as_text));
    }
    Ok(terms.join(", "))
}

/// The `ORDER BY` terms, or `None` when nothing needs ordering.
///
/// The order does **not** depend on the page: the unique key completes the
/// requested sort even on the first page. Ordering page 0 by `name` alone and
/// page 1 by `name, id` would show a tied row twice and hide another, exactly
/// at the boundary nobody inspects. For the same reason a relation ordered by
/// default is ordered on its key from the first page, sort or no sort.
///
/// # Errors
/// [`OxynError::Query`] when a sort names a column the relation does not
/// declare — sent to the server, it would be rejected far from the column that
/// caused it; retrying would not make the column appear — and [`OxynError::NotSupported`] when a page is asked
/// for without any unique key to make the order total.
fn order_by(
    shape: &PreviewShape,
    facts: &RelationFacts,
    style: QuoteStyle,
) -> Result<Option<String>> {
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
            quote_identifier(&sort.column, style)
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
        terms.push(format!("{} ASC", quote_identifier(key, style)));
    }
    Ok((!terms.is_empty()).then(|| terms.join(", ")))
}

#[cfg(test)]
mod tests;
