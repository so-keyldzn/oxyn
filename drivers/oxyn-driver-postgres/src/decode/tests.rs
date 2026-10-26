use arrow::array::{Array, AsArray, StringArray};
use arrow::datatypes::{Int32Type, Schema};

use super::*;

/// Builds a one-dimensional binary PostgreSQL array.
fn encode_array(elements: &[Option<&[u8]>]) -> Vec<u8> {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&1_i32.to_be_bytes());
    encoded.extend_from_slice(&0_i32.to_be_bytes());
    encoded.extend_from_slice(&23_u32.to_be_bytes());
    encoded.extend_from_slice(
        &i32::try_from(elements.len())
            .expect("short test case")
            .to_be_bytes(),
    );
    encoded.extend_from_slice(&1_i32.to_be_bytes());
    for element in elements {
        match element {
            Some(value) => {
                encoded.extend_from_slice(
                    &i32::try_from(value.len())
                        .expect("short test case")
                        .to_be_bytes(),
                );
                encoded.extend_from_slice(value);
            }
            None => encoded.extend_from_slice(&(-1_i32).to_be_bytes()),
        }
    }
    encoded
}

fn new_column(decoding: &PgDecoding) -> ColumnBuilder {
    ColumnBuilder::for_decoding(decoding)
}

fn rendered_text(source: TextSource, bytes: &[u8]) -> Result<String, &'static str> {
    let mut builder = StringBuilder::new();
    render_text(&mut builder, source, bytes)?;
    let array: StringArray = builder.finish();
    Ok(array.value(0).to_owned())
}

#[test]
fn integers_read_back_big_endian() {
    assert_eq!(read_i16(&1234_i16.to_be_bytes()), Ok(1234));
    assert_eq!(read_i32(&(-7_i32).to_be_bytes()), Ok(-7));
    assert_eq!(read_i64(&i64::MAX.to_be_bytes()), Ok(i64::MAX));
    assert_eq!(
        read_u32(&4_000_000_000_u32.to_be_bytes()),
        Ok(4_000_000_000)
    );
}

#[test]
fn a_badly_sized_buffer_returns_an_error_not_a_panic() {
    // The bytes come from the network: a server can lie about sizes.
    assert!(read_i32(&[0, 1]).is_err());
    assert!(read_i64(&[]).is_err());
    assert!(read_i16(&[0, 0, 0]).is_err());
}

#[test]
fn a_date_is_brought_back_to_the_unix_epoch() {
    // 2000-01-01 is 0 on the PostgreSQL side and 10,957 on the Arrow side.
    assert_eq!(read_date(&0_i32.to_be_bytes()), Ok(EPOCH_SHIFT_DAYS));
    // 1970-01-01: -10,957 days before the PostgreSQL epoch.
    assert_eq!(read_date(&(-EPOCH_SHIFT_DAYS).to_be_bytes()), Ok(0));
}

#[test]
fn an_infinite_date_is_refused_rather_than_rendered_wrong() {
    // PostgreSQL accepts `infinity`; Date32 does not. Rendering it as the
    // extreme date or as NULL would be a lie about real data.
    let error = read_date(&i32::MAX.to_be_bytes()).expect_err("refusal expected");
    assert!(error.contains("infinite"), "{error}");
    assert!(read_date(&i32::MIN.to_be_bytes()).is_err());
}

#[test]
fn a_timestamp_is_brought_back_to_the_unix_epoch() {
    assert_eq!(read_timestamp(&0_i64.to_be_bytes()), Ok(EPOCH_SHIFT_MICROS));
    assert!(read_timestamp(&i64::MAX.to_be_bytes()).is_err());
}

#[test]
fn a_time_outside_the_bounds_of_a_day_is_refused() {
    assert_eq!(read_time(&0_i64.to_be_bytes()), Ok(0));
    assert_eq!(
        read_time(&86_399_999_999_i64.to_be_bytes()),
        Ok(86_399_999_999)
    );
    assert!(read_time(&(-1_i64).to_be_bytes()).is_err());
    assert!(read_time(&86_400_000_001_i64.to_be_bytes()).is_err());
}

#[test]
fn a_time_of_24_hours_is_refused_with_an_explicit_message() {
    // Legal in PostgreSQL (`SELECT '24:00'::time`) and not a `Time64`: it
    // must not reach a column whose cells then fail to format and whose
    // CSV and JSON exports abort.
    let error = read_time(&86_400_000_000_i64.to_be_bytes()).expect_err("refusal expected");
    assert!(error.contains("24:00:00"), "{error}");
    assert!(error.contains("cast it to text"), "{error}");
}

