//! What is asked of a relation preview: an order, a predicate, a page.
//!
//! # Two halves that do not look alike
//!
//! The **order** is structured: a column is an identifier the driver quotes,
//! never an expression. Leaving it free would reopen SQL composition on the
//! wrong side of the boundary, on a string Oxyn would insert into a query it
//! composes itself ([I-10](../../../CLAUDE.md#i-10)).
//!
//! The **predicate**, on the other hand, is SQL the user writes. It is the
//! `WHERE` field of the mockup (`272:10667`), and [I-10](../../../CLAUDE.md#i-10)
//! says it unambiguously: "the SQL *the user writes* is sent as is — that is
//! the feature". What is forbidden is Oxyn concatenating an identifier
//! **received from the server**; not passing on what a professional typed.
//!
//! This predicate is not an open door for all that. The final text is
//! reclassified by `oxyn-query` and refused if it becomes mutating, the session
//! is held read-only by the server, and the row bound applies
//! ([ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{OxynError, Result};

/// A sort column, and its direction.
///
/// `column` is the **exact name** of a column of the relation, never an
/// expression: it is the structured half of the request, the one Oxyn composes
/// and quotes itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSort {
    /// Column name, as the catalog gives it.
    pub column: String,
    /// Descending rather than ascending.
    pub descending: bool,
}

impl PreviewSort {
    /// Sorts this column in ascending order.
    #[must_use]
    pub fn ascending(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            descending: false,
        }
    }

    /// Sorts this column in descending order.
    #[must_use]
    pub fn descending(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            descending: true,
        }
    }
}

/// The requested shape of a preview: its order, its predicate, its page.
///
/// Grouped rather than split into three command fields: these three only make
/// sense together — an `offset` without a deterministic order means nothing —
/// and separating them would invite the next caller to forget one.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PreviewShape {
    /// The requested order, empty when the user chose nothing.
    ///
    /// The driver **completes** it with a unique key to break ties: without a
    /// total order, two consecutive pages can show the same row twice and omit
    /// another.
    pub sort: Vec<PreviewSort>,
    /// The predicate written by the user, **without** the `WHERE` keyword.
    ///
    /// Passed as is: neither parsed, nor rewritten, nor completed. An empty or
    /// whitespace-only predicate means "no filter" — inserting a `WHERE` without
    /// a condition would produce a syntax error where the user believes they
    /// cleared everything.
    pub predicate: Option<String>,
    /// How many rows to skip before the requested page.
    ///
    /// `0` is the first page. A non-zero value only makes sense if the order is
    /// deterministic, which the driver checks: otherwise it refuses, rather
    /// than return a page nobody can say the contents of.
    pub offset: u64,
    /// The only columns to read, in this order; `None` reads them all.
    ///
    /// This is what bounds a read to what the user approved: a sample with
    /// three columns ticked does not fetch the other twelve only to throw them
    /// away. Like the order, it is a structured half: **exact names**, which the
    /// driver quotes and refuses when the relation does not declare them, never
    /// expressions ([I-10](../../../CLAUDE.md#i-10)).
    ///
    /// Read through [`Self::projection`], which deduplicates and bounds it.
    /// Absent from a shape serialized before it, it means "all".
    #[serde(default)]
    pub columns: Option<Vec<String>>,
}

/// How many names a projection can carry, duplicates included.
///
/// An Oxyn bound, not an engine's: it caps the size of a command received over
/// IPC or from an agent — counted before deduplication, otherwise a million
/// times the same name would get through —, and exceeds what an approval screen
/// offers to tick.
pub const MAX_PROJECTED_COLUMNS: usize = 1024;

impl PreviewShape {
    /// Asks for nothing in particular: the first page, without order or filter.
    #[must_use]
    pub fn unordered() -> Self {
        Self::default()
    }

    /// The predicate, if one remains once whitespace is trimmed.
    ///
    /// It is the only normalization applied to the user's text, and it does not
    /// change its meaning: a field where only a space remains is an empty field.
    #[must_use]
    pub fn predicate(&self) -> Option<&str> {
        self.predicate
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// Does this request require a **total** order?
    ///
    /// True as soon as a sort is requested or a page other than the first is.
    /// In both cases the driver must complete the order with a unique key,
    /// which costs it a metadata read: that is the only reason a preview pays
    /// for one, and a preview without a request pays for none.
    ///
    /// Sorting alone needs it as much as paging: ordering the first page by one
    /// column and the next by two would make a row reappear exactly at the
    /// boundary.
    #[must_use]
    pub fn needs_total_order(&self) -> bool {
        !self.sort.is_empty() || self.offset > 0
    }

    /// Nothing was requested: no order, no predicate, no next page.
    ///
    /// This is what distinguishes the automatic preview of a table just
    /// selected from a read the user composed.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.sort.is_empty() && self.predicate().is_none() && self.offset == 0
    }

