//! The server's values to `RecordBatch`, without a single panic.
//!
//! Every value is server input ([I-09](../../../CLAUDE.md#i-09)): a `NULL`
//! where the schema forbids it, an integer out of the announced range, text
//! that is not UTF-8, a date with month zero. Each becomes a
//! [`DecodeError`] naming the column — never a panic, never an invented value.
//!
//! The same assembler serves both protocols. The binary protocol gives typed
//! values (`Value::Int`, `Value::Date`…); the text protocol, used only on the
//! 1295 fallback (ADR-0050 §3), gives every value as the server's text. Each
//! decoding accepts both forms.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, Date32Builder, Decimal128Builder, Decimal256Builder,
    DurationMicrosecondBuilder, Float32Builder, Float64Builder, Int8Builder, Int16Builder,
    Int32Builder, Int64Builder, StringBuilder, TimestampMicrosecondBuilder, UInt8Builder,
    UInt16Builder, UInt32Builder, UInt64Builder,
};
use arrow::datatypes::{SchemaRef, i256};
use arrow::record_batch::RecordBatch;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use mysql_async::Value;

use crate::types::MyDecoding;

/// A value the driver could not turn into its column's Arrow type.
///
/// Names the column and the reason, **never the value**: it is the user's
/// data, and this message is displayed, logged and persisted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("column `{column}`: {reason}")]
pub struct DecodeError {
    column: String,
    reason: String,
}

impl DecodeError {
    fn new(column: &str, reason: impl Into<String>) -> Self {
        Self {
            column: column.to_owned(),
            reason: reason.into(),
        }
    }

    /// The column concerned.
    #[must_use]
    pub fn column(&self) -> &str {
        &self.column
    }
}

/// Why a value failed, before the column name is attached.
type Refusal = &'static str;

/// Year, month, day, hour, minute, second, microsecond: the binary protocol's
/// `Value::Date`, which the text protocol's dates are parsed into.
type DateParts = (u16, u8, u8, u8, u8, u8, u32);

const NOT_AN_INTEGER: Refusal = "the server sent a value that is not an integer of this column's \
                                 width";
const NOT_A_FLOAT: Refusal = "the server sent a value that is not a floating-point number";
const NOT_A_DECIMAL: Refusal = "the server sent a value that does not fit this column's \
                                declared precision and scale";
const NOT_UTF8: Refusal = "the server sent text that is not valid UTF-8; read the column with \
                           CAST(… AS BINARY)";
const NOT_BYTES: Refusal = "the server sent a value that contradicts the column's announced \
                            type";
const NOT_A_BIT: Refusal = "the server sent more than 64 bits for a BIT column";
const NOT_A_TIME: Refusal = "the server sent a TIME the driver cannot read";
const ZERO_DATE: Refusal = "a zero or partial date (month or day 0, or a day the calendar does \
                            not have) fits no date type; read the column with CAST(… AS CHAR)";
const NOT_A_DATE: Refusal = "the server sent a date the driver cannot read";

/// One column under construction.
enum ColumnBuilder {
    Int8(Int8Builder),
    Int16(Int16Builder),
    Int32(Int32Builder),
    Int64(Int64Builder),
    UInt8(UInt8Builder),
    UInt16(UInt16Builder),
    UInt32(UInt32Builder),
    UInt64(UInt64Builder),
    Bit(UInt64Builder),
    Float32(Float32Builder),
    Float64(Float64Builder),
    Decimal128(Decimal128Builder, i8),
    Decimal256(Decimal256Builder, i8),
    Date(Date32Builder),
    DateTime(TimestampMicrosecondBuilder),
    Timestamp(TimestampMicrosecondBuilder),
    Time(DurationMicrosecondBuilder),
    Text(StringBuilder),
    Binary(BinaryBuilder),
}

