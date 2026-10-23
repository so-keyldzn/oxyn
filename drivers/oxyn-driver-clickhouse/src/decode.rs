//! Measured `RowBinary` rows to Arrow arrays.
//!
//! [`crate::measure`] has already proven the row whole; this module decodes it
//! into one builder per column, nested like the column's type. What it still
//! refuses is a value that contradicts its type — a `Bool` that is 2, an enum
//! value the type does not declare, a text that is not UTF-8 — each a
//! [`DecodeError`] naming the column, never a guessed value
//! ([I-09](../../../CLAUDE.md#i-09)).
//!
//! No value allocates on its own: text is written straight into the builder's
//! buffer, the 256-bit integers through a stack buffer.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use arrow::array::{
    Array as _, ArrayRef, BooleanBuilder, Date32Builder, Decimal128Builder, Decimal256Builder,
    FixedSizeBinaryBuilder, Float32Builder, Float64Builder, Int8Builder, Int16Builder,
    Int32Builder, Int64Builder, ListArray, MapArray, NullBuilder, StringBuilder, StructArray,
    UInt8Builder, UInt16Builder, UInt32Builder, UInt64Builder, make_array,
};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, FieldRef, Fields, SchemaRef, i256};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};

use crate::measure::{BAD_MARKER, DecodeError, decimal_width, value_error, varint};
use crate::schema::{Column, data_type, field, tick_factor};
use crate::types::ChType;

const NOT_UTF8: &str = "a value is not valid UTF-8; read it with `hex()` or `base64Encode()`";
const BAD_BOOL: &str = "a Bool is neither 0 nor 1";
const UNDECLARED_LABEL: &str = "a value the Enum type does not declare";
const NULL_NOT_ALLOWED: &str = "a NULL in a type that cannot hold one";
const OUT_OF_RANGE: &str = "a value outside what its Arrow type can hold";
const TOO_MANY_ITEMS: &str = "more elements than an Arrow list can index";

/// Appends a fixed-width value and answers its decoded width.
macro_rules! put {
    ($builder:expr, $value:expr, $width:expr) => {{
        $builder.append_value($value);
        Ok($width)
    }};
}

/// Decodes measured rows of one result into batches.
pub struct RowDecoder {
    columns: Vec<Column>,
    schema: SchemaRef,
    builders: Vec<Builder>,
    rows: usize,
    bytes: usize,
}

impl std::fmt::Debug for RowDecoder {
    /// The shape and the fill of the batch, never a value: the builders hold
    /// the user's data.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowDecoder")
            .field("columns", &self.columns.len())
            .field("rows", &self.rows)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl RowDecoder {
    /// A decoder for `columns`, whose batches follow `schema`.
    ///
    /// The caller has refused unsupported types already
    /// ([`ChType::unsupported`]); `zone` must be the one `schema` was built
    /// with.
    #[must_use]
    pub fn new(columns: Vec<Column>, schema: SchemaRef, zone: Option<&str>) -> Self {
        let builders = columns
            .iter()
            .map(|column| builder_for(&column.ty, zone))
            .collect();
        Self {
            columns,
            schema,
            builders,
            rows: 0,
            bytes: 0,
        }
    }

    /// The columns the decoder reads.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Rows in the batch being built.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Bytes the batch being built holds once decoded — what bounds a batch
    /// ([I-06](../../../CLAUDE.md#i-06)), not the bytes received.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Decodes one row [`measure`](crate::measure::measure) proved whole.
    ///
    /// # Errors
    /// [`DecodeError::Value`] when a value contradicts its type. The batch is
    /// then unusable: the cursor fails the result rather than emit it.
    pub fn append(&mut self, row: &[u8]) -> Result<(), DecodeError> {
        let mut reader = Reader { rest: row };
        for (column, builder) in self.columns.iter().zip(&mut self.builders) {
            let added = append(&column.ty, &mut reader, builder, &column.name)?;
            self.bytes = self.bytes.saturating_add(added);
        }
        self.rows += 1;
        Ok(())
    }

    /// The batch built so far; the decoder starts an empty one.
    ///
    /// # Errors
    /// [`DecodeError::Assembly`] if Arrow refuses the arrays.
    pub fn finish(&mut self) -> Result<RecordBatch, DecodeError> {
        let arrays = self
            .builders
            .iter_mut()
            .map(finish)
            .collect::<Result<Vec<_>, _>>()?;
        let options = RecordBatchOptions::new().with_row_count(Some(self.rows));
        self.rows = 0;
        self.bytes = 0;
        RecordBatch::try_new_with_options(Arc::clone(&self.schema), arrays, &options)
            .map_err(|error| DecodeError::Assembly(error.to_string()))
    }
}

/// Reads a measured row; running short is [`DecodeError::Truncated`], which a
/// measured row never meets.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let (head, rest) = self
            .rest
            .split_at_checked(n)
            .ok_or(DecodeError::Truncated)?;
        self.rest = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        self.take(N)?.try_into().map_err(|_| DecodeError::Truncated)
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    fn varint(&mut self, column: &str) -> Result<u64, DecodeError> {
        let (value, end) =
            varint(self.rest, 0, self.rest.len(), column)?.ok_or(DecodeError::Truncated)?;
        self.take(end)?;
        Ok(value)
    }

