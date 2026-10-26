use std::sync::Arc;

use arrow::array::{
    BinaryArray, BooleanArray, Date32Array, Decimal128Array, Decimal256Array,
    DurationMicrosecondArray, Float64Array, Int32Array, Int64Array, ListArray, StringArray,
    StructArray, Time64MicrosecondArray, TimestampMillisecondArray, UInt8Array,
};
use arrow::datatypes::{Field, Fields, Schema, TimeUnit, i256};

use super::*;

fn render(array: Arc<dyn Array>, opts: &FormatOptions) -> CellValue<'static> {
    // `into_owned` detaches the value from the array, which dies at the end
    // of the call: it is the price of a test helper, not of the rendering path.
    format_value(array.as_ref(), 0, opts).into_owned()
}

fn as_text(array: Arc<dyn Array>) -> String {
    let opts = FormatOptions::default();
    render(array, &opts)
        .text()
        .map(str::to_owned)
        .unwrap_or_else(|| "<not rendered>".to_owned())
}

#[test]
fn grouping_off_touches_nothing() {
    // It is the default, hence the hot path: what the server sent comes out
    // as is, whatever the number of digits.
    for value in ["0", "-7", "1234567", "-1234567.89", "abc"] {
        assert_eq!(
            group(value.to_owned(), NumberGrouping::None),
            value,
            "{value}"
        );
    }
}

#[test]
fn thousands_grouping_respects_sign_and_decimals() {
    let case = [
        ("0", "0"),
        ("999", "999"),
        ("1000", "1\u{a0}000"),
        ("-1000", "-1\u{a0}000"),
        ("+1000", "+1\u{a0}000"),
        ("4823917", "4\u{a0}823\u{a0}917"),
        // The decimal part is never grouped: "1 234.567 8" is unreadable,
        // and it is not what typography asks for.
        ("1234.5678", "1\u{a0}234.5678"),
        ("-1234567.89", "-1\u{a0}234\u{a0}567.89"),
        // Nothing to group: the text comes out intact rather than mangled.
        ("", ""),
        ("-", "-"),
        ("NaN", "NaN"),
        ("inf", "inf"),
        // Arrow's formatter returns something other than a number for many
        // types. No slicing may fall in the middle of a character.
        ("—12345", "—12345"),
        ("1 234 €", "1 234 €"),
        ("日本語", "日本語"),
        ("+日本", "+日本"),
    ];
    for (entry, expected) in case {
        assert_eq!(
            group(entry.to_owned(), NumberGrouping::Thousands),
            expected,
            "{entry}"
        );
    }
}

#[test]
fn integers_and_decimals_follow_the_setting() {
    let opts = FormatOptions::default().with_number_grouping(NumberGrouping::Thousands);
    let integers = Arc::new(Int64Array::from(vec![Some(-4_823_917)])) as Arc<dyn Array>;
    assert_eq!(
        render(integers, &opts),
        CellValue::Text(Cow::Owned("-4\u{a0}823\u{a0}917".to_owned()))
    );

    // `value_as_string` places the decimal point; grouping must stop
    // before it.
    let decimals = Decimal128Array::from(vec![Some(123_456_789_i128)])
        .with_precision_and_scale(12, 2)
        .expect("valid precision and scale for a test literal");
    assert_eq!(
        render(Arc::new(decimals), &opts),
        CellValue::Text(Cow::Owned("1\u{a0}234\u{a0}567.89".to_owned()))
    );
}

#[test]
fn a_setting_written_before_grouping_stays_readable() {
    // I-11: a workspace saved before the field was added must not become
    // unreadable. That is what `#[serde(default)]` guarantees, and it is
    // the kind of guarantee that is lost at the first field added without
    // thinking about it.
    let old =
        r#"{"max_len":512,"null_text":"NULL","timestamp_format":null,"binary_display":"Hex"}"#;
    let options: FormatOptions =
        serde_json::from_str(old).expect("an earlier document stays readable");
    assert_eq!(options.number_grouping, NumberGrouping::None);
}

#[test]
fn booleans_are_not_allocated() {
    let array = BooleanArray::from(vec![Some(true), Some(false)]);
    let opts = FormatOptions::default();
    assert_eq!(
        format_value(&array, 0, &opts),
        CellValue::Text(Cow::Borrowed("true"))
    );
    assert_eq!(
        format_value(&array, 1, &opts),
        CellValue::Text(Cow::Borrowed("false"))
    );
}

