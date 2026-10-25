//! A reply element as a cell: its RESP type, its text, its bytes.
//!
//! # Text when it is text, bytes when it is not
//!
//! Redis stores bytes; most of them are UTF-8 text, some are protobuf, msgpack
//! or compressed blobs. A value column is therefore two: `value`, the text,
//! **verbatim**, when the bytes are UTF-8 — a JSON document copied from the
//! grid into a `SET` writes back the same bytes —, and `value_bytes`, the raw
//! bytes, when they are not. Exactly one of the two holds a value; both are
//! null for a RESP null. Nothing is escaped, nothing is guessed, nothing is
//! lost.
//!
//! # A number is not a float
//!
//! An integer reply is a signed 64-bit integer, written in decimal: beyond
//! 2^53 it survives, where a trip through a double would not. A big number is
//! its digits. A double is the shortest text that reads back as the same
//! `f64`, `inf`, `-inf` and `nan` included.
//!
//! # A nested aggregate is shown, not flattened
//!
//! An element that is itself an array or a map is written in one cell, in a
//! compact form close to JSON — strings always quoted, so `1` and `"1"` stay
//! apart —, with what the budget discarded marked in place.

use std::fmt::Write as _;

use crate::resp::{Aggregate, Clipped, Value};

/// One cell, before it joins a column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The RESP type: `bulk`, `integer`, `map`…
    pub kind: &'static str,
    /// The value as text, when it is text.
    pub text: Option<String>,
    /// The value's bytes, when they are not UTF-8.
    pub bytes: Option<Vec<u8>>,
    /// Was part of the value discarded?
    pub truncated: bool,
}

impl Cell {
    /// The cell of one reply element.
    #[must_use]
    pub fn of(value: &Value) -> Self {
        let text = |kind, text: String| Self {
            kind,
            text: Some(text),
            bytes: None,
            truncated: false,
        };
        match value {
            Value::Null => Self {
                kind: "null",
                text: None,
                bytes: None,
                truncated: false,
            },
            Value::Simple(bytes) => Self::of_bytes(
                "simple",
                &Clipped {
                    bytes: bytes.clone(),
                    full_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                },
            ),
            Value::Bulk(clipped) => Self::of_bytes("bulk", clipped),
            Value::Verbatim { text, .. } => Self::of_bytes("verbatim", text),
            Value::Integer(n) => text("integer", n.to_string()),
            Value::Double(x) => text("double", double(*x)),
            Value::Boolean(b) => text("boolean", b.to_string()),
            Value::BigNumber(digits) => text("big_number", digits.clone()),
            Value::Error(bytes) => text("error", String::from_utf8_lossy(bytes).into_owned()),
            Value::List(kind, _) | Value::Elided(kind, _) => Self {
                kind: kind.name(),
                text: Some(compact(value)),
                bytes: None,
                truncated: value.is_truncated(),
            },
            Value::Map(_) => Self {
                kind: Aggregate::Map.name(),
                text: Some(compact(value)),
                bytes: None,
                truncated: value.is_truncated(),
            },
        }
    }

    /// The cell of a string: text when UTF-8, bytes otherwise.
    ///
    /// A value cut in the middle of a UTF-8 character keeps its whole
    /// characters as text: the cut is the budget's, not the data's.
    #[must_use]
    pub fn of_bytes(kind: &'static str, clipped: &Clipped) -> Self {
        let truncated = clipped.is_truncated();
        match std::str::from_utf8(&clipped.bytes) {
            Ok(text) => Self {
                kind,
                text: Some(text.to_owned()),
                bytes: None,
                truncated,
            },
            Err(error) if truncated && error.error_len().is_none() => Self {
                kind,
                text: clipped
                    .bytes
                    .get(..error.valid_up_to())
                    .map(|valid| String::from_utf8_lossy(valid).into_owned()),
                bytes: None,
                truncated,
            },
            Err(_) => Self {
                kind,
                text: None,
                bytes: Some(clipped.bytes.clone()),
                truncated,
            },
        }
    }

    /// The bytes this cell adds to a batch, for the batch's bound.
    #[must_use]
    pub fn size(&self) -> usize {
        self.text.as_ref().map_or(0, String::len) + self.bytes.as_ref().map_or(0, Vec::len) + 16
    }
}

/// A double as the shortest text that reads back as it.
#[must_use]
pub fn double(x: f64) -> String {
    if x.is_nan() {
        "nan".to_owned()
    } else if x.is_infinite() {
        if x > 0.0 { "inf" } else { "-inf" }.to_owned()
    } else {
        format!("{x:?}")
    }
}