#[test]
fn an_interval_keeps_its_three_components_distinct() {
    // A month is not 30 days and a day is not 24 hours: merging them would
    // distort any calendar computation.
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&3_600_000_000_i64.to_be_bytes());
    encoded.extend_from_slice(&2_i32.to_be_bytes());
    encoded.extend_from_slice(&14_i32.to_be_bytes());
    let interval = read_interval(&encoded).expect("valid interval");
    assert_eq!(interval.months, 14);
    assert_eq!(interval.days, 2);
    assert_eq!(interval.nanoseconds, 3_600_000_000_000);
}

#[test]
fn an_interval_that_overflows_in_nanoseconds_is_refused() {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&i64::MAX.to_be_bytes());
    encoded.extend_from_slice(&0_i32.to_be_bytes());
    encoded.extend_from_slice(&0_i32.to_be_bytes());
    assert!(read_interval(&encoded).is_err());
}

#[test]
fn a_timetz_offset_changes_sign() {
    // PostgreSQL counts seconds west; ISO 8601 east. A botched inversion
    // shifts the data without anything reporting it.
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&52_200_000_000_i64.to_be_bytes()); // 14:30:00
    encoded.extend_from_slice(&(-7_200_i32).to_be_bytes()); // 7200 s east
    assert_eq!(render_timetz(&encoded).as_deref(), Ok("14:30:00+02:00"));

    let mut west = Vec::new();
    west.extend_from_slice(&52_200_000_000_i64.to_be_bytes());
    west.extend_from_slice(&18_000_i32.to_be_bytes()); // 5 h west
    assert_eq!(render_timetz(&west).as_deref(), Ok("14:30:00-05:00"));
}

#[test]
fn a_timetz_with_microseconds_keeps_them() {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&52_200_250_000_i64.to_be_bytes());
    encoded.extend_from_slice(&0_i32.to_be_bytes());
    assert_eq!(
        render_timetz(&encoded).as_deref(),
        Ok("14:30:00.250000+00:00")
    );
}

