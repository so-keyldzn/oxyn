//! The decoder against hand-built `RowBinary`: every type's wire layout, and
//! the hostile values the driver contract names.

use arrow::array::{
    Array, AsArray, Decimal256Array, ListArray, MapArray, StringArray, StructArray,
    TimestampMillisecondArray, TimestampSecondArray, UInt64Array,
};
use arrow::datatypes::{Date32Type, Decimal128Type, DurationSecondType, Float32Type, i256};

use super::*;
use crate::measure::{measure, walk};
use crate::schema::schema_for;
use crate::types::parse;

fn decoder(types: &[&str], zone: Option<&str>) -> RowDecoder {
    let columns: Vec<Column> = types
        .iter()
        .enumerate()
        .map(|(rank, text)| Column {
            name: format!("c{rank}"),
            raw_type: (*text).to_owned(),
            ty: parse(text).expect("a valid type"),
        })
        .collect();
    let schema = schema_for(&columns, zone);
    RowDecoder::new(columns, schema, zone)
}

/// Decodes each row after proving it whole, the cursor's order — and checks
/// that what the measure bounded is no less than what the row then took.
fn decode(types: &[&str], rows: &[&[u8]]) -> Result<RecordBatch, DecodeError> {
    let mut decoder = decoder(types, Some("UTC"));
    for row in rows {
        let (length, bound) = walk(decoder.columns(), row)?.ok_or(DecodeError::Truncated)?;
        assert_eq!(length, row.len(), "the row is measured to its end");
        let before = decoder.bytes();
        decoder.append(row)?;
        let taken = decoder.bytes() - before;
        assert!(
            taken <= bound,
            "the row took {taken} bytes, measured at most {bound}"
        );
    }
    decoder.finish()
}

fn text(batch: &RecordBatch, column: usize) -> Vec<Option<String>> {
    batch
        .column(column)
        .as_string::<i32>()
        .iter()
        .map(|value| value.map(str::to_owned))
        .collect()
}

#[test]
fn integers_beyond_two_to_the_fifty_three_survive() {
    let batch = decode(
        &["UInt64"],
        &[&u64::MAX.to_le_bytes(), &(1_u64 << 53 | 1).to_le_bytes()],
    )
    .expect("decoded");
    let values = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .expect("UInt64");
    assert_eq!(values.value(0), u64::MAX);
    assert_eq!(values.value(1), (1 << 53) + 1);
}

#[test]
fn wide_integers_are_exact_decimals_or_exact_text() {
    let batch = decode(
        &["Int128", "UInt128", "Int256", "UInt256"],
        &[&[
            i128::MIN.to_le_bytes().as_slice(),
            u128::MAX.to_le_bytes().as_slice(),
            i256::MIN.to_le_bytes().as_slice(),
            [0xff; 32].as_slice(),
        ]
        .concat()],
    )
    .expect("decoded");
    let int128 = batch
        .column(0)
        .as_primitive::<arrow::datatypes::Decimal256Type>();
    assert_eq!(int128.value_as_string(0), i128::MIN.to_string());
    let uint128 = batch
        .column(1)
        .as_any()
        .downcast_ref::<Decimal256Array>()
        .expect("Decimal256");
    assert_eq!(uint128.value_as_string(0), u128::MAX.to_string());
    assert_eq!(
        text(&batch, 2),
        [Some(
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968"
                .to_owned()
        )]
    );
    assert_eq!(
        text(&batch, 3),
        [Some(
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                .to_owned()
        )]
    );
}

#[test]
fn a_decimal_keeps_its_digits_and_never_becomes_a_float() {
    // 1234567.89 as Decimal(9, 2), then as Decimal(38, 10).
    let batch = decode(
        &["Decimal(9, 2)", "Decimal(38, 10)"],
        &[&[
            123_456_789_i32.to_le_bytes().as_slice(),
            12_345_678_901_234_567_890_i128.to_le_bytes().as_slice(),
        ]
        .concat()],
    )
    .expect("decoded");
    assert_eq!(
        batch
            .column(0)
            .as_primitive::<Decimal128Type>()
            .value_as_string(0),
        "1234567.89"
    );
    assert_eq!(
        batch
            .column(1)
            .as_primitive::<Decimal128Type>()
            .value_as_string(0),
        "1234567890.1234567890"
    );
}

