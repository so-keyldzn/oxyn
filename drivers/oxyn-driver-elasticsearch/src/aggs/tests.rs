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
                        let column = column.as_any().downcast_ref::<StringArray>().expect("utf8");
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

fn expanding(inner: usize) -> Map<String, Value> {
    // One outer bucket whose 1 MiB key repeats under every inner bucket.
    let inner: Vec<Value> = (0..inner)
        .map(|n| serde_json::json!({"key": n, "doc_count": 1}))
        .collect();
    let value = serde_json::json!({
        "outer": {"buckets": [{"key": "k".repeat(1 << 20), "doc_count": 1, "inner": {"buckets": inner}}]}
    });
    serde_json::from_value(value).expect("an object")
}

#[test]
fn rows_reach_the_bounded_table_one_at_a_time() {
    let mut built = 0;
    let refused = rows(&expanding(1000), &mut |_| {
        built += 1;
        Err(OxynError::Query("full".to_owned()))
    });
    assert!(refused.is_err());
    assert_eq!(built, 1, "the walk stops at the first refusal");

    // A thousand rows of 1 MiB: refused at 64 MiB, not built to 1 GiB.
    let raw: IndexMap<String, Box<RawValue>> =
        serde_json::from_str(&Value::Object(expanding(1000)).to_string()).expect("an object");
    let refused = flatten(&raw).expect_err("past the decoded bound");
    assert!(refused.to_string().contains("64 MiB"), "{refused}");
}

#[test]
fn aggregations_beyond_the_column_bound_are_refused() {
    let metrics: Map<String, Value> = (0..=MAX_COLUMNS)
        .map(|n| (format!("m{n}"), serde_json::json!({"value": n})))
        .collect();
    let raw: IndexMap<String, Box<RawValue>> =
        serde_json::from_str(&Value::Object(metrics).to_string()).expect("an object");
    assert!(flatten(&raw).is_err());
}