#[test]
fn integers_of_every_width_render() {
    assert_eq!(as_text(Arc::new(Int32Array::from(vec![-42]))), "-42");
    assert_eq!(
        as_text(Arc::new(Int64Array::from(vec![i64::MIN]))),
        i64::MIN.to_string()
    );
    assert_eq!(as_text(Arc::new(UInt8Array::from(vec![255_u8]))), "255");
}

#[test]
fn text_is_borrowed_not_copied() {
    let array = StringArray::from(vec![Some("hello")]);
    let opts = FormatOptions::default();
    match format_value(&array, 0, &opts) {
        CellValue::Text(Cow::Borrowed(value)) => assert_eq!(value, "hello"),
        other => panic!("expected a borrow, got {other:?}"),
    }
}

#[test]
fn an_absent_value_is_null_not_text() {
    let array = StringArray::from(vec![None::<&str>]);
    let opts = FormatOptions::default();
    assert_eq!(format_value(&array, 0, &opts), CellValue::Null);
}

/// The trap: a text column that really contains `NULL`.
#[test]
fn the_null_string_is_not_an_absent_value() {
    let array = StringArray::from(vec![Some("NULL")]);
    let opts = FormatOptions::default();
    let value = format_value(&array, 0, &opts);
    assert!(!value.is_null());
    assert_eq!(value.text(), Some("NULL"));
}

#[test]
fn a_too_long_value_is_cut_on_a_character_boundary() {
    // 10 characters, 20 bytes: the naive cut at byte 5 would break UTF-8.
    let array = StringArray::from(vec![Some("éàéàéàéàéà")]);
    let opts = FormatOptions::default().with_max_len(5);
    match format_value(&array, 0, &opts) {
        CellValue::Truncated { text, full_bytes } => {
            assert_eq!(text, "éàéàé");
            assert_eq!(full_bytes, 20);
        }
        other => panic!("expected Truncated, got {other:?}"),
    }
}

#[test]
fn a_disabled_cut_leaves_the_whole_value() {
    let long = "x".repeat(10_000);
    let array = StringArray::from(vec![Some(long.as_str())]);
    let opts = FormatOptions::default().with_max_len(0);
    let value = format_value(&array, 0, &opts);
    assert!(!value.is_truncated());
    assert_eq!(value.text().map(str::len), Some(10_000));
}

#[test]
fn binary_renders_as_hexadecimal() {
    let array = BinaryArray::from(vec![Some(b"Hello".as_slice())]);
    let opts = FormatOptions::default();
    assert_eq!(format_value(&array, 0, &opts).text(), Some("48656c6c6f"));
}

#[test]
fn binary_renders_as_base64() {
    let opts = FormatOptions::default().with_binary_display(BinaryDisplay::Base64);
    // The three remainder lengths, which are the three padding cases.
    for (entry, expected) in [
        (b"Hello".as_slice(), "SGVsbG8="),
        (b"Hell".as_slice(), "SGVsbA=="),
        (b"Hel".as_slice(), "SGVs"),
    ] {
        let array = BinaryArray::from(vec![Some(entry)]);
        assert_eq!(
            format_value(&array, 0, &opts).text(),
            Some(expected),
            "input of {} bytes",
            entry.len()
        );
    }
}

#[test]
fn a_large_binary_renders_as_a_size() {
    let large = vec![0_u8; 3_500_000];
    let array = BinaryArray::from(vec![Some(large.as_slice())]);
    let opts = FormatOptions::default().with_binary_display(BinaryDisplay::Size);
    assert_eq!(format_value(&array, 0, &opts).text(), Some("<3.3 MiB>"));
}

/// The point that costs a frame if missed: 4 MB of BLOB are not converted
/// to display 32 characters of it.
#[test]
fn a_large_hexadecimal_binary_encodes_only_what_is_shown() {
    let large = vec![0xab_u8; 1_000_000];
    let array = BinaryArray::from(vec![Some(large.as_slice())]);
    let opts = FormatOptions::default().with_max_len(32);
    match format_value(&array, 0, &opts) {
        CellValue::Truncated { text, full_bytes } => {
            assert!(text.len() <= 40, "{} characters produced", text.len());
            assert_eq!(full_bytes, 1_000_000);
        }
        other => panic!("expected Truncated, got {other:?}"),
    }
}

#[test]
fn a_decimal_carries_its_decimal_point() {
    let array = Decimal128Array::from(vec![Some(12_345_i128)])
        .with_precision_and_scale(10, 2)
        .expect("valid precision and scale for 12345");
    assert_eq!(as_text(Arc::new(array)), "123.45");
}

