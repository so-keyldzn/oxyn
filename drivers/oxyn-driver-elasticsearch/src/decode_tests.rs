use arrow::array::{Array, Int64Array, StringArray, TimestampMillisecondArray, UInt64Array};
use arrow::datatypes::DataType;
use serde_json::Value;

use super::*;
use crate::cells::{MAX_COLUMNS, META_TYPE};
use crate::columnar::Tabular;
use crate::cursor::Next;

fn reply(body: &str) -> Reply {
    Reply {
        status: 200,
        json: true,
        bytes: body.as_bytes().to_vec(),
    }
}

fn text(body: &str) -> Reply {
    Reply {
        status: 200,
        json: false,
        bytes: body.as_bytes().to_vec(),
    }
}

/// A read of `path`, without a body.
fn at(path: &str) -> Asked<'_> {
    Asked {
        path,
        body: None,
        head: false,
        read: true,
    }
}

fn longs(decoded: &Decoded, column: usize) -> Vec<Option<i64>> {
    decoded
        .batches
        .iter()
        .flat_map(|batch| {
            let column = batch
                .column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("int64");
            (0..column.len()).map(|row| (!column.is_null(row)).then(|| column.value(row)))
        })
        .collect()
}

fn text_cell(decoded: &Decoded, column: &str, row: usize) -> Option<String> {
    let batch = decoded.batches.first().expect("a batch");
    let column = batch
        .column_by_name(column)
        .unwrap_or_else(|| panic!("a column `{column}`"));
    let column = column.as_any().downcast_ref::<StringArray>().expect("utf8");
    (!column.is_null(row)).then(|| column.value(row).to_owned())
}

fn names(decoded: &Decoded) -> Vec<String> {
    decoded
        .schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect()
}