    /// The projection to compose: `None` for all columns, otherwise the
    /// requested names, each once, in the order of their first mention.
    ///
    /// A duplicate is not refused — ticking the same column twice says nothing
    /// more than ticking it —, but it is not composed either: two columns of the
    /// same name in a result make reading by name ambiguous.
    ///
    /// # Errors
    /// [`OxynError::Config`] when the list is empty or exceeds
    /// [`MAX_PROJECTED_COLUMNS`]. An empty list is not "all": it is a request
    /// that reads nothing, and composing it as `SELECT *` would read precisely
    /// what nobody approved.
    pub fn projection(&self) -> Result<Option<Vec<&str>>> {
        let Some(columns) = &self.columns else {
            return Ok(None);
        };
        if columns.is_empty() {
            return Err(OxynError::Config(
                "a preview projection must name at least one column".into(),
            ));
        }
        if columns.len() > MAX_PROJECTED_COLUMNS {
            return Err(OxynError::Config(format!(
                "a preview projection names at most {MAX_PROJECTED_COLUMNS} columns"
            )));
        }
        let mut seen = HashSet::with_capacity(columns.len());
        Ok(Some(
            columns
                .iter()
                .map(String::as_str)
                .filter(|name| seen.insert(*name))
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_blank_predicate_filters_nothing() {
        let empty = PreviewShape {
            predicate: Some(String::new()),
            ..PreviewShape::default()
        };
        assert!(empty.predicate().is_none());
        assert!(empty.is_plain(), "a cleared field composes no WHERE");

        let blank = PreviewShape {
            predicate: Some("   \n\t ".into()),
            ..PreviewShape::default()
        };
        assert!(blank.predicate().is_none());
    }

    #[test]
    fn the_user_text_is_not_rewritten() {
        // Surrounding whitespace goes, the rest is intact: no case
        // normalization, no added quotes, no translated operator.
        let shape = PreviewShape {
            predicate: Some("  status = 'active' AND note LIKE '100%'  ".into()),
            ..PreviewShape::default()
        };
        assert_eq!(
            shape.predicate(),
            Some("status = 'active' AND note LIKE '100%'")
        );
    }

    #[test]
    fn a_preview_without_request_differs_from_a_composed_preview() {
        assert!(PreviewShape::unordered().is_plain());
        let sorted = PreviewShape {
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        assert!(!sorted.is_plain());
        let filtered = PreviewShape {
            predicate: Some("id > 10".into()),
            ..PreviewShape::default()
        };
        assert!(!filtered.is_plain());
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        assert!(!page.is_plain());
    }

    #[test]
    fn a_projection_is_deduplicated_in_first_mention_order() {
        assert_eq!(PreviewShape::unordered().projection().ok(), Some(None));
        let shape = PreviewShape {
            columns: Some(vec!["email".into(), "id".into(), "email".into()]),
            ..PreviewShape::default()
        };
        assert_eq!(shape.projection().ok().flatten(), Some(vec!["email", "id"]));
    }

    #[test]
    fn an_empty_or_oversized_projection_is_refused() {
        // Empty, it does not mean "all": composing it as `SELECT *` would read
        // what nobody ticked.
        let empty = PreviewShape {
            columns: Some(Vec::new()),
            ..PreviewShape::default()
        };
        assert!(matches!(empty.projection(), Err(OxynError::Config(_))));

        let at_limit: Vec<String> = (0..MAX_PROJECTED_COLUMNS)
            .map(|n| format!("c{n}"))
            .collect();
        let mut too_many = at_limit.clone();
        too_many.push("one more".into());
        let at_ceiling = PreviewShape {
            columns: Some(at_limit),
            ..PreviewShape::default()
        };
        assert!(at_ceiling.projection().is_ok());
        let over_ceiling = PreviewShape {
            columns: Some(too_many),
            ..PreviewShape::default()
        };
        assert!(matches!(
            over_ceiling.projection(),
            Err(OxynError::Config(_))
        ));
        // The bound counts the names received, duplicates included: a single
        // name repeated endlessly does not slip under it.
        let repeated = PreviewShape {
            columns: Some(vec!["id".to_owned(); MAX_PROJECTED_COLUMNS + 1]),
            ..PreviewShape::default()
        };
        assert!(matches!(repeated.projection(), Err(OxynError::Config(_))));
    }

    #[test]
    fn a_serialized_shape_without_projection_reads_every_column() {
        let older: PreviewShape =
            serde_json::from_str(r#"{"sort":[],"predicate":null,"offset":0}"#)
                .expect("shape from before the projection");
        assert_eq!(older.columns, None);
    }

    #[test]
    fn a_total_order_is_required_by_sorting_as_much_as_by_paging() {
        assert!(!PreviewShape::unordered().needs_total_order());
        // A predicate alone does not change the order: it requires no key, and
        // must therefore not cost a metadata read.
        let filtered = PreviewShape {
            predicate: Some("id > 10".into()),
            ..PreviewShape::default()
        };
        assert!(!filtered.needs_total_order());

        let sorted = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            ..PreviewShape::default()
        };
        assert!(sorted.needs_total_order(), "from the first page on");
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        assert!(page.needs_total_order());
    }
}
