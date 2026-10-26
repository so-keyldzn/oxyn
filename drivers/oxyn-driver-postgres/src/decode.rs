//! From PostgreSQL's binary format to an Arrow `RecordBatch`.
//!
//! This is the product's **per-row, per-value** path: what is written here runs
//! once per cell of a ten-million-row result. One allocation per value here is
//! a design defect, not an optimization to postpone
//! ([rust.md](../../../.claude/rules/rust.md)).
//!
//! # The three rules this module holds
//!
//! **Nothing panics.** The bytes come from the network. No `unwrap`, no slice
//! indexing, no `as` on an integer that can overflow; a buffer that is too
//! short returns a named error ([I-09](../../../CLAUDE.md#i-09)). The reader of
//! the `numeric` module exists exactly for that: `bytes::Buf` panics on a short
//! buffer.
//!
//! **No value enters an error message.** Errors name the column's rank, the
//! PostgreSQL type — sanitized — and what was wrong. Never the data
//! ([I-03](../../../CLAUDE.md#i-03)).
//!
//! **An unknown type does not make the row fail.** It keeps its bytes, as an
//! Arrow `Binary` column marked `opaque`, with its PostgreSQL type name in the
//! metadata ([`crate::types`]): valid UTF-8 is no evidence that a layout is
//! text. A built-in type with a documented layout is rendered as the text the
//! server prints (`render` module). The only values that really make a row
//! fail are those Arrow cannot represent *without lying*: an infinite date, an
//! interval longer than 292 years in microseconds, a `time` of 24:00:00, a
//! multi-dimensional array or one whose lower bound is not 1.
//! Rendering them `NULL` would be a silent lie about real data.
//!
//! # Batch sizing
//!
//! [`BatchAssembler::bytes`] counts the **data bytes** seen on the wire, not
//! the rows. A thousand rows each carrying a one-megabyte BLOB make a gigabyte:
//! a batch sized by row count works on demo tables and triggers the OOM on real
//! ones.

use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Float32Builder, Float64Builder,
    Int16Builder, Int32Builder, Int64Builder, IntervalMonthDayNanoBuilder, ListArray,
    StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder, UInt32Builder,
    UInt64Builder,
};
use arrow::buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow::datatypes::{Field, FieldRef, IntervalMonthDayNano, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use sqlx::Column as _;
use sqlx::Row as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgRow, PgValueFormat};

use crate::numeric::{Reader, render_binary};
use crate::render::value::MAX_RENDERED_BYTES;
use crate::types::PgDecoding;

/// Microseconds between the Unix epoch and PostgreSQL's (2000-01-01).
const EPOCH_SHIFT_MICROS: i64 = 946_684_800_000_000;
/// Days between the Unix epoch and PostgreSQL's.
const EPOCH_SHIFT_DAYS: i32 = 10_957;
/// Microseconds in a day: the exclusive bound of an Arrow `Time64`.
const MICROS_PER_DAY: i64 = 86_400_000_000;
/// Bytes of a `uuid` on the wire.
const UUID_LEN: usize = 16;
/// Version expected at the head of a `jsonb`.
const JSONB_VERSION: u8 = 1;

/// What can go wrong decoding a server response.
///
/// No variant carries a value coming from the database: only a column rank, a
/// sanitized type name, and a `&'static str` detail.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// A value could not be represented.
    #[error("column {ordinal} of type `{pg_type}`: {detail}")]
    Value {
        /// Rank of the column in the result, from zero.
        ordinal: usize,
        /// The PostgreSQL type, sanitized.
        pg_type: String,
        /// What was wrong, without repeating the value.
        detail: &'static str,
    },

    /// The row could not be read: rank out of bounds, missing metadata.
    #[error("unreadable row: {detail}")]
    Row {
        /// What was wrong.
        detail: String,
    },

    /// Arrow refused the built batch. It is a driver bug: the schema and the
    /// column builders have diverged.
    #[error("invalid Arrow batch: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

