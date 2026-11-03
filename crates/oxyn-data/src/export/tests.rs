use std::sync::Arc;

use arrow::array::{Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::ipc::reader::FileReader;
use oxyn_core::ExecStats;

use super::*;
use crate::buffer::BufferLimits;

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]))
}

fn batch_of(start: i32, rows: usize) -> RecordBatch {
    let ids: Vec<i32> = (0..rows)
        .map(|i| start.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
        .collect();
    let names: Vec<Option<String>> = ids
        .iter()
        .map(|i| {
            if i % 2 == 0 {
                Some(format!("n{i}"))
            } else {
                None
            }
        })
        .collect();
    RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(Int32Array::from(ids)),
            Arc::new(StringArray::from(names)),
        ],
    )
    .expect("the columns match the schema built just above")
}

/// Closed buffer, with a budget so small that everything spilled to disk:
/// it is the case that matters, since the export must read back from the
/// temporary file.
fn spilled_buffer() -> ResultBuffer {
    let buffer = ResultBuffer::with_limits(schema(), BufferLimits::default().with_memory_budget(1));
    buffer.push(batch_of(0, 3)).expect("batch accepted");
    buffer.push(batch_of(100, 2)).expect("batch accepted");
    buffer.mark_complete(ExecStats::default());
    assert_eq!(buffer.spilled_batches(), 2);
    buffer
}

fn exporter(format: ExportFormat, opts: &ExportOptions) -> (String, ExportSummary) {
    let buffer = spilled_buffer();
    let mut output: Vec<u8> = Vec::new();
    let summary = export(&buffer, format, &mut output, opts, &CancelToken::new())
        .expect("export without error");
    (
        String::from_utf8(output).expect("the tested formats are UTF-8"),
        summary,
    )
}

#[test]
fn csv_carries_its_header_and_all_its_rows() {
    let (text, summary) = exporter(ExportFormat::Csv, &ExportOptions::default());
    let row_count: Vec<&str> = text.lines().collect();

    assert_eq!(row_count.first(), Some(&"id,name"));
    assert_eq!(row_count.len(), 6, "a header and five rows: {text}");
    assert_eq!(summary.rows, 5);
    assert_eq!(summary.batches, 2);
    assert!(summary.bytes > 0);
}

/// Spilling to disk must change nothing in the exported content.
#[test]
fn the_export_reads_back_spilled_batches() {
    let (text, _) = exporter(ExportFormat::Csv, &ExportOptions::default());
    for expected in ["0,n0", "1,", "2,n2", "100,n100", "101,"] {
        assert!(text.contains(expected), "{expected} missing from:\n{text}");
    }
}

#[test]
fn tsv_uses_the_tab() {
    let (text, _) = exporter(ExportFormat::Tsv, &ExportOptions::default());
    assert!(text.starts_with("id\tname"), "{text}");
}

#[test]
fn the_header_is_optional() {
    let (text, _) = exporter(
        ExportFormat::Csv,
        &ExportOptions::default().with_header(false),
    );
    assert!(!text.starts_with("id,name"), "{text}");
    assert_eq!(text.lines().count(), 5);
}

#[test]
fn the_absent_value_text_is_configurable() {
    let (text, _) = exporter(
        ExportFormat::Csv,
        &ExportOptions::default().with_null_text("\\N"),
    );
    assert!(text.contains("1,\\N"), "{text}");
}

#[test]
fn json_lines_produces_one_object_per_row() {
    let (text, summary) = exporter(ExportFormat::JsonLines, &ExportOptions::default());
    let row_count: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(row_count.len(), 5, "{text}");
    assert!(
        row_count.first().is_some_and(|l| l.starts_with('{')),
        "{text}"
    );
    assert_eq!(summary.rows, 5);
}

#[test]
fn json_produces_a_closed_array() {
    let (text, _) = exporter(ExportFormat::Json, &ExportOptions::default());
    assert!(text.starts_with('['), "{text}");
    assert!(text.trim_end().ends_with(']'), "{text}");
}

