use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{
    Binary, Bson, DateTime, Decimal128, JavaScriptCodeWithScope, RawDocumentBuf, Regex, Timestamp,
    doc,
};

use super::*;

fn raw(document: &bson::Document) -> RawDocumentBuf {
    RawDocumentBuf::try_from(document).expect("a document the test builds is valid BSON")
}

#[test]
fn canonical_keeps_every_type_and_the_key_order() {
    let id = ObjectId::parse_str("65f0a1b2c3d4e5f6a7b8c9d0").expect("hex");
    let document = doc! {
        "z": 1_i32,
        "a": 9_007_199_254_740_993_i64,
        "id": id,
        "price": Bson::Decimal128("12.10".parse::<Decimal128>().expect("decimal")),
        "ratio": 0.5,
        "whole": 2.0,
        "when": DateTime::from_millis(1_700_000_000_123),
        "flag": true,
        "nothing": Bson::Null,
    };
    let text = super::document(&raw(&document), Mode::Canonical).expect("writes");
    assert_eq!(
        text,
        "{\"z\":{\"$numberInt\":\"1\"},\"a\":{\"$numberLong\":\"9007199254740993\"},\
         \"id\":{\"$oid\":\"65f0a1b2c3d4e5f6a7b8c9d0\"},\
         \"price\":{\"$numberDecimal\":\"12.10\"},\
         \"ratio\":{\"$numberDouble\":\"0.5\"},\"whole\":{\"$numberDouble\":\"2.0\"},\
         \"when\":{\"$date\":{\"$numberLong\":\"1700000000123\"}},\"flag\":true,\"nothing\":null}"
    );
}

#[test]
fn relaxed_writes_numbers_and_dates_plainly_but_never_loses_a_digit() {
    let document = doc! {
        "big": 9_007_199_254_740_993_i64,
        "when": DateTime::from_millis(1_700_000_000_123),
        "old": DateTime::from_millis(-1),
        "inf": f64::INFINITY,
        "nan": f64::NAN,
    };
    let text = super::document(&raw(&document), Mode::Relaxed).expect("writes");
    assert_eq!(
        text,
        "{\"big\":9007199254740993,\"when\":{\"$date\":\"2023-11-14T22:13:20.123Z\"},\
         \"old\":{\"$date\":{\"$numberLong\":\"-1\"}},\
         \"inf\":{\"$numberDouble\":\"Infinity\"},\"nan\":{\"$numberDouble\":\"NaN\"}}"
    );
}

#[test]
fn the_rare_types_get_their_wrappers() {
    let document = doc! {
        "ts": Timestamp { time: 1_700_000_000, increment: 7 },
        "re": Regex {
            pattern: "^a\"b".to_owned().try_into().expect("no NUL"),
            options: "im".to_owned().try_into().expect("no NUL"),
        },
        "js": Bson::JavaScriptCode("return 1".to_owned()),
        "scope": Bson::JavaScriptCodeWithScope(JavaScriptCodeWithScope {
            code: "x".to_owned(),
            scope: doc! { "x": 1_i32 },
        }),
        "uuid": Binary { subtype: BinarySubtype::Uuid, bytes: vec![0xab; 16] },
        "bytes": Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 1, 2, 250] },
        "sym": Bson::Symbol("s".to_owned()),
        "min": Bson::MinKey,
        "max": Bson::MaxKey,
        "undef": Bson::Undefined,
    };
    let text = super::document(&raw(&document), Mode::Canonical).expect("writes");
    for expected in [
        "\"ts\":{\"$timestamp\":{\"t\":1700000000,\"i\":7}}",
        "\"re\":{\"$regularExpression\":{\"pattern\":\"^a\\\"b\",\"options\":\"im\"}}",
        "\"js\":{\"$code\":\"return 1\"}",
        "\"scope\":{\"$code\":\"x\",\"$scope\":{\"x\":{\"$numberInt\":\"1\"}}}",
        "\"uuid\":{\"$binary\":{\"base64\":\"q6urq6urq6urq6urq6urqw==\",\"subType\":\"04\"}}",
        "\"bytes\":{\"$binary\":{\"base64\":\"AAEC+g==\",\"subType\":\"00\"}}",
        "\"sym\":{\"$symbol\":\"s\"}",
        "\"min\":{\"$minKey\":1}",
        "\"max\":{\"$maxKey\":1}",
        "\"undef\":{\"$undefined\":true}",
    ] {
        assert!(text.contains(expected), "{expected} missing from {text}");
    }
}