/// A MySQL `DECIMAL(65, 30)`: 65 exact digits, which a float would round
/// after the seventeenth.
#[test]
fn a_wide_decimal_keeps_every_digit() {
    let digits = "12345678901234567890123456789012345123456789012345678901234567890";
    let magnitude = i256::from_string(digits).expect("65 digits fit in 256 bits");
    let array = Decimal256Array::from(vec![Some(magnitude), Some(magnitude.wrapping_neg())])
        .with_precision_and_scale(65, 30)
        .expect("valid precision and scale for a MySQL DECIMAL(65, 30)");
    let opts = FormatOptions::default();
    assert_eq!(
        format_value(&array, 0, &opts).text(),
        Some("12345678901234567890123456789012345.123456789012345678901234567890")
    );
    assert_eq!(
        format_value(&array, 1, &opts).text(),
        Some("-12345678901234567890123456789012345.123456789012345678901234567890")
    );

    let whole = Decimal256Array::from(vec![Some(magnitude)])
        .with_precision_and_scale(65, 0)
        .expect("valid precision and scale for a MySQL DECIMAL(65, 0)");
    assert_eq!(as_text(Arc::new(whole)), digits);

    // Grouping stops at the decimal point, as for `Decimal128`.
    let grouped = FormatOptions::default().with_number_grouping(NumberGrouping::Thousands);
    let small = Decimal256Array::from(vec![Some(i256::from_i128(-123_456_789))])
        .with_precision_and_scale(40, 2)
        .expect("valid precision and scale for a test literal");
    assert_eq!(
        format_value(&small, 0, &grouped).text(),
        Some("-1\u{a0}234\u{a0}567.89")
    );
}

/// A MySQL `TIME` renders as MySQL prints it, not as ISO 8601.
#[test]
fn a_mysql_time_renders_as_hours_minutes_seconds() {
    let bound = 3_020_399_000_000_i64;
    let array = DurationMicrosecondArray::from(vec![
        Some(-bound),
        Some(bound),
        Some(0),
        Some(43_384_500_000),
        Some(i64::MIN),
        None,
        Some(443_045_250_000),
    ]);
    let opts = FormatOptions::default();
    for (row, expected) in [
        (0, "-838:59:59.000000"),
        (1, "838:59:59.000000"),
        (2, "00:00:00.000000"),
        (3, "12:03:04.500000"),
        (4, "-2562047788:00:54.775808"),
        (6, "123:04:05.250000"),
    ] {
        assert_eq!(format_value(&array, row, &opts).text(), Some(expected));
    }
    assert!(format_value(&array, 5, &opts).is_null());
}

#[test]
fn floats_follow_the_arrow_convention() {
    // `ryu`, like the export: 1.0 and not 1.
    assert_eq!(as_text(Arc::new(Float64Array::from(vec![1.0_f64]))), "1.0");
    assert_eq!(as_text(Arc::new(Float64Array::from(vec![0.5_f64]))), "0.5");
}

#[test]
fn dates_and_times_render() {
    // 2021-01-01 = 18,628 days after the epoch.
    assert_eq!(
        as_text(Arc::new(Date32Array::from(vec![18_628]))),
        "2021-01-01"
    );
    let hour = Time64MicrosecondArray::from(vec![3_661_000_000_i64]);
    let rendered = as_text(Arc::new(hour));
    assert!(rendered.starts_with("01:01:01"), "{rendered}");
}

/// The time zone is not decorative: the same value displayed without an
/// offset gives an hour off by as much, and nobody notices.
#[test]
fn a_timestamp_with_a_time_zone_carries_its_offset() {
    let array =
        TimestampMillisecondArray::from(vec![1_609_459_200_000_i64]).with_timezone("+02:00");
    let rendered = as_text(Arc::new(array));
    assert!(rendered.starts_with("2021-01-01T02:00:00"), "{rendered}");
    assert!(rendered.contains("+02:00"), "{rendered}");
}

#[test]
fn postgres_utc_microseconds_render_without_losing_precision_or_nulls() {
    let array =
        arrow::array::TimestampMicrosecondArray::from(vec![Some(1_609_459_200_123_456), None])
            .with_timezone("UTC");
    let opts = FormatOptions::default();
    assert_eq!(
        format_value(&array, 0, &opts).text(),
        Some("2021-01-01T00:00:00.123456Z")
    );
    assert!(format_value(&array, 1, &opts).is_null());
}

#[test]
fn a_custom_timestamp_pattern_is_respected() {
    let array = TimestampMillisecondArray::from(vec![1_609_459_200_000_i64]);
    let opts = FormatOptions::default().with_timestamp_format(Some(Cow::Borrowed("%Y/%m/%d")));
    assert_eq!(format_value(&array, 0, &opts).text(), Some("2021/01/01"));
}

