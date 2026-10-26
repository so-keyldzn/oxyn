use arrow::array::{
    Array, BinaryArray, Date32Array, Decimal128Array, Decimal256Array, DurationMicrosecondArray,
    Int8Array, StringArray, TimestampMicrosecondArray, UInt64Array,
};
use arrow::datatypes::{Field, Schema};

use super::*;
use crate::types::arrow_type;

fn assembler(decodings: &[MyDecoding]) -> BatchAssembler {
    let fields: Vec<Field> = decodings
        .iter()
        .enumerate()
        .map(|(i, d)| Field::new(format!("c{i}"), arrow_type(*d), true))
        .collect();
    BatchAssembler::new(Arc::new(Schema::new(fields)), decodings)
}

fn one(decoding: MyDecoding, value: Value) -> Result<RecordBatch, DecodeError> {
    let mut assembler = assembler(&[decoding]);
    assembler.push(vec![Some(value)])?;
    Ok(assembler.finish().expect("the column matches its schema"))
}

#[test]
fn a_zero_or_partial_date_is_refused_naming_the_column_and_the_remedy() {
    // ADR-0050 §10: neither NULL nor an invented date.
    for decoding in [
        MyDecoding::Date,
        MyDecoding::DateTime,
        MyDecoding::Timestamp,
    ] {
        for value in [
            Value::Date(0, 0, 0, 0, 0, 0, 0),
            Value::Date(2024, 0, 15, 0, 0, 0, 0),
            Value::Bytes(b"0000-00-00".to_vec()),
            Value::Bytes(b"2024-00-15 10:00:00".to_vec()),
        ] {
            let error = one(decoding, value).expect_err("refusal expected");
            assert_eq!(error.column(), "c0");
            assert!(error.to_string().contains("CAST(… AS CHAR)"), "{error}");
            assert!(
                !error.to_string().contains("2024"),
                "no data in the message"
            );
        }
    }
}

#[test]
fn dates_and_timestamps_count_from_the_epoch_in_both_protocols() {
    let binary = one(MyDecoding::Date, Value::Date(1970, 1, 2, 0, 0, 0, 0)).expect("date");
    let text = one(MyDecoding::Date, Value::Bytes(b"1970-01-02".to_vec())).expect("date");
    for batch in [binary, text] {
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<Date32Array>()
            .expect("Date32");
        assert_eq!(column.value(0), 1);
    }
    let stamp = one(
        MyDecoding::Timestamp,
        Value::Bytes(b"1970-01-01 00:00:01.5".to_vec()),
    )
    .expect("timestamp");
    let column = stamp
        .column(0)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("Timestamp");
    assert_eq!(column.value(0), 1_500_000);
}

#[test]
fn a_time_keeps_its_full_range_and_its_sign() {
    let limit = 838 * 3_600 + 59 * 60 + 59;
    for (value, expected) in [
        (Value::Time(true, 34, 22, 59, 59, 0), -limit * 1_000_000),
        (
            Value::Bytes(b"-838:59:59.000000".to_vec()),
            -limit * 1_000_000,
        ),
        (Value::Bytes(b"838:59:59".to_vec()), limit * 1_000_000),
        (Value::Time(false, 0, 0, 0, 1, 250_000), 1_250_000),
    ] {
        let batch = one(MyDecoding::Time, value).expect("time");
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<DurationMicrosecondArray>()
            .expect("Duration");
        assert_eq!(column.value(0), expected);
    }
}

#[test]
fn a_malformed_time_is_refused_rather_than_normalized() {
    // `00:60:00` must not become `01:00:00`, nor `839:00:00` a duration
    // MySQL cannot store: a broken server is reported, not rewritten.
    for value in [
        Value::Time(false, 0, 0, 60, 0, 0),
        Value::Time(false, 0, 0, 0, 60, 0),
        Value::Time(false, 0, 0, 0, 0, 1_000_000),
        Value::Time(false, 34, 23, 0, 0, 0),
        Value::Time(true, 34, 22, 59, 59, 1),
        Value::Bytes(b"00:60:00".to_vec()),
        Value::Bytes(b"00:00:60".to_vec()),
        Value::Bytes(b"839:00:00".to_vec()),
        Value::Bytes(b"-838:59:59.000001".to_vec()),
    ] {
        assert!(
            one(MyDecoding::Time, value.clone()).is_err(),
            "{value:?} must be refused"
        );
    }
}

