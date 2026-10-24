//! Composing a table preview in ClickHouse's dialect.
//!
//! The halves of [ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md):
//! the **order** and the **projection** are structured — every column is
//! quoted with backticks and backslash escapes ([`QuoteStyle::BacktickBackslash`])
//! and refused when the table does not declare it; the **predicate** is SQL
//! the user wrote, and travels untouched inside `WHERE (…\n)`. What keeps the
//! predicate honest is not this module: the server refuses a second
//! statement, the request is `readonly = 1`, and the row bound applies.
//!
//! # No page, ever
//!
//! ClickHouse has no unique key: the sorting key orders granules and may
//! repeat. A total order cannot be completed, so an `OFFSET` is refused for
//! every table — the `NoUniqueKey` case of
//! [ADR-0028](../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md).
//! A requested sort is applied as asked.
//!
//! # A column the driver cannot decode is read as its text
//!
//! A click on a table must not fail on one `Variant` column: such a column is
//! read as `toString(…)`, and an aggregate state — opaque bytes — as
//! `hex(toString(…))`, the raw state with the column's server type kept in the
//! catalog. That needs an explicit column list, which leaves out the
//! `MATERIALIZED`, `ALIAS` and `EPHEMERAL` columns as `*` does.

use oxyn_catalog::CatalogPath;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{ExecLimits, ExecRequest, OxynError, PreviewShape, Result, StatementIntent};

use crate::catalog::relation_of;
use crate::variant::LANGUAGE;

const STYLE: QuoteStyle = QuoteStyle::BacktickBackslash;

/// What a preview needs to know of one column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewColumn {
    /// The column's name. Hostile text.
    pub name: String,
    /// How it is read when the driver cannot decode it as is.
    pub cast: Option<Cast>,
    /// Whether `SELECT *` returns it.
    pub in_star: bool,
}

/// How an undecodable column is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cast {
    /// `toString(…)`: the server's own text of the value.
    Text,
    /// `hex(toString(…))`: the raw bytes of an aggregate state.
    Hex,
}

