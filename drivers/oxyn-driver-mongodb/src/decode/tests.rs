use arrow::array::{Array, AsArray};
use arrow::datatypes::{DataType, Int32Type, Int64Type, TimeUnit, TimestampMillisecondType};
use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Decimal128, RawDocumentBuf, doc};

use super::*;

fn raw(documents: &[bson::Document]) -> Vec<RawDocumentBuf> {
    documents
        .iter()
        .map(|document| RawDocumentBuf::try_from(document).expect("valid BSON"))
        .collect()
}

fn plan(documents: &[RawDocumentBuf]) -> Plan {
    Plan::infer(documents.iter().map(AsRef::as_ref)).expect("readable sample")
}

fn batch(plan: &Plan, documents: &[RawDocumentBuf]) -> RecordBatch {
    let mut assembler = plan.assembler();
    for (row, document) in documents.iter().enumerate() {
        assembler
            .push(document, u64::try_from(row).unwrap_or(0) + 1)
            .expect("fits the plan");
    }
    assembler.finish().expect("consistent")
}

#[test]
fn a_single_type_field_gets_its_arrow_type_and_the_schema_says_it_is_inferred() {
    let documents = raw(&[
        doc! { "_id": 1_i32, "big": 9_007_199_254_740_993_i64, "at": DateTime::from_millis(5) },
        doc! { "_id": 2_i32, "big": 1_i64, "at": Bson::Null },
    ]);
    let plan = plan(&documents);
    let schema = plan.schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(names, ["_id", "big", "at", DOCUMENT_COLUMN]);
    assert_eq!(schema.field(0).data_type(), &DataType::Int32);
    assert_eq!(schema.field(1).data_type(), &DataType::Int64);
    assert_eq!(
        schema.field(2).data_type(),
        &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
    );
    assert_eq!(
        schema
            .metadata()
            .get(META_SAMPLE_DOCUMENTS)
            .map(String::as_str),
        Some("2")
    );
    for field in schema.fields().iter().take(3) {
        assert_eq!(
            field.metadata().get(META_INFERRED).map(String::as_str),
            Some("true")
        );
    }
    let batch = batch(&plan, &documents);
    let big = batch.column(1).as_primitive::<Int64Type>();
    assert_eq!(big.value(0), 9_007_199_254_740_993, "beyond 2^53, exact");
    let at = batch.column(2).as_primitive::<TimestampMillisecondType>();
    assert_eq!(at.value(0), 5);
    assert!(at.is_null(1));
}

#[test]
fn a_field_with_two_types_is_mixed_text_without_coercion() {
    let documents = raw(&[
        doc! { "v": 1_i32 },
        doc! { "v": { "nested": true } },
        doc! { "v": "1" },
    ]);
    let plan = plan(&documents);
    let schema = plan.schema();
    assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(
        schema
            .field(0)
            .metadata()
            .get(META_BSON_TYPE)
            .map(String::as_str),
        Some("mixed")
    );
    let batch = batch(&plan, &documents);
    let values = batch.column(0).as_string::<i32>();
    assert_eq!(values.value(0), "1");
    assert_eq!(values.value(1), "{\"nested\":true}");
    assert_eq!(
        values.value(2),
        "\"1\"",
        "a string stays quoted, not a number"
    );
}

#[test]
fn decimal128_is_text_never_a_double() {
    let documents = raw(&[doc! {
        "price": Bson::Decimal128("9999999999999999999999999999999999".parse::<Decimal128>().expect("34 digits")),
    }]);
    let plan = plan(&documents);
    assert_eq!(plan.schema().field(0).data_type(), &DataType::Utf8);
    let batch = batch(&plan, &documents);
    assert_eq!(
        batch.column(0).as_string::<i32>().value(0),
        "9999999999999999999999999999999999"
    );
}

#[test]
fn a_field_outside_the_sample_lives_in_the_lossless_column() {
    let sample = raw(&[doc! { "a": 1_i32 }]);
    let plan = plan(&sample);
    let later = raw(&[doc! { "a": 2_i32, "late": { "$x": 1_i64 } }]);
    let batch = batch(&plan, &later);
    assert_eq!(batch.num_columns(), 2, "`a` and the document");
    let document = batch.column(1).as_string::<i32>().value(0);
    assert_eq!(
        document,
        "{\"a\":{\"$numberInt\":\"2\"},\"late\":{\"$x\":{\"$numberLong\":\"1\"}}}"
    );
}