impl ColumnBuilder {
    fn new(decoding: MyDecoding) -> Self {
        match decoding {
            MyDecoding::Int8 => Self::Int8(Int8Builder::new()),
            MyDecoding::Int16 => Self::Int16(Int16Builder::new()),
            MyDecoding::Int32 => Self::Int32(Int32Builder::new()),
            MyDecoding::Int64 => Self::Int64(Int64Builder::new()),
            MyDecoding::UInt8 => Self::UInt8(UInt8Builder::new()),
            MyDecoding::UInt16 => Self::UInt16(UInt16Builder::new()),
            MyDecoding::UInt32 => Self::UInt32(UInt32Builder::new()),
            MyDecoding::UInt64 => Self::UInt64(UInt64Builder::new()),
            MyDecoding::Bit => Self::Bit(UInt64Builder::new()),
            MyDecoding::Float32 => Self::Float32(Float32Builder::new()),
            MyDecoding::Float64 => Self::Float64(Float64Builder::new()),
            MyDecoding::Decimal128 { precision, scale } => Self::Decimal128(
                Decimal128Builder::new()
                    .with_precision_and_scale(precision, scale)
                    .unwrap_or_else(|_| Decimal128Builder::new()),
                scale,
            ),
            MyDecoding::Decimal256 { precision, scale } => Self::Decimal256(
                Decimal256Builder::new()
                    .with_precision_and_scale(precision, scale)
                    .unwrap_or_else(|_| Decimal256Builder::new()),
                scale,
            ),
            MyDecoding::Date => Self::Date(Date32Builder::new()),
            MyDecoding::DateTime => Self::DateTime(TimestampMicrosecondBuilder::new()),
            MyDecoding::Timestamp => {
                Self::Timestamp(TimestampMicrosecondBuilder::new().with_timezone("UTC"))
            }
            MyDecoding::Time => Self::Time(DurationMicrosecondBuilder::new()),
            MyDecoding::Text => Self::Text(StringBuilder::new()),
            MyDecoding::Binary => Self::Binary(BinaryBuilder::new()),
        }
    }

    fn append_null(&mut self) {
        match self {
            Self::Int8(b) => b.append_null(),
            Self::Int16(b) => b.append_null(),
            Self::Int32(b) => b.append_null(),
            Self::Int64(b) => b.append_null(),
            Self::UInt8(b) => b.append_null(),
            Self::UInt16(b) => b.append_null(),
            Self::UInt32(b) => b.append_null(),
            Self::UInt64(b) | Self::Bit(b) => b.append_null(),
            Self::Float32(b) => b.append_null(),
            Self::Float64(b) => b.append_null(),
            Self::Decimal128(b, _) => b.append_null(),
            Self::Decimal256(b, _) => b.append_null(),
            Self::Date(b) => b.append_null(),
            Self::DateTime(b) | Self::Timestamp(b) => b.append_null(),
            Self::Time(b) => b.append_null(),
            Self::Text(b) => b.append_null(),
            Self::Binary(b) => b.append_null(),
        }
    }

    /// Appends one non-null value and returns the bytes it adds.
    fn append(&mut self, value: Value) -> Result<usize, Refusal> {
        match self {
            Self::Int8(b) => b.append_value(integer(&value)?),
            Self::Int16(b) => b.append_value(integer(&value)?),
            Self::Int32(b) => b.append_value(integer(&value)?),
            Self::Int64(b) => b.append_value(integer(&value)?),
            Self::UInt8(b) => b.append_value(integer(&value)?),
            Self::UInt16(b) => b.append_value(integer(&value)?),
            Self::UInt32(b) => b.append_value(integer(&value)?),
            Self::UInt64(b) => b.append_value(integer(&value)?),
            Self::Bit(b) => b.append_value(bit(&value)?),
            Self::Float32(b) => b.append_value(float32(&value)?),
            Self::Float64(b) => b.append_value(float64(&value)?),
            Self::Decimal128(b, scale) => {
                let digits = decimal_digits(&value, *scale)?;
                b.append_value(digits.parse::<i128>().map_err(|_| NOT_A_DECIMAL)?);
            }
            Self::Decimal256(b, scale) => {
                let digits = decimal_digits(&value, *scale)?;
                b.append_value(i256::from_string(&digits).ok_or(NOT_A_DECIMAL)?);
            }
            Self::Date(b) => b.append_value(date32(&value)?),
            Self::DateTime(b) | Self::Timestamp(b) => b.append_value(micros_since_epoch(&value)?),
            Self::Time(b) => b.append_value(time_micros(&value)?),
            Self::Text(b) => {
                let bytes = into_bytes(value)?;
                let text = std::str::from_utf8(&bytes).map_err(|_| NOT_UTF8)?;
                b.append_value(text);
                return Ok(bytes.len().saturating_add(4));
            }
            Self::Binary(b) => {
                let bytes = into_bytes(value)?;
                b.append_value(&bytes);
                return Ok(bytes.len().saturating_add(4));
            }
        }
        Ok(self.width())
    }

