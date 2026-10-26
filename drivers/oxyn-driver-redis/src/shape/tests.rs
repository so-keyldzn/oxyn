use rstest::rstest;

use super::*;

fn argv(line: &str) -> Vec<Vec<u8>> {
    crate::argv::parse(line).expect("valid")
}

#[rstest]
#[case("GET k", Head::Bulk(3), Layout::Scalar(ScalarKind::Text))]
#[case("INCR k", Head::Integer(1), Layout::Scalar(ScalarKind::Integer))]
#[case("GET k", Head::Null, Layout::Scalar(ScalarKind::Null))]
#[case(
    "LRANGE k 0 -1",
    Head::Aggregate(Aggregate::Array, 3),
    Layout::Elements
)]
#[case("SMEMBERS k", Head::Aggregate(Aggregate::Set, 3), Layout::Elements)]
#[case("HGETALL k", Head::Aggregate(Aggregate::Map, 3), Layout::Pairs)]
#[case(
    "SCAN 0",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scan(ScanForm::Keys)
)]
#[case(
    "hscan h 0 NOVALUES",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scan(ScanForm::HashFieldNames)
)]
#[case(
    "HSCAN h 0",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scan(ScanForm::HashFields)
)]
#[case(
    "ZRANGE z 0 -1 WITHSCORES",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scored
)]
#[case(
    "ZRANGE z 0 -1",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Elements
)]
// An option word in a key, a bound or an option's value is not the option.
#[case(
    "HSCAN novalues 0 COUNT 10",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scan(ScanForm::HashFields)
)]
#[case(
    "HSCAN h 0 MATCH novalues",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scan(ScanForm::HashFields)
)]
#[case(
    "HSCAN h 0 MATCH x* COUNT 10 NOVALUES",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scan(ScanForm::HashFieldNames)
)]
#[case(
    "ZRANGE withscores 0 -1",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Elements
)]
#[case(
    "ZRANGE z 0 10 BYSCORE LIMIT 0 5 WITHSCORES",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Scored
)]
#[case(
    "XRANGE s - +",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::StreamEntries
)]
#[case("INFO", Head::Verbatim(10), Layout::Info)]
#[case(
    "SLOWLOG GET 10",
    Head::Aggregate(Aggregate::Array, 2),
    Layout::Slowlog
)]
#[case("SLOWLOG LEN", Head::Integer(2), Layout::Scalar(ScalarKind::Integer))]
#[case("client list", Head::Verbatim(90), Layout::Clients)]
#[case("CLIENT INFO", Head::Bulk(90), Layout::Clients)]
#[case("CLIENT GETNAME", Head::Bulk(4), Layout::Scalar(ScalarKind::Text))]
// A command that is not the shape's own falls back to the generic one.
#[case("SCAN 0", Head::Null, Layout::Scalar(ScalarKind::Null))]
fn the_layout_follows_the_reply_then_the_command(
    #[case] line: &str,
    #[case] head: Head,
    #[case] expected: Layout,
) {
    assert_eq!(Layout::choose(&argv(line), &head), expected);
}

#[test]
fn rows_fill_a_batch_in_schema_order() {
    let mut rows = Rows::new(Layout::Elements.schema()).expect("supported columns");
    rows.push(&[
        Slot::Integer(Some(0)),
        Slot::Text(Some("bulk".to_owned())),
        Slot::Text(Some("v".to_owned())),
        Slot::Bytes(None),
        Slot::Boolean(Some(false)),
    ])
    .expect("matching row");
    let batch = rows.take().expect("a batch");
    assert_eq!(batch.num_rows(), 1);
    assert!(rows.is_empty(), "the next batch starts empty");
}

#[test]
fn a_row_that_does_not_match_is_refused_not_shifted() {
    let mut rows = Rows::new(Layout::Elements.schema()).expect("supported columns");
    assert!(rows.push(&[Slot::Integer(Some(0))]).is_err());
    assert!(
        rows.push(&[
            Slot::Text(None),
            Slot::Text(None),
            Slot::Text(None),
            Slot::Bytes(None),
            Slot::Boolean(Some(false)),
        ])
        .is_err()
    );
    assert!(rows.is_empty());
}

#[test]
fn a_batch_closes_on_bytes_not_on_rows() {
    let mut rows = Rows::new(Layout::Scalar(ScalarKind::Text).schema()).expect("supported");
    let big = "x".repeat(BATCH_BYTE_BUDGET / 4);
    let mut pushed = 0;
    while !rows.is_full() {
        rows.push(&[
            Slot::Text(Some(big.clone())),
            Slot::Bytes(None),
            Slot::Text(Some("bulk".to_owned())),
            Slot::Boolean(Some(false)),
        ])
        .expect("matching row");
        pushed += 1;
    }
    assert!(pushed <= 4, "{pushed} rows of a quarter budget each");
}

#[test]
fn info_becomes_sections_names_and_values() {
    let rows = info_rows(
        "# Server\r\nredis_version:8.10.2\r\nodd line\r\n\r\n# Keyspace\r\ndb0:keys=1,expires=0\r\n",
    );
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0], Slot::Text(Some("Server".to_owned())));
    assert_eq!(rows[0][2], Slot::Text(Some("8.10.2".to_owned())));
    assert_eq!(rows[1][1], Slot::Text(Some("odd line".to_owned())));
    assert_eq!(rows[1][2], Slot::Text(None));
    assert_eq!(rows[2][0], Slot::Text(Some("Keyspace".to_owned())));
}