/// A value in one line, close to JSON.
#[must_use]
pub fn compact(value: &Value) -> String {
    let mut out = String::new();
    write_compact(&mut out, value);
    out
}

fn write_compact(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Simple(bytes) => quoted(out, bytes, None),
        Value::Bulk(clipped) | Value::Verbatim { text: clipped, .. } => {
            quoted(
                out,
                &clipped.bytes,
                clipped.is_truncated().then_some(clipped.full_len),
            );
        }
        Value::Integer(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Double(x) => out.push_str(&double(*x)),
        Value::Boolean(b) => {
            let _ = write!(out, "{b}");
        }
        Value::BigNumber(digits) => out.push_str(digits),
        Value::Error(bytes) => {
            out.push_str("(error ");
            quoted(out, bytes, None);
            out.push(')');
        }
        Value::List(_, items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_compact(out, item);
            }
            out.push(']');
        }
        Value::Map(pairs) => {
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_compact(out, key);
                if !matches!(key, Value::Elided(..)) {
                    out.push_str(": ");
                    write_compact(out, item);
                }
            }
            out.push('}');
        }
        Value::Elided(kind, len) => {
            let _ = write!(out, "…{len} {} entries not kept", kind.name());
        }
    }
}

/// A string in double quotes, with JSON's escapes and `\xHH` for a byte that
/// is not UTF-8. `full_len` marks a string that was cut.
fn quoted(out: &mut String, bytes: &[u8], full_len: Option<u64>) {
    out.push('"');
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => {
                    let _ = write!(out, "\\u{:04x}", u32::from(c));
                }
                c => out.push(c),
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(out, "\\x{byte:02x}");
        }
    }
    out.push('"');
    if let Some(full) = full_len {
        let _ = write!(out, "…({full} bytes)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bulk(bytes: &[u8]) -> Value {
        Value::Bulk(Clipped {
            bytes: bytes.to_vec(),
            full_len: u64::try_from(bytes.len()).expect("small"),
        })
    }

    #[test]
    fn utf8_text_is_kept_verbatim_and_bytes_stay_bytes() {
        let json = br#"{"a":"x\ny"}"#;
        let cell = Cell::of(&bulk(json));
        assert_eq!(cell.text.as_deref(), Some(r#"{"a":"x\ny"}"#));
        assert_eq!(cell.bytes, None);

        let blob = Cell::of(&bulk(b"\x00\xff\xfe"));
        assert_eq!(blob.text, None);
        assert_eq!(blob.bytes.as_deref(), Some(b"\x00\xff\xfe".as_slice()));
    }

    #[test]
    fn an_integer_beyond_2_53_survives() {
        let cell = Cell::of(&Value::Integer(i64::MAX));
        assert_eq!(cell.text.as_deref(), Some("9223372036854775807"));
        assert_eq!(cell.kind, "integer");
        let big = Cell::of(&Value::BigNumber(
            "123456789012345678901234567890".to_owned(),
        ));
        assert_eq!(big.text.as_deref(), Some("123456789012345678901234567890"));
    }

    #[test]
    fn a_double_reads_back_as_itself() {
        for x in [0.1, 1e300, -2.5e-310, 3.0, f64::MAX] {
            assert_eq!(double(x).parse::<f64>().expect("a number"), x);
        }
        assert_eq!(double(f64::INFINITY), "inf");
        assert_eq!(double(f64::NEG_INFINITY), "-inf");
        assert_eq!(double(f64::NAN), "nan");
    }

    #[test]
    fn a_value_cut_inside_a_character_keeps_its_whole_characters() {
        let clipped = Clipped {
            bytes: "café".as_bytes()[..4].to_vec(),
            full_len: 100,
        };
        let cell = Cell::of_bytes("bulk", &clipped);
        assert_eq!(cell.text.as_deref(), Some("caf"));
        assert!(cell.truncated);
    }

    #[test]
    fn a_nested_value_is_written_on_one_line_with_quoted_strings() {
        let value = Value::List(
            Aggregate::Array,
            vec![
                bulk(b"1"),
                Value::Integer(1),
                Value::Map(vec![(bulk(b"k\n"), Value::Null)]),
                bulk(b"\xff"),
                Value::Error(b"ERR boom".to_vec()),
                Value::Elided(Aggregate::Set, 9),
            ],
        );
        let cell = Cell::of(&value);
        assert_eq!(
            cell.text.as_deref(),
            Some(r#"["1", 1, {"k\n": null}, "\xff", (error "ERR boom"), …9 set entries not kept]"#)
        );
        assert!(cell.truncated);
        assert_eq!(cell.kind, "array");
    }
}
