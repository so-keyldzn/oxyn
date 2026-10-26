use arrow::array::{Array, BinaryArray, Float64Array, Int64Array, StringArray};

use super::*;

fn observed(values: &[ValueRef<'_>]) -> Observed {
    let mut seen_classes = Observed::default();
    for value in values {
        seen_classes.observe(*value);
    }
    seen_classes
}

fn resolve(values: &[ValueRef<'_>], declared: Option<&str>) -> ColumnPlan {
    ColumnPlan::resolve(
        "c".to_owned(),
        declared.map(str::to_owned),
        observed(values),
    )
}

#[test]
fn sqlite_affinity_rules_are_reproduced_as_they_are() {
    assert_eq!(affinity(Some("INTEGER")), Some(ColumnKind::Int64));
    assert_eq!(affinity(Some("BIGINT")), Some(ColumnKind::Int64));
    assert_eq!(affinity(Some("VARCHAR(255)")), Some(ColumnKind::Utf8));
    assert_eq!(affinity(Some("TEXT")), Some(ColumnKind::Utf8));
    assert_eq!(affinity(Some("BLOB")), Some(ColumnKind::Binary));
    assert_eq!(
        affinity(Some("DOUBLE PRECISION")),
        Some(ColumnKind::Float64)
    );
    assert_eq!(affinity(Some("REAL")), Some(ColumnKind::Float64));
    // NUMERIC affinity: neither integer nor float covers it.
    assert_eq!(affinity(Some("DECIMAL(10,2)")), Some(ColumnKind::Utf8));
    // The surprising consequence of rule 1, and it is correct: SQLite does
    // give INTEGER affinity to `POINT`.
    assert_eq!(affinity(Some("POINT")), Some(ColumnKind::Int64));
    // Nothing declared: no affinity, the values will decide.
    assert_eq!(affinity(None), None);
    assert_eq!(affinity(Some("   ")), None);
}

#[test]
fn a_homogeneous_column_keeps_its_exact_type() {
    assert_eq!(
        resolve(&[ValueRef::Integer(1), ValueRef::Integer(2)], None).kind,
        ColumnKind::Int64
    );
    assert_eq!(
        resolve(&[ValueRef::Real(1.5)], None).kind,
        ColumnKind::Float64
    );
    assert_eq!(
        resolve(&[ValueRef::Text(b"a")], None).kind,
        ColumnKind::Utf8
    );
    assert_eq!(
        resolve(&[ValueRef::Blob(b"\x00\xff")], None).kind,
        ColumnKind::Binary
    );
}

#[test]
fn a_column_mixing_integers_and_floats_falls_back_to_text() {
    // The trap: no `f64` holds every `i64`. Converting
    // 9_007_199_254_740_993 to a float changes it into 9_007_199_254_740_992.
    let plan = resolve(&[ValueRef::Integer(1), ValueRef::Real(1.5)], None);
    assert_eq!(plan.kind, ColumnKind::Utf8);
    assert!(plan.observed.is_mixed());
    assert_eq!(plan.observed.names(), "integer|real");
}

#[test]
fn a_numeric_affinity_column_of_whole_values_stays_text() {
    // Under NUMERIC affinity `10.00` is stored as INTEGER, but `12.50` comes
    // back as REAL: typing the column `Int64` from the first batch would fail
    // the stream at the first fraction.
    for declared in ["DECIMAL(10,2)", "NUMERIC"] {
        let plan = resolve(
            &[ValueRef::Integer(10), ValueRef::Integer(20)],
            Some(declared),
        );
        assert_eq!(plan.kind, ColumnKind::Utf8, "{declared}");
        assert!(!plan.inferred, "text is what the affinity implies");
    }
    let plan = resolve(&[ValueRef::Real(12.5)], Some("DECIMAL(10,2)"));
    assert_eq!(plan.kind, ColumnKind::Utf8);
}

#[test]
fn integer_and_real_declarations_keep_their_exact_type() {
    let whole = [ValueRef::Integer(10), ValueRef::Integer(20)];
    assert_eq!(resolve(&whole, Some("INTEGER")).kind, ColumnKind::Int64);
    assert_eq!(
        resolve(&[ValueRef::Real(1.5)], Some("REAL")).kind,
        ColumnKind::Float64
    );
}

#[test]
fn a_blob_imposes_bytes_on_the_whole_column() {
    for non_blob in [
        ValueRef::Integer(1),
        ValueRef::Real(1.0),
        ValueRef::Text(b"a"),
    ] {
        let plan = resolve(&[non_blob, ValueRef::Blob(b"\x00")], None);
        assert_eq!(
            plan.kind,
            ColumnKind::Binary,
            "only bytes contain everything"
        );
    }
}

#[test]
fn a_text_that_is_not_utf8_counts_as_bytes() {
    // Found here rather than in the middle of the stream: that is the whole
    // point of the probe.
    let plan = resolve(&[ValueRef::Text(b"\xff\xfe")], None);
    assert_eq!(plan.kind, ColumnKind::Binary);
}

#[test]
fn an_entirely_null_column_falls_back_on_the_declaration() {
    let plan = resolve(&[ValueRef::Null, ValueRef::Null], Some("INTEGER"));
    assert_eq!(plan.kind, ColumnKind::Int64);
    assert!(
        !plan.inferred,
        "the type comes from the declaration: nothing is inferred"
    );
    assert!(plan.observed.is_empty());
}

#[test]
fn without_value_or_declaration_text_is_the_fallback() {
    let plan = resolve(&[], None);
    assert_eq!(plan.kind, ColumnKind::Utf8);
    assert!(plan.inferred, "nothing declared it: it is an inference");
}

#[test]
fn a_column_whose_type_does_not_follow_its_declaration_declares_itself_inferred() {
    // `CREATE TABLE t(n INTEGER)` then `INSERT INTO t VALUES('abc')`: SQLite
    // accepts it, and the interface must be able to say that the displayed type
    // is not the schema's.
    let plan = resolve(&[ValueRef::Text(b"abc")], Some("INTEGER"));
    assert_eq!(plan.kind, ColumnKind::Utf8);
    assert!(plan.inferred);

    let champ = plan.field();
    assert_eq!(
        champ.metadata().get(METADATA_INFERRED).map(String::as_str),
        Some("true")
    );
    assert_eq!(
        champ
            .metadata()
            .get(METADATA_DECLARED_TYPE)
            .map(String::as_str),
        Some("INTEGER")
    );
}

#[test]
fn a_mixed_column_declares_it_in_its_metadata() {
    let plan = resolve(&[ValueRef::Integer(1), ValueRef::Text(b"a")], None);
    let champ = plan.field();
    assert_eq!(
        champ
            .metadata()
            .get(METADATA_STORAGE_CLASSES)
            .map(String::as_str),
        Some("integer|text"),
        "the interface must be able to show that the rendering is a fallback"
    );
}

#[test]
fn every_field_is_nullable() {
    // SQLite returns NULL for any expression, and a NOT NULL column seen
    // through an outer join is just as nullable.
    assert!(
        resolve(&[ValueRef::Integer(1)], Some("INTEGER"))
            .field()
            .is_nullable()
    );
}

#[test]
fn accepted_conversions_are_lossless() {
    let mut b = ColumnBuilder::new(ColumnKind::Utf8, 0, 4);
    b.append(ValueRef::Integer(-42)).expect("integer as text");
    b.append(ValueRef::Real(0.1)).expect("float as text");
    b.append(ValueRef::Text(b"caf\xc3\xa9")).expect("text");
    b.append(ValueRef::Null).expect("null");

    let built_array = b.finish();
    let built_array = built_array
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("Utf8 column");
    assert_eq!(built_array.value(0), "-42");
    assert_eq!(
        built_array.value(1),
        "0.1",
        "f64's Display renders the shortest form that reads back identically"
    );
    assert_eq!(built_array.value(2), "café");
    assert!(built_array.is_null(3));
}

#[test]
fn an_integer_beyond_two_to_the_53_does_not_go_through_a_float() {
    // The silent corruption this refusal avoids: 2⁵³+1 would become 2⁵³.
    let mut b = ColumnBuilder::new(ColumnKind::Float64, 2, 1);
    b.append(ValueRef::Integer(1 << 53)).expect("2^53 is exact");

    let err = b
        .append(ValueRef::Integer((1_i64 << 53) + 1))
        .expect_err("expected refusal");
    let SqliteError::ColumnConflict {
        column, resolved, ..
    } = err
    else {
        panic!("wrong variant: {err:?}");
    };
    assert_eq!((column, resolved), (2, "float64"));
}

#[test]
fn a_blob_does_not_slip_into_a_textual_column() {
    let mut b = ColumnBuilder::new(ColumnKind::Utf8, 1, 1);
    let err = b
        .append(ValueRef::Blob(b"\x00\xff"))
        .expect_err("expected refusal");
    assert!(
        matches!(
            err,
            SqliteError::ColumnConflict {
                found: "blob",
                resolved: "utf8",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn an_invalid_text_does_not_slip_into_a_textual_column() {
    let mut b = ColumnBuilder::new(ColumnKind::Utf8, 0, 1);
    let err = b
        .append(ValueRef::Text(b"\xff"))
        .expect_err("expected refusal");
    assert!(matches!(err, SqliteError::ColumnConflict { .. }), "{err:?}");
}

#[test]
fn a_text_does_not_slip_into_an_integer_column() {
    let mut b = ColumnBuilder::new(ColumnKind::Int64, 0, 1);
    assert!(b.append(ValueRef::Text(b"12")).is_err());
    assert!(b.append(ValueRef::Real(1.0)).is_err());
    assert!(b.append(ValueRef::Blob(b"x")).is_err());
    b.append(ValueRef::Integer(7)).expect("an integer passes");
    let built_array = b.finish();
    let built_array = built_array
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("Int64 column");
    assert_eq!(built_array.value(0), 7);
}

#[test]
fn a_bytes_column_holds_everything_without_losing_content() {
    let mut b = ColumnBuilder::new(ColumnKind::Binary, 0, 4);
    b.append(ValueRef::Blob(b"\x00\xff")).expect("bytes");
    b.append(ValueRef::Text(b"abc")).expect("text as bytes");
    b.append(ValueRef::Integer(12)).expect("integer as bytes");
    b.append(ValueRef::Null).expect("null");

    let built_array = b.finish();
    let built_array = built_array
        .as_any()
        .downcast_ref::<BinaryArray>()
        .expect("Binary column");
    assert_eq!(built_array.value(0), b"\x00\xff");
    assert_eq!(built_array.value(1), b"abc");
    assert_eq!(built_array.value(2), b"12");
    assert!(built_array.is_null(3));
}

#[test]
fn a_float_stays_a_float() {
    let mut b = ColumnBuilder::new(ColumnKind::Float64, 0, 2);
    b.append(ValueRef::Real(1.5)).expect("float");
    b.append(ValueRef::Integer(3)).expect("exact integer");
    let built_array = b.finish();
    let built_array = built_array
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("Float64 column");
    assert!((built_array.value(0) - 1.5).abs() < f64::EPSILON);
    assert!((built_array.value(1) - 3.0).abs() < f64::EPSILON);
}

#[test]
fn a_batch_is_measured_in_data_bytes() {
    assert_eq!(value_bytes(ValueRef::Null), 0);
    assert_eq!(value_bytes(ValueRef::Integer(1)), 8);
    assert_eq!(value_bytes(ValueRef::Blob(&[0; 1_048_576])), 1_048_576);
}

#[test]
fn a_value_set_aside_reads_back_identically() {
    for sample in [
        ValueRef::Null,
        ValueRef::Integer(-1),
        ValueRef::Real(2.5),
        ValueRef::Text(b"\xff non-utf8"),
        ValueRef::Blob(b"\x00"),
    ] {
        let captured = ProbeValue::capture(sample);
        assert_eq!(captured.borrow(), sample);
    }
}