#[test]
fn text_types_render_as_the_server_prints_them() {
    let uuid = {
        // 61f0c404-5cb3-11e7-907b-a6006ad3dba0, as ClickHouse sends it: the
        // two 64-bit halves, each little-endian.
        let high: u64 = 0x61f0_c404_5cb3_11e7;
        let low: u64 = 0x907b_a600_6ad3_dba0;
        [high.to_le_bytes(), low.to_le_bytes()].concat()
    };
    let ipv6: [u8; 16] = std::net::Ipv6Addr::LOCALHOST.octets();
    let row = [
        uuid.as_slice(),
        0x0102_0304_u32.to_le_bytes().as_slice(),
        ipv6.as_slice(),
        &[(-2_i8).to_le_bytes()[0]],
        &[2, b'h', b'i'],
    ]
    .concat();
    let batch = decode(
        &["UUID", "IPv4", "IPv6", "Enum8('a' = 1, 'b' = -2)", "String"],
        &[&row],
    )
    .expect("decoded");
    assert_eq!(
        text(&batch, 0),
        [Some("61f0c404-5cb3-11e7-907b-a6006ad3dba0".into())]
    );
    assert_eq!(text(&batch, 1), [Some("1.2.3.4".into())]);
    assert_eq!(text(&batch, 2), [Some("::1".into())]);
    assert_eq!(text(&batch, 3), [Some("b".into())]);
    assert_eq!(text(&batch, 4), [Some("hi".into())]);
}

#[test]
fn instants_keep_their_zone_and_their_precision() {
    let row = [
        1_704_164_645_u32.to_le_bytes().as_slice(),
        1_704_164_645_123_i64.to_le_bytes().as_slice(),
        // `DateTime64(2)`: hundredths, scaled to milliseconds.
        170_416_464_512_i64.to_le_bytes().as_slice(),
    ]
    .concat();
    let batch = decode(
        &[
            "DateTime('Asia/Tokyo')",
            "DateTime64(3)",
            "DateTime64(2, 'Europe/Paris')",
        ],
        &[&row],
    )
    .expect("decoded");
    let seconds = batch
        .column(0)
        .as_any()
        .downcast_ref::<TimestampSecondArray>()
        .expect("seconds");
    assert_eq!(seconds.value(0), 1_704_164_645);
    assert_eq!(seconds.timezone(), Some("Asia/Tokyo"));
    let millis = batch
        .column(1)
        .as_any()
        .downcast_ref::<TimestampMillisecondArray>()
        .expect("milliseconds");
    assert_eq!(millis.value(0), 1_704_164_645_123);
    assert_eq!(
        millis.timezone(),
        Some("UTC"),
        "the query's zone, never the workstation's"
    );
    let hundredths = batch
        .column(2)
        .as_any()
        .downcast_ref::<TimestampMillisecondArray>()
        .expect("milliseconds");
    assert_eq!(hundredths.value(0), 1_704_164_645_120);
}

#[test]
fn dates_times_and_floats_take_their_arrow_layout() {
    let row = [
        19_724_u16.to_le_bytes().as_slice(),
        (-25_567_i32).to_le_bytes().as_slice(),
        (-3_600_i32).to_le_bytes().as_slice(),
        // 1.5 as a BFloat16: the upper half of the `f32`.
        0x3fc0_u16.to_le_bytes().as_slice(),
    ]
    .concat();
    let batch = decode(&["Date", "Date32", "Time", "BFloat16"], &[&row]).expect("decoded");
    assert_eq!(
        batch.column(0).as_primitive::<Date32Type>().value(0),
        19_724
    );
    assert_eq!(
        batch.column(1).as_primitive::<Date32Type>().value(0),
        -25_567
    );
    assert_eq!(
        batch
            .column(2)
            .as_primitive::<DurationSecondType>()
            .value(0),
        -3_600
    );
    assert!((batch.column(3).as_primitive::<Float32Type>().value(0) - 1.5).abs() < f32::EPSILON);
}