#[test]
fn a_type_outside_the_sample_is_an_error_naming_it_and_keeps_the_rows_before() {
    let sample = raw(&[doc! { "n": 1_i32 }]);
    let plan = plan(&sample);
    let mut assembler = plan.assembler();
    let documents = raw(&[doc! { "n": 2_i32 }, doc! { "n": { "a": 1_i32 } }]);
    let first = documents.first().expect("two documents");
    let second = documents.get(1).expect("two documents");
    assembler.push(first, 1).expect("an int");
    let error = assembler
        .push(second, 2)
        .expect_err("a document in an int column");
    assert_eq!(
        error,
        DecodeError::TypeOutsideSample {
            row: 2,
            field: "n".to_owned(),
            expected: "int",
            found: "object",
            sample: 1,
        }
    );
    let batch = assembler.finish().expect("the first row is whole");
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.column(0).as_primitive::<Int32Type>().value(0), 2);
}

#[test]
fn a_document_key_named_like_the_lossless_column_does_not_collide() {
    let documents = raw(&[doc! { DOCUMENT_COLUMN: 1_i32 }]);
    let plan = plan(&documents);
    let names: Vec<String> = plan
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(
        names,
        [DOCUMENT_COLUMN.to_owned(), format!("{DOCUMENT_COLUMN}#")]
    );
}

#[test]
fn a_repeated_key_inside_one_document_keeps_the_first_in_its_column() {
    // BSON allows it; a JSON reader would keep one silently.
    let bytes = vec![
        19, 0, 0, 0, // length
        0x10, b'a', 0, 1, 0, 0, 0, // "a": 1
        0x10, b'a', 0, 2, 0, 0, 0, // "a": 2
        0,
    ];
    let document = RawDocumentBuf::from_bytes(bytes).expect("framing is valid");
    let documents = vec![document];
    let plan = plan(&documents);
    let batch = batch(&plan, &documents);
    assert_eq!(batch.column(0).as_primitive::<Int32Type>().value(0), 1);
    assert_eq!(
        batch.column(1).as_string::<i32>().value(0),
        "{\"a\":{\"$numberInt\":\"1\"},\"a\":{\"$numberInt\":\"2\"}}",
        "both values in the lossless column"
    );
}

#[test]
fn binary_columns_keep_their_subtype() {
    let documents = raw(&[doc! {
        "u": Binary { subtype: BinarySubtype::Uuid, bytes: vec![7; 16] },
    }]);
    let plan = plan(&documents);
    let field = plan.schema().field(0).clone();
    assert_eq!(field.data_type(), &DataType::Binary);
    assert_eq!(
        field.metadata().get(META_BSON_SUBTYPE).map(String::as_str),
        Some("04")
    );
    let other = raw(&[doc! { "u": Binary { subtype: BinarySubtype::Generic, bytes: vec![1] } }]);
    let mut assembler = plan.assembler();
    assert!(matches!(
        assembler.push(other.first().expect("one"), 1),
        Err(DecodeError::TypeOutsideSample { .. })
    ));
}

#[test]
fn the_column_count_is_capped_and_the_rest_is_declared_omitted() {
    let mut wide = bson::Document::new();
    for index in 0..MAX_FIELD_COLUMNS + 10 {
        wide.insert(format!("f{index}"), 1_i32);
    }
    let documents = raw(&[wide]);
    let plan = plan(&documents);
    let schema = plan.schema();
    assert_eq!(schema.fields().len(), MAX_FIELD_COLUMNS + 1);
    assert_eq!(
        schema
            .metadata()
            .get(META_OMITTED_FIELDS)
            .map(String::as_str),
        Some("10")
    );
}

#[test]
fn a_null_only_field_accepts_any_later_value() {
    let plan = plan(&raw(&[doc! { "x": Bson::Null }]));
    let later = raw(&[doc! { "x": [1_i32, "a"] }]);
    let batch = batch(&plan, &later);
    assert_eq!(batch.column(0).as_string::<i32>().value(0), "[1,\"a\"]");
}

#[test]
fn an_unreadable_sample_is_refused() {
    let bytes = vec![10, 0, 0, 0, 0x42, b'a', 0, 1, 0, 0];
    let document = RawDocumentBuf::from_bytes(bytes).expect("framing is valid");
    let error = Plan::infer([document.as_ref()]).expect_err("unknown type");
    assert!(
        matches!(error, DecodeError::Malformed { row: 1, .. }),
        "{error:?}"
    );
}