    fn sized(&mut self, column: &str) -> Result<&'a [u8], DecodeError> {
        let length = usize::try_from(self.varint(column)?).map_err(|_| DecodeError::Truncated)?;
        self.take(length)
    }
}

/// One Arrow builder per ClickHouse type, nested like the type.
enum Builder {
    Bool(BooleanBuilder),
    I8(Int8Builder),
    I16(Int16Builder),
    I32(Int32Builder),
    I64(Int64Builder),
    U8(UInt8Builder),
    U16(UInt16Builder),
    U32(UInt32Builder),
    U64(UInt64Builder),
    F32(Float32Builder),
    F64(Float64Builder),
    D128(Decimal128Builder),
    D256(Decimal256Builder),
    Text(StringBuilder),
    Fixed(FixedSizeBinaryBuilder),
    Date32(Date32Builder),
    /// Seconds or ticks, given the timestamp or duration type at the end: the
    /// layouts are the same `i64`.
    Ticks {
        values: Int64Builder,
        factor: i64,
        target: DataType,
    },
    Null(NullBuilder),
    List {
        offsets: Vec<i32>,
        item: FieldRef,
        child: Box<Builder>,
    },
    Struct {
        fields: Fields,
        children: Vec<Builder>,
        len: usize,
    },
    Map {
        offsets: Vec<i32>,
        entries: FieldRef,
        keys: Box<Builder>,
        values: Box<Builder>,
    },
}