    /// The fixed width of a value of this column, for the byte budget.
    const fn width(&self) -> usize {
        match self {
            Self::Int8(_) | Self::UInt8(_) => 1,
            Self::Int16(_) | Self::UInt16(_) => 2,
            Self::Int32(_) | Self::UInt32(_) | Self::Float32(_) | Self::Date(_) => 4,
            Self::Decimal128(..) => 16,
            Self::Decimal256(..) => 32,
            _ => 8,
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::Int8(b) => Arc::new(b.finish()),
            Self::Int16(b) => Arc::new(b.finish()),
            Self::Int32(b) => Arc::new(b.finish()),
            Self::Int64(b) => Arc::new(b.finish()),
            Self::UInt8(b) => Arc::new(b.finish()),
            Self::UInt16(b) => Arc::new(b.finish()),
            Self::UInt32(b) => Arc::new(b.finish()),
            Self::UInt64(b) | Self::Bit(b) => Arc::new(b.finish()),
            Self::Float32(b) => Arc::new(b.finish()),
            Self::Float64(b) => Arc::new(b.finish()),
            Self::Decimal128(b, _) => Arc::new(b.finish()),
            Self::Decimal256(b, _) => Arc::new(b.finish()),
            Self::Date(b) => Arc::new(b.finish()),
            Self::DateTime(b) | Self::Timestamp(b) => Arc::new(b.finish()),
            Self::Time(b) => Arc::new(b.finish()),
            Self::Text(b) => Arc::new(b.finish()),
            Self::Binary(b) => Arc::new(b.finish()),
        }
    }
}

/// Accumulates rows into one batch, counting its bytes.
pub struct BatchAssembler {
    schema: SchemaRef,
    columns: Vec<ColumnBuilder>,
    rows: usize,
    bytes: usize,
}

impl std::fmt::Debug for BatchAssembler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchAssembler")
            .field("columns", &self.columns.len())
            .field("rows", &self.rows)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl BatchAssembler {
    /// An empty assembler for a schema and its decoding plan, aligned.
    #[must_use]
    pub fn new(schema: SchemaRef, decodings: &[MyDecoding]) -> Self {
        Self {
            schema,
            columns: decodings.iter().copied().map(ColumnBuilder::new).collect(),
            rows: 0,
            bytes: 0,
        }
    }

    /// Appends one row, given as the server's values in column order.
    ///
    /// `None` is a value the library reports as already taken, which only a
    /// driver bug can produce: it is refused like a malformed value.
    ///
    /// # Errors
    /// [`DecodeError`] naming the first column whose value cannot be carried.
    /// The row is then **partly** appended: the assembler must be dropped, not
    /// finished — the stream stops on the first error.
    pub fn push(&mut self, values: Vec<Option<Value>>) -> Result<(), DecodeError> {
        if values.len() != self.columns.len() {
            return Err(DecodeError::new(
                "*",
                "the server sent a row whose width differs from its column definitions",
            ));
        }
        let fields = self.schema.fields();
        for (index, (column, value)) in self.columns.iter_mut().zip(values).enumerate() {
            let name = fields.get(index).map_or("?", |field| field.name().as_str());
            match value {
                None => return Err(DecodeError::new(name, "the value is missing from the row")),
                Some(Value::NULL) => column.append_null(),
                Some(value) => {
                    let added = column
                        .append(value)
                        .map_err(|reason| DecodeError::new(name, reason))?;
                    self.bytes = self.bytes.saturating_add(added);
                }
            }
        }
        self.rows = self.rows.saturating_add(1);
        Ok(())
    }

    /// Rows accumulated since the last batch.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Approximate bytes accumulated since the last batch.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Nothing accumulated?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Closes the batch and starts the next one.
    ///
    /// # Errors
    /// Arrow's, if the columns disagree with the schema — a driver bug.
    pub fn finish(&mut self) -> Result<RecordBatch, arrow::error::ArrowError> {
        let arrays: Vec<ArrayRef> = self.columns.iter_mut().map(ColumnBuilder::finish).collect();
        self.rows = 0;
        self.bytes = 0;
        RecordBatch::try_new(SchemaRef::clone(&self.schema), arrays)
    }
}

