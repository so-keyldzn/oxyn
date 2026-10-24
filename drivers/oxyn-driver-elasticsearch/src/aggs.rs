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

use crate::cells::{TextRows, text_schema};

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
    let mut rows = Vec::new();
    if bucket_aggs(&parsed).count() > 1 {
        // Sibling bucket aggregations: their buckets do not pair into rows.
        for (name, value) in &parsed {
            let mut row = Row::new();
            row.insert("aggregation".to_owned(), Value::String(name.clone()));
            row.insert("result".to_owned(), value.clone());
            rows.push(row);
        }
    } else {
        walk(&Row::new(), &parsed, &mut rows);
    }
    let mut columns: IndexMap<&str, ()> = IndexMap::new();
    for row in &rows {
        for key in row.keys() {
            columns.insert(key.as_str(), ());
        }
    }
    let schema = text_schema(columns.keys().map(|name| (*name, None)));
    let mut table = TextRows::new(Arc::clone(&schema))?;
    for row in &rows {
        table.push(columns.keys().map(|name| row.get(*name).and_then(text)))?;
    }
    Ok((schema, table.finish()?))
}

fn walk(prefix: &Row, aggs: &Map<String, Value>, rows: &mut Vec<Row>) {
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
        rows.push(level);
        return;
    };
    if buckets.is_empty() {
        rows.push(level);
        return;
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
        walk(&row, &children, rows);
    }
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
mod tests {
    use arrow::array::{Array, StringArray};

    use super::*;

    fn table(aggregations: Value) -> (Vec<String>, Vec<Vec<Option<String>>>) {
        let raw: IndexMap<String, Box<RawValue>> =
            serde_json::from_str(&aggregations.to_string()).expect("an object");
        let (schema, batches) = flatten(&raw).expect("flattened");
        let names = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();
        let mut rows = Vec::new();
        for batch in &batches {
            for row in 0..batch.num_rows() {
                rows.push(
                    batch
                        .columns()
                        .iter()
                        .map(|column| {
                            let column =
                                column.as_any().downcast_ref::<StringArray>().expect("utf8");
                            (!column.is_null(row)).then(|| column.value(row).to_owned())
                        })
                        .collect(),
                );
            }
        }
        (names, rows)
    }

    #[test]
    fn nested_buckets_give_one_row_per_innermost_bucket() {
        let (names, rows) = table(serde_json::json!({
            "by_status": {"buckets": [
                {"key": "500", "doc_count": 3, "by_host": {"buckets": [
                    {"key": "a", "doc_count": 2, "bytes_avg": {"value": 10.5}},
                    {"key": "b", "doc_count": 1, "bytes_avg": {"value": null}}
                ]}},
                {"key": "200", "doc_count": 9, "by_host": {"buckets": []}}
            ]},
            "total": {"value": 12}
        }));
        assert_eq!(
            names,
            [
                "total",
                "by_status",
                "by_status.doc_count",
                "by_host",
                "by_host.doc_count",
                "bytes_avg"
            ]
        );
        let s = |text: &str| Some(text.to_owned());
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            [s("12"), s("500"), s("3"), s("a"), s("2"), s("10.5")]
        );
        assert_eq!(rows[1], [s("12"), s("500"), s("3"), s("b"), s("1"), None]);
        assert_eq!(rows[2], [s("12"), s("200"), s("9"), None, None, None]);
    }

    #[test]
    fn a_date_histogram_shows_its_key_as_string_and_stats_spread() {
        let (names, rows) = table(serde_json::json!({
            "per_day": {"buckets": [
                {"key": 1_704_067_200_000_i64, "key_as_string": "2024-01-01", "doc_count": 4,
                 "load": {"count": 4, "min": 1.0, "max": 3.0, "avg": 2.0, "sum": 8.0}}
            ]}
        }));
        // A metric's values come in the server's order, whatever else the
        // build links.
        assert_eq!(
            names,
            [
                "per_day",
                "per_day.doc_count",
                "load.count",
                "load.min",
                "load.max",
                "load.avg",
                "load.sum"
            ]
        );
        assert_eq!(
            rows[0].first().cloned().flatten().as_deref(),
            Some("2024-01-01")
        );
    }

    #[test]
    fn a_long_key_beyond_two_to_the_53_survives() {
        let (_, rows) = table(
            serde_json::from_str(
                r#"{"ids": {"buckets": [{"key": 9007199254740993, "doc_count": 1}]}}"#,
            )
            .expect("JSON"),
        );
        assert_eq!(rows[0][0].as_deref(), Some("9007199254740993"));
    }

    #[test]
    fn sibling_bucket_aggregations_are_kept_whole_not_paired() {
        let (names, rows) = table(serde_json::json!({
            "a": {"buckets": [{"key": "x", "doc_count": 1}]},
            "b": {"buckets": [{"key": "y", "doc_count": 2}]}
        }));
        assert_eq!(names, ["aggregation", "result"]);
        assert_eq!(rows.len(), 2);
        assert!(
            rows[1][1]
                .as_deref()
                .is_some_and(|json| json.contains("\"y\""))
        );
    }

    #[test]
    fn a_nested_sibling_is_shown_once_not_under_every_inner_bucket() {
        let hosts: Vec<Value> = (0..100)
            .map(|n| serde_json::json!({"key": format!("h{n}"), "doc_count": 1}))
            .collect();
        let paths = serde_json::json!({"buckets": [{"key": "/a", "doc_count": 1}]});
        let (names, rows) = table(serde_json::json!({
            "per_day": {"buckets": [
                {"key": 1, "doc_count": 100, "by_host": {"buckets": hosts}, "by_path": paths}
            ]}
        }));
        assert_eq!(rows.len(), 100);
        let column = names
            .iter()
            .position(|name| name == "by_path")
            .expect("the sibling's column");
        assert!(
            rows[0][column]
                .as_deref()
                .is_some_and(|json| json.contains("/a"))
        );
        assert!(rows.iter().skip(1).all(|row| row[column].is_none()));
        // Keys and counts still repeat, as a grid reads them.
        let day = names
            .iter()
            .position(|name| name == "per_day")
            .expect("a key");
        assert!(rows.iter().all(|row| row[day].as_deref() == Some("1")));
    }

    #[test]
    fn percentiles_and_keyed_filters_get_their_own_columns() {
        let (names, rows) = table(serde_json::json!({
            "kinds": {"buckets": {"errors": {"doc_count": 3, "latency": {"values": {"50.0": 12.0, "99.0": 80.5}}}}}
        }));
        assert_eq!(
            names,
            ["kinds", "kinds.doc_count", "latency.50.0", "latency.99.0"]
        );
        assert_eq!(rows[0][0].as_deref(), Some("errors"));
        assert_eq!(rows[0][3].as_deref(), Some("80.5"));
    }
}