fn builder_for(ty: &ChType, zone: Option<&str>) -> Builder {
    match ty {
        ChType::Nullable(inner) => builder_for(inner, zone),
        ChType::Bool => Builder::Bool(BooleanBuilder::new()),
        ChType::Int(8) => Builder::I8(Int8Builder::new()),
        ChType::Int(16) => Builder::I16(Int16Builder::new()),
        ChType::Int(32) => Builder::I32(Int32Builder::new()),
        ChType::Int(64) | ChType::Interval => Builder::I64(Int64Builder::new()),
        ChType::UInt(8) => Builder::U8(UInt8Builder::new()),
        ChType::UInt(16) => Builder::U16(UInt16Builder::new()),
        ChType::UInt(32) => Builder::U32(UInt32Builder::new()),
        ChType::UInt(64) => Builder::U64(UInt64Builder::new()),
        ChType::Int(128) | ChType::UInt(128) => {
            Builder::D256(Decimal256Builder::new().with_data_type(data_type(ty, zone)))
        }
        ChType::Float32 | ChType::BFloat16 => Builder::F32(Float32Builder::new()),
        ChType::Float64 => Builder::F64(Float64Builder::new()),
        ChType::Decimal { precision, .. } if *precision <= 38 => {
            Builder::D128(Decimal128Builder::new().with_data_type(data_type(ty, zone)))
        }
        ChType::Decimal { .. } => {
            Builder::D256(Decimal256Builder::new().with_data_type(data_type(ty, zone)))
        }
        ChType::FixedString(width) => match i32::try_from(*width) {
            Ok(width) => Builder::Fixed(FixedSizeBinaryBuilder::new(width)),
            Err(_) => Builder::Null(NullBuilder::new()),
        },
        ChType::Date | ChType::Date32 => Builder::Date32(Date32Builder::new()),
        ChType::DateTime(_) | ChType::Time => Builder::Ticks {
            values: Int64Builder::new(),
            factor: 1,
            target: data_type(ty, zone),
        },
        ChType::DateTime64 { precision, .. } | ChType::Time64(precision) => Builder::Ticks {
            values: Int64Builder::new(),
            factor: tick_factor(*precision),
            target: data_type(ty, zone),
        },
        ChType::Nothing | ChType::Unsupported(_) => Builder::Null(NullBuilder::new()),
        ChType::Array(inner) => Builder::List {
            offsets: vec![0],
            item: Arc::new(field("item", inner, zone)),
            child: Box::new(builder_for(inner, zone)),
        },
        ChType::Tuple(elements) => match data_type(ty, zone) {
            DataType::Struct(fields) => Builder::Struct {
                fields,
                children: elements
                    .iter()
                    .map(|(_, element)| builder_for(element, zone))
                    .collect(),
                len: 0,
            },
            _ => Builder::Null(NullBuilder::new()),
        },
        ChType::Map(key, value) => match data_type(ty, zone) {
            DataType::Map(entries, _) => Builder::Map {
                offsets: vec![0],
                entries,
                keys: Box::new(builder_for(key, zone)),
                values: Box::new(builder_for(value, zone)),
            },
            _ => Builder::Null(NullBuilder::new()),
        },
        ChType::String
        | ChType::Json
        | ChType::Int(_)
        | ChType::UInt(_)
        | ChType::Uuid
        | ChType::Ipv4
        | ChType::Ipv6
        | ChType::Enum { .. } => Builder::Text(StringBuilder::new()),
    }
}