/// The Arrow IPC round trip is the only export that converts nothing: what
/// comes out must be exactly what went in.
#[test]
fn arrow_ipc_makes_an_exact_round_trip() {
    let buffer = spilled_buffer();
    let mut output: Vec<u8> = Vec::new();
    let summary = export(
        &buffer,
        ExportFormat::ArrowIpc,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    )
    .expect("export without error");
    assert_eq!(summary.rows, 5);

    let reader = FileReader::try_new(std::io::Cursor::new(output), None)
        .expect("the written file must be a valid Arrow file");
    assert_eq!(reader.schema().fields(), schema().fields());

    let reread: Vec<RecordBatch> = reader
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("reading batches back");
    assert_eq!(reread.len(), 2);
    assert_eq!(reread.first(), Some(&batch_of(0, 3)));
    assert_eq!(reread.get(1), Some(&batch_of(100, 2)));
}

#[test]
fn parquet_is_refused_explicitly() {
    let buffer = spilled_buffer();
    let mut output: Vec<u8> = Vec::new();
    match export(
        &buffer,
        ExportFormat::Parquet,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    ) {
        Err(DataError::UnsupportedFormat { format }) => assert_eq!(format, "parquet"),
        other => panic!("expected UnsupportedFormat, got {other:?}"),
    }
    assert!(output.is_empty(), "nothing must be written");
}

/// The failure mode to avoid: a partial file that looks complete.
#[test]
fn a_result_in_progress_is_not_exported_by_accident() {
    let buffer = ResultBuffer::new(schema(), 1 << 20);
    buffer.push(batch_of(0, 3)).expect("batch accepted");
    let mut output: Vec<u8> = Vec::new();

    match export(
        &buffer,
        ExportFormat::Csv,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    ) {
        Err(DataError::IncompleteResult) => {}
        other => panic!("expected IncompleteResult, got {other:?}"),
    }

    // Declared explicitly, it is allowed.
    let summary = export(
        &buffer,
        ExportFormat::Csv,
        &mut output,
        &ExportOptions::default().allowing_incomplete(),
        &CancelToken::new(),
    )
    .expect("explicit export of a partial result");
    assert_eq!(summary.rows, 3);
}

/// Closed but truncated: the case nothing on screen tells from a whole
/// result. No option allows it, not even `allowing_incomplete`.
#[test]
fn a_truncated_result_never_exports() {
    let buffer = spilled_buffer();
    buffer.mark_truncated();

    for opts in [
        ExportOptions::default(),
        ExportOptions::default().allowing_incomplete(),
    ] {
        let mut output: Vec<u8> = Vec::new();
        match export(
            &buffer,
            ExportFormat::Csv,
            &mut output,
            &opts,
            &CancelToken::new(),
        ) {
            Err(DataError::TruncatedResult) => {}
            other => panic!("expected TruncatedResult, got {other:?}"),
        }
        assert!(output.is_empty(), "nothing may be written");
    }
}