/// The bytes of a string or binary value.
///
/// Both protocols send such a column as `Value::Bytes`; any other variant can
/// only come from a server that contradicts its column definition. It is
/// refused rather than rendered: `Value::as_sql` would overflow on a hostile
/// `TIME`.
fn into_bytes(value: Value) -> Result<Vec<u8>, Refusal> {
    match value {
        Value::Bytes(bytes) => Ok(bytes),
        _ => Err(NOT_BYTES),
    }
}

/// The server's text of a value, for the text protocol.
fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Bytes(bytes) => std::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// An integer of the target width, from either protocol.
fn integer<T>(value: &Value) -> Result<T, Refusal>
where
    T: TryFrom<i64> + TryFrom<u64> + std::str::FromStr,
{
    match value {
        Value::Int(v) => T::try_from(*v).map_err(|_| NOT_AN_INTEGER),
        Value::UInt(v) => T::try_from(*v).map_err(|_| NOT_AN_INTEGER),
        other => text_of(other)
            .and_then(|text| text.trim().parse::<T>().ok())
            .ok_or(NOT_AN_INTEGER),
    }
}

/// `BIT(n)`: up to eight big-endian bytes, in both protocols.
fn bit(value: &Value) -> Result<u64, Refusal> {
    match value {
        Value::Bytes(bytes) => {
            if bytes.len() > 8 {
                return Err(NOT_A_BIT);
            }
            Ok(bytes
                .iter()
                .fold(0_u64, |acc, byte| (acc << 8) | u64::from(*byte)))
        }
        Value::UInt(v) => Ok(*v),
        Value::Int(v) => u64::try_from(*v).map_err(|_| NOT_A_BIT),
        _ => Err(NOT_A_BIT),
    }
}

fn float32(value: &Value) -> Result<f32, Refusal> {
    match value {
        Value::Float(v) => Ok(*v),
        other => text_of(other)
            .and_then(|text| text.trim().parse::<f32>().ok())
            .ok_or(NOT_A_FLOAT),
    }
}

fn float64(value: &Value) -> Result<f64, Refusal> {
    match value {
        Value::Double(v) => Ok(*v),
        Value::Float(v) => Ok(f64::from(*v)),
        other => text_of(other)
            .and_then(|text| text.trim().parse::<f64>().ok())
            .ok_or(NOT_A_FLOAT),
    }
}

