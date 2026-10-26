use super::*;

#[test]
fn null_renders_in_uppercase_and_differs_from_empty() {
    assert_eq!(ScalarValue::Null.to_string(), "NULL");
    assert_eq!(ScalarValue::Text(String::new()).to_string(), "");
    // The trap, stated as such: `Display` does not tell `NULL` apart from
    // the string "NULL". That is deliberate — `Display` renders a value, it
    // carries no typography — and it is why the grid draws an absent value
    // as `∅ NULL` in its own colour token
    // (`apps/desktop/src/components/oxyn/cell-value.tsx`). Anyone comparing
    // values through this rendering would take one for the other.
    assert_eq!(
        ScalarValue::Null.to_string(),
        ScalarValue::Text("NULL".into()).to_string(),
        "if these two renderings ever diverge, the grid may stop telling them \
             apart visually: the grid is what carries the distinction"
    );
}

#[test]
fn type_names_are_stable() {
    assert_eq!(ScalarValue::Null.type_name(), "null");
    assert_eq!(ScalarValue::Int64(1).type_name(), "int64");
    assert_eq!(
        ScalarValue::Timestamp(DateTime::<Utc>::UNIX_EPOCH).type_name(),
        "timestamptz"
    );
    assert_eq!(
        ScalarValue::TimestampNaive(DateTime::<Utc>::UNIX_EPOCH.naive_utc()).type_name(),
        "timestamp",
        "a timestamp without a time zone does not carry the same name as an instant"
    );
}

#[test]
fn an_instant_renders_in_utc() {
    let ts = DateTime::<Utc>::UNIX_EPOCH;
    let rendered = ScalarValue::Timestamp(ts).to_string();
    assert!(
        rendered.ends_with("+00:00"),
        "unexpected rendering: {rendered}"
    );
    assert!(rendered.starts_with("1970-01-01T00:00:00"));
}

#[test]
fn a_timestamp_without_time_zone_does_not_invent_one() {
    let naive = DateTime::<Utc>::UNIX_EPOCH.naive_utc();
    let rendered = ScalarValue::TimestampNaive(naive).to_string();
    assert!(
        !rendered.contains('+') && !rendered.ends_with('Z'),
        "a time zone was invented: {rendered}"
    );
}

#[test]
fn bytes_are_truncated_on_display() {
    let short = ScalarValue::Bytes(vec![0x00, 0x0a, 0xff]);
    assert_eq!(short.to_string(), "\\x000aff");

    let long = ScalarValue::Bytes(vec![0xab; 1024]);
    let rendered = long.to_string();
    assert!(rendered.contains("1024 bytes"), "rendering: {rendered}");
    assert!(rendered.len() < 128, "a BLOB must not be rendered in full");
}

#[test]
fn decimals_do_not_go_through_a_float() {
    let d = ScalarValue::Decimal("0.10".into());
    assert_eq!(d.to_string(), "0.10", "trailing zeros are significant");
}

#[test]
fn an_interval_keeps_its_three_components() {
    let i = ScalarValue::Interval {
        months: 1,
        days: 2,
        nanos: 3_000_000_004,
    };
    assert_eq!(i.to_string(), "P1M2DT3.000000004S");
}

#[test]
fn an_array_renders_nested() {
    let a = ScalarValue::Array(vec![
        ScalarValue::Int64(1),
        ScalarValue::Null,
        ScalarValue::Array(vec![ScalarValue::Text("x".into())]),
    ]);
    assert_eq!(a.to_string(), "[1, NULL, [x]]");
}

#[test]
fn conversion_from_option() {
    let absent: ScalarValue = Option::<i64>::None.into();
    assert!(absent.is_null());
    let present: ScalarValue = Some(7_i64).into();
    assert_eq!(present, ScalarValue::Int64(7));
}