#[test]
fn a_cancellation_interrupts_the_export() {
    let buffer = spilled_buffer();
    let ct = CancelToken::new();
    ct.cancel();
    let mut output: Vec<u8> = Vec::new();

    match export(
        &buffer,
        ExportFormat::Csv,
        &mut output,
        &ExportOptions::default(),
        &ct,
    ) {
        Err(DataError::Cancelled) => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

#[test]
fn every_format_declared_writable_is_really_written() {
    // `is_supported` governs what the interface offers. A divergence
    // between this list and `export`'s `match` breaks nothing here: it
    // breaks three clicks later, after the user named their file, and
    // leaves an empty file behind.
    let buffer = spilled_buffer();
    for format in [
        ExportFormat::Csv,
        ExportFormat::Tsv,
        ExportFormat::Json,
        ExportFormat::JsonLines,
        ExportFormat::Parquet,
        ExportFormat::ArrowIpc,
        ExportFormat::Sql,
        ExportFormat::Markdown,
    ] {
        let mut output: Vec<u8> = Vec::new();
        let written = export(
            &buffer,
            format,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .is_ok();
        assert_eq!(
            written,
            is_supported(format),
            "{format} : `is_supported` dit {}, `export` dit {written}",
            is_supported(format)
        );
    }
}

/// Every Arrow type the MySQL driver emits (ADR-0050), one column each.
fn mysql_buffer() -> ResultBuffer {
    use arrow::array::{
        BinaryArray, Date32Array, Decimal128Array, Decimal256Array, DurationMicrosecondArray,
        Float32Array, Float64Array, Int8Array, Int16Array, Int64Array, TimestampMicrosecondArray,
        UInt8Array, UInt16Array, UInt32Array, UInt64Array,
    };
    use arrow::datatypes::{TimeUnit, i256};

    let wide =
        i256::from_string("12345678901234567890123456789012345123456789012345678901234567890")
            .expect("65 digits fit in 256 bits");
    let geometry =
        std::collections::HashMap::from([("oxyn:mysql_type".to_owned(), "GEOMETRY".to_owned())]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("i8", DataType::Int8, true),
        Field::new("i16", DataType::Int16, true),
        Field::new("i32", DataType::Int32, true),
        Field::new("i64", DataType::Int64, true),
        Field::new("u8", DataType::UInt8, true),
        Field::new("year", DataType::UInt16, true),
        Field::new("u32", DataType::UInt32, true),
        Field::new("bit", DataType::UInt64, true),
        Field::new("f32", DataType::Float32, true),
        Field::new("f64", DataType::Float64, true),
        Field::new("d128", DataType::Decimal128(10, 2), true),
        Field::new("d256", DataType::Decimal256(65, 30), true),
        Field::new("d256_whole", DataType::Decimal256(65, 0), true),
        Field::new("day", DataType::Date32, true),
        Field::new(
            "datetime",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            true,
        ),
        Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            true,
        ),
        Field::new("time", DataType::Duration(TimeUnit::Microsecond), true),
        Field::new("text", DataType::Utf8, true),
        Field::new("geom", DataType::Binary, true).with_metadata(geometry),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int8Array::from(vec![Some(-128), None])),
        Arc::new(Int16Array::from(vec![Some(-32_768), None])),
        Arc::new(Int32Array::from(vec![Some(i32::MIN), None])),
        Arc::new(Int64Array::from(vec![Some(i64::MIN), None])),
        Arc::new(UInt8Array::from(vec![Some(255), None])),
        Arc::new(UInt16Array::from(vec![Some(2155), None])),
        Arc::new(UInt32Array::from(vec![Some(u32::MAX), None])),
        Arc::new(UInt64Array::from(vec![Some(u64::MAX), None])),
        Arc::new(Float32Array::from(vec![Some(1.5), None])),
        Arc::new(Float64Array::from(vec![Some(-0.25), None])),
        Arc::new(
            Decimal128Array::from(vec![Some(12_345), None])
                .with_precision_and_scale(10, 2)
                .expect("valid DECIMAL(10, 2)"),
        ),
        Arc::new(
            Decimal256Array::from(vec![Some(wide.wrapping_neg()), None])
                .with_precision_and_scale(65, 30)
                .expect("valid DECIMAL(65, 30)"),
        ),
        Arc::new(
            Decimal256Array::from(vec![Some(wide), None])
                .with_precision_and_scale(65, 0)
                .expect("valid DECIMAL(65, 0)"),
        ),
        Arc::new(Date32Array::from(vec![Some(18_628), None])),
        Arc::new(TimestampMicrosecondArray::from(vec![Some(0), None])),
        Arc::new(TimestampMicrosecondArray::from(vec![Some(0), None]).with_timezone("UTC")),
        Arc::new(DurationMicrosecondArray::from(vec![
            Some(-3_020_399_000_000),
            None,
        ])),
        Arc::new(StringArray::from(vec![Some("x"), None])),
        Arc::new(BinaryArray::from(vec![Some(b"\x01".as_slice()), None])),
    ];
    let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
    buffer
        .push(RecordBatch::try_new(schema, columns).expect("columns match the schema"))
        .expect("batch accepted");
    buffer.mark_complete(ExecStats::default());
    buffer
}

fn export_text(buffer: &ResultBuffer, format: ExportFormat) -> String {
    let mut output: Vec<u8> = Vec::new();
    export(
        buffer,
        format,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    )
    .unwrap_or_else(|error| panic!("{format} exports every MySQL type: {error}"));
    String::from_utf8(output).expect("text formats are UTF-8")
}

#[test]
fn every_mysql_type_exports_in_every_text_format() {
    let buffer = mysql_buffer();
    let negative = "-12345678901234567890123456789012345.123456789012345678901234567890";
    let whole = "12345678901234567890123456789012345123456789012345678901234567890";
    for format in [ExportFormat::Csv, ExportFormat::Tsv] {
        let text = export_text(&buffer, format);
        // The grid's text, not ISO 8601's `-PT3023999S`.
        assert!(text.contains("-838:59:59.000000"), "{format}: {text}");
        assert!(!text.contains("PT"), "{format}: {text}");
        assert!(text.contains(negative), "{format}: {text}");
        assert!(text.contains(whole), "{format}: {text}");
    }
    for format in [ExportFormat::Json, ExportFormat::JsonLines] {
        let text = export_text(&buffer, format);
        // A string in JSON, like every text the grid shows.
        assert!(
            text.contains(r#""time":"-838:59:59.000000""#),
            "{format}: {text}"
        );
        // Decimals stay bare numbers, digit for digit.
        assert!(
            text.contains(&format!(r#""d256":{negative}"#)),
            "{format}: {text}"
        );
        assert!(
            text.contains(&format!(r#""d256_whole":{whole}"#)),
            "{format}: {text}"
        );
    }
}

/// Arrow IPC converts nothing: the duration stays a duration.
#[test]
fn arrow_ipc_keeps_mysql_types_native() {
    let buffer = mysql_buffer();
    let mut output: Vec<u8> = Vec::new();
    export(
        &buffer,
        ExportFormat::ArrowIpc,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    )
    .expect("export without error");
    let reader = FileReader::try_new(std::io::Cursor::new(output), None)
        .expect("the written file must be a valid Arrow file");
    assert_eq!(&reader.schema(), buffer.schema());
}

/// A NULL is written `null`, never dropped: a column NULL in every row
/// stays in every record, and `""` stays distinct from `null`. Parsed
/// back over two batches, in both JSON formats.
#[test]
fn json_keeps_null_properties_and_all_null_columns() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("note", DataType::Utf8, true),
        Field::new("optional", DataType::Utf8, true),
    ]));
    let batch = |ids: Vec<i32>, notes: Vec<Option<&str>>| {
        let empty: Vec<Option<&str>> = vec![None; ids.len()];
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(notes)),
                Arc::new(StringArray::from(empty)),
            ],
        )
        .expect("the columns match the schema")
    };
    let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
    buffer
        .push(batch(vec![1, 2], vec![Some("a"), Some("")]))
        .expect("batch accepted");
    buffer
        .push(batch(vec![3, 4], vec![None, Some("d")]))
        .expect("batch accepted");
    buffer.mark_complete(ExecStats::default());

    for format in [ExportFormat::Json, ExportFormat::JsonLines] {
        let text = export_text(&buffer, format);
        let records: Vec<serde_json::Value> = if format == ExportFormat::Json {
            serde_json::from_str(&text).expect("a JSON array")
        } else {
            text.lines()
                .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
                .collect()
        };
        assert_eq!(records.len(), 4, "{format}: {text}");
        for record in &records {
            let keys: Vec<&str> = record
                .as_object()
                .expect("each record is an object")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, ["id", "note", "optional"], "{format}: {text}");
            assert_eq!(record["optional"], serde_json::Value::Null, "{format}");
        }
        let notes: Vec<&serde_json::Value> = records.iter().map(|r| &r["note"]).collect();
        assert_eq!(
            notes,
            [
                &serde_json::json!("a"),
                &serde_json::json!(""),
                &serde_json::Value::Null,
                &serde_json::json!("d"),
            ],
            "{format}: {text}"
        );
    }
}