/// Makes a type name showable in an error message.
///
/// A type name comes from the server — hence from untrusted input: nothing
/// prevents a user type from being named with terminal control sequences. What
/// comes out of here is bounded and restricted to a harmless alphabet.
fn sanitize_type(name: &str) -> String {
    const MAX: usize = 64;
    let acceptable = !name.is_empty()
        && name.len() <= MAX
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']' | '"'));
    if acceptable {
        name.to_owned()
    } else {
        "<unrepresentable type>".to_owned()
    }
}

/// Accumulates PostgreSQL rows into Arrow batches.
///
/// Reusable: [`BatchAssembler::finish`] empties the builders and returns the
/// batch, the assembler starts over for the next one. That is what lets a
/// stream never materialize more than one batch at a time
/// ([I-06](../../../CLAUDE.md#i-06)).
#[derive(Debug)]
pub struct BatchAssembler {
    schema: SchemaRef,
    columns: Vec<ColumnBuilder>,
    rows: usize,
    bytes: usize,
}

impl BatchAssembler {
    /// Prepares an assembler for this schema and this decoding plan.
    ///
    /// Both come from [`crate::types::schema_for`] and are aligned by
    /// construction; a divergence would be a bug, and [`Self::finish`] would
    /// report it as such.
    #[must_use]
    pub fn new(schema: SchemaRef, decodings: &[PgDecoding]) -> Self {
        let columns = decodings.iter().map(ColumnBuilder::for_decoding).collect();
        Self {
            schema,
            columns,
            rows: 0,
            bytes: 0,
        }
    }

    /// Rows accumulated since the last batch.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Data bytes accumulated since the last batch.
    ///
    /// It is the measure that must decide when to cut a batch — not
    /// [`Self::rows`].
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Nothing has been accumulated since the last batch.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Adds a row.
    ///
    /// # Errors
    /// [`DecodeError::Row`] if the row does not have the expected number of
    /// columns, [`DecodeError::Value`] if a value is not representable.
    pub fn push(&mut self, row: &PgRow) -> Result<(), DecodeError> {
        if row.columns().len() != self.columns.len() {
            return Err(DecodeError::Row {
                detail: format!(
                    "{} columns received, {} announced by the schema",
                    row.columns().len(),
                    self.columns.len()
                ),
            });
        }

        // The bound of one rendered value holds per value; the batch is cut only
        // after the whole row. Without a bound across the row, 1,600 columns of
        // nested records would each render up to the per-value limit before
        // the cursor could cut anything (I-06).
        let mut rendered: usize = 0;
        for (ordinal, column) in self.columns.iter_mut().enumerate() {
            let raw_value = row.try_get_raw(ordinal).map_err(|err| DecodeError::Row {
                detail: err.to_string(),
            })?;
            let format = raw_value.format();
            // `as_bytes` returns an error on a missing value; that is how `sqlx`
            // distinguishes NULL, not through a variant.
            let encoded = raw_value.as_bytes().ok();
            self.bytes = self.bytes.saturating_add(encoded.map_or(0, <[u8]>::len));

            // The type name is composed only on the error path: here, one
            // allocation per cell would cost the budget of a whole result.
            let appended = column.append(format, encoded).and_then(|produced| {
                rendered = rendered.saturating_add(produced);
                if rendered > MAX_RENDERED_BYTES {
                    return Err(
                        "row rendered larger than 16 MiB: cast its values to text in the query",
                    );
                }
                Ok(produced)
            });
            let produced = match appended {
                Ok(produced) => produced,
                Err(detail) => {
                    return Err(DecodeError::Value {
                        ordinal,
                        pg_type: sanitize_type(
                            row.columns()
                                .get(ordinal)
                                .map_or("unknown", |c| c.type_info().name()),
                        ),
                        detail,
                    });
                }
            };
            // A rendered value's text can be far larger than its bytes on the
            // wire: the batch budget counts what is **held**, not what arrived.
            self.bytes = self.bytes.saturating_add(produced);
        }

        self.rows = self.rows.saturating_add(1);
        Ok(())
    }

