//! Composing a relation preview in MySQL's dialect.
//!
//! Two halves that do not look alike ([ADR-0020]): the **order** and the
//! **projection** are structured, so every column is quoted with backticks and
//! refused when the relation does not declare it; the **predicate** is SQL the
//! user wrote, and travels untouched. What keeps the predicate honest is not
//! this module: the prepare proves the statement is one statement, the session
//! is `READ ONLY` for it, and the row bound applies.
//!
//! [ADR-0020]: ../../../docs/adr/0020-apercu-trie-filtre-parcouru.md

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{ExecLimits, ExecRequest, OxynError, PreviewShape, Result, StatementIntent};

use crate::variant::LANGUAGE;

/// MySQL quotes identifiers with backticks.
const STYLE: QuoteStyle = QuoteStyle::Backtick;

/// What the catalog says about a relation, as far as ordering and projecting
/// need it. Read only when the shape needs it: an empty value means « nothing
/// was read ».
#[derive(Debug, Default)]
pub(crate) struct RelationFacts {
    columns: Vec<String>,
    /// The primary key's columns, empty when none is declared.
    key: Vec<String>,
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
        }
    }
}

/// The preview statement.
///
/// # Errors
/// [`OxynError::Config`] for a limit outside `1..=1000`,
/// [`OxynError::CatalogUnavailable`] for a path that names no database and
/// relation, [`OxynError::Query`] for a column the relation does not declare,
/// [`OxynError::NotSupported`] for a page without a unique key.
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
    let (Some(database), Some(relation), None) =
        (path.namespace(), path.relation(), path.catalog())
    else {
        return Err(OxynError::CatalogUnavailable(
            "a MySQL preview path names a database and a relation".into(),
        ));
    };
    let qualified = CatalogPath::for_relation(None, Some(database), relation)?.qualify(STYLE);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let projection = projection(shape, facts)?;
    let order = order_by(shape, facts)?;

    let mut text = format!("SELECT {projection} FROM {qualified}");
    if let Some(predicate) = shape.predicate() {
        // The newline ends a trailing `--` or `#` comment, which would
        // otherwise swallow the ORDER BY and the LIMIT; the parentheses turn
        // an unterminated `/*` into a syntax error instead of an unbounded
        // read. `WHERE (X)` means `WHERE X` for any boolean expression.
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

    Ok(ExecRequest::new(LANGUAGE, text)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

/// `*`, or the requested columns, each quoted and checked.
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

/// The `ORDER BY` terms, completed by the primary key so that pages never
/// repeat nor skip a row, or `None` when nothing needs ordering.
fn order_by(shape: &PreviewShape, facts: &RelationFacts) -> Result<Option<String>> {
    if shape.sort.is_empty() && shape.offset == 0 {
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
mod tests {
    use oxyn_catalog::model::{Field, LogicalType, RelationKind};
    use oxyn_core::{ErrorClass, PreviewSort};

    use super::*;

    fn facts(columns: &[(&str, bool)]) -> RelationFacts {
        let fields = columns
            .iter()
            .enumerate()
            .map(|(rank, (name, key))| {
                let position = u32::try_from(rank).expect("fewer than 2^32 test columns");
                let field = Field::new(*name, position, LogicalType::Text, "varchar(10)");
                if *key { field.primary_key() } else { field }
            })
            .collect::<Vec<_>>();
        RelationFacts::of(&Relation::new("t", RelationKind::Table).with_fields(fields))
    }

    fn path() -> CatalogPath {
        CatalogPath::for_relation(None, Some("shop"), "t").expect("valid path")
    }

    #[test]
    fn a_plain_preview_is_bounded_and_read_only() {
        let request = request(
            &path(),
            200,
            &PreviewShape::unordered(),
            &RelationFacts::default(),
        )
        .expect("plain preview");
        assert_eq!(request.text, "SELECT * FROM `shop`.`t` LIMIT 200");
        assert!(request.limits.read_only);
        assert_eq!(request.limits.max_rows, Some(200));
        assert_eq!(request.language, LANGUAGE);
    }

    #[test]
    fn hostile_names_are_quoted_never_concatenated() {
        let hostile = "x`; DROP TABLE audit; --";
        let path = CatalogPath::for_relation(None, Some("shop"), hostile).expect("legal name");
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending(hostile)],
            ..PreviewShape::default()
        };
        let request =
            request(&path, 10, &shape, &facts(&[(hostile, true)])).expect("quoted preview");
        assert_eq!(
            request.text,
            "SELECT * FROM `shop`.`x``; DROP TABLE audit; --` \
             ORDER BY `x``; DROP TABLE audit; --` DESC LIMIT 10"
        );
    }

    #[test]
    fn a_sort_is_completed_by_the_primary_key_and_a_page_needs_one() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            offset: 50,
            ..PreviewShape::default()
        };
        let request = request(
            &path(),
            50,
            &shape,
            &facts(&[("id", true), ("name", false)]),
        )
        .expect("page");
        assert_eq!(
            request.text,
            "SELECT * FROM `shop`.`t` ORDER BY `name` ASC, `id` ASC LIMIT 50 OFFSET 50"
        );
        let refused = request_err(&shape, &facts(&[("name", false)]));
        assert!(
            matches!(refused, OxynError::NotSupported { .. }),
            "{refused:?}"
        );
    }

    fn request_err(shape: &PreviewShape, facts: &RelationFacts) -> OxynError {
        request(&path(), 50, shape, facts).expect_err("refusal expected")
    }

    #[test]
    fn unknown_columns_are_refused_permanently() {
        let sort = PreviewShape {
            sort: vec![PreviewSort::ascending("missing")],
            ..PreviewShape::default()
        };
        let projected = PreviewShape {
            columns: Some(vec!["missing".to_owned()]),
            ..PreviewShape::default()
        };
        for shape in [sort, projected] {
            let refused = request_err(&shape, &facts(&[("id", true)]));
            assert!(matches!(refused, OxynError::Query(_)), "{refused:?}");
            assert_eq!(refused.class(), ErrorClass::Permanent);
        }
    }

    #[test]
    fn the_predicate_goes_as_is_in_parentheses_and_ends_its_line() {
        let shape = PreviewShape {
            predicate: Some("amount > 10 # trailing".to_owned()),
            ..PreviewShape::default()
        };
        let request = request(&path(), 10, &shape, &RelationFacts::default()).expect("filter");
        assert_eq!(
            request.text,
            "SELECT * FROM `shop`.`t` WHERE (amount > 10 # trailing\n) LIMIT 10"
        );
    }

    #[test]
    fn a_projection_reads_only_the_named_columns() {
        let shape = PreviewShape {
            columns: Some(vec!["name".to_owned(), "id".to_owned(), "name".to_owned()]),
            ..PreviewShape::default()
        };
        let request = request(
            &path(),
            10,
            &shape,
            &facts(&[("id", true), ("name", false)]),
        )
        .expect("projection");
        assert_eq!(request.text, "SELECT `name`, `id` FROM `shop`.`t` LIMIT 10");
    }

    #[test]
    fn a_path_without_database_or_with_a_catalog_is_refused() {
        for path in [
            CatalogPath::for_relation(None, None, "t").expect("valid"),
            CatalogPath::for_relation(Some("c"), Some("shop"), "t").expect("valid"),
        ] {
            assert!(
                request(
                    &path,
                    10,
                    &PreviewShape::unordered(),
                    &RelationFacts::default()
                )
                .is_err()
            );
        }
        assert!(
            request(
                &path(),
                0,
                &PreviewShape::unordered(),
                &RelationFacts::default()
            )
            .is_err()
        );
        assert!(
            request(
                &path(),
                1001,
                &PreviewShape::unordered(),
                &RelationFacts::default()
            )
            .is_err()
        );
    }
}
