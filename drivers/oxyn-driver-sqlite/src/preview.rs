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
mod tests {
    use oxyn_catalog::model::{Field, LogicalType, RelationKind};
    use oxyn_core::PreviewSort;

    use super::*;

    /// A relation described as the catalog would return it.
    fn facts(columns: &[(&str, bool)]) -> RelationFacts {
        let fields = columns
            .iter()
            .enumerate()
            .map(|(rang, (name, key))| {
                let position = u32::try_from(rang).expect("fewer than 2^32 test columns");
                let mut field = Field::new(*name, position, LogicalType::Text, "TEXT");
                field.is_primary_key = *key;
                field
            })
            .collect::<Vec<_>>();
        RelationFacts::of(
            &Relation::new("t", RelationKind::Table)
                .with_fields(fields)
                .with_key_order_preview(),
        )
    }

    fn path() -> CatalogPath {
        CatalogPath::for_relation(None, None, "t").expect("valid path")
    }

    fn compose(shape: &PreviewShape, facts: &RelationFacts) -> Result<ExecRequest> {
        request(&path(), 200, shape, facts)
    }

    #[test]
    fn a_preview_without_request_composes_neither_where_nor_order_by_when_no_key_is_known() {
        let request =
            compose(&PreviewShape::unordered(), &RelationFacts::default()).expect("simple preview");
        assert_eq!(request.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
        assert!(request.params.is_empty());
        // A relation described without a key — a view — is read as the engine
        // returns it: no page could be offered over it either way.
        let keyless = compose(&PreviewShape::unordered(), &facts(&[("name", false)]))
            .expect("keyless preview");
        assert_eq!(keyless.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
    }

    #[test]
    fn a_preview_without_request_comes_in_key_order() {
        let request = compose(
            &PreviewShape::unordered(),
            &facts(&[("id", true), ("name", false)]),
        )
        .expect("keyed preview");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" ASC LIMIT 200"
        );
        // A filter changes the rows, not the order they come in.
        let filtered = PreviewShape {
            predicate: Some("name <> ''".into()),
            ..PreviewShape::default()
        };
        let request = compose(&filtered, &facts(&[("id", true), ("name", false)]))
            .expect("filtered keyed preview");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" WHERE (name <> ''\n) ORDER BY \"id\" ASC LIMIT 200"
        );
    }