/// A complete buffer of one row over `names`, column `i` holding `i`.
fn one_row_named(names: &[&str]) -> ResultBuffer {
    let schema = Arc::new(Schema::new(
        names
            .iter()
            .map(|name| Field::new(*name, DataType::Int32, false))
            .collect::<Vec<_>>(),
    ));
    let columns = (0..names.len())
        .map(|i| {
            let value = i32::try_from(i).expect("a few test columns");
            Arc::new(Int32Array::from(vec![value])) as ArrayRef
        })
        .collect();
    let buffer = ResultBuffer::new(Arc::clone(&schema), 1 << 20);
    buffer
        .push(RecordBatch::try_new(schema, columns).expect("the columns match the schema"))
        .expect("batch accepted");
    buffer.mark_complete(ExecStats::default());
    buffer
}

/// Duplicate names — an alias repeated, or `SELECT *` over a join — are
/// refused in JSON and JSON Lines, named once each, and the message asks
/// for aliases (issue #183).
#[test]
fn json_refuses_duplicate_column_names_and_names_them() {
    let buffer = one_row_named(&["id", "name", "id", "name", "id", "total"]);
    for format in [ExportFormat::Json, ExportFormat::JsonLines] {
        let mut output: Vec<u8> = Vec::new();
        let outcome = export(
            &buffer,
            format,
            &mut output,
            &ExportOptions::default(),
            &CancelToken::new(),
        );
        let Err(DataError::DuplicateColumnNames { names, .. }) = &outcome else {
            panic!("{format}: expected a refusal, got {outcome:?}");
        };
        assert_eq!(names, &["id", "name"], "{format}");
        let message = outcome.expect_err("refused").to_string();
        assert!(message.contains("\"id\", \"name\""), "{message}");
        assert!(message.contains("aliases"), "{message}");
        assert!(output.is_empty(), "{format}: nothing is written");
    }
}