#[test]
fn a_jsonb_loses_its_version_byte_and_nothing_else() {
    let mut encoded = vec![JSONB_VERSION];
    encoded.extend_from_slice(br#"{"a": 1}"#);
    assert_eq!(
        rendered_text(TextSource::Jsonb, &encoded).as_deref(),
        Ok(r#"{"a": 1}"#)
    );
    // An unknown version is refused: the rest is no longer JSON.
    assert!(rendered_text(TextSource::Jsonb, &[9, b'{']).is_err());
}

#[test]
fn a_uuid_is_rendered_in_canonical_form() {
    let encoded: [u8; 16] = [
        0x67, 0xe5, 0x50, 0x44, 0x10, 0xb1, 0x42, 0x6f, 0x9d, 0x0c, 0x45, 0x1f, 0x8a, 0xd0, 0x5b,
        0x1a,
    ];
    assert_eq!(
        rendered_text(TextSource::Uuid, &encoded).as_deref(),
        Ok("67e55044-10b1-426f-9d0c-451f8ad05b1a")
    );
    assert!(rendered_text(TextSource::Uuid, &encoded[..8]).is_err());
}

#[test]
fn opaque_binary_is_preserved_even_when_it_is_valid_utf8() {
    let mut column = new_column(&PgDecoding::Opaque);
    let bytes = b"\x00\x00\x004";
    column
        .append(PgValueFormat::Binary, Some(bytes))
        .expect("opaque bytes");
    let large = vec![0xab; 9000];
    column
        .append(PgValueFormat::Binary, Some(&large))
        .expect("large opaque value");
    column.append(PgValueFormat::Binary, None).expect("null");
    let array = column.finish().expect("binary column");
    let values = array
        .as_binary_opt::<i32>()
        .expect("raw bytes, not guessed text");
    assert_eq!(values.value(0), bytes);
    assert_eq!(values.value(1), large);
    assert!(values.is_null(2));
}

#[test]
fn an_empty_array_and_a_missing_array_are_not_confused() {
    let mut column = new_column(&PgDecoding::List(Box::new(PgDecoding::Int32)));
    // `{}`: zero elements, but the list exists.
    let empty_array = encode_array(&[]);
    column
        .append(PgValueFormat::Binary, Some(empty_array.as_slice()))
        .expect("valid empty array");
    // `NULL`: no list at all.
    column.append(PgValueFormat::Binary, None).expect("null");

    let arrow_array = column.finish().expect("list built");
    assert_eq!(arrow_array.len(), 2);
    assert!(!arrow_array.is_null(0), "{{}} is not NULL");
    assert!(arrow_array.is_null(1));
}

#[test]
fn an_integer_array_keeps_its_elements_and_its_holes() {
    let mut column = new_column(&PgDecoding::List(Box::new(PgDecoding::Int32)));
    let one = 1_i32.to_be_bytes();
    let three = 3_i32.to_be_bytes();
    let encoded = encode_array(&[Some(&one), None, Some(&three)]);
    column
        .append(PgValueFormat::Binary, Some(encoded.as_slice()))
        .expect("valid array");

    let arrow_array = column.finish().expect("list built");
    let lists = arrow_array.as_list_opt::<i32>().expect("a column of lists");
    let first_value = lists.value(0);
    let integers = first_value
        .as_primitive_opt::<Int32Type>()
        .expect("32-bit integers");
    assert_eq!(integers.len(), 3);
    assert_eq!(integers.value(0), 1);
    assert!(integers.is_null(1), "a NULL element stays NULL");
    assert_eq!(integers.value(2), 3);
}

#[test]
fn a_multi_dimensional_array_is_refused_not_flattened() {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&2_i32.to_be_bytes());
    encoded.extend_from_slice(&0_i32.to_be_bytes());
    encoded.extend_from_slice(&23_u32.to_be_bytes());
    let error = split_array(&encoded).expect_err("refusal expected");
    assert!(error.contains("dimensional"), "{error}");
}

#[test]
fn an_array_whose_lower_bound_is_not_one_is_refused_not_shifted() {
    // `'[0:2]={10,20,30}'::int[]`: one dimension of three elements, indexed
    // from 0. The list would lose the index, so `arr[1]` would read 20.
    for lower_bound in [0_i32, 2, -5, i32::MIN] {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&1_i32.to_be_bytes());
        encoded.extend_from_slice(&0_i32.to_be_bytes());
        encoded.extend_from_slice(&23_u32.to_be_bytes());
        encoded.extend_from_slice(&3_i32.to_be_bytes());
        encoded.extend_from_slice(&lower_bound.to_be_bytes());
        for element in [10_i32, 20, 30] {
            encoded.extend_from_slice(&4_i32.to_be_bytes());
            encoded.extend_from_slice(&element.to_be_bytes());
        }
        let error = split_array(&encoded).expect_err("refusal expected");
        assert!(error.contains("lower bound"), "{error}");
        assert!(error.contains("cast it to text"), "{error}");
    }
}

#[test]
fn a_hostile_array_length_does_not_exhaust_memory() {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&1_i32.to_be_bytes());
    encoded.extend_from_slice(&0_i32.to_be_bytes());
    encoded.extend_from_slice(&23_u32.to_be_bytes());
    encoded.extend_from_slice(&i32::MAX.to_be_bytes());
    encoded.extend_from_slice(&1_i32.to_be_bytes());
    assert!(split_array(&encoded).is_err());
}

#[test]
fn a_hostile_type_name_does_not_go_out_as_is_in_an_error() {
    // A user type can carry terminal control sequences.
    assert_eq!(sanitize_type("int4"), "int4");
    assert_eq!(sanitize_type("BYTEA[]"), "BYTEA[]");
    assert_eq!(sanitize_type("\u{1b}[2J"), "<unrepresentable type>");
    assert_eq!(sanitize_type(""), "<unrepresentable type>");
    assert_eq!(sanitize_type(&"a".repeat(200)), "<unrepresentable type>");
}

#[test]
fn a_result_without_columns_keeps_its_row_count() {
    // An `INSERT` has no column but has affected rows: without
    // `with_row_count`, Arrow would build a zero-row batch.
    let schema = Arc::new(Schema::empty());
    let mut assembler = BatchAssembler::new(schema, &[]);
    assembler.rows = 3;
    let batch = assembler.finish().expect("batch without columns");
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.num_columns(), 0);
}

#[test]
fn the_assembler_starts_over_empty_after_each_batch() {
    let schema = Arc::new(Schema::empty());
    let mut assembler = BatchAssembler::new(schema, &[]);
    assembler.rows = 5;
    assembler.bytes = 4_096;
    let _ = assembler.finish().expect("batch without columns");
    assert!(assembler.is_empty());
    assert_eq!(assembler.bytes(), 0);
}