/// Decodes one value into its builder; answers the bytes it adds once decoded.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per type: splitting it would scatter the wire format"
)]
fn append(
    ty: &ChType,
    reader: &mut Reader<'_>,
    builder: &mut Builder,
    column: &str,
) -> Result<usize, DecodeError> {
    if let ChType::Nullable(inner) = ty {
        return match reader.byte()? {
            1 => append_null(builder, column).map(|()| 1),
            0 => append(inner, reader, builder, column),
            _ => Err(value_error(column, BAD_MARKER)),
        };
    }
    match (ty, builder) {
        (ChType::Nothing, builder) => {
            reader.take(1)?;
            append_null(builder, column).map(|()| 1)
        }
        (ChType::Bool, Builder::Bool(b)) => match reader.byte()? {
            0 => put!(b, false, 1),
            1 => put!(b, true, 1),
            _ => Err(value_error(column, BAD_BOOL)),
        },
        (ChType::Int(8), Builder::I8(b)) => put!(b, i8::from_le_bytes(reader.array()?), 1),
        (ChType::Int(16), Builder::I16(b)) => put!(b, i16::from_le_bytes(reader.array()?), 2),
        (ChType::Int(32), Builder::I32(b)) => put!(b, i32::from_le_bytes(reader.array()?), 4),
        (ChType::Int(64) | ChType::Interval, Builder::I64(b)) => {
            put!(b, i64::from_le_bytes(reader.array()?), 8)
        }
        (ChType::UInt(8), Builder::U8(b)) => put!(b, reader.byte()?, 1),
        (ChType::UInt(16), Builder::U16(b)) => put!(b, u16::from_le_bytes(reader.array()?), 2),
        (ChType::UInt(32), Builder::U32(b)) => put!(b, u32::from_le_bytes(reader.array()?), 4),
        (ChType::UInt(64), Builder::U64(b)) => put!(b, u64::from_le_bytes(reader.array()?), 8),
        (ChType::Int(128), Builder::D256(b)) => {
            put!(b, i256::from_i128(i128::from_le_bytes(reader.array()?)), 32)
        }
        (ChType::UInt(128), Builder::D256(b)) => {
            let low = u128::from_le_bytes(reader.array()?);
            // Zero-extended: the high half of an `i256` is zero for every
            // `u128`, so the value stays positive.
            put!(b, i256::from_parts(low, 0), 32)
        }
        (ChType::Int(256), Builder::Text(b)) => {
            let value = i256::from_le_bytes(reader.array()?);
            write_text(b, format_args!("{value}"))
        }
        (ChType::UInt(256), Builder::Text(b)) => write_unsigned_256(b, reader.array()?),
        (ChType::Float32, Builder::F32(b)) => put!(b, f32::from_le_bytes(reader.array()?), 4),
        (ChType::BFloat16, Builder::F32(b)) => {
            let half = u16::from_le_bytes(reader.array()?);
            put!(b, f32::from_bits(u32::from(half) << 16), 4)
        }
        (ChType::Float64, Builder::F64(b)) => put!(b, f64::from_le_bytes(reader.array()?), 8),
        (ChType::Decimal { precision, .. }, Builder::D128(b)) => {
            let value = match decimal_width(*precision) {
                4 => i128::from(i32::from_le_bytes(reader.array()?)),
                8 => i128::from(i64::from_le_bytes(reader.array()?)),
                _ => i128::from_le_bytes(reader.array()?),
            };
            put!(b, value, 16)
        }
        (ChType::Decimal { .. }, Builder::D256(b)) => {
            put!(b, i256::from_le_bytes(reader.array()?), 32)
        }
        (ChType::String | ChType::Json, Builder::Text(b)) => {
            let bytes = reader.sized(column)?;
            let text = std::str::from_utf8(bytes).map_err(|_| value_error(column, NOT_UTF8))?;
            put!(b, text, text.len().saturating_add(4))
        }
        (ChType::FixedString(width), Builder::Fixed(b)) => {
            b.append_value(reader.take(*width)?)
                .map_err(|error| DecodeError::Assembly(error.to_string()))?;
            Ok(*width)
        }
        (ChType::Uuid, Builder::Text(b)) => {
            let high = u64::from_le_bytes(reader.array()?);
            let low = u64::from_le_bytes(reader.array()?);
            write_text(
                b,
                format_args!(
                    "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                    high >> 32,
                    (high >> 16) & 0xffff,
                    high & 0xffff,
                    low >> 48,
                    low & 0xffff_ffff_ffff
                ),
            )
        }
        (ChType::Ipv4, Builder::Text(b)) => {
            let address = Ipv4Addr::from(u32::from_le_bytes(reader.array()?));
            write_text(b, format_args!("{address}"))
        }
        (ChType::Ipv6, Builder::Text(b)) => {
            let address = Ipv6Addr::from(reader.array::<16>()?);
            write_text(b, format_args!("{address}"))
        }
        (ChType::Enum { bits, labels }, Builder::Text(b)) => {
            let value = if *bits == 8 {
                i16::from(i8::from_le_bytes(reader.array()?))
            } else {
                i16::from_le_bytes(reader.array()?)
            };
            let label = labels
                .iter()
                .find(|(_, declared)| *declared == value)
                .map(|(label, _)| label.as_str())
                .ok_or_else(|| value_error(column, UNDECLARED_LABEL))?;
            put!(b, label, label.len().saturating_add(4))
        }
        (ChType::Date, Builder::Date32(b)) => {
            put!(b, i32::from(u16::from_le_bytes(reader.array()?)), 4)
        }
        (ChType::Date32, Builder::Date32(b)) => put!(b, i32::from_le_bytes(reader.array()?), 4),
        (ChType::DateTime(_), Builder::Ticks { values, .. }) => {
            put!(values, i64::from(u32::from_le_bytes(reader.array()?)), 8)
        }
        (ChType::Time, Builder::Ticks { values, .. }) => {
            put!(values, i64::from(i32::from_le_bytes(reader.array()?)), 8)
        }
        (ChType::DateTime64 { .. } | ChType::Time64(_), Builder::Ticks { values, factor, .. }) => {
            let scaled = i64::from_le_bytes(reader.array()?)
                .checked_mul(*factor)
                .ok_or_else(|| value_error(column, OUT_OF_RANGE))?;
            put!(values, scaled, 8)
        }
        (ChType::Array(inner), Builder::List { offsets, child, .. }) => {
            let count = reader.varint(column)?;
            let mut added = 4_usize;
            for _ in 0..count {
                added = added.saturating_add(append(inner, reader, child, column)?);
            }
            push_offset(offsets, count, column)?;
            Ok(added)
        }
        (ChType::Tuple(elements), Builder::Struct { children, len, .. }) => {
            let mut added = 0_usize;
            for ((_, element), child) in elements.iter().zip(children.iter_mut()) {
                added = added.saturating_add(append(element, reader, child, column)?);
            }
            *len += 1;
            Ok(added)
        }
        (
            ChType::Map(key, value),
            Builder::Map {
                offsets,
                keys,
                values,
                ..
            },
        ) => {
            let count = reader.varint(column)?;
            let mut added = 4_usize;
            for _ in 0..count {
                added = added.saturating_add(append(key, reader, keys, column)?);
                added = added.saturating_add(append(value, reader, values, column)?);
            }
            push_offset(offsets, count, column)?;
            Ok(added)
        }
        _ => Err(value_error(column, "a type and its builder disagree")),
    }
}

