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
mod tests {
    use oxyn_catalog::model::{Field, LogicalType, RelationKind};
    use oxyn_core::PreviewSort;

    use super::*;

    /// A relation described as the catalog would return it.
    fn relation(columns: &[(&str, bool)]) -> Relation {
        let fields = columns
            .iter()
            .enumerate()
            .map(|(rank, (name, key))| {
                let position = u32::try_from(rank).expect("fewer than 2^32 test columns");
                let mut field = Field::new(*name, position, LogicalType::Text, "text");
                field.is_primary_key = *key;
                field
            })
            .collect::<Vec<_>>();
        Relation::new("t", RelationKind::Table).with_fields(fields)
    }

    /// The facts of an ordinary PostgreSQL table, whose previews the driver
    /// vouches come in key order.
    fn facts(columns: &[(&str, bool)]) -> RelationFacts {
        RelationFacts::of(&relation(columns).with_key_order_preview())
    }

    /// The facts as the typed column list gives them: the key in index order.
    fn typed(columns: &[&str], key: &[&str]) -> RelationFacts {
        RelationFacts::typed(
            columns.iter().map(|name| (*name).to_owned()).collect(),
            key.iter().map(|name| (*name).to_owned()).collect(),
            true,
        )
    }

    fn path() -> CatalogPath {
        CatalogPath::for_relation(None, Some("public"), "t").expect("valid path")
    }

    fn compose(shape: &PreviewShape, facts: &RelationFacts) -> Result<ExecRequest> {
        request("db", SqlDialect::Postgres, &path(), 200, shape, facts)
    }

    #[test]
    fn a_preview_without_request_composes_neither_where_nor_order_by_when_no_key_is_known() {
        let request =
            compose(&PreviewShape::unordered(), &RelationFacts::default()).expect("plain preview");
        assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
        assert!(request.params.is_empty());
        // A relation vouched for but without a key — a keyless table — is read
        // as the server returns it: no page could be offered over it either way.
        for keyless in [facts(&[("name", false)]), typed(&["name"], &[])] {
            let request = compose(&PreviewShape::unordered(), &keyless).expect("keyless preview");
            assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
        }
    }

