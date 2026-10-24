//! BSON to Extended JSON text, keys in the order the server sent them.
//!
//! The grid shows a document as text, and that text must say what the server
//! holds: an `Int64` is not an `Int32`, a `Decimal128` is not a double, a field
//! order is data (an index key, a sort specification). `bson`'s own conversion
//! goes through `serde_json::Value`, whose map sorts its keys unless a feature
//! the workspace does not enable is on; this writer reads the raw bytes and
//! keeps their order.
//!
//! Two modes, from the MongoDB Extended JSON v2 specification (RESEARCH-NOTES,
//! 2026-10-22):
//!
//! * **canonical** keeps every type: `{"$numberInt": "1"}`, `{"$numberLong":
//!   "1"}`. It is the `$document` column, the one that loses nothing, and what
//!   a console accepts back;
//! * **relaxed** writes numbers and dates as JSON numbers and ISO strings. It
//!   is the view of a nested or mixed field, where an `Int32` and an `Int64`
//!   both read `1`: the type lives in the canonical column.
//!
//! # What this module refuses
//!
//! A document nested deeper than [`MAX_DEPTH`] — a server stores at most 100
//! levels, a hostile one can send more, and recursion has a stack — and any
//! element `bson` cannot frame: an unknown type byte (BSON gives an unknown
//! element no length, so nothing after it can be read), a string that is not
//! UTF-8, a truncated length. The error names what was refused; nothing is
//! replaced silently.

use std::fmt::Write as _;

use bson::raw::{RawArray, RawBsonRef, RawDocument};
use bson::spec::BinarySubtype;

/// Nesting beyond which a document is refused rather than recursed into.
pub(crate) const MAX_DEPTH: usize = 200;

/// The two renderings of the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Every type wrapped: lossless.
    Canonical,
    /// Numbers and dates as plain JSON where the specification allows it.
    Relaxed,
}

/// Why a value could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EjsonError {
    /// `bson` refused to frame or decode an element.
    #[error("the server sent BSON that cannot be read: {0}")]
    Malformed(String),
    /// Deeper than [`MAX_DEPTH`].
    #[error("a document is nested more than {MAX_DEPTH} levels deep")]
    TooDeep,
}

impl From<bson::error::Error> for EjsonError {
    fn from(error: bson::error::Error) -> Self {
        Self::Malformed(error.to_string())
    }
}

/// A whole document.
pub(crate) fn document(doc: &RawDocument, mode: Mode) -> Result<String, EjsonError> {
    let mut out = String::with_capacity(doc.as_bytes().len().saturating_mul(2));
    write_document(&mut out, doc, mode, 0)?;
    Ok(out)
}

/// One value, as a field cell shows it.
pub(crate) fn value(value: RawBsonRef<'_>, mode: Mode) -> Result<String, EjsonError> {
    let mut out = String::new();
    write_value(&mut out, value, mode, 0)?;
    Ok(out)
}

fn write_document(
    out: &mut String,
    doc: &RawDocument,
    mode: Mode,
    depth: usize,
) -> Result<(), EjsonError> {
    if depth > MAX_DEPTH {
        return Err(EjsonError::TooDeep);
    }
    out.push('{');
    for (index, element) in doc.iter_elements().enumerate() {
        let element = element?;
        if index > 0 {
            out.push(',');
        }
        write_string(out, element.key().as_str());
        out.push(':');
        write_value(out, element.value()?, mode, depth.saturating_add(1))?;
    }
    out.push('}');
    Ok(())
}

fn write_array(
    out: &mut String,
    array: &RawArray,
    mode: Mode,
    depth: usize,
) -> Result<(), EjsonError> {
    if depth > MAX_DEPTH {
        return Err(EjsonError::TooDeep);
    }
    out.push('[');
    for (index, item) in array.into_iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_value(out, item?, mode, depth.saturating_add(1))?;
    }
    out.push(']');
    Ok(())
}

