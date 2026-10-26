//! What an answer looks like on the grid.
//!
//! The REST API answers in many shapes; each gets the table a professional
//! expects, and none loses a value (ADR-0062 § Result shapes):
//!
//! | Answer | Rows |
//! |---|---|
//! | `hits.hits` of a search | one per document (`crate::hits`) |
//! | a search with no hit and aggregations | the aggregations, flattened (`crate::aggs`) |
//! | ES\|QL, `_sql`, OpenSearch SQL | their columns, typed (`crate::columnar`) |
//! | an array of objects (`_cat?format=json`) | one per object |
//! | `text/plain` of `_cat?v` | one per line, split on the header's columns |
//! | any other `text/plain` (`_cat` without `v`) | one per line, every line |
//! | a running asynchronous search | one, its status: `id`, `is_running` |
//! | any other object (`_cluster/health`) | one, a column per key |
//! | `HEAD` | its status, and whether the target exists |
//!
//! The request decides the reading, never the answer's look: the endpoint
//! names the tabular API, the body its orientation, `v` whether a `_cat`
//! text starts with a header. A read whose answer says it is incomplete is
//! refused (`crate::complete`).

use std::sync::Arc;
use std::time::Duration;

use arrow::array::{ArrayRef, BooleanArray, UInt16Array};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use indexmap::IndexMap;
use oxyn_core::{OxynError, Result};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::cells::{TextRows, cell_text, text_schema};
use crate::columnar::{Columns, Shape};
use crate::cursor::Asked;
use crate::hits::{Hit, HitLayout, sources};
use crate::http::Reply;
use crate::{aggs, columnar, complete};

/// A decoded answer.
pub(crate) struct Decoded {
    pub(crate) schema: SchemaRef,
    pub(crate) batches: Vec<RecordBatch>,
    /// The server's own `took`, when it reports one.
    pub(crate) took: Option<Duration>,
    /// What the answer says about the pages after it.
    pub(crate) page: PageState,
    /// The layout of the hits, which a stream's later pages keep.
    pub(crate) layout: Option<HitLayout>,
    /// The columns and shape of a tabular answer, which `_sql`'s later pages
    /// keep.
    pub(crate) tabular: Option<(Columns, Shape)>,
}

/// What the answer says about the pages after it.
#[derive(Debug, Default)]
pub(crate) struct PageState {
    /// Rows this page held.
    pub(crate) rows: usize,
    /// The `sort` values of the last hit, for `search_after`.
    pub(crate) last_sort: Option<Box<RawValue>>,
    /// The point in time to search in next, as the server renewed it.
    pub(crate) pit_id: Option<String>,
    pub(crate) scroll_id: Option<String>,
    /// The cursor of `_sql`, Elasticsearch's and OpenSearch's.
    pub(crate) sql_cursor: Option<String>,
}

/// A search answer, every value left raw.
#[derive(Deserialize)]
struct Search {
    took: Option<u64>,
    hits: Option<Hits>,
    aggregations: Option<IndexMap<String, Box<RawValue>>>,
    pit_id: Option<String>,
    #[serde(rename = "_scroll_id")]
    scroll_id: Option<String>,
}

#[derive(Deserialize)]
struct Hits {
    hits: Vec<Hit>,
}

/// How a page relates to the stream it belongs to.
#[derive(Clone, Copy)]
pub(crate) enum Stream<'a> {
    /// A single answer.
    No,
    /// The first page of a stream: its hits get the `_other` column.
    First,
    /// A later page, in the first page's layout.
    Next(&'a HitLayout),
}

/// Decodes the answer to `asked`.
pub(crate) fn decode(reply: &Reply, asked: &Asked<'_>, stream: Stream<'_>) -> Result<Decoded> {
    if asked.head {
        return status_row(reply.status);
    }
    if !reply.json {
        return lines(&reply.bytes, asked.cat_header());
    }
    let trimmed = reply.bytes.trim_ascii_start();
    if trimmed.is_empty() {
        return status_row(reply.status);
    }
    if trimmed.first() == Some(&b'[') {
        return array_rows(&reply.bytes);
    }
    let object = object(&reply.bytes)?;
    if asked.read
        && let Some(why) = complete::incomplete(&object, complete::Read::of(asked.path))
    {
        return Err(OxynError::Query(why));
    }
    // A running asynchronous query is a status: what it holds so far is
    // partial, and its row shows `is_running` and the `id` to fetch it by.
    let running = object
        .get("is_running")
        .is_some_and(|running| running.get() == "true");
    if !running
        && let Some(shape) = asked.shape()
        && let Some(decoded) = columnar::decode(&object, shape)?
    {
        return Ok(decoded);
    }
    if object.contains_key("hits") {
        let search: Search = serde_json::from_slice(&reply.bytes).map_err(|_| {
            OxynError::Query("the search answer does not have the shape of one".to_owned())
        })?;
        return hits(search, stream);
    }
    object_row(&object)
}

/// Decodes a later page of an SQL cursor, in its first page's columns and
/// shape, refused like a first page when it says it is incomplete.
pub(crate) fn next_tabular(reply: &Reply, columns: &Columns, shape: Shape) -> Result<Decoded> {
    let object = object(&reply.bytes)?;
    if let Some(why) = complete::incomplete(&object, complete::Read::Other) {
        return Err(OxynError::Query(why));
    }
    columnar::next_page(columns, &object, shape)
}

