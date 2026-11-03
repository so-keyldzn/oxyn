//! Scalar values of the domain.
//!
//! [`ScalarValue`] is **not** the result model: results are
//! `arrow::RecordBatch` (ADR-0002) and never go through this type. `ScalarValue`
//! serves isolated values: bound parameters of a query, an edited cell, a value
//! shown in an inspector.
//!
//! Time handling follows
//! [`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md): an instant with a
//! time zone travels in UTC ([`Timestamp`](ScalarValue::Timestamp)), a
//! timestamp without a time zone travels **without inventing one**
//! ([`TimestampNaive`](ScalarValue::TimestampNaive)). The two are never
//! confused: it is this confusion that shifts a value by two hours in the
//! database, invisibly and permanently.
//!
//! Decimals travel as text ([`Decimal`](ScalarValue::Decimal)): no floating
//! type represents `0.1` exactly, and a monetary value rounded along the way is
//! a silent corruption.

use std::fmt;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A scalar value, as it crosses a driver's boundary.
///
/// The enumeration is **closed**, unlike the repository's convention on public
/// enumerations. The reason is the driver contract: each driver keeps a type
/// mapping table **in both directions**, and that table is a `match`. Adding a
/// scalar type must break the compilation of every driver, so that the
/// question "and this one, how do I render it?" gets asked — rather than being
/// absorbed by a `_ =>` that would produce a silently wrong conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ScalarValue {
    /// No value. Always distinct from the empty string and from zero.
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    Int64(i64),
    /// Double-precision float.
    Float64(f64),
    /// Exact decimal, kept as text so that nothing is lost.
    Decimal(String),
    /// Text. The encoding is already validated: what arrives from the server
    /// and is not valid UTF-8 becomes [`Bytes`](Self::Bytes), not a mangled text.
    Text(String),
    /// Opaque byte sequence.
    Bytes(Vec<u8>),
    /// UUID.
    Uuid(Uuid),
    /// Date without time or time zone.
    Date(NaiveDate),
    /// Time without date or time zone.
    Time(NaiveTime),
    /// Absolute instant, carried in UTC.
    Timestamp(DateTime<Utc>),
    /// Timestamp without a time zone. None is assigned to it on read.
    TimestampNaive(NaiveDateTime),
    /// Interval, broken down as PostgreSQL does: months have no fixed
    /// duration, and neither do days as soon as there is a clock change.
    /// Flattening them into a single duration would be wrong.
    Interval {
        /// Number of months.
        months: i32,
        /// Number of days.
        days: i32,
        /// Remainder, in nanoseconds.
        nanos: i64,
    },
    /// JSON document.
    Json(serde_json::Value),
    /// Array, homogeneous or not, depending on what the source accepts.
    Array(Vec<ScalarValue>),
}

impl ScalarValue {
    /// Maximum number of bytes rendered by [`fmt::Display`] for
    /// [`Bytes`](Self::Bytes) before truncation.
    const PREVIEW_BYTES: usize = 32;

    /// Stable name of the type, usable in a message or a type mapping table.
    ///
    /// These names are part of the API: they appear in conversion error
    /// messages and in the drivers' mapping tables.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int64(_) => "int64",
            Self::Float64(_) => "float64",
            Self::Decimal(_) => "decimal",
            Self::Text(_) => "text",
            Self::Bytes(_) => "bytes",
            Self::Uuid(_) => "uuid",
            Self::Date(_) => "date",
            Self::Time(_) => "time",
            Self::Timestamp(_) => "timestamptz",
            Self::TimestampNaive(_) => "timestamp",
            Self::Interval { .. } => "interval",
            Self::Json(_) => "json",
            Self::Array(_) => "array",
        }
    }

    /// Is the value absent?
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl fmt::Display for ScalarValue {
    /// Readable rendering, meant for display and tests.
    ///
    /// It is **not** a SQL literal: `Text` is not put in quotes and nothing is
    /// escaped. Composing SQL from this rendering would be exactly the mistake
    /// that [`DRIVER-CONTRACT` §6](../../../docs/DRIVER-CONTRACT.md) forbids;
    /// values are **bound**, they are not concatenated.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("NULL"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int64(i) => write!(f, "{i}"),
            Self::Float64(x) => write!(f, "{x}"),
            Self::Decimal(d) => f.write_str(d),
            Self::Text(t) => f.write_str(t),
            Self::Bytes(b) => {
                f.write_str("\\x")?;
                for byte in b.iter().take(Self::PREVIEW_BYTES) {
                    write!(f, "{byte:02x}")?;
                }
                if b.len() > Self::PREVIEW_BYTES {
                    write!(f, "… ({} bytes)", b.len())?;
                }
                Ok(())
            }
            Self::Uuid(u) => write!(f, "{u}"),
            Self::Date(d) => write!(f, "{d}"),
            Self::Time(t) => write!(f, "{t}"),
            // RFC 3339 in UTC: never converted to the machine's time zone.
            Self::Timestamp(ts) => write!(f, "{}", ts.to_rfc3339()),
            Self::TimestampNaive(ts) => write!(f, "{}", ts.format("%Y-%m-%dT%H:%M:%S%.f")),
            Self::Interval {
                months,
                days,
                nanos,
            } => {
                // ISO 8601 form: the three components stay distinct.
                let seconds = nanos / 1_000_000_000;
                let remainder = (nanos % 1_000_000_000).unsigned_abs();
                write!(f, "P{months}M{days}DT{seconds}.{remainder:09}S")
            }
            Self::Json(v) => write!(f, "{v}"),
            Self::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
        }
    }
}