/// The formats that keep columns by position still export them all.
#[test]
fn positional_formats_keep_duplicate_column_names() {
    let buffer = one_row_named(&["id", "id"]);
    assert_eq!(export_text(&buffer, ExportFormat::Csv), "id,id\n0,1\n");
    assert_eq!(export_text(&buffer, ExportFormat::Tsv), "id\tid\n0\t1\n");
    let mut output: Vec<u8> = Vec::new();
    export(
        &buffer,
        ExportFormat::ArrowIpc,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    )
    .expect("Arrow IPC keeps duplicate names");
}

/// Distinct names stay distinct: case counts, and a name that merely looks
/// like a suffixed copy is its own key.
#[test]
fn json_accepts_names_that_differ_only_by_case_or_suffix() {
    let buffer = one_row_named(&["id", "ID", "id_2"]);
    let text = export_text(&buffer, ExportFormat::Json);
    let records: Vec<serde_json::Value> = serde_json::from_str(&text).expect("a JSON array");
    assert_eq!(
        records,
        [serde_json::json!({"id": 0, "ID": 1, "id_2": 2})],
        "{text}"
    );
}

#[test]
fn an_empty_result_produces_an_empty_file_not_an_error() {
    let buffer = ResultBuffer::new(schema(), 1 << 20);
    buffer.mark_complete(ExecStats::default());
    let mut output: Vec<u8> = Vec::new();

    let summary = export(
        &buffer,
        ExportFormat::Csv,
        &mut output,
        &ExportOptions::default(),
        &CancelToken::new(),
    )
    .expect("export without error");

    assert_eq!(summary.rows, 0);
    assert_eq!(summary.batches, 0);
    assert!(output.is_empty());
}