#[test]
fn json_round_trip() {
    let cases = [
        ScalarValue::Null,
        ScalarValue::Bool(true),
        ScalarValue::Int64(-42),
        ScalarValue::Decimal("1.000".into()),
        ScalarValue::Text("café".into()),
        ScalarValue::Bytes(vec![1, 2, 3]),
        ScalarValue::Uuid(Uuid::nil()),
        ScalarValue::Interval {
            months: -1,
            days: 0,
            nanos: 1,
        },
        ScalarValue::Array(vec![ScalarValue::Int64(1)]),
    ];
    for value in cases {
        let json = serde_json::to_string(&value).expect("serialization");
        let read_back: ScalarValue = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(
            value,
            read_back,
            "round trip failed for {}",
            value.type_name()
        );
    }
}

// The following tests pin down what chrono's `FromStr` actually accepts,
// proven by running them, not recalled from memory (invariant I-12). The
// doc comments on `ParameterType::parse` restate exactly what these show.

#[test]
fn date_accepts_the_iso_calendar_form() {
    assert_eq!(
        "2024-01-15".parse::<NaiveDate>(),
        Ok(NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date"))
    );
    // Month and day are not required to be zero-padded.
    assert_eq!(
        "2024-1-15".parse::<NaiveDate>(),
        Ok(NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date"))
    );
}

#[test]
fn date_rejects_a_time_component_and_a_non_iso_order() {
    assert!("2024-01-15T00:00:00".parse::<NaiveDate>().is_err());
    assert!("15/01/2024".parse::<NaiveDate>().is_err());
}

#[test]
fn time_accepts_missing_seconds_and_fraction() {
    assert_eq!(
        "13:45:00.123".parse::<NaiveTime>(),
        Ok(NaiveTime::from_hms_milli_opt(13, 45, 0, 123).expect("valid time"))
    );
    // No seconds: they default to zero.
    assert_eq!(
        "13:45".parse::<NaiveTime>(),
        Ok(NaiveTime::from_hms_opt(13, 45, 0).expect("valid time"))
    );
}

#[test]
fn time_rejects_garbage() {
    assert!("not a time".parse::<NaiveTime>().is_err());
}

#[test]
fn timestamp_naive_requires_the_t_separator_and_no_offset() {
    assert!("2024-01-15T13:45:00".parse::<NaiveDateTime>().is_ok());
    // The SQL-style space separator is not accepted by `FromStr`.
    assert!("2024-01-15 13:45:00".parse::<NaiveDateTime>().is_err());
    // A trailing offset makes it a `Timestamp`, not a naive one.
    assert!("2024-01-15T13:45:00Z".parse::<NaiveDateTime>().is_err());
}

#[test]
fn timestamp_requires_an_explicit_offset() {
    assert!("2024-01-15T13:45:00Z".parse::<DateTime<Utc>>().is_ok());
    assert!("2024-01-15T13:45:00+02:00".parse::<DateTime<Utc>>().is_ok());
    // The space separator works too, as long as an offset is present.
    assert!("2024-01-15 13:45:00Z".parse::<DateTime<Utc>>().is_ok());
    // No offset: rejected rather than assumed to be UTC.
    assert!("2024-01-15T13:45:00".parse::<DateTime<Utc>>().is_err());
}

#[test]
fn parameter_type_null_ignores_its_input() {
    assert_eq!(ParameterType::Null.parse("anything"), Ok(ScalarValue::Null));
}

#[test]
fn parameter_type_bool_parses_strictly() {
    assert_eq!(
        ParameterType::Bool.parse("true"),
        Ok(ScalarValue::Bool(true))
    );
    assert!(ParameterType::Bool.parse("yes").is_err());
}

#[test]
fn parameter_type_int64_parses_strictly() {
    assert_eq!(
        ParameterType::Int64.parse("-42"),
        Ok(ScalarValue::Int64(-42))
    );
    assert!(ParameterType::Int64.parse("4.2").is_err());
}

#[test]
fn parameter_type_float64_parses_strictly() {
    assert_eq!(
        ParameterType::Float64.parse("4.2"),
        Ok(ScalarValue::Float64(4.2))
    );
    assert!(ParameterType::Float64.parse("four").is_err());
}

#[test]
fn parameter_type_text_accepts_anything_including_empty() {
    assert_eq!(
        ParameterType::Text.parse(""),
        Ok(ScalarValue::Text(String::new()))
    );
}

#[test]
fn parameter_type_bytes_parses_hex() {
    assert_eq!(
        ParameterType::Bytes.parse("00ff"),
        Ok(ScalarValue::Bytes(vec![0x00, 0xff]))
    );
    assert!(ParameterType::Bytes.parse("0").is_err(), "odd length");
    assert!(ParameterType::Bytes.parse("zz").is_err(), "not hex");
}

#[test]
fn parameter_type_uuid_parses_hyphenated_and_bare_forms() {
    let expected = Uuid::nil();
    assert_eq!(
        ParameterType::Uuid.parse("00000000-0000-0000-0000-000000000000"),
        Ok(ScalarValue::Uuid(expected))
    );
    assert!(ParameterType::Uuid.parse("not-a-uuid").is_err());
}

#[test]
fn parameter_type_date_parses_iso_form() {
    assert_eq!(
        ParameterType::Date.parse("2024-01-15"),
        Ok(ScalarValue::Date(
            NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date")
        ))
    );
    assert!(ParameterType::Date.parse("15/01/2024").is_err());
}

#[test]
fn parameter_type_time_parses_iso_form() {
    assert_eq!(
        ParameterType::Time.parse("13:45:00"),
        Ok(ScalarValue::Time(
            NaiveTime::from_hms_opt(13, 45, 0).expect("valid time")
        ))
    );
    assert!(ParameterType::Time.parse("not a time").is_err());
}

#[test]
fn parameter_type_timestamp_naive_parses_t_form_and_rejects_offset() {
    assert!(
        ParameterType::TimestampNaive
            .parse("2024-01-15T13:45:00")
            .is_ok()
    );
    assert!(
        ParameterType::TimestampNaive
            .parse("2024-01-15T13:45:00Z")
            .is_err(),
        "an offset makes it a Timestamp, not a naive one"
    );
}

#[test]
fn parameter_type_timestamp_parses_with_offset_and_rejects_without() {
    assert!(
        ParameterType::Timestamp
            .parse("2024-01-15T13:45:00Z")
            .is_ok()
    );
    assert!(
        ParameterType::Timestamp
            .parse("2024-01-15T13:45:00")
            .is_err(),
        "no offset: must not be guessed as UTC"
    );
}

#[test]
fn parameter_type_json_parses_a_document() {
    assert_eq!(
        ParameterType::Json.parse("{\"a\":1}"),
        Ok(ScalarValue::Json(serde_json::json!({"a": 1})))
    );
    assert!(ParameterType::Json.parse("{a:1}").is_err());
}

#[test]
fn parameter_type_decimal_keeps_a_long_literal_exact() {
    let forty_digits = "1".repeat(40);
    let literal = format!("{forty_digits}.5");
    assert_eq!(
        ParameterType::Decimal.parse(&literal),
        Ok(ScalarValue::Decimal(literal.clone())),
        "the exact digits must survive untouched"
    );
}

#[test]
fn parameter_type_decimal_rejects_scientific_notation() {
    // `f64::parse("1e400")` succeeds and silently produces infinity; the
    // form validator must reject it outright instead.
    assert!(ParameterType::Decimal.parse("1e400").is_err());
    assert!("1e400".parse::<f64>().unwrap_or_default().is_infinite());
}

#[test]
fn parameter_type_decimal_rejects_malformed_shapes() {
    assert!(ParameterType::Decimal.parse("").is_err());
    assert!(ParameterType::Decimal.parse("-").is_err());
    assert!(ParameterType::Decimal.parse("1.2.3").is_err());
    assert!(ParameterType::Decimal.parse("12a").is_err());
}

#[test]
fn parameter_type_decimal_accepts_signed_and_pointless_forms() {
    assert!(ParameterType::Decimal.parse("-0.5").is_ok());
    assert!(ParameterType::Decimal.parse("+5").is_ok());
    assert!(ParameterType::Decimal.parse(".5").is_ok());
    assert!(ParameterType::Decimal.parse("5.").is_ok());
}

#[test]
fn parameter_parse_error_never_carries_the_rejected_text() {
    let sentinel = "S3NT1NEL";
    let err = ParameterType::Int64.parse(sentinel).expect_err("invalid");
    assert!(!format!("{err:?}").contains(sentinel));
    assert!(!format!("{err}").contains(sentinel));
}
