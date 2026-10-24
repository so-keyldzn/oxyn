//! The documents of a search, as rows.
//!
//! One row per hit: `_index`, `_id`, `_score`, then the `_source` keys in the
//! order they first appear, then every other key a hit carries — `fields`,
//! `highlight`, `sort`, `_routing`, `inner_hits`, `_explanation`. A nested
//! object or an array stays one cell, its JSON text: any field may hold
//! several values, and flattening `actor.id` out of an array of actors would
//! pair values the document does not pair.
//!
//! A disabled `_source` leaves only the metadata columns, which is what the
//! server returned. Nothing is dropped: the layout of a stream is fixed by
//! its first page, and a `_source` key a later page brings goes into the
//! `_other` column, one JSON object per row.

use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use indexmap::IndexMap;
use oxyn_core::{OxynError, Result};
use serde_json::value::RawValue;

use crate::cells::{TextRows, cell_text, text_schema};

/// A hit, every value left raw.
pub(crate) type Hit = IndexMap<String, Box<RawValue>>;

/// The keys of a hit that come first, in this order.
const LEADING: [&str; 3] = ["_index", "_id", "_score"];

/// The column of a stream that holds the `_source` keys its first page did
/// not have.
pub(crate) const OTHER: &str = "_other";

/// Where a column's cells come from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Column {
    /// A key of the hit itself.
    Hit(String),
    /// A top-level key of `_source`.
    Source(String),
    /// The `_source` keys no other column holds.
    Other,
}

/// The columns of a page of hits, and how to fill them.
#[derive(Debug, Clone)]
pub(crate) struct HitLayout {
    columns: Vec<Column>,
    pub(crate) schema: SchemaRef,
}

impl HitLayout {
    /// The layout of a page. `stream` adds the `_other` column a later page
    /// may need.
    pub(crate) fn of(documents: &[Hit], sources: &[Option<Hit>], stream: bool) -> Self {
        let mut hit_keys: IndexMap<&str, ()> = LEADING.iter().map(|key| (*key, ())).collect();
        for hit in documents {
            for key in hit.keys().filter(|key| *key != "_source") {
                hit_keys.insert(key.as_str(), ());
            }
        }
        let mut source_keys: IndexMap<&str, ()> = IndexMap::new();
        for source in sources.iter().flatten() {
            for key in source.keys() {
                source_keys.insert(key.as_str(), ());
            }
        }
        let leading = LEADING.len();
        let mut columns: Vec<(String, Column)> = hit_keys
            .keys()
            .take(leading)
            .map(|key| ((*key).to_owned(), Column::Hit((*key).to_owned())))
            .collect();
        for key in source_keys.keys() {
            // A `_source` key spelled like a hit key (`sort`, `fields`) keeps
            // its own column, named `_source.sort`.
            let name = if hit_keys.contains_key(key) {
                format!("_source.{key}")
            } else {
                (*key).to_owned()
            };
            columns.push((name, Column::Source((*key).to_owned())));
        }
        for key in hit_keys.keys().skip(leading) {
            columns.push(((*key).to_owned(), Column::Hit((*key).to_owned())));
        }
        if stream {
            columns.push((OTHER.to_owned(), Column::Other));
        }
        let schema = text_schema(columns.iter().map(|(name, _)| (name.as_str(), None)));
        Self {
            columns: columns.into_iter().map(|(_, column)| column).collect(),
            schema,
        }
    }

    /// The rows of a page, in this layout.
    pub(crate) fn rows(
        &self,
        documents: &[Hit],
        sources: &[Option<Hit>],
    ) -> Result<Vec<RecordBatch>> {
        let mut rows = TextRows::new(Arc::clone(&self.schema));
        for (hit, source) in documents.iter().zip(sources) {
            rows.push(self.columns.iter().map(|column| {
                match column {
                    Column::Hit(key) => hit.get(key).and_then(|value| cell_text(value)),
                    Column::Source(key) => source
                        .as_ref()
                        .and_then(|source| source.get(key))
                        .and_then(|value| cell_text(value)),
                    Column::Other => self.other(source.as_ref()),
                }
            }))?;
        }
        rows.finish()
    }

    /// The `_source` keys of a hit no column holds, as one JSON object.
    fn other(&self, source: Option<&Hit>) -> Option<String> {
        let source = source?;
        let rest: IndexMap<&str, &RawValue> = source
            .iter()
            .filter(|(key, _)| {
                !self
                    .columns
                    .iter()
                    .any(|column| matches!(column, Column::Source(held) if held == *key))
            })
            .map(|(key, value)| (key.as_str(), value.as_ref()))
            .collect();
        if rest.is_empty() {
            return None;
        }
        serde_json::to_string(&rest).ok()
    }
}

/// The `_source` of every hit, parsed one level deep.
pub(crate) fn sources(documents: &[Hit]) -> Result<Vec<Option<Hit>>> {
    documents
        .iter()
        .map(|hit| {
            hit.get("_source")
                .map(|source| serde_json::from_str(source.get()))
                .transpose()
        })
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| OxynError::Query("a hit's `_source` is not a JSON object".to_owned()))
}