    /// Closes the current batch and starts over empty.
    ///
    /// # Errors
    /// [`DecodeError::Arrow`] if the built batch does not match the schema —
    /// which would be a driver bug, not faulty data.
    pub fn finish(&mut self) -> Result<RecordBatch, DecodeError> {
        let mut arrays = Vec::with_capacity(self.columns.len());
        for column in &mut self.columns {
            arrays.push(column.finish()?);
        }
        let rows = self.rows;
        self.rows = 0;
        self.bytes = 0;

        // A result without columns — an `INSERT`, a `SET` — still has a row
        // count, which `try_new` cannot infer from zero columns.
        if arrays.is_empty() {
            let options = RecordBatchOptions::new().with_row_count(Some(rows));
            return Ok(RecordBatch::try_new_with_options(
                Arc::clone(&self.schema),
                arrays,
                &options,
            )?);
        }
        Ok(RecordBatch::try_new(Arc::clone(&self.schema), arrays)?)
    }
}

/// An Arrow column builder, with what it takes to pour PostgreSQL binary into
/// it.
#[derive(Debug)]
enum ColumnBuilder {
    Bool(BooleanBuilder),
    Int16(Int16Builder),
    Int32(Int32Builder),
    Int64(Int64Builder),
    UInt32(UInt32Builder),
    UInt64(UInt64Builder),
    Float32(Float32Builder),
    Float64(Float64Builder),
    /// All `Utf8` columns, distinguished by how the text is obtained.
    Text(StringBuilder, TextSource),
    /// A `Utf8` column whose text [`crate::render`] rebuilds from a binary
    /// layout. The scratch buffer is reused from one value to the next: an
    /// allocation per cell would cost the budget of a whole result.
    Rendered {
        builder: StringBuilder,
        decoding: PgDecoding,
        scratch: String,
    },
    Binary(BinaryBuilder),
    Date(Date32Builder),
    Time(Time64MicrosecondBuilder),
    Timestamp(TimestampMicrosecondBuilder),
    Interval(IntervalMonthDayNanoBuilder),
    List(Box<ListColumn>),
}

/// Where the text of a `Utf8` column comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextSource {
    /// The bytes **are** the text.
    Raw,
    /// A version byte, then the text.
    Jsonb,
    /// The `numeric` binary format, rendered as exact decimal.
    Numeric,
    /// Sixteen bytes, rendered in canonical form.
    Uuid,
    /// Time and offset, rendered in ISO 8601.
    TimeTz,
}

/// A column of lists, built by hand.
///
/// Arrow's `ListBuilder` would require going through `Box<dyn ArrayBuilder>`
/// and back down with `downcast_mut` for every element; assembling the offsets
/// oneself avoids that detour and keeps the child's type.
#[derive(Debug)]
struct ListColumn {
    field: FieldRef,
    child: ColumnBuilder,
    /// Cumulative offsets, starting at zero.
    offsets: Vec<i32>,
    /// One entry per row: is the list itself non-null?
    validity: Vec<bool>,
    /// Elements poured into the child since the last batch.
    elements: i32,
}