impl From<bool> for ScalarValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for ScalarValue {
    fn from(v: i64) -> Self {
        Self::Int64(v)
    }
}

impl From<f64> for ScalarValue {
    fn from(v: f64) -> Self {
        Self::Float64(v)
    }
}

impl From<String> for ScalarValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&str> for ScalarValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}

impl From<Uuid> for ScalarValue {
    fn from(v: Uuid) -> Self {
        Self::Uuid(v)
    }
}

impl<T> From<Option<T>> for ScalarValue
where
    T: Into<ScalarValue>,
{
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

/// Scalar type a user can pick and type text for — the bound-parameter editor
/// (the front's parameter editor) and anything else that turns typed text
/// into a [`ScalarValue`].
///
/// This is a strict subset of the type space [`ScalarValue`] can hold.
/// [`ScalarValue::Interval`] and [`ScalarValue::Array`] have no unambiguous
/// textual form a user could type in a single field — an interval mixes
/// months, days and nanoseconds with no canonical separator, and an array
/// needs its own per-element type and a nesting syntax — so they are not
/// offered here at all, rather than accepted and silently misparsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParameterType {
    Null,
    Bool,
    Int64,
    Float64,
    Decimal,
    Text,
    Bytes,
    Uuid,
    Date,
    Time,
    Timestamp,
    TimestampNaive,
    Json,
}

impl ParameterType {
    /// Every variant, in the order a picker should list them. The single
    /// source of that order and of the variant list itself: duplicating it at
    /// call sites is exactly how a selector ends up offering a type its
    /// editor then refuses.
    pub const ALL: [Self; 13] = [
        Self::Null,
        Self::Bool,
        Self::Int64,
        Self::Float64,
        Self::Decimal,
        Self::Text,
        Self::Bytes,
        Self::Uuid,
        Self::Date,
        Self::Time,
        Self::Timestamp,
        Self::TimestampNaive,
        Self::Json,
    ];