/// A decimal's unscaled digits at the column's scale, as text for the integer
/// parser of the column's width.
///
/// The server sends a `DECIMAL` as its text in both protocols. More fraction
/// digits than the scale is refused rather than rounded: rounding would change
/// an amount without saying so.
fn decimal_digits(value: &Value, scale: i8) -> Result<String, Refusal> {
    let text = text_of(value).ok_or(NOT_A_DECIMAL)?.trim();
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let scale = usize::try_from(scale).map_err(|_| NOT_A_DECIMAL)?;
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > scale
    {
        return Err(NOT_A_DECIMAL);
    }
    let mut digits = String::with_capacity(whole.len() + scale + 1);
    if negative {
        digits.push('-');
    }
    digits.push_str(if whole.is_empty() { "0" } else { whole });
    digits.push_str(fraction);
    for _ in fraction.len()..scale {
        digits.push('0');
    }
    Ok(digits)
}

/// The epoch, as `Date32` and timestamps count from it.
fn epoch() -> NaiveDate {
    NaiveDate::default()
}

/// A calendar date from the server's parts. Month or day zero — MySQL's zero
/// and partial dates — and impossible days are refused (ADR-0050 §10).
fn calendar_date(year: u16, month: u8, day: u8) -> Result<NaiveDate, Refusal> {
    if month == 0 || day == 0 {
        return Err(ZERO_DATE);
    }
    NaiveDate::from_ymd_opt(i32::from(year), u32::from(month), u32::from(day)).ok_or(ZERO_DATE)
}

/// Splits `YYYY-MM-DD[ HH:MM:SS[.ffffff]]` into the binary protocol's parts.
fn parse_datetime_text(text: &str) -> Result<DateParts, Refusal> {
    let text = text.trim();
    let (date, time) = text.split_once([' ', 'T']).unwrap_or((text, ""));
    let mut parts = date.splitn(3, '-');
    let year = next_number::<u16>(&mut parts)?;
    let month = next_number::<u8>(&mut parts)?;
    let day = next_number::<u8>(&mut parts)?;
    if time.is_empty() {
        return Ok((year, month, day, 0, 0, 0, 0));
    }
    let (hour, minute, second, micros) = parse_clock(time).ok_or(NOT_A_DATE)?;
    let hour = u8::try_from(hour).map_err(|_| NOT_A_DATE)?;
    Ok((year, month, day, hour, minute, second, micros))
}

fn next_number<'a, T: std::str::FromStr>(
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<T, Refusal> {
    parts
        .next()
        .and_then(|part| part.parse::<T>().ok())
        .ok_or(NOT_A_DATE)
}

/// `H+:MM:SS[.f{1,6}]` into hours, minutes, seconds and microseconds.
fn parse_clock(text: &str) -> Option<(u32, u8, u8, u32)> {
    let (clock, fraction) = text.split_once('.').unwrap_or((text, ""));
    let mut parts = clock.splitn(3, ':');
    let hours = parts.next()?.parse::<u32>().ok()?;
    let minutes = parts.next()?.parse::<u8>().ok()?;
    let seconds = parts.next()?.parse::<u8>().ok()?;
    if fraction.len() > 6 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut micros: u32 = 0;
    for position in 0..6 {
        let digit = fraction
            .as_bytes()
            .get(position)
            .map_or(0, |byte| u32::from(byte.saturating_sub(b'0')));
        micros = micros.checked_mul(10)?.checked_add(digit)?;
    }
    Some((hours, minutes, seconds, micros))
}

/// The parts of a `DATE`, `DATETIME` or `TIMESTAMP`, from either protocol.
fn date_parts(value: &Value) -> Result<DateParts, Refusal> {
    match value {
        Value::Date(year, month, day, hour, minute, second, micros) => {
            Ok((*year, *month, *day, *hour, *minute, *second, *micros))
        }
        other => parse_datetime_text(text_of(other).ok_or(NOT_A_DATE)?),
    }
}

fn date32(value: &Value) -> Result<i32, Refusal> {
    let (year, month, day, ..) = date_parts(value)?;
    let date = calendar_date(year, month, day)?;
    i32::try_from(date.signed_duration_since(epoch()).num_days()).map_err(|_| NOT_A_DATE)
}