fn write_value(
    out: &mut String,
    value: RawBsonRef<'_>,
    mode: Mode,
    depth: usize,
) -> Result<(), EjsonError> {
    let relaxed = mode == Mode::Relaxed;
    match value {
        RawBsonRef::Double(number) => write_double(out, number, relaxed),
        RawBsonRef::String(text) => write_string(out, text),
        RawBsonRef::Array(array) => write_array(out, array, mode, depth)?,
        RawBsonRef::Document(doc) => write_document(out, doc, mode, depth)?,
        RawBsonRef::Boolean(flag) => out.push_str(if flag { "true" } else { "false" }),
        RawBsonRef::Null => out.push_str("null"),
        RawBsonRef::RegularExpression(regex) => {
            out.push_str("{\"$regularExpression\":{\"pattern\":");
            write_string(out, regex.pattern.as_str());
            out.push_str(",\"options\":");
            write_string(out, regex.options.as_str());
            out.push_str("}}");
        }
        RawBsonRef::JavaScriptCode(code) => {
            out.push_str("{\"$code\":");
            write_string(out, code);
            out.push('}');
        }
        RawBsonRef::JavaScriptCodeWithScope(code) => {
            out.push_str("{\"$code\":");
            write_string(out, code.code);
            out.push_str(",\"$scope\":");
            write_document(out, code.scope, mode, depth.saturating_add(1))?;
            out.push('}');
        }
        RawBsonRef::Int32(number) => {
            if relaxed {
                let _ = write!(out, "{number}");
            } else {
                let _ = write!(out, "{{\"$numberInt\":\"{number}\"}}");
            }
        }
        RawBsonRef::Int64(number) => {
            if relaxed {
                let _ = write!(out, "{number}");
            } else {
                let _ = write!(out, "{{\"$numberLong\":\"{number}\"}}");
            }
        }
        RawBsonRef::Timestamp(stamp) => {
            let _ = write!(
                out,
                "{{\"$timestamp\":{{\"t\":{},\"i\":{}}}}}",
                stamp.time, stamp.increment
            );
        }
        RawBsonRef::Binary(binary) => {
            let subtype: u8 = binary.subtype.into();
            out.push_str("{\"$binary\":{\"base64\":\"");
            base64_into(out, binary.bytes);
            let _ = write!(out, "\",\"subType\":\"{subtype:02x}\"}}}}");
        }
        RawBsonRef::ObjectId(id) => {
            let _ = write!(out, "{{\"$oid\":\"{}\"}}", id.to_hex());
        }
        RawBsonRef::DateTime(date) => {
            let millis = date.timestamp_millis();
            match iso_date(millis) {
                Some(iso) if relaxed => {
                    let _ = write!(out, "{{\"$date\":\"{iso}\"}}");
                }
                _ => {
                    let _ = write!(out, "{{\"$date\":{{\"$numberLong\":\"{millis}\"}}}}");
                }
            }
        }
        RawBsonRef::Symbol(symbol) => {
            out.push_str("{\"$symbol\":");
            write_string(out, symbol);
            out.push('}');
        }
        RawBsonRef::Decimal128(decimal) => {
            let _ = write!(out, "{{\"$numberDecimal\":\"{decimal}\"}}");
        }
        RawBsonRef::Undefined => out.push_str("{\"$undefined\":true}"),
        RawBsonRef::MaxKey => out.push_str("{\"$maxKey\":1}"),
        RawBsonRef::MinKey => out.push_str("{\"$minKey\":1}"),
        RawBsonRef::DbPointer(_) => {
            // `bson` keeps the pointer's namespace and id private; its serde
            // form is the specification's `{"$dbPointer": {"$ref", "$id"}}`.
            let text = serde_json::to_string(&value)
                .map_err(|error| EjsonError::Malformed(error.to_string()))?;
            out.push_str(&text);
        }
    }
    Ok(())
}

/// A double as the specification spells it: `Infinity`, `-Infinity` and `NaN`
/// are always wrapped, since JSON has no literal for them.
fn write_double(out: &mut String, number: f64, relaxed: bool) {
    if !number.is_finite() {
        let text = if number.is_nan() {
            "NaN"
        } else if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        };
        let _ = write!(out, "{{\"$numberDouble\":\"{text}\"}}");
        return;
    }
    // Rust's `Display` is the shortest text that reads back to the same
    // double; a missing point would make the relaxed form an integer.
    let mut text = format!("{number}");
    if !text.contains(['.', 'e', 'E']) {
        text.push_str(".0");
    }
    if relaxed {
        out.push_str(&text);
    } else {
        let _ = write!(out, "{{\"$numberDouble\":\"{text}\"}}");
    }
}

/// A JSON string literal: quote, backslash and control characters escaped.
pub(crate) fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Standard base64 with padding, as `$binary.base64` requires.
fn base64_into(out: &mut String, bytes: &[u8]) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let symbol = |index: u32| {
        ALPHABET
            .get(usize::try_from(index & 0x3f).unwrap_or(0))
            .map_or('=', |&b| char::from(b))
    };
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().map_or(0, u32::from);
        let second = chunk.get(1).copied().map_or(0, u32::from);
        let third = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (first << 16) | (second << 8) | third;
        out.push(symbol(triple >> 18));
        out.push(symbol(triple >> 12));
        out.push(if chunk.len() > 1 {
            symbol(triple >> 6)
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { symbol(triple) } else { '=' });
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for the years the relaxed form allows,
/// 1970 to 9999; `None` outside, where the canonical number is written.
///
/// Computed from the milliseconds with the proleptic Gregorian calendar (the
/// days-from-civil inverse), not through a time library: BSON dates are UTC by
/// definition and no time zone is ever involved.
pub(crate) fn iso_date(millis: i64) -> Option<String> {
    if millis < 0 {
        return None;
    }
    let days = millis.div_euclid(86_400_000);
    let in_day = millis.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    if year > 9999 {
        return None;
    }
    let hours = in_day / 3_600_000;
    let minutes = in_day / 60_000 % 60;
    let seconds = in_day / 1000 % 60;
    let fraction = in_day % 1000;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{fraction:03}Z"
    ))
}

/// Year, month and day of a count of days since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, valid over the whole `i64` range this module passes).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days.saturating_add(719_468);
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The specification's name of a binary subtype, for a column's metadata.
pub(crate) fn subtype_code(subtype: BinarySubtype) -> u8 {
    subtype.into()
}

#[cfg(test)]
mod tests;