impl ColumnBuilder {
    /// The builder matching a decoding plan.
    fn for_decoding(decoding: &PgDecoding) -> Self {
        match decoding {
            PgDecoding::Bool => Self::Bool(BooleanBuilder::new()),
            PgDecoding::Int16 => Self::Int16(Int16Builder::new()),
            PgDecoding::Int32 => Self::Int32(Int32Builder::new()),
            PgDecoding::Int64 => Self::Int64(Int64Builder::new()),
            PgDecoding::UInt32 => Self::UInt32(UInt32Builder::new()),
            PgDecoding::UInt64 => Self::UInt64(UInt64Builder::new()),
            PgDecoding::Float32 => Self::Float32(Float32Builder::new()),
            PgDecoding::Float64 => Self::Float64(Float64Builder::new()),
            PgDecoding::Text => Self::Text(StringBuilder::new(), TextSource::Raw),
            PgDecoding::Jsonb => Self::Text(StringBuilder::new(), TextSource::Jsonb),
            PgDecoding::Numeric => Self::Text(StringBuilder::new(), TextSource::Numeric),
            PgDecoding::Uuid => Self::Text(StringBuilder::new(), TextSource::Uuid),
            PgDecoding::TimeTz => Self::Text(StringBuilder::new(), TextSource::TimeTz),
            PgDecoding::Rendered(_)
            | PgDecoding::Range(_)
            | PgDecoding::Multirange(_)
            | PgDecoding::Record(_) => Self::Rendered {
                builder: StringBuilder::new(),
                decoding: decoding.clone(),
                scratch: String::new(),
            },
            PgDecoding::Opaque | PgDecoding::Bytes => Self::Binary(BinaryBuilder::new()),
            PgDecoding::Date => Self::Date(Date32Builder::new()),
            PgDecoding::Time => Self::Time(Time64MicrosecondBuilder::new()),
            PgDecoding::Timestamp => Self::Timestamp(TimestampMicrosecondBuilder::new()),
            PgDecoding::TimestampTz => {
                Self::Timestamp(TimestampMicrosecondBuilder::new().with_timezone("UTC"))
            }
            PgDecoding::Interval => Self::Interval(IntervalMonthDayNanoBuilder::new()),
            PgDecoding::List(element) => Self::List(Box::new(ListColumn {
                field: Arc::new(Field::new("item", element.arrow_type(), true)),
                child: Self::for_decoding(element),
                offsets: vec![0],
                validity: Vec::new(),
                elements: 0,
            })),
        }
    }

    /// Adds a value, or `NULL` when `bytes` is absent.
    ///
    /// Returns the bytes of text a rendered column **produced**, beyond those
    /// received: the batch budget must see them.
    fn append(
        &mut self,
        format: PgValueFormat,
        bytes: Option<&[u8]>,
    ) -> Result<usize, &'static str> {
        let Some(bytes) = bytes else {
            self.append_null();
            return Ok(0);
        };

        // The driver uses only the extended protocol, where the server always
        // answers in binary. The text format can only come from a path that
        // does not exist yet; textual columns support it for free, the others
        // refuse it rather than guess.
        if format == PgValueFormat::Text {
            return match self {
                Self::Text(builder, _) | Self::Rendered { builder, .. } => {
                    let text =
                        std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 text from the server")?;
                    builder.append_value(text);
                    Ok(0)
                }
                _ => Err("the simple protocol's text format is not decoded"),
            };
        }