/// The preview statement.
///
/// # Errors
/// [`OxynError::Config`] for a limit outside `1..=1000` or an empty
/// projection; [`OxynError::CatalogUnavailable`] for a path that names no
/// database and table; [`OxynError::Query`] for a column the table does not
/// declare; [`OxynError::NotSupported`] for any page past the first.
pub fn request(
    path: &CatalogPath,
    limit: u32,
    shape: &PreviewShape,
    columns: &[PreviewColumn],
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    let (database, table) = relation_of(path)?;
    if shape.offset > 0 {
        return Err(OxynError::NotSupported {
            capability: "preview pagination: a ClickHouse table has no unique key, and an OFFSET \
                         without a total order repeats rows and skips others without saying so"
                .to_owned(),
        });
    }
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let qualified = format!(
        "{}.{}",
        quote_identifier(database, STYLE),
        quote_identifier(table, STYLE)
    );

    let mut text = format!("SELECT {} FROM {qualified}", projection(shape, columns)?);
    if let Some(predicate) = shape.predicate() {
        // The newline ends a trailing `--` or `#` comment, which would
        // otherwise swallow the ORDER BY and the LIMIT; the parentheses turn
        // an unterminated `/*` into a syntax error instead of an unbounded
        // read.
        text.push_str(" WHERE (");
        text.push_str(predicate);
        text.push_str("\n)");
    }
    if let Some(order) = order_by(shape, columns)? {
        text.push_str(" ORDER BY ");
        text.push_str(&order);
    }
    text.push_str(&format!(" LIMIT {limit}"));

    Ok(ExecRequest::new(LANGUAGE, text)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

/// `*`, or the columns to read, each quoted, checked, and cast if needed.
fn projection(shape: &PreviewShape, columns: &[PreviewColumn]) -> Result<String> {
    let chosen: Vec<&PreviewColumn> = match shape.projection()? {
        Some(names) => names
            .into_iter()
            .map(|name| {
                columns
                    .iter()
                    .find(|column| column.name == name)
                    .ok_or_else(|| {
                        OxynError::Query(format!(
                            "cannot read `{name}` in a preview: the table does not declare that \
                             column"
                        ))
                    })
            })
            .collect::<Result<_>>()?,
        None if columns.iter().all(|column| column.cast.is_none()) => {
            return Ok("*".to_owned());
        }
        None => columns.iter().filter(|column| column.in_star).collect(),
    };
    if chosen.is_empty() {
        return Ok("*".to_owned());
    }
    Ok(chosen
        .into_iter()
        .map(|column| {
            let quoted = quote_identifier(&column.name, STYLE);
            match column.cast {
                None => quoted,
                Some(Cast::Text) => format!("toString({quoted}) AS {quoted}"),
                Some(Cast::Hex) => format!("hex(toString({quoted})) AS {quoted}"),
            }
        })
        .collect::<Vec<_>>()
        .join(", "))
}

/// The requested `ORDER BY`, or `None` when nothing was asked.
fn order_by(shape: &PreviewShape, columns: &[PreviewColumn]) -> Result<Option<String>> {
    let mut terms = Vec::with_capacity(shape.sort.len());
    for sort in &shape.sort {
        if !columns.iter().any(|column| column.name == sort.column) {
            return Err(OxynError::Query(format!(
                "cannot sort a preview on `{}`: the table does not declare that column",
                sort.column
            )));
        }
        let direction = if sort.descending { "DESC" } else { "ASC" };
        terms.push(format!(
            "{} {direction}",
            quote_identifier(&sort.column, STYLE)
        ));
    }
    Ok((!terms.is_empty()).then(|| terms.join(", ")))
}

#[cfg(test)]
mod tests {
    use oxyn_core::{ErrorClass, PreviewSort};

    use super::*;

    fn column(name: &str) -> PreviewColumn {
        PreviewColumn {
            name: name.to_owned(),
            cast: None,
            in_star: true,
        }
    }

    fn path(table: &str) -> CatalogPath {
        CatalogPath::for_relation(None, Some("events"), table).expect("valid path")
    }

    #[test]
    fn a_plain_preview_is_bounded_read_only_and_unordered() {
        let request = request(
            &path("hits"),
            200,
            &PreviewShape::unordered(),
            &[column("id")],
        )
        .expect("plain preview");
        assert_eq!(request.text, "SELECT * FROM `events`.`hits` LIMIT 200");
        assert!(request.limits.read_only);
        assert_eq!(request.limits.max_rows, Some(200));
        assert_eq!(request.intent, StatementIntent::Read);
        assert_eq!(request.language, LANGUAGE);
    }

    #[test]
    fn hostile_names_are_escaped_never_concatenated() {
        let hostile = r"x\`; DROP TABLE audit; --";
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending(hostile)],
            ..PreviewShape::default()
        };
        let request = request(&path(hostile), 10, &shape, &[column(hostile)]).expect("quoted");
        assert_eq!(
            request.text,
            r"SELECT * FROM `events`.`x\\\`; DROP TABLE audit; --` ORDER BY `x\\\`; DROP TABLE audit; --` DESC LIMIT 10"
        );
    }

    #[test]
    fn any_page_past_the_first_is_refused_for_want_of_a_unique_key() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("id")],
            offset: 50,
            ..PreviewShape::default()
        };
        let error = request(&path("hits"), 50, &shape, &[column("id")]).expect_err("no page");
        assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    }

    #[test]
    fn the_predicate_is_fenced_so_a_comment_cannot_swallow_the_limit() {
        let shape = PreviewShape {
            predicate: Some("id > 3 -- the rest".to_owned()),
            ..PreviewShape::default()
        };
        let request = request(&path("hits"), 5, &shape, &[column("id")]).expect("filtered");
        assert_eq!(
            request.text,
            "SELECT * FROM `events`.`hits` WHERE (id > 3 -- the rest\n) LIMIT 5"
        );
    }

    #[test]
    fn an_undeclared_column_is_a_permanent_refusal() {
        for shape in [
            PreviewShape {
                sort: vec![PreviewSort::ascending("ghost")],
                ..PreviewShape::default()
            },
            PreviewShape {
                columns: Some(vec!["ghost".to_owned()]),
                ..PreviewShape::default()
            },
        ] {
            let error = request(&path("hits"), 5, &shape, &[column("id")]).expect_err("refused");
            assert!(matches!(error, OxynError::Query(_)), "{error:?}");
            assert_eq!(error.class(), ErrorClass::Permanent);
        }
    }

    #[test]
    fn undecodable_columns_are_read_as_text_and_hidden_columns_stay_hidden() {
        let columns = [
            column("id"),
            PreviewColumn {
                name: "v".into(),
                cast: Some(Cast::Text),
                in_star: true,
            },
            PreviewColumn {
                name: "state".into(),
                cast: Some(Cast::Hex),
                in_star: true,
            },
            PreviewColumn {
                name: "derived".into(),
                cast: None,
                in_star: false,
            },
        ];
        let request =
            request(&path("hits"), 5, &PreviewShape::unordered(), &columns).expect("composed");
        assert_eq!(
            request.text,
            "SELECT `id`, toString(`v`) AS `v`, hex(toString(`state`)) AS `state` \
             FROM `events`.`hits` LIMIT 5"
        );
    }
}