#[test]
fn a_list_renders() {
    let array =
        ListArray::from_iter_primitive::<arrow::datatypes::Int32Type, _, _>(vec![Some(vec![
            Some(1),
            Some(2),
            None,
        ])]);
    // The exact form belongs to Arrow; what is checked here is that the
    // nested column is rendered rather than declared unrecoverable.
    let rendered = as_text(Arc::new(array));
    assert!(rendered.starts_with('['), "{rendered}");
    assert!(
        rendered.contains('1') && rendered.contains('2'),
        "{rendered}"
    );
    assert!(rendered.ends_with(']'), "{rendered}");
}

#[test]
fn a_struct_renders() {
    let fields = Fields::from(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Utf8, false),
    ]);
    let array = StructArray::new(
        fields,
        vec![
            Arc::new(Int32Array::from(vec![7])),
            Arc::new(StringArray::from(vec!["sept"])),
        ],
        None,
    );
    let rendered = as_text(Arc::new(array));
    assert!(
        rendered.contains('7') && rendered.contains("sept"),
        "{rendered}"
    );
}

#[test]
fn an_entirely_null_column_renders_null() {
    let array = arrow::array::NullArray::new(3);
    let opts = FormatOptions::default();
    assert_eq!(format_value(&array, 1, &opts), CellValue::Null);
}

/// An out-of-bounds index is a caller bug, never a panic
/// ([I-09](../../../CLAUDE.md#i-09)).
#[test]
fn an_out_of_bounds_index_does_not_panic() {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1, 2]))])
        .expect("batch built for the test");
    let opts = FormatOptions::default();

    assert!(matches!(
        format_cell(&batch, 99, 0, &opts),
        CellValue::Unrenderable { .. }
    ));
    assert!(matches!(
        format_cell(&batch, 0, 99, &opts),
        CellValue::Unrenderable { .. }
    ));
}

#[test]
fn format_cell_goes_through_the_batch() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("at", DataType::Timestamp(TimeUnit::Millisecond, None), true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("one"), None])),
            Arc::new(TimestampMillisecondArray::from(vec![Some(0), None])),
        ],
    )
    .expect("batch built for the test");
    let opts = FormatOptions::default();

    assert_eq!(format_cell(&batch, 0, 0, &opts).text(), Some("1"));
    assert_eq!(format_cell(&batch, 0, 1, &opts).text(), Some("one"));
    assert!(format_cell(&batch, 1, 1, &opts).is_null());
    assert!(format_cell(&batch, 1, 2, &opts).is_null());
    assert_eq!(
        format_cell(&batch, 0, 2, &opts).text(),
        Some("1970-01-01T00:00:00")
    );
}

/// A schema with one timestamp column, with or without a time zone.
fn timestamp_schema(zones: &[Option<&str>]) -> Schema {
    Schema::new(
        zones
            .iter()
            .enumerate()
            .map(|(index, zone)| {
                Field::new(
                    format!("t{index}"),
                    DataType::Timestamp(TimeUnit::Microsecond, zone.map(Into::into)),
                    true,
                )
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn the_display_time_zone_is_derived_from_the_schema() {
    // No timestamp column: nothing to announce.
    assert_eq!(
        timestamp_display(&Schema::new(vec![Field::new("id", DataType::Int32, false)])),
        TimestampDisplay::Absent
    );
    // The common PostgreSQL case: `timestamptz` arrives in UTC.
    assert_eq!(
        timestamp_display(&timestamp_schema(&[Some("UTC"), Some("UTC")])),
        TimestampDisplay::Uniform("UTC".to_owned())
    );
    // Two different zones: naming one would wrongly describe the other.
    assert_eq!(
        timestamp_display(&timestamp_schema(&[Some("UTC"), Some("+02:00")])),
        TimestampDisplay::Mixed
    );
}

#[test]
fn a_timestamp_without_time_zone_does_not_announce_one() {
    // `timestamp without time zone` carries no time zone. It is the flaw
    // this test keeps closed: announcing "UTC" here would invent
    // information the server did not send, and the value itself renders
    // without `Z` or offset — the two must stay consistent.
    assert_eq!(
        timestamp_display(&timestamp_schema(&[None, None])),
        TimestampDisplay::Absent
    );
    // Mixed with a column that does carry one: only that one counts.
    assert_eq!(
        timestamp_display(&timestamp_schema(&[None, Some("UTC")])),
        TimestampDisplay::Uniform("UTC".to_owned())
    );
}

#[test]
fn the_null_replacement_text_is_configurable() {
    let opts = FormatOptions::default().with_null_text("(empty)");
    assert_eq!(CellValue::Null.display_with(&opts), "(empty)");
}