    #[test]
    fn a_composite_key_orders_on_every_column_in_position_order() {
        let request = compose(
            &PreviewShape::unordered(),
            &facts(&[("order", true), ("line", true), ("note", false)]),
        )
        .expect("composite key");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"order\" ASC, \"line\" ASC LIMIT 200"
        );
    }

    #[test]
    fn a_virtual_table_is_ordered_only_when_the_user_sorts_it() {
        // Ordering a `vec0` table makes SQLite read every vector into a
        // temporary b-tree before the first page: that cost is the user's to
        // ask for, not the price of opening the table.
        let fields = vec![
            Field::new("chunk_id", 0, LogicalType::Text, "TEXT").primary_key(),
            Field::new("embedding", 1, LogicalType::Text, "TEXT"),
        ];
        // What `describe_relation` returns for a virtual table: no flag.
        let relation = Relation::new("t", RelationKind::Table).with_fields(fields);
        let facts = RelationFacts::of(&relation);

        let plain = compose(&PreviewShape::unordered(), &facts).expect("plain virtual preview");
        assert_eq!(plain.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");

        let sorted = PreviewShape {
            sort: vec![PreviewSort::ascending("chunk_id")],
            ..PreviewShape::default()
        };
        let request = compose(&sorted, &facts).expect("sorted virtual preview");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"chunk_id\" ASC LIMIT 200"
        );
        // Its key still completes a sort, so a page over it stays honest.
        let page = PreviewShape {
            offset: 200,
            ..sorted
        };
        assert!(compose(&page, &facts).is_ok());
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
            "SELECT * FROM \"main\".\"t\" ORDER BY \"name\" DESC, \"id\" ASC LIMIT 200"
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
            "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" DESC LIMIT 200"
        );
    }

    #[test]
    fn a_page_without_requested_sort_orders_on_the_key_alone() {
        let shape = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("page");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
        );
    }

    #[test]
    fn a_hostile_sort_column_is_quoted_never_concatenated() {
        let path = CatalogPath::for_relation(None, Some("db\"; --"), "ta\"ble")
            .expect("legal hostile identifiers");
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("col\"umn")],
            ..PreviewShape::default()
        };
        let request = request(&path, 200, &shape, &facts(&[("col\"umn", false)])).expect("sort");
        assert_eq!(
            request.text,
            "SELECT * FROM \"db\"\"; --\".\"ta\"\"ble\" ORDER BY \"col\"\"umn\" ASC LIMIT 200"
        );
    }

    #[test]
    fn an_unknown_sort_column_is_refused_before_the_engine() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("missing")],
            ..PreviewShape::default()
        };
        let refused_with = compose(&shape, &facts(&[("id", true)])).expect_err("expected refusal");
        assert_permanent_refusal(&refused_with);
    }

    /// A column the relation does not declare will not reappear on the next
    /// attempt: the error is permanent, never `Transient`.
    fn assert_permanent_refusal(refused_with: &OxynError) {
        assert!(
            matches!(refused_with, OxynError::Query(message) if message.contains("missing")),
            "{refused_with}"
        );
        assert_eq!(refused_with.class(), oxyn_core::ErrorClass::Permanent);
        assert!(!refused_with.is_retryable(), "{refused_with}");
    }

    #[test]
    fn a_page_without_unique_key_is_refused_saying_why() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            offset: 200,
            ..PreviewShape::default()
        };
        let refused_with =
            compose(&shape, &facts(&[("name", false)])).expect_err("no pagination possible");
        let OxynError::NotSupported { capability } = &refused_with else {
            panic!("expected refusal, got {refused_with}");
        };
        assert!(capability.contains("unique key"), "{capability}");
        // The same relation stays viewable on its first page: that is today's
        // preview, and it has lost nothing.
        let first_page = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            ..PreviewShape::default()
        };
        assert!(compose(&first_page, &facts(&[("name", false)])).is_ok());
    }

    #[test]
    fn the_predicate_goes_as_is_in_parentheses_and_ends_its_line() {
        let shape = PreviewShape {
            // An end-of-line comment: without the line break, it would swallow
            // the ORDER BY and the LIMIT, and the preview would read the whole table.
            predicate: Some("amount > 100 -- beyond a hundred".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true)])).expect("predicate");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" WHERE (amount > 100 -- beyond a hundred\n\
             ) ORDER BY \"id\" ASC LIMIT 200"
        );
        assert!(request.limits.read_only);
    }

    #[test]
    fn an_unclosed_block_comment_cannot_swallow_the_limit() {
        // No line break ends a `/*`: the opening parenthesis is what makes the
        // statement incomplete, hence refused by the engine, instead of an
        // unbounded read nothing would report.
        let shape = PreviewShape {
            predicate: Some("amount > 100 /*".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("predicate");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" WHERE (amount > 100 /*\n) LIMIT 200"
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
            "SELECT * FROM \"main\".\"t\" WHERE ((a > 0 AND b < 2) OR c IS NULL\n) LIMIT 200"
        );
    }

    #[test]
    fn an_empty_predicate_composes_no_where() {
        let shape = PreviewShape {
            predicate: Some("   ".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("empty predicate");
        assert_eq!(request.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
    }

    #[test]
    fn a_projection_reads_only_the_named_columns_quoted_in_order() {
        let hostile = "e\"; DROP TABLE audit; --";
        let shape = PreviewShape {
            columns: Some(vec![hostile.into(), "id".into(), hostile.into()]),
            sort: vec![PreviewSort::ascending("name")],
            ..PreviewShape::default()
        };
        let request = compose(
            &shape,
            &facts(&[("id", true), ("name", false), (hostile, false)]),
        )
        .expect("projection");
        assert_eq!(
            request.text,
            "SELECT \"e\"\"; DROP TABLE audit; --\", \"id\" FROM \"main\".\"t\" \
             ORDER BY \"name\" ASC, \"id\" ASC LIMIT 200"
        );
        assert!(request.limits.read_only);
    }

    #[test]
    fn an_unknown_projected_column_or_an_empty_projection_is_refused() {
        let missing = PreviewShape {
            columns: Some(vec!["id".into(), "missing".into()]),
            ..PreviewShape::default()
        };
        let refused_with =
            compose(&missing, &facts(&[("id", true)])).expect_err("expected refusal");
        assert_permanent_refusal(&refused_with);
        let no_columns = PreviewShape {
            columns: Some(Vec::new()),
            ..PreviewShape::default()
        };
        assert!(matches!(
            compose(&no_columns, &facts(&[("id", true)])),
            Err(OxynError::Config(_))
        ));
    }

    #[test]
    fn preview_quotes_database_and_relation_and_supports_catalog_paths() {
        for path in [
            CatalogPath::for_relation(Some("db\"; --"), None, "t\"; DROP TABLE audit; --"),
            CatalogPath::for_relation(None, Some("db\"; --"), "t\"; DROP TABLE audit; --"),
            CatalogPath::for_relation(
                Some("db\"; --"),
                Some("db\"; --"),
                "t\"; DROP TABLE audit; --",
            ),
        ] {
            let request = request(
                &path.expect("valid path"),
                200,
                &PreviewShape::unordered(),
                &RelationFacts::default(),
            )
            .expect("valid preview");
            assert_eq!(
                request.text,
                "SELECT * FROM \"db\"\"; --\".\"t\"\"; DROP TABLE audit; --\" LIMIT 200"
            );
            assert_eq!(request.language, QueryLanguage::Sql(SqlDialect::Sqlite));
            assert_eq!(request.limits.max_rows, Some(200));
            assert!(request.limits.read_only);
        }
    }

    #[test]
    fn preview_rejects_invalid_limits_and_conflicting_database_levels() {
        let plain = PreviewShape::unordered();
        let facts = RelationFacts::default();
        let path = CatalogPath::for_relation(None, None, "t").expect("valid path");
        for limit in [0, 1001, u32::MAX] {
            assert!(matches!(
                request(&path, limit, &plain, &facts),
                Err(OxynError::Config(_))
            ));
        }
        for limit in [1, 1000] {
            assert!(request(&path, limit, &plain, &facts).is_ok());
        }
        assert!(request(&CatalogPath::empty(), 200, &plain, &facts).is_err());
        let path = CatalogPath::for_relation(Some("db"), Some("imaginary_schema"), "t")
            .expect("valid path");
        assert!(matches!(
            request(&path, 200, &plain, &facts),
            Err(OxynError::CatalogUnavailable(_))
        ));
    }
}