fn micros_since_epoch(value: &Value) -> Result<i64, Refusal> {
    let (year, month, day, hour, minute, second, micros) = date_parts(value)?;
    let date = calendar_date(year, month, day)?;
    let time = NaiveTime::from_hms_micro_opt(
        u32::from(hour),
        u32::from(minute),
        u32::from(second),
        micros,
    )
    .ok_or(NOT_A_DATE)?;
    Ok(NaiveDateTime::new(date, time).and_utc().timestamp_micros())
}

/// The largest `TIME` magnitude MySQL and MariaDB store, `838:59:59.000000`.
const MAX_TIME_MICROS: u64 = (838 * 3_600 + 59 * 60 + 59) * 1_000_000;

/// A `TIME` as signed microseconds: its range is ±838:59:59, not a day.
fn time_micros(value: &Value) -> Result<i64, Refusal> {
    let (negative, hours, minutes, seconds, micros) = match value {
        Value::Time(negative, days, hours, minutes, seconds, micros) => {
            let hours = u64::from(*days)
                .checked_mul(24)
                .and_then(|h| h.checked_add(u64::from(*hours)))
                .ok_or(NOT_A_TIME)?;
            (*negative, hours, *minutes, *seconds, *micros)
        }
        other => {
            let text = text_of(other).ok_or(NOT_A_TIME)?.trim();
            let (negative, clock) = match text.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, text),
            };
            let (hours, minutes, seconds, micros) = parse_clock(clock).ok_or(NOT_A_TIME)?;
            (negative, u64::from(hours), minutes, seconds, micros)
        }
    };
    // Out-of-range components are refused rather than carried: the arithmetic
    // below would turn `00:60:00` into a plausible `01:00:00`.
    if minutes > 59 || seconds > 59 || micros > 999_999 {
        return Err(NOT_A_TIME);
    }
    let total = hours
        .checked_mul(3_600)
        .and_then(|s| s.checked_add(u64::from(minutes).checked_mul(60)?))
        .and_then(|s| s.checked_add(u64::from(seconds)))
        .and_then(|s| s.checked_mul(1_000_000))
        .and_then(|us| us.checked_add(u64::from(micros)))
        .ok_or(NOT_A_TIME)?;
    if total > MAX_TIME_MICROS {
        return Err(NOT_A_TIME);
    }
    let magnitude = i64::try_from(total).map_err(|_| NOT_A_TIME)?;
    Ok(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use arrow::array::{
        Array, BinaryArray, Date32Array, Decimal128Array, Decimal256Array,
        DurationMicrosecondArray, Int8Array, StringArray, TimestampMicrosecondArray, UInt64Array,
    };
    use arrow::datatypes::{Field, Schema};

    use super::*;
    use crate::types::arrow_type;

    fn assembler(decodings: &[MyDecoding]) -> BatchAssembler {
        let fields: Vec<Field> = decodings
            .iter()
            .enumerate()
            .map(|(i, d)| Field::new(format!("c{i}"), arrow_type(*d), true))
            .collect();
        BatchAssembler::new(Arc::new(Schema::new(fields)), decodings)
    }

    fn one(decoding: MyDecoding, value: Value) -> Result<RecordBatch, DecodeError> {
        let mut assembler = assembler(&[decoding]);
        assembler.push(vec![Some(value)])?;
        Ok(assembler.finish().expect("the column matches its schema"))
    }

    #[test]
    fn a_zero_or_partial_date_is_refused_naming_the_column_and_the_remedy() {
        // ADR-0050 §10: neither NULL nor an invented date.
        for decoding in [
            MyDecoding::Date,
            MyDecoding::DateTime,
            MyDecoding::Timestamp,
        ] {
            for value in [
                Value::Date(0, 0, 0, 0, 0, 0, 0),
                Value::Date(2024, 0, 15, 0, 0, 0, 0),
                Value::Bytes(b"0000-00-00".to_vec()),
                Value::Bytes(b"2024-00-15 10:00:00".to_vec()),
            ] {
                let error = one(decoding, value).expect_err("refusal expected");
                assert_eq!(error.column(), "c0");
                assert!(error.to_string().contains("CAST(… AS CHAR)"), "{error}");
                assert!(
                    !error.to_string().contains("2024"),
                    "no data in the message"
                );
            }
        }
    }

    #[test]
    fn dates_and_timestamps_count_from_the_epoch_in_both_protocols() {
        let binary = one(MyDecoding::Date, Value::Date(1970, 1, 2, 0, 0, 0, 0)).expect("date");
        let text = one(MyDecoding::Date, Value::Bytes(b"1970-01-02".to_vec())).expect("date");
        for batch in [binary, text] {
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<Date32Array>()
                .expect("Date32");
            assert_eq!(column.value(0), 1);
        }
        let stamp = one(
            MyDecoding::Timestamp,
            Value::Bytes(b"1970-01-01 00:00:01.5".to_vec()),
        )
        .expect("timestamp");
        let column = stamp
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .expect("Timestamp");
        assert_eq!(column.value(0), 1_500_000);
    }

    #[test]
    fn a_time_keeps_its_full_range_and_its_sign() {
        let limit = 838 * 3_600 + 59 * 60 + 59;
        for (value, expected) in [
            (Value::Time(true, 34, 22, 59, 59, 0), -limit * 1_000_000),
            (
                Value::Bytes(b"-838:59:59.000000".to_vec()),
                -limit * 1_000_000,
            ),
            (Value::Bytes(b"838:59:59".to_vec()), limit * 1_000_000),
            (Value::Time(false, 0, 0, 0, 1, 250_000), 1_250_000),
        ] {
            let batch = one(MyDecoding::Time, value).expect("time");
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<DurationMicrosecondArray>()
                .expect("Duration");
            assert_eq!(column.value(0), expected);
        }
    }

    #[test]
    fn a_malformed_time_is_refused_rather_than_normalized() {
        // `00:60:00` must not become `01:00:00`, nor `839:00:00` a duration
        // MySQL cannot store: a broken server is reported, not rewritten.
        for value in [
            Value::Time(false, 0, 0, 60, 0, 0),
            Value::Time(false, 0, 0, 0, 60, 0),
            Value::Time(false, 0, 0, 0, 0, 1_000_000),
            Value::Time(false, 34, 23, 0, 0, 0),
            Value::Time(true, 34, 22, 59, 59, 1),
            Value::Bytes(b"00:60:00".to_vec()),
            Value::Bytes(b"00:00:60".to_vec()),
            Value::Bytes(b"839:00:00".to_vec()),
            Value::Bytes(b"-838:59:59.000001".to_vec()),
        ] {
            assert!(
                one(MyDecoding::Time, value.clone()).is_err(),
                "{value:?} must be refused"
            );
        }
    }

    #[test]
    fn a_decimal_is_exact_up_to_65_digits() {
        let text = "-12345678901234567890123456789012345.123456789012345678901234567890";
        let batch = one(
            MyDecoding::Decimal256 {
                precision: 65,
                scale: 30,
            },
            Value::Bytes(text.as_bytes().to_vec()),
        )
        .expect("DECIMAL(65,30)");
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal256Array>()
            .expect("Decimal256");
        assert_eq!(column.value_as_string(0), text);

        let small = one(
            MyDecoding::Decimal128 {
                precision: 10,
                scale: 2,
            },
            Value::Bytes(b"12.5".to_vec()),
        )
        .expect("DECIMAL(10,2)");
        let column = small
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .expect("Decimal128");
        assert_eq!(column.value(0), 1250, "padded to the scale");
    }

    #[test]
    fn a_decimal_with_more_digits_than_its_scale_is_refused_not_rounded() {
        let error = one(
            MyDecoding::Decimal128 {
                precision: 10,
                scale: 2,
            },
            Value::Bytes(b"1.005".to_vec()),
        )
        .expect_err("refusal expected");
        assert!(error.to_string().contains("precision"), "{error}");
    }

    #[test]
    fn bigint_unsigned_max_survives_and_an_out_of_range_integer_is_refused() {
        let batch = one(MyDecoding::UInt64, Value::UInt(u64::MAX)).expect("u64");
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("UInt64");
        assert_eq!(column.value(0), u64::MAX);
        let text = one(
            MyDecoding::UInt64,
            Value::Bytes(b"18446744073709551615".to_vec()),
        )
        .expect("u64 as text");
        assert_eq!(text.num_rows(), 1);

        // A server that lies about a TINYINT's range.
        assert!(one(MyDecoding::Int8, Value::Int(300)).is_err());
        let fine = one(MyDecoding::Int8, Value::Int(100)).expect("TINYINT(1) holding 100");
        let column = fine
            .column(0)
            .as_any()
            .downcast_ref::<Int8Array>()
            .expect("Int8");
        assert_eq!(column.value(0), 100);
    }

    #[test]
    fn bits_are_read_big_endian_and_more_than_64_are_refused() {
        let batch = one(MyDecoding::Bit, Value::Bytes(vec![0x00, 0x05])).expect("BIT(12)");
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("UInt64");
        assert_eq!(column.value(0), 5);
        assert!(one(MyDecoding::Bit, Value::Bytes(vec![1; 9])).is_err());
    }

    #[test]
    fn invalid_utf8_is_refused_and_bytes_stay_bytes() {
        assert!(one(MyDecoding::Text, Value::Bytes(vec![0xff, 0xfe])).is_err());
        let batch = one(MyDecoding::Binary, Value::Bytes(vec![0xff, 0xfe])).expect("bytes");
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("Binary");
        assert_eq!(column.value(0), &[0xff, 0xfe]);
        let text = one(MyDecoding::Text, Value::Bytes("é".as_bytes().to_vec())).expect("text");
        let column = text
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8");
        assert_eq!(column.value(0), "é");
    }

    #[test]
    fn a_null_is_null_whatever_the_schema_says_and_a_short_row_is_refused() {
        let batch = one(MyDecoding::Int64, Value::NULL).expect("NULL");
        assert!(batch.column(0).is_null(0));
        let mut assembler = assembler(&[MyDecoding::Int64, MyDecoding::Text]);
        assert!(assembler.push(vec![Some(Value::Int(1))]).is_err());
        assert!(assembler.push(vec![None, None]).is_err());
    }

    #[test]
    fn garbage_never_panics() {
        // I-09: every decoding against every hostile shape.
        let decodings = [
            MyDecoding::Int8,
            MyDecoding::UInt16,
            MyDecoding::Int64,
            MyDecoding::Bit,
            MyDecoding::Float32,
            MyDecoding::Float64,
            MyDecoding::Decimal128 {
                precision: 10,
                scale: 2,
            },
            MyDecoding::Decimal256 {
                precision: 65,
                scale: 30,
            },
            MyDecoding::Date,
            MyDecoding::DateTime,
            MyDecoding::Timestamp,
            MyDecoding::Time,
            MyDecoding::Text,
            MyDecoding::Binary,
        ];
        let values = [
            Value::Bytes(Vec::new()),
            Value::Bytes(b"-".to_vec()),
            Value::Bytes(b".".to_vec()),
            Value::Bytes(b"99999999999999999999999999999999999999999999".to_vec()),
            Value::Bytes(b"9999-99-99 99:99:99.9999999".to_vec()),
            Value::Bytes(b"::".to_vec()),
            Value::Bytes(vec![0xff; 40]),
            Value::Int(i64::MIN),
            Value::UInt(u64::MAX),
            Value::Float(f32::NAN),
            Value::Double(f64::INFINITY),
            Value::Date(u16::MAX, 255, 255, 255, 255, 255, u32::MAX),
            Value::Time(true, u32::MAX, 255, 255, 255, u32::MAX),
        ];
        for decoding in decodings {
            for value in &values {
                let _ = one(decoding, value.clone());
            }
        }
    }
}