        let mut produced = 0;
        match self {
            Self::Bool(builder) => {
                let byte = bytes.first().ok_or("empty boolean")?;
                builder.append_value(*byte != 0);
            }
            Self::Int16(builder) => builder.append_value(read_i16(bytes)?),
            Self::Int32(builder) => builder.append_value(read_i32(bytes)?),
            Self::Int64(builder) => builder.append_value(read_i64(bytes)?),
            Self::UInt32(builder) => builder.append_value(read_u32(bytes)?),
            Self::UInt64(builder) => {
                let encoded: [u8; 8] = bytes.try_into().map_err(|_| "xid8 of unexpected size")?;
                builder.append_value(u64::from_be_bytes(encoded));
            }
            Self::Float32(builder) => {
                let encoded: [u8; 4] = bytes.try_into().map_err(|_| "float4 of unexpected size")?;
                builder.append_value(f32::from_be_bytes(encoded));
            }
            Self::Float64(builder) => {
                let encoded: [u8; 8] = bytes.try_into().map_err(|_| "float8 of unexpected size")?;
                builder.append_value(f64::from_be_bytes(encoded));
            }
            Self::Text(builder, source) => {
                render_text(builder, *source, bytes)?;
            }
            Self::Rendered {
                builder,
                decoding,
                scratch,
            } => {
                scratch.clear();
                crate::render::value::value(decoding, bytes, scratch, 0)?;
                builder.append_value(scratch.as_str());
                produced = scratch.len();
            }
            Self::Binary(builder) => builder.append_value(bytes),
            Self::Date(builder) => builder.append_value(read_date(bytes)?),
            Self::Time(builder) => builder.append_value(read_time(bytes)?),
            Self::Timestamp(builder) => builder.append_value(read_timestamp(bytes)?),
            Self::Interval(builder) => builder.append_value(read_interval(bytes)?),
            Self::List(column) => produced = column.append(bytes)?,
        }
        Ok(produced)
    }

    /// Adds a missing value.
    fn append_null(&mut self) {
        match self {
            Self::Bool(builder) => builder.append_null(),
            Self::Int16(builder) => builder.append_null(),
            Self::Int32(builder) => builder.append_null(),
            Self::Int64(builder) => builder.append_null(),
            Self::UInt32(builder) => builder.append_null(),
            Self::UInt64(builder) => builder.append_null(),
            Self::Float32(builder) => builder.append_null(),
            Self::Float64(builder) => builder.append_null(),
            Self::Text(builder, _) | Self::Rendered { builder, .. } => builder.append_null(),
            Self::Binary(builder) => builder.append_null(),
            Self::Date(builder) => builder.append_null(),
            Self::Time(builder) => builder.append_null(),
            Self::Timestamp(builder) => builder.append_null(),
            Self::Interval(builder) => builder.append_null(),
            Self::List(column) => column.append_null(),
        }
    }

    /// Closes the column and starts over empty.
    fn finish(&mut self) -> Result<ArrayRef, DecodeError> {
        let array: ArrayRef = match self {
            Self::Bool(builder) => Arc::new(builder.finish()),
            Self::Int16(builder) => Arc::new(builder.finish()),
            Self::Int32(builder) => Arc::new(builder.finish()),
            Self::Int64(builder) => Arc::new(builder.finish()),
            Self::UInt32(builder) => Arc::new(builder.finish()),
            Self::UInt64(builder) => Arc::new(builder.finish()),
            Self::Float32(builder) => Arc::new(builder.finish()),
            Self::Float64(builder) => Arc::new(builder.finish()),
            Self::Text(builder, _) | Self::Rendered { builder, .. } => Arc::new(builder.finish()),
            Self::Binary(builder) => Arc::new(builder.finish()),
            Self::Date(builder) => Arc::new(builder.finish()),
            Self::Time(builder) => Arc::new(builder.finish()),
            Self::Timestamp(builder) => Arc::new(builder.finish()),
            Self::Interval(builder) => Arc::new(builder.finish()),
            Self::List(column) => column.finish()?,
        };
        Ok(array)
    }
}

impl ListColumn {
    /// Adds a one-dimensional PostgreSQL array.
    fn append(&mut self, bytes: &[u8]) -> Result<usize, &'static str> {
        let elements = split_array(bytes)?;
        let mut produced: usize = 0;
        for element in elements {
            produced = produced.saturating_add(self.child.append(PgValueFormat::Binary, element)?);
            self.elements = self
                .elements
                .checked_add(1)
                .ok_or("more than 2 billion elements in one batch")?;
        }
        self.offsets.push(self.elements);
        self.validity.push(true);
        Ok(produced)
    }

    /// Adds a missing list. `NULL` and `{}` are not the same: the first has no
    /// elements, the second has zero.
    fn append_null(&mut self) {
        self.offsets.push(self.elements);
        self.validity.push(false);
    }

    /// Closes the column and starts over empty.
    fn finish(&mut self) -> Result<ArrayRef, DecodeError> {
        let values = self.child.finish()?;
        let offset_values = std::mem::replace(&mut self.offsets, vec![0]);
        let validity_bits = std::mem::take(&mut self.validity);
        self.elements = 0;

        let array = ListArray::try_new(
            Arc::clone(&self.field),
            OffsetBuffer::new(ScalarBuffer::from(offset_values)),
            values,
            Some(NullBuffer::from(validity_bits)),
        )?;
        Ok(Arc::new(array))
    }
}