/// Appends a value formatted straight into the builder's buffer.
fn write_text(
    builder: &mut StringBuilder,
    value: std::fmt::Arguments<'_>,
) -> Result<usize, DecodeError> {
    let before = builder.values_slice().len();
    builder
        .write_fmt(value)
        .map_err(|_| DecodeError::Assembly("formatting a value".to_owned()))?;
    builder.append_value("");
    Ok(builder
        .values_slice()
        .len()
        .saturating_sub(before)
        .saturating_add(4))
}

/// The decimal text of an unsigned 256-bit integer.
///
/// `i256` cannot hold the upper half of the range, so the digits come from a
/// long division on four 64-bit limbs, into a stack buffer.
fn write_unsigned_256(builder: &mut StringBuilder, raw: [u8; 32]) -> Result<usize, DecodeError> {
    let mut limbs = [0_u64; 4];
    for (limb, chunk) in limbs.iter_mut().zip(raw.as_chunks::<8>().0) {
        *limb = u64::from_le_bytes(*chunk);
    }
    // 2^256 has 78 decimal digits.
    let mut digits = [0_u8; 78];
    let mut start = digits.len();
    loop {
        let mut remainder = 0_u64;
        for limb in limbs.iter_mut().rev() {
            let current = (u128::from(remainder) << 64) | u128::from(*limb);
            // `remainder < 10`, so the quotient fits 64 bits and the new
            // remainder is below 10.
            *limb = u64::try_from(current / 10).map_err(|_| DecodeError::Truncated)?;
            remainder = u64::try_from(current % 10).map_err(|_| DecodeError::Truncated)?;
        }
        start = start.checked_sub(1).ok_or(DecodeError::Truncated)?;
        let digit = u8::try_from(remainder).map_err(|_| DecodeError::Truncated)?;
        *digits.get_mut(start).ok_or(DecodeError::Truncated)? = b'0' + digit;
        if limbs.iter().all(|limb| *limb == 0) {
            break;
        }
    }
    let text = digits
        .get(start..)
        .and_then(|slice| std::str::from_utf8(slice).ok())
        .ok_or(DecodeError::Truncated)?;
    put!(builder, text, text.len().saturating_add(4))
}

fn push_offset(offsets: &mut Vec<i32>, count: u64, column: &str) -> Result<(), DecodeError> {
    let last = offsets.last().copied().unwrap_or(0);
    let next = i32::try_from(count)
        .ok()
        .and_then(|count| last.checked_add(count))
        .ok_or_else(|| value_error(column, TOO_MANY_ITEMS))?;
    offsets.push(next);
    Ok(())
}