#[test]
fn nested_types_keep_their_nulls_and_their_shape() {
    // [1, NULL], (7, 'x'), {'k': 1}
    let row = [&[2, 0, 1, 1][..], &[7, 1, b'x'], &[1, 1, b'k', 1, 0, 0, 0]].concat();
    let batch = decode(
        &[
            "Array(Nullable(UInt8))",
            "Tuple(UInt8, s String)",
            "Map(String, UInt32)",
        ],
        // [], (9, ''), {}
        &[&row, &[0, 9, 0, 0]],
    )
    .expect("decoded");
    let list = batch
        .column(0)
        .as_any()
        .downcast_ref::<ListArray>()
        .expect("list");
    let first = list.value(0);
    assert_eq!(first.len(), 2);
    assert!(first.is_null(1));
    assert_eq!(list.value(1).len(), 0, "the second row's array is empty");
    let tuple = batch
        .column(1)
        .as_any()
        .downcast_ref::<StructArray>()
        .expect("struct");
    assert_eq!(tuple.len(), 2);
    let map = batch
        .column(2)
        .as_any()
        .downcast_ref::<MapArray>()
        .expect("map");
    assert_eq!(map.value(0).len(), 1);
    assert_eq!(map.value(1).len(), 0);
}

#[test]
fn a_null_reaches_only_a_nullable_column() {
    let batch = decode(
        &["Nullable(String)", "Nullable(Nothing)"],
        &[&[1, 1], &[0, 1, b'a', 1]],
    )
    .expect("decoded");
    assert_eq!(text(&batch, 0), [None, Some("a".into())]);
    // A `NullArray` holds no validity buffer: its nulls are logical.
    assert_eq!(batch.column(1).logical_null_count(), 2);
    assert!(batch.schema().field(0).is_nullable());
    let strict = decoder(&["String"], None);
    assert!(!strict.schema.field(0).is_nullable());
}

#[test]
fn a_value_that_is_not_utf8_fails_naming_the_column_not_the_value() {
    let error = decode(&["String"], &[&[2, 0xc3, 0x28]]).expect_err("invalid UTF-8");
    let DecodeError::Value { column, reason } = &error else {
        panic!("a value error: {error:?}");
    };
    assert_eq!(column, "c0");
    assert!(reason.contains("hex()"), "{reason}");
}

#[test]
fn values_that_contradict_their_type_are_refused() {
    assert!(matches!(
        decode(&["Bool"], &[&[2]]),
        Err(DecodeError::Value { reason, .. }) if reason == BAD_BOOL
    ));
    assert!(matches!(
        decode(&["Enum8('a' = 1)"], &[&[5]]),
        Err(DecodeError::Value { reason, .. }) if reason == UNDECLARED_LABEL
    ));
    // `DateTime64(1)` ticks scaled by 100 past `i64`.
    assert!(matches!(
        decode(&["DateTime64(1)"], &[&i64::MAX.to_le_bytes()]),
        Err(DecodeError::Value { reason, .. }) if reason == OUT_OF_RANGE
    ));
}

#[test]
fn a_batch_counts_its_decoded_bytes_not_its_rows() {
    let mut decoder = decoder(&["String"], None);
    let mut row = vec![0x80, 0x08];
    row.extend(std::iter::repeat_n(b'x', 1024));
    decoder.append(&row).expect("decoded");
    assert!(decoder.bytes() >= 1024, "{}", decoder.bytes());
    assert_eq!(decoder.rows(), 1);
    let batch = decoder.finish().expect("finished");
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(decoder.bytes(), 0, "a fresh batch starts empty");
    assert_eq!(
        batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("text")
            .value(0)
            .len(),
        1024
    );
}

#[test]
fn a_row_cut_short_is_never_decoded_into_a_batch() {
    let decoder = decoder(&["UInt32", "String"], None);
    let row = [1, 0, 0, 0, 3, b'a', b'b'];
    assert_eq!(measure(decoder.columns(), &row), Ok(None));
}