/// Writes the textual representation of a value into the builder.
fn render_text(
    builder: &mut StringBuilder,
    source: TextSource,
    bytes: &[u8],
) -> Result<(), &'static str> {
    match source {
        TextSource::Raw => {
            let text = std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 text from the server")?;
            builder.append_value(text);
        }
        TextSource::Jsonb => {
            let (version, tail) = bytes.split_first().ok_or("empty jsonb")?;
            if *version != JSONB_VERSION {
                return Err("unknown jsonb version");
            }
            let text = std::str::from_utf8(tail).map_err(|_| "non-UTF-8 jsonb")?;
            builder.append_value(text);
        }
        TextSource::Numeric => {
            let rendered = render_binary(bytes).ok_or("unreadable numeric")?;
            builder.append_value(&rendered);
        }
        TextSource::Uuid => {
            let encoded: [u8; UUID_LEN] =
                bytes.try_into().map_err(|_| "uuid of unexpected size")?;
            let identifier = uuid::Uuid::from_bytes(encoded);
            let mut buffer = uuid::Uuid::encode_buffer();
            let canonical: &str = identifier.hyphenated().encode_lower(&mut buffer);
            builder.append_value(canonical);
        }
        TextSource::TimeTz => {
            builder.append_value(&render_timetz(bytes)?);
        }
    }
    Ok(())
}

/// Renders a `timetz` in ISO 8601: `14:30:00.250000+02:00`.
///
/// PostgreSQL transmits the offset in **seconds west** of UTC; ISO notation
/// counts it east. The sign flips, and that is exactly the kind of inversion
/// that shifts data by two hours without anyone seeing it
/// ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)).
fn render_timetz(bytes: &[u8]) -> Result<String, &'static str> {
    let mut reader = Reader::new(bytes);
    let micros = reader.i64().ok_or("truncated timetz")?;
    let west = reader.i32().ok_or("timetz without an offset")?;

    if !(0..=86_400_000_000).contains(&micros) {
        return Err("timetz outside the bounds of a day");
    }
    let seconds = micros / 1_000_000;
    let rest = micros % 1_000_000;
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;

    let east = west.checked_neg().ok_or("nonsensical timetz offset")?;
    let sign = if east < 0 { '-' } else { '+' };
    let absolute = east.unsigned_abs();
    let hours_offset = absolute / 3_600;
    let minutes_offset = (absolute % 3_600) / 60;

    let mut out = String::with_capacity(24);
    let _ = write!(out, "{hours:02}:{minutes:02}:{secs:02}");
    if rest != 0 {
        let _ = write!(out, ".{rest:06}");
    }
    let _ = write!(out, "{sign}{hours_offset:02}:{minutes_offset:02}");
    Ok(out)
}

/// A big-endian `int2`.
fn read_i16(bytes: &[u8]) -> Result<i16, &'static str> {
    let encoded: [u8; 2] = bytes.try_into().map_err(|_| "int2 of unexpected size")?;
    Ok(i16::from_be_bytes(encoded))
}

/// A big-endian `int4`.
fn read_i32(bytes: &[u8]) -> Result<i32, &'static str> {
    let encoded: [u8; 4] = bytes.try_into().map_err(|_| "int4 of unexpected size")?;
    Ok(i32::from_be_bytes(encoded))
}

/// A big-endian `int8`.
fn read_i64(bytes: &[u8]) -> Result<i64, &'static str> {
    let encoded: [u8; 8] = bytes.try_into().map_err(|_| "int8 of unexpected size")?;
    Ok(i64::from_be_bytes(encoded))
}

/// A big-endian `oid`.
fn read_u32(bytes: &[u8]) -> Result<u32, &'static str> {
    let encoded: [u8; 4] = bytes.try_into().map_err(|_| "oid of unexpected size")?;
    Ok(u32::from_be_bytes(encoded))
}

/// A `date`: days since 2000-01-01, brought back to the Unix epoch.
fn read_date(bytes: &[u8]) -> Result<i32, &'static str> {
    let days = read_i32(bytes)?;
    if days == i32::MIN || days == i32::MAX {
        // `infinity` and `-infinity` are legal dates in PostgreSQL. No `Date32`
        // value represents them; rendering them as the extreme date would be
        // wrong, and `NULL` would be a lie.
        return Err("infinite date, not representable as Date32 — cast it to text");
    }
    days.checked_add(EPOCH_SHIFT_DAYS)
        .ok_or("date outside the bounds of Date32")
}