fn append_null(builder: &mut Builder, column: &str) -> Result<(), DecodeError> {
    match builder {
        Builder::Bool(b) => b.append_null(),
        Builder::I8(b) => b.append_null(),
        Builder::I16(b) => b.append_null(),
        Builder::I32(b) => b.append_null(),
        Builder::I64(b) => b.append_null(),
        Builder::U8(b) => b.append_null(),
        Builder::U16(b) => b.append_null(),
        Builder::U32(b) => b.append_null(),
        Builder::U64(b) => b.append_null(),
        Builder::F32(b) => b.append_null(),
        Builder::F64(b) => b.append_null(),
        Builder::D128(b) => b.append_null(),
        Builder::D256(b) => b.append_null(),
        Builder::Text(b) => b.append_null(),
        Builder::Fixed(b) => b.append_null(),
        Builder::Date32(b) => b.append_null(),
        Builder::Ticks { values, .. } => values.append_null(),
        Builder::Null(b) => b.append_null(),
        Builder::List { .. } | Builder::Struct { .. } | Builder::Map { .. } => {
            return Err(value_error(column, NULL_NOT_ALLOWED));
        }
    }
    Ok(())
}

fn finish(builder: &mut Builder) -> Result<ArrayRef, DecodeError> {
    let assembly = |error: arrow::error::ArrowError| DecodeError::Assembly(error.to_string());
    Ok(match builder {
        Builder::Bool(b) => Arc::new(b.finish()),
        Builder::I8(b) => Arc::new(b.finish()),
        Builder::I16(b) => Arc::new(b.finish()),
        Builder::I32(b) => Arc::new(b.finish()),
        Builder::I64(b) => Arc::new(b.finish()),
        Builder::U8(b) => Arc::new(b.finish()),
        Builder::U16(b) => Arc::new(b.finish()),
        Builder::U32(b) => Arc::new(b.finish()),
        Builder::U64(b) => Arc::new(b.finish()),
        Builder::F32(b) => Arc::new(b.finish()),
        Builder::F64(b) => Arc::new(b.finish()),
        Builder::D128(b) => Arc::new(b.finish()),
        Builder::D256(b) => Arc::new(b.finish()),
        Builder::Text(b) => Arc::new(b.finish()),
        Builder::Fixed(b) => Arc::new(b.finish()),
        Builder::Date32(b) => Arc::new(b.finish()),
        Builder::Null(b) => Arc::new(b.finish()),
        Builder::Ticks { values, target, .. } => {
            let data = values
                .finish()
                .into_data()
                .into_builder()
                .data_type(target.clone())
                .build()
                .map_err(assembly)?;
            make_array(data)
        }
        Builder::List {
            offsets,
            item,
            child,
        } => {
            let values = finish(child)?;
            let offsets = OffsetBuffer::new(std::mem::replace(offsets, vec![0]).into());
            Arc::new(ListArray::try_new(Arc::clone(item), offsets, values, None).map_err(assembly)?)
        }
        Builder::Struct {
            fields,
            children,
            len,
        } => {
            let arrays = children
                .iter_mut()
                .map(finish)
                .collect::<Result<Vec<_>, _>>()?;
            let rows = std::mem::take(len);
            let array = StructArray::try_new(fields.clone(), arrays, None).map_err(assembly)?;
            if array.len() != rows {
                return Err(DecodeError::Assembly(
                    "a tuple's elements disagree on its length".to_owned(),
                ));
            }
            Arc::new(array)
        }
        Builder::Map {
            offsets,
            entries,
            keys,
            values,
        } => {
            let DataType::Struct(entry_fields) = entries.data_type() else {
                return Err(DecodeError::Assembly("a map without entries".to_owned()));
            };
            let pairs = StructArray::try_new(
                entry_fields.clone(),
                vec![finish(keys)?, finish(values)?],
                None,
            )
            .map_err(assembly)?;
            let offsets = OffsetBuffer::new(std::mem::replace(offsets, vec![0]).into());
            Arc::new(
                MapArray::try_new(Arc::clone(entries), offsets, pairs, None, false)
                    .map_err(assembly)?,
            )
        }
    })
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
