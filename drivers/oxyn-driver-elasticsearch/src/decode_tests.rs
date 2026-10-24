use arrow::array::{Array, StringArray, TimestampMillisecondArray, UInt64Array};
use arrow::datatypes::DataType;

use super::*;
use crate::cells::META_TYPE;

fn reply(body: &str) -> Reply {
    Reply {
        status: 200,
        json: true,
        bytes: body.as_bytes().to_vec(),
    }
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
        false,
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
        false,
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
        false,
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
        false,
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
        false,
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
        false,
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&table), ["health", "status", "index"]);
    assert_eq!(text_cell(&table, "index", 1).as_deref(), Some("web"));
    let ragged = decode(&text("a b\nc d e\n"), false, Stream::No).expect("decoded");
    assert_eq!(names(&ragged), ["line"]);
}

#[test]
fn an_array_of_objects_is_a_table_and_an_object_one_row() {
    let cat = decode(
        &reply(r#"[{"index":"a","docs.count":"3"},{"index":"b","pri":"1"}]"#),
        false,
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&cat), ["index", "docs.count", "pri"]);
    let health = decode(
        &reply(r#"{"cluster_name":"c","status":"green","number_of_nodes":3}"#),
        false,
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
        false,
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
        false,
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
        false,
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
        true,
        Stream::No,
    )
    .expect("decoded");
    assert_eq!(names(&decoded), ["status", "exists"]);
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
        assert!(decode(&reply(body), false, Stream::No).is_err(), "{body}");
    }
    let invalid_utf8 = Reply {
        status: 200,
        json: true,
        bytes: b"{\"a\":\"\xff\"}".to_vec(),
    };
    assert!(decode(&invalid_utf8, false, Stream::No).is_err());
}