/// A `time`: microseconds since midnight.
fn read_time(bytes: &[u8]) -> Result<i64, &'static str> {
    let micros = read_i64(bytes)?;
    if micros == MICROS_PER_DAY {
        // `'24:00:00'::time` is legal in PostgreSQL — a closing time — but
        // `Time64` stops at 23:59:59.999999: the grid cell would fail to format
        // and Arrow's CSV and JSON writers would abort the whole export with a
        // cast error. Same family as an infinite date.
        return Err("time 24:00:00, not representable as Time64 — cast it to text");
    }
    if !(0..MICROS_PER_DAY).contains(&micros) {
        return Err("time outside the bounds of a day");
    }
    Ok(micros)
}

/// A `timestamp` or `timestamptz`: microseconds since 2000-01-01, brought back
/// to the Unix epoch.
fn read_timestamp(bytes: &[u8]) -> Result<i64, &'static str> {
    let micros = read_i64(bytes)?;
    if micros == i64::MIN || micros == i64::MAX {
        return Err("infinite timestamp, not representable — cast it to text");
    }
    micros
        .checked_add(EPOCH_SHIFT_MICROS)
        .ok_or("timestamp out of bounds")
}

/// An `interval`: microseconds, days, months — in that order on the wire.
fn read_interval(bytes: &[u8]) -> Result<IntervalMonthDayNano, &'static str> {
    let mut reader = Reader::new(bytes);
    let micros = reader.i64().ok_or("truncated interval")?;
    let days = reader.i32().ok_or("interval without days")?;
    let months = reader.i32().ok_or("interval without months")?;
    let nanos = micros
        .checked_mul(1_000)
        .ok_or("interval longer than 292 years in microseconds, not representable")?;
    Ok(IntervalMonthDayNano::new(months, days, nanos))
}

/// Splits a binary PostgreSQL array into elements.
///
/// Returns one slice per element, `None` for a missing element. The element
/// type announced in the header is **ignored**: the decoding plan comes from the
/// `RowDescription`, which is authoritative, and following the header would let
/// a server have anything decoded as anything.
pub(crate) fn split_array(bytes: &[u8]) -> Result<Vec<Option<&[u8]>>, &'static str> {
    let mut reader = Reader::new(bytes);
    let dimensions = reader.i32().ok_or("truncated array header")?;
    let _flags = reader.i32().ok_or("truncated array header")?;
    let _element_type = reader.u32().ok_or("truncated array header")?;

    if dimensions == 0 {
        return Ok(Vec::new());
    }
    if dimensions != 1 {
        // An Arrow list is one-dimensional. Flattening would lose the shape
        // without saying so; refusing names it.
        return Err("multi-dimensional array, not representable as an Arrow list");
    }

    let length = reader.i32().ok_or("truncated array dimension")?;
    let lower_bound = reader.i32().ok_or("truncated array dimension")?;
    if lower_bound != 1 {
        // `'[0:2]={10,20,30}'::int[]` is legal, and an Arrow list has no bound:
        // shown as `[10, 20, 30]`, it makes `arr[1]` read the wrong element.
        // Refusing names it, as for a second dimension.
        return Err(
            "array whose lower bound is not 1, not representable as an Arrow list — cast it to text",
        );
    }
    let length = usize::try_from(length).map_err(|_| "negative array length")?;

    // Each element costs at least its four length bytes: a value larger than
    // what the buffer can hold is hostile, and `with_capacity` on such a value
    // would exhaust memory.
    if length > reader.rest().len() / 4 + 1 {
        return Err("array length inconsistent with the buffer received");
    }

    let mut elements = Vec::with_capacity(length);
    for _ in 0..length {
        let size = reader.i32().ok_or("truncated array element")?;
        if size == -1 {
            elements.push(None);
            continue;
        }
        let size = usize::try_from(size).map_err(|_| "negative element size")?;
        elements.push(Some(reader.take(size).ok_or("truncated array element")?));
    }
    Ok(elements)
}

#[cfg(test)]
mod tests;