fn object(bytes: &[u8]) -> Result<IndexMap<String, Box<RawValue>>> {
    serde_json::from_slice(bytes)
        .map_err(|_| OxynError::Query("the server's answer is not valid JSON".to_owned()))
}

fn hits(search: Search, stream: Stream<'_>) -> Result<Decoded> {
    let took = search.took.map(Duration::from_millis);
    let documents = search.hits.map(|hits| hits.hits).unwrap_or_default();
    if documents.is_empty()
        && matches!(stream, Stream::No)
        && let Some(aggregations) = &search.aggregations
    {
        let (schema, batches) = aggs::flatten(aggregations)?;
        return Ok(Decoded {
            schema,
            batches,
            took,
            page: PageState::default(),
            layout: None,
            tabular: None,
        });
    }
    let sources = sources(&documents)?;
    let layout = match stream {
        Stream::Next(layout) => layout.clone(),
        Stream::First => HitLayout::of(&documents, &sources, true),
        Stream::No => HitLayout::of(&documents, &sources, false),
    };
    let last_sort = documents
        .last()
        .and_then(|hit| hit.get("sort"))
        .map(|sort| RawValue::from_string(sort.get().to_owned()))
        .transpose()
        .map_err(|_| OxynError::Query("a hit's `sort` is not JSON".to_owned()))?;
    Ok(Decoded {
        schema: Arc::clone(&layout.schema),
        batches: layout.rows(&documents, &sources)?,
        took,
        page: PageState {
            rows: documents.len(),
            last_sort,
            pit_id: search.pit_id,
            scroll_id: search.scroll_id,
            sql_cursor: None,
        },
        layout: Some(layout),
        tabular: None,
    })
}

fn array_rows(bytes: &[u8]) -> Result<Decoded> {
    let items: Vec<Box<RawValue>> = serde_json::from_slice(bytes)
        .map_err(|_| OxynError::Query("the server's answer is not valid JSON".to_owned()))?;
    let objects: Option<Vec<IndexMap<String, Box<RawValue>>>> = items
        .iter()
        .map(|item| serde_json::from_str(item.get()).ok())
        .collect();
    let Some(objects) = objects else {
        // An array of scalars or of mixed shapes: one value per row.
        let schema = text_schema([("value", None)]);
        let mut rows = TextRows::new(Arc::clone(&schema))?;
        for item in &items {
            rows.push([cell_text(item)])?;
        }
        return Ok(simple(schema, rows.finish()?));
    };
    let mut keys: IndexMap<&str, ()> = IndexMap::new();
    for object in &objects {
        for key in object.keys() {
            keys.insert(key.as_str(), ());
        }
    }
    let schema = text_schema(keys.keys().map(|key| (*key, None)));
    let mut rows = TextRows::new(Arc::clone(&schema))?;
    for object in &objects {
        rows.push(
            keys.keys()
                .map(|key| object.get(*key).and_then(|value| cell_text(value))),
        )?;
    }
    Ok(simple(schema, rows.finish()?))
}

fn object_row(object: &IndexMap<String, Box<RawValue>>) -> Result<Decoded> {
    let schema = text_schema(object.keys().map(|key| (key.as_str(), None)));
    let mut rows = TextRows::new(Arc::clone(&schema))?;
    rows.push(object.values().map(|value| cell_text(value)))?;
    Ok(simple(schema, rows.finish()?))
}

/// A `text/plain` answer: the `_cat` APIs without `format=json`.
///
/// With `?v`, the first line names the columns, and every line splits into
/// as many words as the header: the table is then that. When a line does not
/// split into exactly as many words — a value with a space — it would be cut
/// in the wrong place, so the whole answer is one `line` column instead.
/// Without `v` there is no header to read: a first line that splits like the
/// others is a row all the same, so every line is a `line`.
fn lines(bytes: &[u8], header_asked: bool) -> Result<Decoded> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let header: Vec<&str> = lines
        .first()
        .map(|line| line.split_whitespace().collect())
        .unwrap_or_default();
    let tabular = header_asked
        && !header.is_empty()
        && lines
            .iter()
            .all(|line| line.split_whitespace().count() == header.len());
    if tabular {
        let schema = text_schema(header.iter().map(|name| (*name, None)));
        let mut rows = TextRows::new(Arc::clone(&schema))?;
        for line in lines.iter().skip(1) {
            rows.push(line.split_whitespace().map(|word| Some(word.to_owned())))?;
        }
        return Ok(simple(schema, rows.finish()?));
    }
    let schema = text_schema([("line", None)]);
    let mut rows = TextRows::new(Arc::clone(&schema))?;
    for line in lines {
        rows.push([Some(line.to_owned())])?;
    }
    Ok(simple(schema, rows.finish()?))
}

fn status_row(status: u16) -> Result<Decoded> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("status", DataType::UInt16, false),
        Field::new("exists", DataType::Boolean, false),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(UInt16Array::from(vec![status])),
        Arc::new(BooleanArray::from(vec![(200..300).contains(&status)])),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
        .map_err(|error| OxynError::Internal(format!("a status batch is invalid: {error}")))?;
    Ok(simple(schema, vec![batch]))
}

/// A single answer with no page after it.
pub(crate) fn simple(schema: SchemaRef, batches: Vec<RecordBatch>) -> Decoded {
    Decoded {
        schema,
        batches,
        took: None,
        page: PageState::default(),
        layout: None,
        tabular: None,
    }
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