#[test]
fn a_hostile_key_is_escaped_not_interpreted() {
    let document = doc! { "a\"b\n\u{1b}[2J; DROP": 1_i32 };
    let text = super::document(&raw(&document), Mode::Relaxed).expect("writes");
    assert_eq!(text, "{\"a\\\"b\\n\\u001b[2J; DROP\":1}");
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
}

#[test]
fn an_unknown_element_type_is_an_error_that_names_it() {
    // A document holding one element of type 0x42, which BSON does not define.
    let bytes = vec![10, 0, 0, 0, 0x42, b'a', 0, 1, 0, 0];
    let document = bson::RawDocument::from_bytes(&bytes).expect("framing is valid");
    let error = super::document(document, Mode::Canonical).expect_err("unknown type");
    assert!(error.to_string().contains("66"), "{error}");
}

#[test]
fn a_string_that_is_not_utf8_is_refused_never_replaced() {
    // {"s": "\xff"}: length 2 (one byte and the NUL).
    let bytes = vec![14, 0, 0, 0, 0x02, b's', 0, 2, 0, 0, 0, 0xff, 0, 0];
    let document = bson::RawDocument::from_bytes(&bytes).expect("framing is valid");
    assert!(super::document(document, Mode::Canonical).is_err());
}

#[test]
fn every_truncation_of_a_document_fails_cleanly() {
    let document = raw(&doc! {
        "a": "text",
        "n": { "deep": [1_i32, 2_i64, "x"] },
        "b": Binary { subtype: BinarySubtype::Generic, bytes: vec![1; 40] },
    });
    let bytes = document.as_bytes();
    let mut refused = 0;
    for cut in 5..bytes.len() {
        // A reply cut short, re-framed so that the outer length and the final
        // NUL agree: only the elements inside can tell it is truncated.
        let mut truncated = bytes.get(..cut).unwrap_or_default().to_vec();
        let length = i32::try_from(cut).expect("a test document is small");
        truncated.splice(0..4, length.to_le_bytes());
        if let Some(last) = truncated.last_mut() {
            *last = 0;
        }
        if let Ok(view) = bson::RawDocument::from_bytes(&truncated)
            && super::document(view, Mode::Canonical).is_err()
        {
            refused += 1;
        }
    }
    assert!(refused > 0, "no truncation was detected");
}

#[test]
fn a_document_nested_past_the_bound_is_refused() {
    let mut document = doc! { "leaf": 1_i32 };
    for _ in 0..=MAX_DEPTH {
        document = doc! { "n": document };
    }
    let error = super::document(&raw(&document), Mode::Canonical).expect_err("too deep");
    assert_eq!(error, EjsonError::TooDeep);
}

#[test]
fn iso_dates_follow_the_proleptic_calendar() {
    assert_eq!(iso_date(0).as_deref(), Some("1970-01-01T00:00:00.000Z"));
    assert_eq!(
        iso_date(951_782_400_000).as_deref(),
        Some("2000-02-29T00:00:00.000Z")
    );
    assert_eq!(
        iso_date(253_402_300_799_999).as_deref(),
        Some("9999-12-31T23:59:59.999Z")
    );
    assert_eq!(iso_date(253_402_300_800_000), None, "year 10000");
    assert_eq!(iso_date(-1), None, "before 1970");
    assert_eq!(iso_date(i64::MAX), None);
}