    /// Human-readable name, used in pickers and in error messages.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Null => "NULL",
            Self::Bool => "Boolean",
            Self::Int64 => "Int64",
            Self::Float64 => "Float64",
            Self::Decimal => "Decimal",
            Self::Text => "Text",
            Self::Bytes => "Bytes (hex)",
            Self::Uuid => "UUID",
            Self::Date => "Date",
            Self::Time => "Time",
            Self::Timestamp => "Timestamp with timezone",
            Self::TimestampNaive => "Timestamp without timezone",
            Self::Json => "JSON",
        }
    }

    /// Parses `text` as a value of this type.
    ///
    /// The formats accepted for the date/time variants are documented on each
    /// variant of this enum, and each is pinned down by a test in this module
    /// that exercises `chrono`'s `FromStr` directly — they describe what the
    /// dependency actually does, not a recollection of it (invariant I-12).
    ///
    /// # Errors
    ///
    /// Returns [`ParameterParseError`] if `text` does not match the format
    /// for `self`. The error never contains `text`: see
    /// [`ParameterParseError`] for why.
    pub fn parse(self, text: &str) -> Result<ScalarValue, ParameterParseError> {
        match self {
            Self::Null => Ok(ScalarValue::Null),
            Self::Bool => text
                .parse()
                .map(ScalarValue::Bool)
                .map_err(|_| ParameterParseError),
            Self::Int64 => text
                .parse()
                .map(ScalarValue::Int64)
                .map_err(|_| ParameterParseError),
            Self::Float64 => text
                .parse()
                .map(ScalarValue::Float64)
                .map_err(|_| ParameterParseError),
            Self::Decimal => is_decimal_literal(text)
                .then(|| ScalarValue::Decimal(text.to_owned()))
                .ok_or(ParameterParseError),
            Self::Text => Ok(ScalarValue::Text(text.to_owned())),
            Self::Bytes => parse_hex(text).map(ScalarValue::Bytes),
            Self::Uuid => text
                .parse::<Uuid>()
                .map(ScalarValue::Uuid)
                .map_err(|_| ParameterParseError),
            // Accepts the calendar-date form `YYYY-MM-DD` (e.g. `2024-01-15`);
            // month and day do not need zero-padding. Anything with a time
            // component, or a non-ISO order such as `DD/MM/YYYY`, is
            // rejected. Proven by `date_accepts_the_iso_calendar_form` below.
            Self::Date => text
                .parse::<NaiveDate>()
                .map(ScalarValue::Date)
                .map_err(|_| ParameterParseError),
            // Accepts `HH:MM:SS`, `HH:MM:SS.fraction`, and also `HH:MM`
            // (seconds default to zero); the hour does not need zero-padding.
            // Proven by `time_accepts_missing_seconds_and_fraction` below.
            Self::Time => text
                .parse::<NaiveTime>()
                .map(ScalarValue::Time)
                .map_err(|_| ParameterParseError),
            // Accepts `YYYY-MM-DDTHH:MM:SS` and its fractional-second form.
            // Only the `T` separator works: the SQL-style space
            // (`YYYY-MM-DD HH:MM:SS`) is rejected, and so is anything
            // carrying a timezone offset — an offset makes it a [`Timestamp`]
            // ([`Self::Timestamp`]), not a naive one. Proven by
            // `timestamp_naive_requires_the_t_separator_and_no_offset` below.
            Self::TimestampNaive => text
                .parse::<NaiveDateTime>()
                .map(ScalarValue::TimestampNaive)
                .map_err(|_| ParameterParseError),
            // Accepts RFC 3339: a `T` or space date/time separator, then an
            // explicit UTC offset (`Z`, or `+HH:MM`/`-HH:MM`), with an
            // optional fractional second. A timestamp with no offset is
            // rejected rather than assumed to be UTC — guessing a fuzzy
            // instant into an exact one would silently shift it whenever the
            // guess is wrong. Proven by
            // `timestamp_requires_an_explicit_offset` below.
            Self::Timestamp => text
                .parse::<DateTime<Utc>>()
                .map(ScalarValue::Timestamp)
                .map_err(|_| ParameterParseError),
            Self::Json => serde_json::from_str::<serde_json::Value>(text)
                .map(ScalarValue::Json)
                .map_err(|_| ParameterParseError),
        }
    }
}

/// Why [`ParameterType::parse`] rejected a value.
///
/// Carries nothing: structurally, there is no field to hold the rejected
/// text, so neither `Display` nor the derived `Debug` can leak it into a log,
/// an error banner, or a crash report (invariant I-03) — a bound parameter is
/// exactly the kind of value that can turn out to be a password pasted into
/// the wrong field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("value does not match the expected parameter type")]
pub struct ParameterParseError;

/// Checks that `text` is a plain decimal literal: an optional sign, at least
/// one digit, and at most one decimal point among the digits.
///
/// Deliberately excludes scientific notation (`1e400`) rather than routing
/// through `f64` to validate the shape: `f64::parse` accepts `1e400` and
/// silently turns it into infinity, and rejects nothing about the *shape* of
/// a long literal — the two failures this function exists to avoid. The
/// digits are kept exactly as typed, so a 40-digit literal loses nothing.
fn is_decimal_literal(text: &str) -> bool {
    let mut chars = text.chars();
    let first = chars.clone().next();
    if matches!(first, Some('+' | '-')) {
        chars.next();
    }
    let mut has_digit = false;
    let mut has_dot = false;
    for ch in chars {
        match ch {
            '0'..='9' => has_digit = true,
            '.' if !has_dot => has_dot = true,
            _ => return false,
        }
    }
    has_digit
}

/// Decodes a hex string (as `Bytes` parameters are typed) without indexing a
/// slice by a byte offset derived from user input: `str::get` on a
/// non-boundary index returns `None` rather than panicking.
fn parse_hex(text: &str) -> Result<Vec<u8>, ParameterParseError> {
    if !text.len().is_multiple_of(2) {
        return Err(ParameterParseError);
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(text.get(index..index + 2).unwrap_or_default(), 16)
                .map_err(|_| ParameterParseError)
        })
        .collect()
}

#[cfg(test)]
mod tests;
