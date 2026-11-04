//! The statement behind a click on a table: a bounded `SELECT` Oxyn composes.
//!
//! The SQL **Oxyn composes** never concatenates a received name: relation and
//! column names go through [`quote_identifier`], values are literals of the
//! page (`LIMIT`, `OFFSET`) or the user's own predicate
//! ([I-10](../../../CLAUDE.md#i-10), DRIVER-CONTRACT §6). The one exception is
//! the predicate, which is the user's SQL and which four barriers bound
//! (ADR-0020): the executor reclassifies it, a read is never allowed to write,
//! the rows are bounded, and the fragment is parenthesized.
//!
//! The composition is the SQLite driver's, because the dialect is: this crate
//! may not depend on that one, and nothing here is specific to Turso except
//! that there is one database, `main`.

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result, SqlDialect,
    StatementIntent,
};

use crate::catalog::MAIN;

const STYLE: QuoteStyle = QuoteStyle::Double;

/// What a relation declares, as a preview needs it.
#[derive(Debug, Default)]
pub(crate) struct RelationFacts {
    columns: Vec<String>,
    key: Vec<String>,
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
            // In the index's order, not the table's (ADR-0068).
            key: relation
                .primary_key()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            ordered_by_default: relation.previews_in_key_order(),
        }
    }
}

/// The database a path names: there is only `main`.
pub(crate) fn database_of(path: &CatalogPath) -> Result<&str> {
    match (path.catalog(), path.namespace()) {
        (Some(catalog), Some(namespace)) if catalog != namespace => Err(
            OxynError::CatalogUnavailable("a preview path has conflicting database levels".into()),
        ),
        (Some(database), _) | (None, Some(database)) if database != MAIN => Err(OxynError::Query(
            "a Turso connection has one database, `main`".to_owned(),
        )),
        _ => Ok(MAIN),
    }
}

/// The request that reads `limit` rows of the relation `path` names.
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
        // catches what the other lets through. The newline ends a trailing `--`
        // comment, which would swallow the ORDER BY and the LIMIT. The
        // parentheses catch an unterminated `/*`, which no newline ends: SQLite
        // then refuses the statement instead of running an unbounded scan.
        // `WHERE (X)` means exactly `WHERE X` for any boolean expression.
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
        // A column already sorted on makes the order total on its own.
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

    use super::*;

    fn shape() -> PreviewShape {
        PreviewShape::unordered()
    }

    fn table(keyed: bool) -> Relation {
        let mut id = Field::new("id", 0, LogicalType::INT64, "INTEGER");
        if keyed {
            id.is_primary_key = true;
            id.primary_key_rank = Some(1);
        }
        let name = Field::new("na\"me", 1, LogicalType::Text, "TEXT");
        let relation = Relation::new("t", RelationKind::Table).with_fields(vec![id, name]);
        if keyed {
            relation.with_key_order_preview()
        } else {
            relation
        }
    }

    fn path(name: &str) -> CatalogPath {
        CatalogPath::for_relation(None, Some(MAIN), name).expect("path")
    }

    #[test]
    fn a_hostile_relation_name_is_quoted_not_concatenated() {
        let evil = "users\"; DROP TABLE audit; --";
        let request =
            request(&path(evil), 10, &shape(), &RelationFacts::of(&table(false))).expect("request");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"users\"\"; DROP TABLE audit; --\" LIMIT 10"
        );
        assert_eq!(request.intent, StatementIntent::Read);
        assert_eq!(request.limits.max_rows, Some(10));
    }

    #[test]
    fn a_keyed_relation_previews_in_key_order() {
        let request =
            request(&path("t"), 5, &shape(), &RelationFacts::of(&table(true))).expect("request");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" ASC LIMIT 5"
        );
    }

    #[test]
    fn the_predicate_is_parenthesized_after_a_newline() {
        let mut shaped = shape();
        shaped.predicate = Some("id > 1 -- trailing".to_owned());
        let request =
            request(&path("t"), 5, &shaped, &RelationFacts::of(&table(false))).expect("request");
        assert!(
            request
                .text
                .contains("WHERE (id > 1 -- trailing\n) LIMIT 5"),
            "{}",
            request.text
        );
    }

    #[test]
    fn a_column_the_relation_does_not_declare_is_a_permanent_refusal() {
        let mut shaped = shape();
        shaped.sort = vec![oxyn_core::PreviewSort::ascending("nope")];
        let error = request(&path("t"), 5, &shaped, &RelationFacts::of(&table(true)))
            .expect_err("unknown column");
        assert!(matches!(error, OxynError::Query(_)), "{error:?}");
    }

    #[test]
    fn another_database_than_main_is_refused() {
        let other = CatalogPath::for_relation(None, Some("other"), "t").expect("path");
        let error = request(&other, 5, &shape(), &RelationFacts::of(&table(false)))
            .expect_err("one database");
        assert!(matches!(error, OxynError::Query(_)), "{error:?}");
    }

    #[test]
    fn the_limit_is_bounded() {
        for limit in [0, 1001] {
            assert!(
                request(&path("t"), limit, &shape(), &RelationFacts::default()).is_err(),
                "{limit}"
            );
        }
    }
}
