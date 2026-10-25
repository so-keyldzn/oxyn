use super::*;
use crate::http::RESPONSE_BYTE_LIMIT;

fn object(text: &str) -> IndexMap<String, Box<RawValue>> {
    serde_json::from_str(text).expect("an object")
}

const ROWS: Shape = Shape {
    tabular: Tabular::Esql,
    columnar: false,
};

/// What the budget charges for `rows` cells of `kind`, in one batch.
fn charged(kind: Kind, values: &[&RawValue]) -> usize {
    values
        .iter()
        .map(|value| kind.cell_bits(value))
        .sum::<usize>()
        .div_ceil(8)
        + COLUMN_OVERHEAD
}

fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).expect("valid JSON")
}

#[test]
fn the_cost_charged_is_the_arrow_cost_not_the_json_length() {
    let (one, null, word) = (raw("1"), raw("null"), raw(r#""abc""#));
    let (one, null, word): (&RawValue, &RawValue, &RawValue) = (&one, &null, &word);
    let cases: [(Kind, Vec<&RawValue>); 4] = [
        // A `1` is one byte of JSON and eight of `Int64`.
        (Kind::Int64, vec![one; 1000]),
        // A validity bitmap appears with the first null.
        (Kind::Int64, [one, null].repeat(500)),
        // Three bytes of text, an `i32` offset, and the offsets' extra slot.
        (Kind::Text, vec![word; 1000]),
        (Kind::Text, [word, null].repeat(500)),
    ];
    for (kind, values) in cases {
        let array = column(kind, &values);
        let real = array.get_array_memory_size();
        let charged = charged(kind, &values);
        assert!(
            real <= charged,
            "{kind:?}: Arrow takes {real} bytes, the budget charged {charged}"
        );
        // At most the padding, and a validity bitmap a column without null
        // does not allocate.
        assert!(
            charged <= real + COLUMN_OVERHEAD + values.len().div_ceil(8),
            "{kind:?}: the budget charged {charged} for {real} bytes"
        );
    }
    let json: usize = vec![one; 1000].iter().map(|value| value.get().len()).sum();
    assert!(charged(Kind::Int64, &vec![one; 1000]) >= 8 * json);
}

/// An ES|QL answer of `rows` rows of `width` `long` zeros, and its length.
fn zeros(rows: usize, width: usize) -> String {
    let columns: Vec<String> = (0..width)
        .map(|n| format!(r#"{{"name":"c{n}","type":"long"}}"#))
        .collect();
    let row = format!("[{}]", vec!["0"; width].join(","));
    format!(
        r#"{{"columns":[{}],"values":[{}]}}"#,
        columns.join(","),
        vec![row.as_str(); rows].join(",")
    )
}

#[test]
fn the_budget_is_the_whole_answer_not_one_batch() {
    // 64 `long` columns: two bytes of JSON a cell, eight bytes and a bit of
    // Arrow. Just past 64 MiB of Arrow, the answer is under the 16 MiB a
    // reply may weigh on the wire, and every batch under 4 MiB.
    let width = 256;
    let rows = (DECODED_LIMIT * 8).div_ceil(width * 65) + 1;
    let beyond = zeros(rows, width);
    assert!(beyond.len() < RESPONSE_BYTE_LIMIT, "{} bytes", beyond.len());
    let refused = decode(&object(&beyond), ROWS)
        .err()
        .expect("past the decoded bound");
    assert!(refused.to_string().contains("64 MiB"), "{refused}");

    let within = decode(&object(&zeros(rows / 2, width)), ROWS)
        .expect("decoded")
        .expect("tabular");
    assert!(within.batches.len() > 1, "{} batches", within.batches.len());
    let real: usize = within
        .batches
        .iter()
        .map(RecordBatch::get_array_memory_size)
        .sum();
    assert!(real <= DECODED_LIMIT, "{real} bytes of Arrow");
    for batch in &within.batches {
        assert!(batch.get_array_memory_size() <= BATCH_BYTES + width * COLUMN_OVERHEAD);
    }
}

#[test]
fn a_later_page_is_bounded_like_the_first() {
    let width = 256;
    let first = decode(
        &object(&zeros(1, width)),
        Shape {
            tabular: Tabular::Sql,
            columnar: false,
        },
    )
    .expect("decoded");
    assert!(first.is_none(), "an `_sql` answer has `rows`, not `values`");
    let sql = zeros(1, width).replace("\"values\"", "\"rows\"");
    let shape = Shape {
        tabular: Tabular::Sql,
        columnar: false,
    };
    let first = decode(&object(&sql), shape)
        .expect("decoded")
        .expect("tabular");
    let (columns, shape) = first.tabular.expect("columns");
    let rows = (DECODED_LIMIT * 8).div_ceil(width * 65) + 1;
    let later = zeros(rows, width).replace("\"values\"", "\"rows\"");
    assert!(next_page(&columns, &object(&later), shape).is_err());
}

#[test]
fn endpoints_name_their_api() {
    assert_eq!(Tabular::of_path("/_query"), Some(Tabular::Esql));
    assert_eq!(
        Tabular::of_path("/_query/async/FmNJ?wait_for_completion_timeout=2s"),
        Some(Tabular::Esql)
    );
    assert_eq!(Tabular::of_path("/_sql?format=json"), Some(Tabular::Sql));
    assert_eq!(
        Tabular::of_path("/_plugins/_sql"),
        Some(Tabular::OpenSearchSql)
    );
    assert_eq!(
        Tabular::of_path("/_opendistro/_ppl"),
        Some(Tabular::OpenSearchSql)
    );
    assert_eq!(Tabular::of_path("/_query_rules"), None);
    assert_eq!(Tabular::of_path("/_sqlx"), None);
    assert_eq!(Tabular::of_path("/logs/_search"), None);
    assert!(!Shape::new(Tabular::OpenSearchSql, true).columnar);
}
