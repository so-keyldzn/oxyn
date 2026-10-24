//! The type table, crossed with a real server: every decoded value is
//! compared with what the server computes for it in the same row, never with
//! a constant written from memory.

use arrow::array::{Array, AsArray};
use arrow::datatypes::{
    DataType, Int64Type, TimeUnit, TimestampNanosecondType, TimestampSecondType, UInt64Type,
};
use arrow::record_batch::RecordBatch;
use arrow::util::display::array_value_to_string;
use oxyn_core::{ErrorClass, ExecLimits};

use super::{code, read, run, session};
use crate::META_CLICKHOUSE_TYPE;

/// One row, one batch.
async fn row(session: &dyn oxyn_driver::Session, sql: &str) -> RecordBatch {
    let batches = run(session, read(sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut rows = batches.into_iter().filter(|batch| batch.num_rows() > 0);
    let batch = rows.next().unwrap_or_else(|| panic!("{sql}: no row"));
    assert_eq!(batch.num_rows(), 1, "{sql}");
    batch
}

fn text(batch: &RecordBatch, name: &str) -> String {
    let column = batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"));
    array_value_to_string(column, 0).expect("displayable")
}

fn data_type<'a>(batch: &'a RecordBatch, name: &str) -> &'a DataType {
    batch
        .schema_ref()
        .field_with_name(name)
        .unwrap_or_else(|_| panic!("no column {name}"))
        .data_type()
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn integers_and_decimals_beyond_a_float_survive_exactly() {
    let Some(session) = session().await else {
        return;
    };
    let batch = row(
        &*session,
        "SELECT toUInt64('18446744073709551615') AS u64, \
                toInt64('-9223372036854775808') AS i64, \
                toInt128('-170141183460469231731687303715884105728') AS i128, \
                toString(i128) AS i128_text, \
                toUInt128('340282366920938463463374607431768211455') AS u128, \
                toString(u128) AS u128_text, \
                toUInt256('115792089237316195423570985008687907853269984665640564039457584007913129639935') AS u256, \
                toString(u256) AS u256_text, \
                toInt256('-57896044618658097711785492504343953926634992332820282019728792003956564819968') AS i256, \
                toString(i256) AS i256_text, \
                toDecimal256('-1234567890123456789012345678901234567890123456.123456789012345678901234567891', 30) AS d256, \
                toString(d256) AS d256_text, \
                toDecimal64('-9999999999.12345678', 8) AS d64, \
                toString(d64) AS d64_text, \
                toDecimal32('3.14', 2) AS d32",
    )
    .await;
    let u64s = batch
        .column_by_name("u64")
        .expect("u64")
        .as_primitive::<UInt64Type>();
    assert_eq!(
        u64s.value(0),
        u64::MAX,
        "a UInt64 above 2^53 stays an integer"
    );
    let i64s = batch
        .column_by_name("i64")
        .expect("i64")
        .as_primitive::<Int64Type>();
    assert_eq!(i64s.value(0), i64::MIN);
    assert_eq!(data_type(&batch, "i128"), &DataType::Decimal256(39, 0));
    assert_eq!(text(&batch, "i128"), text(&batch, "i128_text"));
    assert_eq!(text(&batch, "u128"), text(&batch, "u128_text"));
    assert_eq!(data_type(&batch, "u256"), &DataType::Utf8);
    assert_eq!(text(&batch, "u256"), text(&batch, "u256_text"));
    assert_eq!(text(&batch, "i256"), text(&batch, "i256_text"));
    assert_eq!(data_type(&batch, "d256"), &DataType::Decimal256(76, 30));
    assert_eq!(text(&batch, "d256"), text(&batch, "d256_text"));
    assert_eq!(data_type(&batch, "d64"), &DataType::Decimal128(18, 8));
    assert_eq!(text(&batch, "d64"), text(&batch, "d64_text"));
    assert_eq!(text(&batch, "d32"), "3.14");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn instants_keep_the_servers_zone_and_dates_their_range() {
    let Some(session) = session().await else {
        return;
    };
    let batch = row(
        &*session,
        "SELECT toDateTime64('2024-03-10 01:59:59.123456789', 9, 'America/New_York') AS dt64, \
                toUnixTimestamp64Nano(dt64) AS dt64_nanos, \
                toDateTime('2024-01-01 00:00:00', 'Asia/Tokyo') AS tokyo, \
                toUnixTimestamp(tokyo) AS tokyo_seconds, \
                toDateTime('2024-01-01 00:00:00') AS plain, \
                timezone() AS server_zone, \
                toDate('2149-06-06') AS date_max, \
                toDate32('1900-01-01') AS date32_min, \
                toDateTime64('2024-01-01 00:00:00.05', 2, 'UTC') AS centis, \
                toUnixTimestamp64Milli(centis) AS centis_millis",
    )
    .await;
    assert_eq!(
        data_type(&batch, "dt64"),
        &DataType::Timestamp(TimeUnit::Nanosecond, Some("America/New_York".into()))
    );
    let nanos = batch
        .column_by_name("dt64")
        .expect("dt64")
        .as_primitive::<TimestampNanosecondType>()
        .value(0);
    assert_eq!(nanos.to_string(), text(&batch, "dt64_nanos"));
    assert_eq!(
        data_type(&batch, "tokyo"),
        &DataType::Timestamp(TimeUnit::Second, Some("Asia/Tokyo".into()))
    );
    let seconds = batch
        .column_by_name("tokyo")
        .expect("tokyo")
        .as_primitive::<TimestampSecondType>()
        .value(0);
    assert_eq!(seconds.to_string(), text(&batch, "tokyo_seconds"));
    // No declared zone: the query's zone, as the response names it — the
    // server's, never the workstation's.
    let server_zone = text(&batch, "server_zone");
    assert_eq!(
        data_type(&batch, "plain"),
        &DataType::Timestamp(TimeUnit::Second, Some(server_zone.as_str().into()))
    );
    assert_eq!(data_type(&batch, "date_max"), &DataType::Date32);
    assert_eq!(text(&batch, "date_max"), "2149-06-06");
    assert_eq!(text(&batch, "date32_min"), "1900-01-01");
    // Hundredths scaled to milliseconds, not read as milliseconds.
    let millis = batch
        .column_by_name("centis")
        .expect("centis")
        .as_primitive::<arrow::datatypes::TimestampMillisecondType>()
        .value(0);
    assert_eq!(millis.to_string(), text(&batch, "centis_millis"));
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn text_like_types_read_as_the_server_prints_them() {
    let Some(session) = session().await else {
        return;
    };
    let batch = row(
        &*session,
        "SELECT toUUID('61f0c404-5cb3-11e7-907b-a6006ad3dba0') AS uuid, toString(uuid) AS uuid_text, \
                toIPv4('192.168.1.1') AS ip4, toString(ip4) AS ip4_text, \
                toIPv6('2001:db8::1') AS ip6, toString(ip6) AS ip6_text, \
                CAST('b', 'Enum8(\\'a\\' = 1, \\'b\\' = -2)') AS e8, \
                toFixedString('ab', 4) AS fs, \
                toLowCardinality('lc') AS lc, \
                true AS flag, \
                INTERVAL 3 DAY AS iv, \
                toBFloat16(1.5) AS bf",
    )
    .await;
    for (value, server) in [
        ("uuid", "uuid_text"),
        ("ip4", "ip4_text"),
        ("ip6", "ip6_text"),
    ] {
        assert_eq!(text(&batch, value), text(&batch, server), "{value}");
    }
    assert_eq!(text(&batch, "e8"), "b", "the label, not the number");
    assert_eq!(data_type(&batch, "fs"), &DataType::FixedSizeBinary(4));
    let fixed = batch
        .column_by_name("fs")
        .expect("fs")
        .as_fixed_size_binary()
        .value(0);
    assert_eq!(fixed, b"ab\0\0", "the server's padding is kept");
    assert_eq!(text(&batch, "lc"), "lc");
    assert_eq!(text(&batch, "flag"), "true");
    assert_eq!(text(&batch, "iv"), "3");
    let field = batch
        .schema_ref()
        .field_with_name("iv")
        .expect("iv")
        .clone();
    assert_eq!(
        field
            .metadata()
            .get(META_CLICKHOUSE_TYPE)
            .map(String::as_str),
        Some("IntervalDay"),
        "the unit travels in the metadata"
    );
    assert_eq!(text(&batch, "bf"), "1.5");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn nested_types_and_nulls_keep_their_shape() {
    let Some(session) = session().await else {
        return;
    };
    let batch = row(
        &*session,
        "SELECT CAST(NULL, 'Nullable(String)') AS n, NULL AS nothing, \
                CAST([1, NULL, 3], 'Array(Nullable(Int32))') AS arr, \
                CAST(tuple(1, 'x'), 'Tuple(a UInt8, b String)') AS tup, \
                map('k', 1, 'l', 2) AS m, \
                CAST((1.5, 2.5), 'Point') AS pt, \
                [[1, 2], [], [3]] AS nested",
    )
    .await;
    assert!(batch.column_by_name("n").expect("n").is_null(0));
    assert_eq!(
        batch
            .column_by_name("nothing")
            .expect("nothing")
            .logical_null_count(),
        1
    );
    assert_eq!(text(&batch, "arr"), "[1, , 3]");
    assert_eq!(text(&batch, "tup"), "{a: 1, b: x}");
    assert_eq!(text(&batch, "m"), "{k: 1, l: 2}");
    assert_eq!(text(&batch, "pt"), "{1: 1.5, 2: 2.5}");
    assert_eq!(text(&batch, "nested"), "[[1, 2], [], [3]]");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn json_arrives_as_the_servers_text() {
    let Some(session) = session().await else {
        return;
    };
    let batch = row(
        &*session,
        "SELECT CAST('{\"a\": 1, \"b\": {\"c\": \"x\"}}', 'JSON') AS j",
    )
    .await;
    assert_eq!(data_type(&batch, "j"), &DataType::Utf8);
    let json: serde_json::Value =
        serde_json::from_str(&text(&batch, "j")).expect("the server's JSON text");
    assert_eq!(json["b"]["c"], "x");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn an_undecodable_type_or_text_is_refused_naming_the_column() {
    let Some(session) = session().await else {
        return;
    };
    // An aggregate state has no length on the wire: refused before any row.
    let error = run(
        &*session,
        read("SELECT uniqState(number) AS state FROM numbers(10)"),
    )
    .await
    .expect_err("refused");
    assert!(error.to_string().contains("state"), "{error}");
    assert!(error.to_string().contains("AggregateFunction"), "{error}");
    assert_eq!(error.class(), ErrorClass::Permanent);
    // A String that is not UTF-8: refused naming the column, not the value.
    let error = run(&*session, read("SELECT unhex('C328') AS raw"))
        .await
        .expect_err("refused");
    assert!(error.to_string().contains("raw"), "{error}");
    assert!(error.to_string().contains("hex()"), "{error}");
    assert_eq!(code(&error), None);
    // Read as hex, as the message says, it goes through.
    let batches = run(&*session, read("SELECT hex(unhex('C328')) AS raw"))
        .await
        .expect("hex is text");
    assert_eq!(
        array_value_to_string(batches[0].column(0), 0).expect("displayable"),
        "C328"
    );
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_large_random_sample_decodes_like_the_servers_text() {
    let Some(session) = session().await else {
        return;
    };
    // Many rows of every scalar layout, compared value by value with the
    // server's own `toString`: the decoder agrees with the server, not with a
    // table written from memory.
    let sql = "SELECT n, toString(n) AS n_text, i, toString(i) AS i_text, \
                      f, toString(f) AS f_text \
               FROM (SELECT rand64() AS n, toInt128(rand64()) * -toInt128(rand64()) AS i, \
                            toUInt256(rand64()) * toUInt256(rand64()) * toUInt256(rand64()) AS f \
                     FROM numbers(20000))";
    let request = read(sql).with_limits(ExecLimits::default().with_max_rows(None));
    let batches = run(&*session, request).await.expect("sample");
    let mut rows = 0;
    for batch in &batches {
        for row in 0..batch.num_rows() {
            for (value, server) in [("n", "n_text"), ("i", "i_text"), ("f", "f_text")] {
                let decoded =
                    array_value_to_string(batch.column_by_name(value).expect("value"), row)
                        .expect("displayable");
                let printed =
                    array_value_to_string(batch.column_by_name(server).expect("server"), row)
                        .expect("displayable");
                assert_eq!(decoded, printed, "{value} at row {row}");
            }
            rows += 1;
        }
    }
    assert_eq!(rows, 20_000);
    session.close().await.expect("close");
}