    #[test]
    fn a_preview_without_request_comes_in_key_order() {
        for keyed in [
            facts(&[("id", true), ("name", false)]),
            typed(&["id", "name"], &["id"]),
        ] {
            let request = compose(&PreviewShape::unordered(), &keyed).expect("keyed preview");
            assert_eq!(
                request.text,
                "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" ASC LIMIT 200"
            );
        }
        // A filter changes the rows, not the order they come in.
        let filtered = PreviewShape {
            predicate: Some("name <> ''".into()),
            ..PreviewShape::default()
        };
        let request = compose(&filtered, &typed(&["id", "name"], &["id"])).expect("filtered");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" WHERE (name <> ''\n) ORDER BY \"id\" ASC LIMIT 200"
        );
    }

    #[test]
    fn a_composite_key_orders_in_index_order_not_table_order() {
        // `PRIMARY KEY (b, a)` over columns `a, b`: ordering `a, b` cannot use
        // the index and sorts the whole table to return 200 rows.
        let request = compose(
            &PreviewShape::unordered(),
            &typed(&["a", "b", "note"], &["b", "a"]),
        )
        .expect("composite key");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"b\" ASC, \"a\" ASC LIMIT 200"
        );
    }

    #[test]
    fn a_relation_not_vouched_for_is_ordered_only_when_asked() {
        // An inheritance parent: its key exists, but no child carries its index.
        let unvouched = [
            RelationFacts::of(&relation(&[("id", true), ("name", false)])),
            RelationFacts::typed(vec!["id".into(), "name".into()], vec!["id".into()], false),
        ];
        for facts in &unvouched {
            let plain = compose(&PreviewShape::unordered(), facts).expect("plain preview");
            assert_eq!(plain.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
            // Its key still completes a page, so the page stays honest.
            let page = PreviewShape {
                offset: 200,
                ..PreviewShape::default()
            };
            assert_eq!(
                compose(&page, facts).expect("page").text,
                "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
            );
        }
    }

    #[test]
    fn a_redshift_preview_is_never_ordered_by_default() {
        // What `describe_relation` returns on Redshift: a key, never the flag.
        let described = RelationFacts::of(&relation(&[("id", true), ("name", false)]));
        let request = request(
            "db",
            SqlDialect::Redshift,
            &path(),
            200,
            &PreviewShape::unordered(),
            &described,
        )
        .expect("redshift preview");
        assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
    }

    #[test]
    fn a_sort_is_quoted_and_completed_by_the_primary_key() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending("name")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("sort");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"name\" DESC, \"id\" ASC LIMIT 200"
        );
    }

    #[test]
    fn a_key_already_sorted_is_not_repeated() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending("id")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("sort");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" DESC LIMIT 200"
        );
    }

    #[test]
    fn a_composite_key_completes_the_order_in_catalog_order() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            offset: 400,
            ..PreviewShape::default()
        };
        let request =
            compose(&shape, &facts(&[("a", true), ("b", true), ("name", false)])).expect("page");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"name\" ASC, \"a\" ASC, \"b\" ASC \
             LIMIT 200 OFFSET 400"
        );
    }

    #[test]
    fn a_page_without_requested_sort_is_ordered_by_the_key_alone() {
        let shape = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("page");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
        );
    }

    #[test]
    fn a_hostile_sort_column_is_quoted_never_concatenated() {
        let path = CatalogPath::for_relation(None, Some("s\"; --"), "ta\"ble")
            .expect("legal hostile identifiers");
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("col\"onne")],
            ..PreviewShape::default()
        };
        let request = request(
            "db",
            SqlDialect::Postgres,
            &path,
            200,
            &shape,
            &facts(&[("col\"onne", false)]),
        )
        .expect("sort");
        assert_eq!(
            request.text,
            "SELECT * FROM \"s\"\"; --\".\"ta\"\"ble\" ORDER BY \"col\"\"onne\" ASC LIMIT 200"
        );
    }

    #[test]
    fn an_unknown_sort_column_is_refused_before_the_server() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("missing")],
            ..PreviewShape::default()
        };
        let error = compose(&shape, &facts(&[("id", true)])).expect_err("refusal expected");
        permanent_refusal(&error);
    }

    /// A column the relation does not declare will not reappear on the next
    /// attempt: the error is permanent, never `Transient`.
    fn permanent_refusal(error: &OxynError) {
        assert!(
            matches!(error, OxynError::Query(message) if message.contains("missing")),
            "{error}"
        );
        assert_eq!(error.class(), oxyn_core::ErrorClass::Permanent);
        assert!(!error.is_retryable(), "{error}");
    }

    #[test]
    fn a_page_without_unique_key_is_refused_saying_why() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            offset: 200,
            ..PreviewShape::default()
        };
        let error = compose(&shape, &facts(&[("name", false)])).expect_err("pagination impossible");
        let OxynError::NotSupported { capability } = &error else {
            panic!("refusal expected, got {error}");
        };
        assert!(capability.contains("unique key"), "{capability}");
        // The same relation stays viewable on its first page: that is today's
        // preview, and it has lost nothing.
        let first_value = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            ..PreviewShape::default()
        };
        assert!(compose(&first_value, &facts(&[("name", false)])).is_ok());
    }

    #[test]
    fn the_predicate_is_sent_as_is_in_parentheses_and_ends_its_line() {
        let shape = PreviewShape {
            // An end-of-line comment: without the newline, it would swallow the
            // ORDER BY and the LIMIT, and the preview would read the whole table.
            predicate: Some("amount > 100 -- over a hundred".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true)])).expect("predicate");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" WHERE (amount > 100 -- over a hundred\n\
             ) ORDER BY \"id\" ASC LIMIT 200"
        );
        assert!(request.limits.read_only);
    }

    #[test]
    fn an_unterminated_block_comment_cannot_swallow_the_limit() {
        // No newline ends a `/*`: the opening parenthesis is what makes the
        // statement incomplete, hence refused by the engine, instead of an
        // unbounded read that nothing would report.
        let shape = PreviewShape {
            predicate: Some("amount > 100 /*".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("predicate");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" WHERE (amount > 100 /*\n) LIMIT 200"
        );
    }

    #[test]
    fn an_already_parenthesized_predicate_keeps_its_meaning() {
        let shape = PreviewShape {
            predicate: Some("(a > 0 AND b < 2) OR c IS NULL".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("predicate");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" WHERE ((a > 0 AND b < 2) OR c IS NULL\n) LIMIT 200"
        );
    }

    #[test]
    fn an_empty_predicate_composes_no_where() {
        let shape = PreviewShape {
            predicate: Some("   ".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("empty predicate");
        assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
    }

    #[test]
    fn projected_columns_are_quoted_and_keep_their_names() {
        let path = CatalogPath::for_relation(None, Some("public"), "t").expect("path");
        let request = request_with_columns(
            "db",
            SqlDialect::Postgres,
            &path,
            1,
            &[
                ("id".into(), false),
                ("acl\"; DROP TABLE audit; --".into(), true),
            ],
            &PreviewShape::unordered(),
            &RelationFacts::default(),
        )
        .expect("preview");
        assert_eq!(
            request.text,
            "SELECT \"id\", \"acl\"\"; DROP TABLE audit; --\"::pg_catalog.text AS \"acl\"\"; DROP TABLE audit; --\" FROM \"public\".\"t\" LIMIT 1"
        );
        assert_eq!(request.limits.max_rows, Some(1));
        assert!(request.limits.read_only);
    }

    #[test]
    fn a_projection_reads_only_the_named_columns_and_keeps_their_conversion() {
        let hostile = "acl\"; DROP TABLE audit; --";
        let shape = PreviewShape {
            columns: Some(vec![hostile.into(), "id".into(), hostile.into()]),
            ..PreviewShape::default()
        };
        let request = request_with_columns(
            "db",
            SqlDialect::Postgres,
            &path(),
            5,
            &[
                ("id".into(), false),
                ("secret".into(), false),
                (hostile.into(), true),
            ],
            &shape,
            &RelationFacts::default(),
        )
        .expect("projection");
        // `secret`, not requested, is not read; the duplicate is not composed.
        assert_eq!(
            request.text,
            "SELECT \"acl\"\"; DROP TABLE audit; --\"::pg_catalog.text AS \
             \"acl\"\"; DROP TABLE audit; --\", \"id\" FROM \"public\".\"t\" LIMIT 5"
        );
        assert!(request.limits.read_only);
    }

    #[test]
    fn a_projection_without_typed_list_is_checked_against_the_description() {
        // Redshift: no typed list is read, the description stands in for it.
        let shape = PreviewShape {
            columns: Some(vec!["name".into()]),
            ..PreviewShape::default()
        };
        let request = request(
            "db",
            SqlDialect::Redshift,
            &path(),
            5,
            &shape,
            // Redshift's description never carries the flag.
            &RelationFacts::of(&relation(&[("id", true), ("name", false)])),
        )
        .expect("projection");
        assert_eq!(
            request.text,
            "SELECT \"name\" FROM \"public\".\"t\" LIMIT 5"
        );
    }

    #[test]
    fn an_unknown_projected_column_is_refused_before_the_server() {
        let shape = PreviewShape {
            columns: Some(vec!["missing".into(), "id".into()]),
            ..PreviewShape::default()
        };
        let typed = request_with_columns(
            "db",
            SqlDialect::Postgres,
            &path(),
            5,
            &[("id".into(), false)],
            &shape,
            &RelationFacts::default(),
        );
        let described_relation = compose(&shape, &facts(&[("id", true)]));
        // Nothing read, nothing known: a name does not pass for lack of checking.
        let nothing = compose(&shape, &RelationFacts::default());
        for error in [typed, described_relation, nothing] {
            permanent_refusal(&error.expect_err("refusal expected"));
        }
        let empty_shape = PreviewShape {
            columns: Some(Vec::new()),
            ..PreviewShape::default()
        };
        assert!(matches!(
            compose(&empty_shape, &facts(&[("id", true)])),
            Err(OxynError::Config(_))
        ));
    }

    #[test]
    fn preview_quotes_schema_and_relation_but_never_qualifies_with_database() {
        let path = CatalogPath::for_relation(
            Some("db\"; ignored"),
            Some("s\"; --"),
            "t\"; DROP TABLE audit; --",
        )
        .expect("valid hostile identifiers");
        for dialect in [SqlDialect::Postgres, SqlDialect::Redshift] {
            let request = request(
                "db\"; ignored",
                dialect,
                &path,
                200,
                &PreviewShape::unordered(),
                &RelationFacts::default(),
            )
            .expect("current database");
            assert_eq!(
                request.text,
                "SELECT * FROM \"s\"\"; --\".\"t\"\"; DROP TABLE audit; --\" LIMIT 200"
            );
            assert_eq!(request.language, QueryLanguage::Sql(dialect));
            assert!(request.limits.read_only);
            assert_eq!(request.limits.max_rows, Some(200));
            assert!(!request.is_mutating());
        }
    }

    #[test]
    fn preview_refuses_other_databases_missing_levels_and_invalid_limits() {
        let dialect = SqlDialect::Postgres;
        let plain = PreviewShape::unordered();
        let facts = RelationFacts::default();
        let path =
            CatalogPath::for_relation(Some("other"), Some("public"), "t").expect("valid path");
        assert!(matches!(
            request("current", dialect, &path, 200, &plain, &facts),
            Err(OxynError::CatalogUnavailable(_))
        ));
        let path = CatalogPath::for_relation(None, Some("public"), "t").expect("valid path");
        for limit in [0, 1001, u32::MAX] {
            assert!(matches!(
                request("current", dialect, &path, limit, &plain, &facts),
                Err(OxynError::Config(_))
            ));
        }
        for limit in [1, 1000] {
            assert!(request("current", dialect, &path, limit, &plain, &facts).is_ok());
        }
        let no_schema = CatalogPath::for_relation(None, None, "t").expect("valid path");
        assert!(request("current", dialect, &no_schema, 200, &plain, &facts).is_err());
        assert!(
            request(
                "current",
                dialect,
                &CatalogPath::empty(),
                200,
                &plain,
                &facts
            )
            .is_err()
        );
    }
}