#[test]
fn hits_become_rows_with_metadata_source_and_every_other_key() {
    let decoded = decode(
        &reply(
            r#"{"took":3,"hits":{"hits":[
                {"_index":"logs","_id":"1","_score":1.5,"_source":{"msg":"a","n":18446744073709551615,"tags":["x","y"]},"sort":[1]},
                {"_index":"logs","_id":"2","_score":null,"_source":{"msg":"b","extra":{"k":1}},"highlight":{"msg":["<em>b</em>"]}}
            ]}}"#,
        ),
        &at("/_search"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(
        names(&decoded),
        [
            "_index",
            "_id",
            "_score",
            "msg",
            "n",
            "tags",
            "extra",
            "sort",
            "highlight"
        ]
    );
    assert_eq!(
        text_cell(&decoded, "n", 0).as_deref(),
        Some("18446744073709551615")
    );
    assert_eq!(
        text_cell(&decoded, "tags", 0).as_deref(),
        Some(r#"["x","y"]"#)
    );
    assert_eq!(text_cell(&decoded, "_score", 1), None);
    assert_eq!(
        text_cell(&decoded, "n", 1),
        None,
        "absent is null, not dropped"
    );
    assert_eq!(
        text_cell(&decoded, "extra", 1).as_deref(),
        Some(r#"{"k":1}"#)
    );
    assert_eq!(decoded.took, Some(Duration::from_millis(3)));
    assert_eq!(decoded.page.rows, 2);
}

#[test]
fn a_disabled_source_leaves_the_metadata_and_a_source_key_named_like_a_hit_key_keeps_its_own_column()
 {
    let decoded = decode(
        &reply(r#"{"hits":{"hits":[{"_index":"a","_id":"1","_score":1,"sort":[5],"_source":{"sort":"mine"}},{"_index":"a","_id":"2","_score":1}]}}"#),
        &at("/_search"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(
        names(&decoded),
        ["_index", "_id", "_score", "_source.sort", "sort"]
    );
    assert_eq!(
        text_cell(&decoded, "_source.sort", 0).as_deref(),
        Some("mine")
    );
    assert_eq!(text_cell(&decoded, "sort", 0).as_deref(), Some("[5]"));
}

#[test]
fn a_later_page_keeps_the_first_layout_and_puts_new_keys_in_other() {
    let first = decode(
        &reply(r#"{"pit_id":"p2","hits":{"hits":[{"_index":"a","_id":"1","_score":null,"_source":{"x":1},"sort":[1,0]}]}}"#),
        &at("/_search"),
        Stream::First,
    )
    .expect("first");
    assert_eq!(
        names(&first),
        ["_index", "_id", "_score", "x", "sort", "_other"]
    );
    assert_eq!(first.page.pit_id.as_deref(), Some("p2"));
    assert_eq!(
        first.page.last_sort.as_ref().map(|sort| sort.get()),
        Some("[1,0]")
    );
    let layout = first.layout.as_ref().expect("a layout");
    let next = decode(
        &reply(r#"{"hits":{"hits":[{"_index":"a","_id":"2","_score":null,"_source":{"x":2,"y":"new"},"sort":[2,0]}]}}"#),
        &at("/_search"),
        Stream::Next(layout),
    )
    .expect("next");
    assert_eq!(names(&next), names(&first));
    assert_eq!(
        text_cell(&next, "_other", 0).as_deref(),
        Some(r#"{"y":"new"}"#)
    );
}

#[test]
fn aggregations_without_hits_become_their_table() {
    let decoded = decode(
        &reply(
            r#"{"hits":{"hits":[]},"aggregations":{"by":{"buckets":[{"key":"a","doc_count":2}]}}}"#,
        ),
        &at("/_search"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&decoded), ["by", "by.doc_count"]);
}

#[test]
fn cat_text_splits_on_its_header_and_falls_back_to_lines() {
    let text = |body: &str| Reply {
        status: 200,
        json: false,
        bytes: body.as_bytes().to_vec(),
    };
    let table = decode(
        &text("health status index\ngreen  open   logs\nyellow open   web\n"),
        &at("/_cat/indices?v"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&table), ["health", "status", "index"]);
    assert_eq!(text_cell(&table, "index", 1).as_deref(), Some("web"));
    let ragged =
        decode(&text("a b\nc d e\n"), &at("/_cat/indices?v"), Stream::No).expect("decoded");
    assert_eq!(names(&ragged), ["line"]);
}

#[test]
fn an_array_of_objects_is_a_table_and_an_object_one_row() {
    let cat = decode(
        &reply(r#"[{"index":"a","docs.count":"3"},{"index":"b","pri":"1"}]"#),
        &at("/_cat/indices?format=json"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&cat), ["index", "docs.count", "pri"]);
    let health = decode(
        &reply(r#"{"cluster_name":"c","status":"green","number_of_nodes":3}"#),
        &at("/_cluster/health"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(
        names(&health),
        ["cluster_name", "status", "number_of_nodes"]
    );
    assert_eq!(
        text_cell(&health, "number_of_nodes", 0).as_deref(),
        Some("3")
    );
}

#[test]
fn esql_columns_are_typed_and_a_multi_valued_column_stays_text() {
    let decoded = decode(
        &reply(
            r#"{"took":5,"columns":[{"name":"n","type":"unsigned_long"},{"name":"at","type":"date"},{"name":"tags","type":"keyword"},{"name":"c","type":"long"}],
                "values":[[18446744073709551615,"2024-01-01T00:00:00.000Z","a",1],[null,null,"b",[2,3]]]}"#,
        ),
        &at("/_query"),
        Stream::No,
    )
    .expect("decoded");
    let batch = decoded.batches.first().expect("a batch");
    let n = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .expect("u64");
    assert_eq!(n.value(0), u64::MAX);
    assert!(n.is_null(1));
    let at = batch
        .column(1)
        .as_any()
        .downcast_ref::<TimestampMillisecondArray>()
        .expect("timestamp");
    assert_eq!(at.value(0), 1_704_067_200_000);
    assert_eq!(
        decoded.schema.field(3).data_type(),
        &DataType::Utf8,
        "an array in a long column"
    );
    assert_eq!(
        decoded
            .schema
            .field(3)
            .metadata()
            .get(META_TYPE)
            .map(String::as_str),
        Some("long")
    );
}

#[test]
fn a_timestamp_without_offset_is_text_not_a_guessed_zone() {
    let decoded = decode(
        &reply(r#"{"schema":[{"name":"t","type":"timestamp"},{"name":"d","type":"date"}],"datarows":[["2024-01-01 00:00:00","2024-01-01 00:00:00"]],"total":1,"size":1,"status":200}"#),
        &at("/_plugins/_sql"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(decoded.schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(decoded.schema.field(1).data_type(), &DataType::Utf8);
}

#[test]
fn sql_reports_its_cursor() {
    let decoded = decode(
        &reply(r#"{"columns":[{"name":"a","type":"integer"}],"rows":[[1],[2]],"cursor":"c1"}"#),
        &at("/_sql"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(decoded.page.sql_cursor.as_deref(), Some("c1"));
    assert_eq!(decoded.schema.field(0).data_type(), &DataType::Int32);
}

#[test]
fn a_head_is_its_status() {
    let decoded = decode(
        &Reply {
            status: 404,
            json: false,
            bytes: Vec::new(),
        },
        &Asked {
            head: true,
            ..at("/")
        },
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&decoded), ["status", "exists"]);
}

#[test]
fn source_keys_beyond_the_column_bound_go_into_other_not_away() {
    // One hit per distinct key: a sparse grid of 1,100 columns × 1,100 rows.
    let hits: Vec<String> = (0..1100)
        .map(|n| format!(r#"{{"_index":"logs","_id":"{n}","_source":{{"k{n}":{n}}}}}"#))
        .collect();
    let body = format!(r#"{{"hits":{{"hits":[{}]}}}}"#, hits.join(","));
    let decoded = decode(&reply(&body), &at("/_search"), Stream::No).expect("decoded");
    let names = names(&decoded);
    assert_eq!(names.len(), 3 + 1024 + 1, "metadata, held keys, `_other`");
    assert_eq!(names.last().map(String::as_str), Some("_other"));
    let rows: usize = decoded.batches.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(rows, 1100);
    let last = decoded.batches.last().expect("a batch");
    let other = last
        .column_by_name("_other")
        .expect("`_other`")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("utf8");
    assert_eq!(other.value(other.len() - 1), r#"{"k1099":1099}"#);
}

#[test]
fn hostile_answers_are_errors_not_panics() {
    for body in [
        "{",
        r#"{"hits":{"hits":[{"_source":"not an object"}]}}"#,
        r#"{"hits":{"hits":"no"}}"#,
        r#"{"columns":[{"name":"a","type":"long"}],"values":[[1,2]]}"#,
        "[1, {\"a\": ",
    ] {
        assert!(
            decode(&reply(body), &at("/_query"), Stream::No).is_err(),
            "{body}"
        );
    }
    let invalid_utf8 = Reply {
        status: 200,
        json: true,
        bytes: b"{\"a\":\"\xff\"}".to_vec(),
    };
    assert!(decode(&invalid_utf8, &at("/_search"), Stream::No).is_err());
}

// --- The request decides the reading -------------------------------------

#[test]
fn a_headerless_cat_answer_keeps_its_first_row() {
    // `GET /_cat/indices` without `v`: equal-width rows, none of them a header.
    let body = "green open logs 1 1\nyellow open web 1 1\n";
    let decoded = decode(&text(body), &at("/_cat/indices"), Stream::No).expect("decoded");
    assert_eq!(names(&decoded), ["line"]);
    assert_eq!(
        text_cell(&decoded, "line", 0).as_deref(),
        Some("green open logs 1 1")
    );
    assert_eq!(decoded.batches[0].num_rows(), 2);
    let refused = decode(&text(body), &at("/_cat/indices?v=false"), Stream::No).expect("decoded");
    assert_eq!(names(&refused), ["line"], "`v=false` asks for no header");
    let asked = decode(
        &text("health status index\ngreen open logs\n"),
        &at("/_cat/indices?h=health,status,index&v=true"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&asked), ["health", "status", "index"]);
    let one = decode(
        &text("index\nlogs\n"),
        &at("/_cat/indices?v&h=index"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&one), ["index"], "one asked column is a header too");
}

#[test]
fn a_text_answer_outside_cat_is_its_lines() {
    let decoded =
        decode(&text("a b\nc d\n"), &at("/_sql?format=txt&v"), Stream::No).expect("decoded");
    assert_eq!(names(&decoded), ["line"]);
}

#[test]
fn columns_and_values_outside_a_tabular_endpoint_are_one_object_row() {
    let decoded = decode(
        &reply(r#"{"columns":[{"name":"a","type":"long"}],"values":[[1]]}"#),
        &at("/my-index/_doc/1"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&decoded), ["columns", "values"]);
    assert!(decoded.tabular.is_none());
}

fn with_body<'a>(path: &'a str, body: &'a Value) -> Asked<'a> {
    Asked {
        body: Some(body),
        ..at(path)
    }
}

#[test]
fn a_columnar_answer_is_read_by_columns() {
    let columnar = serde_json::json!({"query": "FROM t", "columnar": true});
    let square = decode(
        &reply(r#"{"columns":[{"name":"a","type":"long"},{"name":"b","type":"long"}],"values":[[1,2],[3,4]]}"#),
        &with_body("/_query", &columnar),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(
        longs(&square, 0),
        [Some(1), Some(2)],
        "column `a` is [1, 2]"
    );
    assert_eq!(
        longs(&square, 1),
        [Some(3), Some(4)],
        "rows are [1, 3] and [2, 4]"
    );

    let tall = decode(
        &reply(r#"{"columns":[{"name":"n","type":"long"},{"name":"s","type":"keyword"}],"values":[[1,2,3],["x","y",null]]}"#),
        &with_body("/_query", &columnar),
        Stream::No,
    )
    .expect("two columns of three rows");
    assert_eq!(longs(&tall, 0), [Some(1), Some(2), Some(3)]);
    assert_eq!(text_cell(&tall, "s", 1).as_deref(), Some("y"));
    assert_eq!(text_cell(&tall, "s", 2), None);

    let rows = decode(
        &reply(r#"{"columns":[{"name":"a","type":"long"},{"name":"b","type":"long"}],"values":[[1,2],[3,4]]}"#),
        &at("/_query"),
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(
        longs(&rows, 0),
        [Some(1), Some(3)],
        "without `columnar`, rows"
    );

    for values in [r"[[1,2],[3]]", r"[[1],[2],[3]]"] {
        let body = format!(
            r#"{{"columns":[{{"name":"a","type":"long"}},{{"name":"b","type":"long"}}],"values":{values}}}"#
        );
        assert!(
            decode(&reply(&body), &with_body("/_query", &columnar), Stream::No).is_err(),
            "{values}"
        );
    }
}

#[test]
fn an_sql_columnar_answer_is_sql_and_keeps_its_cursor_and_orientation() {
    let columnar =
        serde_json::json!({"query": "SELECT a FROM t", "fetch_size": 2, "columnar": true});
    let asked = with_body("/_sql?format=json", &columnar);
    let decoded = decode(
        &reply(r#"{"columns":[{"name":"a","type":"long"}],"values":[[1,2]],"cursor":"c1"}"#),
        &asked,
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(longs(&decoded, 0), [Some(1), Some(2)]);
    let Some(Next::Sql { cursor, shape, .. }) = asked.next(&decoded) else {
        panic!("an SQL cursor to follow");
    };
    assert_eq!(cursor, "c1");
    assert_eq!(shape.tabular, Tabular::Sql);
    assert!(shape.columnar, "the next page is asked columnar again");

    let esql = decode(
        &reply(r#"{"columns":[{"name":"a","type":"long"}],"values":[[1,2]]}"#),
        &with_body("/_query", &columnar),
        Stream::No,
    )
    .expect("decoded");
    assert!(at("/_query").next(&esql).is_none(), "ES|QL has no cursor");
}

#[test]
fn a_later_sql_page_keeps_the_first_pages_orientation_and_bounds() {
    let columnar = serde_json::json!({"query": "SELECT a, b FROM t", "columnar": true});
    let first = decode(
        &reply(r#"{"columns":[{"name":"a","type":"long"},{"name":"b","type":"keyword"}],"values":[[1],["x"]],"cursor":"c1"}"#),
        &with_body("/_sql", &columnar),
        Stream::No,
    )
    .expect("decoded");
    let (columns, shape) = first.tabular.clone().expect("tabular");
    let next = next_tabular(
        &reply(r#"{"values":[[2,3],["y","z"]],"cursor":"c2"}"#),
        &columns,
        shape,
    )
    .expect("a later page");
    assert_eq!(longs(&next, 0), [Some(2), Some(3)]);
    assert_eq!(next.page.sql_cursor.as_deref(), Some("c2"));
    assert!(
        next_tabular(&reply(r#"{"values":[[2,3],["y"]]}"#), &columns, shape).is_err(),
        "a ragged later page"
    );
    assert!(
        next_tabular(
            &reply(r#"{"values":[[2],["y"]],"is_partial":true}"#),
            &columns,
            shape
        )
        .is_err(),
        "a later page that says it is partial"
    );
}

fn esql_columns(count: usize) -> String {
    let columns: Vec<String> = (0..count)
        .map(|n| format!(r#"{{"name":"c{n}","type":"long"}}"#))
        .collect();
    let row: Vec<&str> = (0..count).map(|_| "1").collect();
    format!(
        r#"{{"columns":[{}],"values":[[{}]]}}"#,
        columns.join(","),
        row.join(",")
    )
}

#[test]
fn a_tabular_answer_takes_4096_columns_and_refuses_4097() {
    let at_bound = decode(
        &reply(&esql_columns(MAX_COLUMNS)),
        &at("/_query"),
        Stream::No,
    )
    .expect("4096 columns");
    assert_eq!(at_bound.schema.fields().len(), MAX_COLUMNS);
    let beyond = decode(
        &reply(&esql_columns(MAX_COLUMNS + 1)),
        &at("/_query"),
        Stream::No,
    )
    .err()
    .expect("4097 columns are refused");
    assert!(beyond.to_string().contains("4097 columns"), "{beyond}");
    // The same bound on Elasticsearch SQL and OpenSearch SQL.
    let sql = esql_columns(MAX_COLUMNS + 1).replace("\"values\"", "\"rows\"");
    assert!(decode(&reply(&sql), &at("/_sql"), Stream::No).is_err());
    let opensearch = esql_columns(MAX_COLUMNS + 1)
        .replace("\"columns\"", "\"schema\"")
        .replace("\"values\"", "\"datarows\"");
    assert!(decode(&reply(&opensearch), &at("/_plugins/_sql"), Stream::No).is_err());
}

// --- A read whose answer says it is incomplete ----------------------------

#[test]
fn a_search_that_timed_out_or_lost_shards_is_refused_not_shown_complete() {
    let timed_out = r#"{"took":5,"timed_out":true,"_shards":{"total":1,"successful":1,"skipped":0,"failed":0},"hits":{"hits":[{"_index":"a","_id":"1","_score":1,"_source":{}}]}}"#;
    let refused = decode(&reply(timed_out), &at("/a/_search"), Stream::No)
        .err()
        .expect("a partial search");
    assert!(refused.to_string().contains("timed_out"), "{refused}");
    let write = Asked {
        read: false,
        ..at("/a/_search")
    };
    assert!(
        decode(&reply(timed_out), &write, Stream::No).is_ok(),
        "a write's partial outcome is the session's to report"
    );

    let aggregation = r#"{"timed_out":false,"_shards":{"total":2,"successful":1,"failed":1,"failures":[{"shard":0,"index":"b","reason":{"type":"illegal_argument_exception","reason":"no"}}]},"hits":{"hits":[]},"aggregations":{"by":{"buckets":[{"key":"a","doc_count":2}]}}}"#;
    assert!(decode(&reply(aggregation), &at("/a,b/_search"), Stream::No).is_err());

    let count = r#"{"count":7,"_shards":{"total":2,"successful":1,"skipped":0,"failed":1}}"#;
    assert!(decode(&reply(count), &at("/a,b/_count"), Stream::No).is_err());
}

#[test]
fn a_later_page_that_timed_out_is_refused_rather_read_as_the_last() {
    let first = decode(
        &reply(r#"{"pit_id":"p","timed_out":false,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[{"_index":"a","_id":"1","_score":null,"_source":{"x":1},"sort":[1]}]}}"#),
        &at("/_search"),
        Stream::First,
    )
    .expect("first");
    let layout = first.layout.as_ref().expect("a layout");
    // Shorter than its `size`: complete, it would end the stream quietly.
    let short = r#"{"pit_id":"p","timed_out":true,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]}}"#;
    assert!(decode(&reply(short), &at("/_search"), Stream::Next(layout)).is_err());
    let scroll = r#"{"_scroll_id":"s","timed_out":false,"_shards":{"total":2,"successful":1,"failed":1},"hits":{"hits":[]}}"#;
    assert!(decode(&reply(scroll), &at("/_search/scroll"), Stream::Next(layout)).is_err());
}

#[test]
fn a_partial_esql_answer_is_refused_before_its_columns_are_read() {
    let partial =
        r#"{"took":3,"is_partial":true,"columns":[{"name":"a","type":"long"}],"values":[[1]]}"#;
    let refused = decode(&reply(partial), &at("/_query"), Stream::No)
        .err()
        .expect("a partial ES|QL answer");
    assert!(refused.to_string().contains("is_partial"), "{refused}");
    let complete =
        r#"{"took":3,"is_partial":false,"columns":[{"name":"a","type":"long"}],"values":[[1]]}"#;
    assert!(decode(&reply(complete), &at("/_query"), Stream::No).is_ok());
}

#[test]
fn a_running_asynchronous_query_is_its_status_and_an_ended_partial_one_refused() {
    let running = r#"{"id":"FmNJ","is_running":true,"is_partial":true,"columns":[{"name":"a","type":"long"}],"values":[[1]]}"#;
    let status = decode(&reply(running), &at("/_query/async"), Stream::No).expect("a status");
    assert!(status.tabular.is_none(), "not shown as the result");
    assert_eq!(text_cell(&status, "is_running", 0).as_deref(), Some("true"));
    assert_eq!(text_cell(&status, "id", 0).as_deref(), Some("FmNJ"));
    let ended = r#"{"id":"FmNJ","is_running":false,"is_partial":true,"response":{"timed_out":false,"_shards":{"total":2,"successful":1,"failed":1},"hits":{"hits":[]}}}"#;
    assert!(decode(&reply(ended), &at("/_async_search/FmNJ"), Stream::No).is_err());
}

#[test]
fn an_msearch_with_a_failed_or_partial_search_is_refused() {
    let failed = r#"{"took":1,"responses":[{"took":1,"timed_out":false,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]},"status":200},{"error":{"type":"index_not_found_exception","reason":"no such index [x]"},"status":404}]}"#;
    let refused = decode(&reply(failed), &at("/_msearch"), Stream::No)
        .err()
        .expect("a failed sub-search");
    assert!(
        refused.to_string().contains("search 2 of the 2"),
        "{refused}"
    );
    let fine = r#"{"took":1,"responses":[{"took":1,"timed_out":false,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]},"status":200}]}"#;
    assert!(decode(&reply(fine), &at("/_msearch"), Stream::No).is_ok());
}