#[test]
fn a_decimal_is_exact_up_to_65_digits() {
    let text = "-12345678901234567890123456789012345.123456789012345678901234567890";
    let batch = one(
        MyDecoding::Decimal256 {
            precision: 65,
            scale: 30,
        },
        Value::Bytes(text.as_bytes().to_vec()),
    )
    .expect("DECIMAL(65,30)");
    let column = batch
        .column(0)
        .as_any()
        .downcast_ref::<Decimal256Array>()
        .expect("Decimal256");
    assert_eq!(column.value_as_string(0), text);

    let small = one(
        MyDecoding::Decimal128 {
            precision: 10,
            scale: 2,
        },
        Value::Bytes(b"12.5".to_vec()),
    )
    .expect("DECIMAL(10,2)");
    let column = small
        .column(0)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("Decimal128");
    assert_eq!(column.value(0), 1250, "padded to the scale");
}

#[test]
fn a_decimal_with_more_digits_than_its_scale_is_refused_not_rounded() {
    let error = one(
        MyDecoding::Decimal128 {
            precision: 10,
            scale: 2,
        },
        Value::Bytes(b"1.005".to_vec()),
    )
    .expect_err("refusal expected");
    assert!(error.to_string().contains("precision"), "{error}");
}

#[test]
fn bigint_unsigned_max_survives_and_an_out_of_range_integer_is_refused() {
    let batch = one(MyDecoding::UInt64, Value::UInt(u64::MAX)).expect("u64");
    let column = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .expect("UInt64");
    assert_eq!(column.value(0), u64::MAX);
    let text = one(
        MyDecoding::UInt64,
        Value::Bytes(b"18446744073709551615".to_vec()),
    )
    .expect("u64 as text");
    assert_eq!(text.num_rows(), 1);

    // A server that lies about a TINYINT's range.
    assert!(one(MyDecoding::Int8, Value::Int(300)).is_err());
    let fine = one(MyDecoding::Int8, Value::Int(100)).expect("TINYINT(1) holding 100");
    let column = fine
        .column(0)
        .as_any()
        .downcast_ref::<Int8Array>()
        .expect("Int8");
    assert_eq!(column.value(0), 100);
}

#[test]
fn bits_are_read_big_endian_and_more_than_64_are_refused() {
    let batch = one(MyDecoding::Bit, Value::Bytes(vec![0x00, 0x05])).expect("BIT(12)");
    let column = batch
        .column(0)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .expect("UInt64");
    assert_eq!(column.value(0), 5);
    assert!(one(MyDecoding::Bit, Value::Bytes(vec![1; 9])).is_err());
}

#[test]
fn invalid_utf8_is_refused_and_bytes_stay_bytes() {
    assert!(one(MyDecoding::Text, Value::Bytes(vec![0xff, 0xfe])).is_err());
    let batch = one(MyDecoding::Binary, Value::Bytes(vec![0xff, 0xfe])).expect("bytes");
    let column = batch
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .expect("Binary");
    assert_eq!(column.value(0), &[0xff, 0xfe]);
    let text = one(MyDecoding::Text, Value::Bytes("é".as_bytes().to_vec())).expect("text");
    let column = text
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("Utf8");
    assert_eq!(column.value(0), "é");
}

#[test]
fn a_null_is_null_whatever_the_schema_says_and_a_short_row_is_refused() {
    let batch = one(MyDecoding::Int64, Value::NULL).expect("NULL");
    assert!(batch.column(0).is_null(0));
    let mut assembler = assembler(&[MyDecoding::Int64, MyDecoding::Text]);
    assert!(assembler.push(vec![Some(Value::Int(1))]).is_err());
    assert!(assembler.push(vec![None, None]).is_err());
}

#[test]
fn garbage_never_panics() {
    // I-09: every decoding against every hostile shape.
    let decodings = [
        MyDecoding::Int8,
        MyDecoding::UInt16,
        MyDecoding::Int64,
        MyDecoding::Bit,
        MyDecoding::Float32,
        MyDecoding::Float64,
        MyDecoding::Decimal128 {
            precision: 10,
            scale: 2,
        },
        MyDecoding::Decimal256 {
            precision: 65,
            scale: 30,
        },
        MyDecoding::Date,
        MyDecoding::DateTime,
        MyDecoding::Timestamp,
        MyDecoding::Time,
        MyDecoding::Text,
        MyDecoding::Binary,
    ];
    let values = [
        Value::Bytes(Vec::new()),
        Value::Bytes(b"-".to_vec()),
        Value::Bytes(b".".to_vec()),
        Value::Bytes(b"99999999999999999999999999999999999999999999".to_vec()),
        Value::Bytes(b"9999-99-99 99:99:99.9999999".to_vec()),
        Value::Bytes(b"::".to_vec()),
        Value::Bytes(vec![0xff; 40]),
        Value::Int(i64::MIN),
        Value::UInt(u64::MAX),
        Value::Float(f32::NAN),
        Value::Double(f64::INFINITY),
        Value::Date(u16::MAX, 255, 255, 255, 255, 255, u32::MAX),
        Value::Time(true, u32::MAX, 255, 255, 255, u32::MAX),
    ];
    for decoding in decodings {
        for value in &values {
            let _ = one(decoding, value.clone());
        }
    }
}
