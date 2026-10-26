//! Aggregations as a table.
//!
//! A search with no hit and aggregations is read the way a data table reads
//! it: one row per innermost bucket, a column for each bucket level's key and
//! `doc_count`, and a column for each metric. `terms` on `status` with an
//! `avg` of `bytes` gives `status`, `status.doc_count`, `bytes_avg`.
//!
//! * a single-value metric (`avg`, `sum`, `cardinality`…) is a column named
//!   after the aggregation; a multi-value one (`stats`, `percentiles`) a
//!   column per value, `stats.min`;
//! * a single-bucket aggregation (`filter`, `missing`, `nested`) gives its
//!   `doc_count` and its metrics under its name, `errors.doc_count`;
//! * anything the table cannot place — two bucket aggregations side by side,
//!   whose rows do not pair, or `top_hits` — is the JSON text of a cell, never
//!   dropped, on the first row of its level only. Two sibling bucket
//!   aggregations at the top give one row per aggregation, with its JSON.

use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use indexmap::IndexMap;
use oxyn_core::{OxynError, Result};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::cells::{MAX_COLUMNS, TextRows, text_schema};

type Row = IndexMap<String, Value>;

/// The table of a search's aggregations.
pub(crate) fn flatten(
    aggregations: &IndexMap<String, Box<RawValue>>,
) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let parsed: Map<String, Value> = aggregations
        .iter()
        .map(|(name, value)| Ok((name.clone(), serde_json::from_str(value.get())?)))
        .collect::<std::result::Result<_, serde_json::Error>>()
        .map_err(|_| OxynError::Query("an aggregation is not valid JSON".to_owned()))?;
    // Two walks, and no row kept between them: the first names the columns,
    // the second hands each row to the bounded table as it is built. Kept
    // whole before the table, a key repeated under each of a million inner
    // buckets would weigh gigabytes before any bound could refuse it.
    let mut columns: IndexMap<String, ()> = IndexMap::new();
    rows(&parsed, &mut |row| {
        for key in row.into_keys() {
            columns.insert(key, ());
        }
        if columns.len() > MAX_COLUMNS {
            return Err(OxynError::Query(format!(
                "the aggregations give more than the {MAX_COLUMNS} columns a table takes: \
                 split them"
            )));
        }
        Ok(())
    })?;
    let schema = text_schema(columns.keys().map(|name| (name.as_str(), None)));
    let mut table = TextRows::new(Arc::clone(&schema))?;
    rows(&parsed, &mut |row| {
        table.push(columns.keys().map(|name| row.get(name).and_then(text)))
    })?;
    Ok((schema, table.finish()?))
}

/// What receives the rows, one at a time.
type Sink<'a> = dyn FnMut(Row) -> Result<()> + 'a;

fn rows(parsed: &Map<String, Value>, sink: &mut Sink<'_>) -> Result<()> {
    if bucket_aggs(parsed).count() > 1 {
        // Sibling bucket aggregations: their buckets do not pair into rows.
        for (name, value) in parsed {
            let mut row = Row::new();
            row.insert("aggregation".to_owned(), Value::String(name.clone()));
            row.insert("result".to_owned(), value.clone());
            sink(row)?;
        }
        return Ok(());
    }
    walk(&Row::new(), parsed, sink)
}

fn walk(prefix: &Row, aggs: &Map<String, Value>, sink: &mut Sink<'_>) -> Result<()> {
    let mut level = prefix.clone();
    let walked = bucket_aggs(aggs).next().map(|(name, _)| name);
    for (name, value) in aggs {
        if buckets(value).is_none() {
            metric(&mut level, name, value);
        } else if Some(name.as_str()) != walked {
            // A second bucket aggregation of the same level does not pair
            // with the first: its JSON is a cell of this level.
            level.insert(name.clone(), value.clone());
        }
    }
    let Some((name, buckets)) = bucket_aggs(aggs).next() else {
        return sink(level);
    };
    if buckets.is_empty() {
        return sink(level);
    }
    // A key or a count repeats on every row below it, as a grid reads it. An
    // object or an array — a sibling aggregation, `top_hits` — is shown on
    // the first row only: repeated under each of a thousand inner buckets, a
    // 16 MiB answer would decode to gigabytes.
    let scalars: Row = level
        .iter()
        .filter(|(_, value)| !value.is_object() && !value.is_array())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (index, (keyed, bucket)) in buckets.into_iter().enumerate() {
        let mut row = if index == 0 {
            level.clone()
        } else {
            scalars.clone()
        };
        let key = bucket
            .get("key_as_string")
            .or_else(|| bucket.get("key"))
            .cloned()
            .or_else(|| keyed.map(|key| Value::String(key.to_owned())))
            .unwrap_or(Value::Null);
        row.insert(name.to_owned(), key);
        if let Some(count) = bucket.get("doc_count") {
            row.insert(format!("{name}.doc_count"), count.clone());
        }
        for bound in ["from", "to"] {
            if let Some(value) = bucket.get(bound) {
                row.insert(format!("{name}.{bound}"), value.clone());
            }
        }
        let children: Map<String, Value> = bucket
            .iter()
            .filter(|(key, child)| {
                !matches!(
                    key.as_str(),
                    "key"
                        | "key_as_string"
                        | "doc_count"
                        | "from"
                        | "to"
                        | "from_as_string"
                        | "to_as_string"
                ) && child.is_object()
            })
            .map(|(key, child)| (key.clone(), child.clone()))
            .collect();
        walk(&row, &children, sink)?;
    }
    Ok(())
}

/// The buckets of one aggregation, with the name of a keyed one.
type Buckets<'a> = Vec<(Option<&'a str>, &'a Map<String, Value>)>;

/// The bucket aggregations of a level, with their buckets.
fn bucket_aggs(aggs: &Map<String, Value>) -> impl Iterator<Item = (&str, Buckets<'_>)> {
    aggs.iter()
        .filter_map(|(name, value)| Some((name.as_str(), buckets(value)?)))
}

/// The buckets of an aggregation, in either form: a list, or keyed by name
/// (`"keyed": true`, `filters`).
fn buckets(value: &Value) -> Option<Buckets<'_>> {
    match value.get("buckets")? {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(Value::as_object)
                .map(|b| (None, b))
                .collect(),
        ),
        Value::Object(keyed) => Some(
            keyed
                .iter()
                .filter_map(|(key, bucket)| Some((Some(key.as_str()), bucket.as_object()?)))
                .collect(),
        ),
        _ => None,
    }
}

fn metric(row: &mut Row, name: &str, value: &Value) {
    let Some(object) = value.as_object() else {
        row.insert(name.to_owned(), value.clone());
        return;
    };
    if let Some(single) = object.get("value") {
        let single = object.get("value_as_string").unwrap_or(single);
        row.insert(name.to_owned(), single.clone());
        return;
    }
    for (key, field) in object {
        match field {
            // `percentiles` and its kin: `{"values": {"50.0": 12.5}}`.
            Value::Object(values) if key == "values" => {
                for (percent, value) in values {
                    row.insert(format!("{name}.{percent}"), value.clone());
                }
            }
            Value::Object(child)
                if child.contains_key("value") || child.contains_key("doc_count") =>
            {
                metric(row, &format!("{name}.{key}"), field);
            }
            _ => {
                row.insert(format!("{name}.{key}"), field.clone());
            }
        }
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests;
