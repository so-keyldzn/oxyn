//! From DuckDB's Arrow export to Oxyn's Arrow, column by column.
//!
//! # Why a copy, and why this one
//!
//! `duckdb-rs` 1.10506.1 exports each DuckDB chunk through the Arrow C data
//! interface into **its** Arrow, version 58; the workspace speaks Arrow 59
//! ([ADR-0002](../../../docs/adr/0002-arrow-result-model.md)). The two
//! crates' types are distinct to the compiler, so no Arrow 58 value can cross
//! this crate's boundary ([ADR-0059](../../../docs/adr/0059-duckdb-driver-on-duckdb-rs-bundled.md)).
//! The rebuild here is safe code: each buffer is copied into an Arrow 59
//! buffer, the layout is rebuilt with [`ArrayData::builder`], and
//! `build()` validates it in full — offsets, lengths, UTF-8, dictionary keys —
//! so a malformed buffer is a [`Decoded`] error, never a panic here. What
//! comes before is trusted: `duckdb-rs` imports the chunk into Arrow 58
//! without validating it, from the engine's memory in this same process.
//!
//! # What the copy fixes
//!
//! A few DuckDB types leave under a carrier that loses what they are
//! ([`crate::logical`]). The logical type of the column, at every nesting
//! level, decides:
//!
//! | DuckDB | DuckDB's carrier | Oxyn's Arrow | Loss |
//! |---|---|---|---|
//! | `HUGEINT` | `Decimal128(38, 0)` | `Decimal256(39, 0)` | none: every `i128` fits 39 digits |
//! | `UHUGEINT` | `Decimal128(38, 0)`, values above `i128::MAX` negative | `Decimal256(39, 0)` | none: the bits are read back as unsigned |
//! | `TIMESTAMP WITH TIME ZONE` | `Timestamp(µs, <session zone>)` | `Timestamp(µs, "UTC")` | none: DuckDB stores the instant in UTC |
//! | `BIT` | DuckDB's internal bytes | `Utf8`, `'0101…'` as DuckDB prints it | none |
//! | `TIME WITH TIME ZONE` | `Time64(µs)` without the offset | refused | the offset; cast to `VARCHAR` |
//! | unknown identifier | — | refused | everything; cast to `VARCHAR` |
//!
//! Every other type is copied with its carrier: integers of every width and
//! sign (a `UBIGINT` above 2⁵³ stays exact), `DECIMAL(w, s)` with its width,
//! `TIMESTAMP` without a zone, `INTERVAL` as month-day-nano, `UUID` and
//! `VARCHAR` as text, `BLOB` as bytes, `GEOMETRY` as WKB with its extension
//! name, `ENUM` as a dictionary, lists, arrays, maps, structs and unions with
//! their children converted the same way. `BIGNUM` keeps DuckDB's binary
//! encoding, its name in the field metadata ([`METADATA_TYPE`]): the bytes are
//! exact, a reader that wants the digits casts to `VARCHAR`.
//!
//! # Batches bounded in bytes
//!
//! DuckDB hands over chunks of up to 2,048 rows. A chunk whose Arrow form
//! exceeds the byte bound is split ([`bounded`]): a thousand rows of one
//! megabyte each must not travel as one gigabyte. A single row larger than
//! the bound travels alone — a row cannot be cut.

use std::sync::Arc;

use arrow::array::{
    Array, ArrayData, ArrayRef, AsArray, Decimal256Array, RecordBatch, StringBuilder, UInt32Array,
    make_array,
};
use arrow::buffer::{BooleanBuffer, Buffer, NullBuffer};
use arrow::datatypes::{
    DataType, Decimal128Type, Decimal256Type, Field, FieldRef, Fields, IntervalUnit, Schema,
    SchemaRef, TimeUnit, UnionFields, UnionMode, i256,
};
use duckdb::arrow as a58;

use crate::logical::DuckType;
use crate::options::BatchLimits;

/// The field metadata key carrying the DuckDB type of a result column.
pub const METADATA_TYPE: &str = "oxyn.duckdb.type";

/// The zone a `TIMESTAMP WITH TIME ZONE` is carried in.
const UTC: &str = "UTC";

/// What a 128-bit DuckDB integer becomes: 39 digits hold both `i128::MIN`
/// and `u128::MAX`.
const WIDE_PRECISION: u8 = 39;

/// A conversion failure, worded for the user.
pub(crate) type Decoded<T> = std::result::Result<T, String>;

/// The Arrow 59 schema of a result, from DuckDB's schema and logical types.
///
/// Refuses a column whose type has no faithful form, before any row is read.
pub(crate) fn target_schema(
    source: &a58::datatypes::Schema,
    types: &[DuckType],
) -> Decoded<SchemaRef> {
    if source.fields().len() != types.len() {
        return Err(format!(
            "DuckDB described {} column(s) and typed {}",
            source.fields().len(),
            types.len()
        ));
    }
    let mut fields = Vec::with_capacity(types.len());
    for (field, ty) in source.fields().iter().zip(types) {
        if let Some(name) = ty.unfaithful() {
            return Err(format!(
                "column `{}` is of type {name}, which has no faithful Arrow form in Oxyn; \
                 cast it to VARCHAR in the query",
                field.name()
            ));
        }
        let mut metadata = field.metadata().clone();
        metadata.insert(METADATA_TYPE.to_owned(), ty.name());
        // Nullable whatever DuckDB declares: a `NULL` where the schema forbids
        // one is data to show, not a batch to refuse.
        fields.push(
            Field::new(field.name(), target_type(field.data_type(), ty)?, true)
                .with_metadata(metadata),
        );
    }
    Ok(Arc::new(Schema::new(fields)))
}

/// Converts one DuckDB chunk into a batch of `schema`.
pub(crate) fn convert_chunk(
    chunk: &a58::array::StructArray,
    schema: &SchemaRef,
    types: &[DuckType],
) -> Decoded<RecordBatch> {
    let columns = chunk.columns();
    if columns.len() != types.len() || columns.len() != schema.fields().len() {
        return Err(format!(
            "DuckDB returned a chunk of {} column(s) for a result of {}",
            columns.len(),
            schema.fields().len()
        ));
    }
    let mut converted: Vec<ArrayRef> = Vec::with_capacity(columns.len());
    for ((column, ty), field) in columns.iter().zip(types).zip(schema.fields()) {
        let data = convert_data(&column.to_data(), ty, field.data_type())?;
        converted.push(make_array(data));
    }
    RecordBatch::try_new(Arc::clone(schema), converted).map_err(|err| err.to_string())
}

/// Splits a batch until every piece fits the byte and row bounds.
///
/// Each piece is a copy of its rows ([`arrow::compute::take`], view columns
/// compacted), so the size it reports is the size it holds — except the
/// values of a dictionary (an `ENUM`), which every piece shares. When halving
/// a piece copies more than it frees, shared values dominate it: it stops
/// being split for its bytes, since each split would count them once more and
/// a one-row piece would weigh the whole dictionary. Order is preserved.
pub(crate) fn bounded(batch: RecordBatch, limits: BatchLimits) -> Decoded<Vec<RecordBatch>> {
    let mut done = Vec::new();
    let mut pending = vec![batch];
    while let Some(piece) = pending.pop() {
        let rows = piece.num_rows();
        let bytes = piece.get_array_memory_size();
        let over_rows = rows > limits.max_rows();
        if rows < 2 || !(over_rows || bytes > limits.max_bytes()) {
            done.push(piece);
            continue;
        }
        let half = rows / 2;
        let right = rows_of(&piece, half, rows - half)?;
        let left = rows_of(&piece, 0, half)?;
        let halves = left
            .get_array_memory_size()
            .saturating_add(right.get_array_memory_size());
        if !over_rows && halves.saturating_mul(2) > bytes.saturating_mul(3) {
            done.push(piece);
            continue;
        }
        pending.push(right);
        pending.push(left);
    }
    Ok(done)
}

/// A compact copy of `len` rows starting at `start`.
fn rows_of(batch: &RecordBatch, start: usize, len: usize) -> Decoded<RecordBatch> {
    let first = u32::try_from(start).map_err(|err| err.to_string())?;
    let count = u32::try_from(len).map_err(|err| err.to_string())?;
    let end = first
        .checked_add(count)
        .ok_or_else(|| "a batch of more than u32::MAX rows".to_owned())?;
    let indices = UInt32Array::from_iter_values(first..end);
    let taken =
        arrow::compute::take_record_batch(batch, &indices).map_err(|err| err.to_string())?;
    compact_views(&taken)
}

/// `take` keeps every data buffer of a view column — `Utf8View`, which DuckDB
/// exports after `SET produce_arrow_string_view = true`; `gc` keeps only the
/// bytes its rows point to.
fn compact_views(batch: &RecordBatch) -> Decoded<RecordBatch> {
    let columns = batch
        .columns()
        .iter()
        .map(|column| {
            if let Some(text) = column.as_string_view_opt() {
                Arc::new(text.gc()) as ArrayRef
            } else if let Some(bytes) = column.as_binary_view_opt() {
                Arc::new(bytes.gc()) as ArrayRef
            } else {
                Arc::clone(column)
            }
        })
        .collect();
    RecordBatch::try_new(batch.schema(), columns).map_err(|err| err.to_string())
}

/// The Arrow 59 type a column of DuckDB type `ty`, exported as `source`,
/// becomes.
fn target_type(source: &a58::datatypes::DataType, ty: &DuckType) -> Decoded<DataType> {
    use a58::datatypes::DataType as S;
    match (ty, source) {
        (DuckType::HugeInt | DuckType::UHugeInt, S::Decimal128(38, 0) | S::FixedSizeBinary(16)) => {
            Ok(DataType::Decimal256(WIDE_PRECISION, 0))
        }
        (DuckType::TimestampTz, S::Timestamp(unit, _)) => {
            Ok(DataType::Timestamp(time_unit(*unit), Some(UTC.into())))
        }
        (DuckType::Bit, S::Binary | S::LargeBinary | S::BinaryView) => Ok(DataType::Utf8),
        (DuckType::HugeInt | DuckType::UHugeInt | DuckType::TimestampTz | DuckType::Bit, _) => {
            Err(carrier(ty, source))
        }
        (DuckType::TimeTz | DuckType::Unknown(_), _) => Err(format!(
            "{} has no faithful Arrow form in Oxyn; cast it to VARCHAR",
            ty.name()
        )),
        _ => same_shape(source, ty),
    }
}

/// The error of a carrier DuckDB is not known to use for a type.
fn carrier(ty: &DuckType, source: &a58::datatypes::DataType) -> String {
    format!(
        "DuckDB exported {} as {source}, a carrier this driver does not read; \
         was `arrow_lossless_conversion` or `arrow_output_version` changed in this session?",
        ty.name()
    )
}

/// The same Arrow type, in Arrow 59, children converted.
fn same_shape(source: &a58::datatypes::DataType, ty: &DuckType) -> Decoded<DataType> {
    use a58::datatypes::DataType as S;
    let children = ty.arrow_children();
    let child = |index: usize| children.get(index).cloned().unwrap_or(DuckType::Plain(""));
    Ok(match source {
        S::Null => DataType::Null,
        S::Boolean => DataType::Boolean,
        S::Int8 => DataType::Int8,
        S::Int16 => DataType::Int16,
        S::Int32 => DataType::Int32,
        S::Int64 => DataType::Int64,
        S::UInt8 => DataType::UInt8,
        S::UInt16 => DataType::UInt16,
        S::UInt32 => DataType::UInt32,
        S::UInt64 => DataType::UInt64,
        S::Float16 => DataType::Float16,
        S::Float32 => DataType::Float32,
        S::Float64 => DataType::Float64,
        S::Timestamp(unit, zone) => DataType::Timestamp(time_unit(*unit), zone.clone()),
        S::Date32 => DataType::Date32,
        S::Date64 => DataType::Date64,
        S::Time32(unit) => DataType::Time32(time_unit(*unit)),
        S::Time64(unit) => DataType::Time64(time_unit(*unit)),
        S::Duration(unit) => DataType::Duration(time_unit(*unit)),
        S::Interval(unit) => DataType::Interval(interval_unit(*unit)),
        S::Binary => DataType::Binary,
        S::FixedSizeBinary(size) => DataType::FixedSizeBinary(*size),
        S::LargeBinary => DataType::LargeBinary,
        S::BinaryView => DataType::BinaryView,
        S::Utf8 => DataType::Utf8,
        S::LargeUtf8 => DataType::LargeUtf8,
        S::Utf8View => DataType::Utf8View,
        S::List(field) => DataType::List(field_of(field, &child(0))?),
        S::ListView(field) => DataType::ListView(field_of(field, &child(0))?),
        S::FixedSizeList(field, size) => {
            DataType::FixedSizeList(field_of(field, &child(0))?, *size)
        }
        S::LargeList(field) => DataType::LargeList(field_of(field, &child(0))?),
        S::LargeListView(field) => DataType::LargeListView(field_of(field, &child(0))?),
        S::Struct(fields) => DataType::Struct(fields_of(fields.iter(), &children)?),
        S::Union(fields, mode) => {
            let ids: Vec<i8> = fields.iter().map(|(id, _)| id).collect();
            let converted = fields_of(fields.iter().map(|(_, field)| field), &children)?;
            let union = UnionFields::try_new(ids, converted.iter().cloned())
                .map_err(|err| err.to_string())?;
            DataType::Union(union, union_mode(*mode))
        }
        S::Dictionary(key, value) => DataType::Dictionary(
            Box::new(same_shape(key, &DuckType::Plain(""))?),
            Box::new(target_type(value, &child(0))?),
        ),
        S::Decimal32(precision, scale) => DataType::Decimal32(*precision, *scale),
        S::Decimal64(precision, scale) => DataType::Decimal64(*precision, *scale),
        S::Decimal128(precision, scale) => DataType::Decimal128(*precision, *scale),
        S::Decimal256(precision, scale) => DataType::Decimal256(*precision, *scale),
        S::Map(field, sorted) => DataType::Map(field_of(field, &child(0))?, *sorted),
        S::RunEndEncoded(..) => {
            return Err(format!(
                "DuckDB exported {} as a run-end encoded array, which this driver does not read",
                ty.name()
            ));
        }
    })
}

/// A field, converted, with its name, nullability and metadata.
fn field_of(field: &a58::datatypes::FieldRef, ty: &DuckType) -> Decoded<FieldRef> {
    Ok(Arc::new(
        Field::new(
            field.name(),
            target_type(field.data_type(), ty)?,
            field.is_nullable(),
        )
        .with_metadata(field.metadata().clone()),
    ))
}

/// Fields paired with the logical types of the same members.
fn fields_of<'a>(
    fields: impl Iterator<Item = &'a a58::datatypes::FieldRef>,
    types: &[DuckType],
) -> Decoded<Fields> {
    let mut converted = Vec::new();
    for (index, field) in fields.enumerate() {
        let ty = types.get(index).cloned().unwrap_or(DuckType::Plain(""));
        converted.push(field_of(field, &ty)?);
    }
    Ok(Fields::from(converted))
}

const fn time_unit(unit: a58::datatypes::TimeUnit) -> TimeUnit {
    match unit {
        a58::datatypes::TimeUnit::Second => TimeUnit::Second,
        a58::datatypes::TimeUnit::Millisecond => TimeUnit::Millisecond,
        a58::datatypes::TimeUnit::Microsecond => TimeUnit::Microsecond,
        a58::datatypes::TimeUnit::Nanosecond => TimeUnit::Nanosecond,
    }
}

const fn interval_unit(unit: a58::datatypes::IntervalUnit) -> IntervalUnit {
    match unit {
        a58::datatypes::IntervalUnit::YearMonth => IntervalUnit::YearMonth,
        a58::datatypes::IntervalUnit::DayTime => IntervalUnit::DayTime,
        a58::datatypes::IntervalUnit::MonthDayNano => IntervalUnit::MonthDayNano,
    }
}

const fn union_mode(mode: a58::datatypes::UnionMode) -> UnionMode {
    match mode {
        a58::datatypes::UnionMode::Sparse => UnionMode::Sparse,
        a58::datatypes::UnionMode::Dense => UnionMode::Dense,
    }
}

/// The children types of an Arrow 59 type, in `child_data` order.
fn child_types(target: &DataType) -> Vec<DataType> {
    match target {
        DataType::List(field)
        | DataType::ListView(field)
        | DataType::FixedSizeList(field, _)
        | DataType::LargeList(field)
        | DataType::LargeListView(field)
        | DataType::Map(field, _) => vec![field.data_type().clone()],
        DataType::Struct(fields) => fields.iter().map(|f| f.data_type().clone()).collect(),
        DataType::Union(fields, _) => fields.iter().map(|(_, f)| f.data_type().clone()).collect(),
        DataType::Dictionary(_, value) => vec![(**value).clone()],
        DataType::RunEndEncoded(ends, values) => {
            vec![ends.data_type().clone(), values.data_type().clone()]
        }
        _ => Vec::new(),
    }
}

/// Rebuilds one column's data as `target`.
fn convert_data(
    source: &a58::array::ArrayData,
    ty: &DuckType,
    target: &DataType,
) -> Decoded<ArrayData> {
    match ty {
        DuckType::HugeInt => return wide_integer(source, false),
        DuckType::UHugeInt => return wide_integer(source, true),
        DuckType::Bit => return bit_strings(source),
        _ => {}
    }
    let types = ty.arrow_children();
    let targets = child_types(target);
    if source.child_data().len() != targets.len() {
        return Err(format!(
            "DuckDB exported {} with {} child array(s) where {} were expected",
            ty.name(),
            source.child_data().len(),
            targets.len()
        ));
    }
    let mut children = Vec::with_capacity(targets.len());
    for (index, (child, target)) in source.child_data().iter().zip(&targets).enumerate() {
        let child_type = types.get(index).cloned().unwrap_or(DuckType::Plain(""));
        children.push(convert_data(child, &child_type, target)?);
    }
    copy(source, target.clone(), children)
}

/// The same layout, buffers copied into Arrow 59 and validated.
fn copy(
    source: &a58::array::ArrayData,
    target: DataType,
    children: Vec<ArrayData>,
) -> Decoded<ArrayData> {
    let buffers = source
        .buffers()
        .iter()
        .map(|buffer| Buffer::from_slice_ref(buffer.as_slice()))
        .collect();
    let nulls = match source.nulls() {
        None => None,
        Some(nulls) => {
            let bytes = nulls.buffer().as_slice();
            let fits = nulls
                .offset()
                .checked_add(nulls.len())
                .zip(bytes.len().checked_mul(8))
                .is_some_and(|(end, bits)| end <= bits);
            if !fits {
                return Err("DuckDB returned a validity bitmap shorter than its array".to_owned());
            }
            Some(NullBuffer::new(BooleanBuffer::new(
                Buffer::from_slice_ref(bytes),
                nulls.offset(),
                nulls.len(),
            )))
        }
    };
    ArrayData::builder(target)
        .len(source.len())
        .offset(source.offset())
        .nulls(nulls)
        .buffers(buffers)
        .child_data(children)
        .build()
        .map_err(|err| format!("DuckDB returned an array Oxyn cannot validate: {err}"))
}

/// A `HUGEINT` or `UHUGEINT` column as `Decimal256(39, 0)`.
///
/// From DuckDB's default carrier, `Decimal128(38, 0)` holding the 128 bits —
/// read back as unsigned for a `UHUGEINT` —, or from the 16-byte little-endian
/// form `arrow_lossless_conversion` selects.
fn wide_integer(source: &a58::array::ArrayData, unsigned: bool) -> Decoded<ArrayData> {
    use a58::datatypes::DataType as S;
    let widen = move |value: i128| {
        if unsigned {
            i256::from_parts(u128::from_le_bytes(value.to_le_bytes()), 0)
        } else {
            i256::from_i128(value)
        }
    };
    let widened: Decimal256Array = match source.data_type() {
        S::Decimal128(38, 0) => {
            let narrow = make_array(copy(source, DataType::Decimal128(38, 0), Vec::new())?);
            let Some(narrow) = narrow.as_primitive_opt::<Decimal128Type>() else {
                return Err("a Decimal128 column did not read as Decimal128".to_owned());
            };
            narrow.unary::<_, Decimal256Type>(widen)
        }
        S::FixedSizeBinary(16) => {
            let bytes = make_array(copy(source, DataType::FixedSizeBinary(16), Vec::new())?);
            let Some(bytes) = bytes.as_fixed_size_binary_opt() else {
                return Err("a 16-byte column did not read as fixed-size binary".to_owned());
            };
            let mut values = Vec::with_capacity(bytes.len());
            for value in bytes.iter() {
                values.push(match value {
                    None => None,
                    Some(raw) => {
                        let raw: [u8; 16] = raw
                            .try_into()
                            .map_err(|_| "a 128-bit integer of the wrong width".to_owned())?;
                        Some(widen(i128::from_le_bytes(raw)))
                    }
                });
            }
            Decimal256Array::from(values)
        }
        other => {
            return Err(format!(
                "DuckDB exported a 128-bit integer as {other}, a carrier this driver does not read"
            ));
        }
    };
    widened
        .with_precision_and_scale(WIDE_PRECISION, 0)
        .map(|array| array.into_data())
        .map_err(|err| err.to_string())
}

/// A `BIT` column as text, `'0101…'`, the way DuckDB prints it.
///
/// DuckDB's bytes: the first holds the number of padding bits (0–7) at the
/// head of the second; the bits follow, most significant first (`Bit::ToString`,
/// `src/common/types/bit.cpp`, v1.5.6).
fn bit_strings(source: &a58::array::ArrayData) -> Decoded<ArrayData> {
    use a58::datatypes::DataType as S;
    let target = match source.data_type() {
        S::Binary => DataType::Binary,
        S::LargeBinary => DataType::LargeBinary,
        S::BinaryView => DataType::BinaryView,
        other => return Err(format!("DuckDB exported BIT as {other}")),
    };
    let array = make_array(copy(source, target, Vec::new())?);
    let values: Vec<Option<&[u8]>> = if let Some(binary) = array.as_binary_opt::<i32>() {
        binary.iter().collect()
    } else if let Some(binary) = array.as_binary_opt::<i64>() {
        binary.iter().collect()
    } else if let Some(binary) = array.as_binary_view_opt() {
        binary.iter().collect()
    } else {
        return Err("a BIT column did not read as binary".to_owned());
    };
    let mut builder = StringBuilder::with_capacity(values.len(), values.len().saturating_mul(8));
    let mut text = String::new();
    for value in values {
        match value {
            None => builder.append_null(),
            Some(bytes) => {
                bits_of(bytes, &mut text)?;
                builder.append_value(&text);
            }
        }
    }
    Ok(builder.finish().into_data())
}

/// Writes the bits of one DuckDB `BIT` value into `text`.
fn bits_of(bytes: &[u8], text: &mut String) -> Decoded<()> {
    text.clear();
    let Some((&padding, data)) = bytes.split_first() else {
        return Err("a BIT value without its padding byte".to_owned());
    };
    if padding > 7 {
        return Err(format!("a BIT value with {padding} padding bits"));
    }
    for (index, byte) in data.iter().enumerate() {
        let skip = if index == 0 { padding } else { 0 };
        for bit in skip..8 {
            text.push(if byte & (0x80 >> bit) == 0 { '0' } else { '1' });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_read_like_duckdb_prints_them() {
        let mut text = String::new();
        // `'10101'::BIT`: three padding bits, then 10101.
        bits_of(&[3, 0b1111_0101], &mut text).expect("valid bits");
        assert_eq!(text, "10101");
        bits_of(&[0, 0b1000_0000, 0b0000_0001], &mut text).expect("valid bits");
        assert_eq!(text, "1000000000000001");
        assert!(bits_of(&[], &mut text).is_err());
        assert!(bits_of(&[8, 0], &mut text).is_err());
    }

    #[test]
    fn a_batch_over_the_byte_bound_is_split_without_losing_order() {
        let values: Vec<String> = (0..64).map(|i| format!("{i:0>1000}")).collect();
        let column: ArrayRef = Arc::new(arrow::array::StringArray::from(values));
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, true)]));
        let batch = RecordBatch::try_new(schema, vec![column]).expect("batch");
        let limits = BatchLimits::new().with_max_bytes(10_000);
        let pieces = bounded(batch, limits).expect("split");
        assert!(pieces.len() > 1);
        let mut next = 0_usize;
        for piece in &pieces {
            assert!(
                piece.num_rows() == 1 || piece.get_array_memory_size() <= 10_000,
                "{} bytes",
                piece.get_array_memory_size()
            );
            let strings = piece.column(0).as_string::<i32>();
            for value in strings.iter() {
                assert_eq!(value, Some(format!("{next:0>1000}").as_str()));
                next += 1;
            }
        }
        assert_eq!(next, 64);
    }

    #[test]
    fn a_row_over_the_bound_travels_alone() {
        let column: ArrayRef = Arc::new(arrow::array::StringArray::from(vec!["x".repeat(4096)]));
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, true)]));
        let batch = RecordBatch::try_new(schema, vec![column]).expect("batch");
        let pieces = bounded(batch, BatchLimits::new().with_max_bytes(16)).expect("split");
        assert_eq!(pieces.len(), 1);
    }

    #[test]
    fn a_dictionary_over_the_bound_is_not_split_row_by_row() {
        // An `ENUM` whose values outweigh the bound: every piece would carry
        // them, and 2,048 one-row pieces would count them 2,048 times.
        let values = arrow::array::StringArray::from(
            (0..512).map(|i| format!("{i:0>1000}")).collect::<Vec<_>>(),
        );
        let keys = arrow::array::UInt16Array::from_iter_values((0..2048).map(|i| i % 512));
        let column: ArrayRef = Arc::new(
            arrow::array::DictionaryArray::try_new(keys, Arc::new(values)).expect("dictionary"),
        );
        let schema = Arc::new(Schema::new(vec![Field::new(
            "v",
            column.data_type().clone(),
            true,
        )]));
        let batch = RecordBatch::try_new(schema, vec![column]).expect("batch");
        let limits = BatchLimits::new().with_max_bytes(100_000);
        let pieces = bounded(batch, limits).expect("split");
        assert_eq!(pieces.len(), 1);
        assert_eq!(
            pieces.iter().map(RecordBatch::num_rows).sum::<usize>(),
            2048
        );
    }

    #[test]
    fn a_view_column_is_split_by_the_bytes_of_its_rows() {
        let values: Vec<String> = (0..64).map(|i| format!("{i:0>1000}")).collect();
        let column: ArrayRef = Arc::new(arrow::array::StringViewArray::from_iter_values(
            values.iter(),
        ));
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8View, true)]));
        let batch = RecordBatch::try_new(schema, vec![column]).expect("batch");
        let pieces = bounded(batch, BatchLimits::new().with_max_bytes(10_000)).expect("split");
        assert!(pieces.len() > 1);
        for piece in &pieces {
            assert!(
                piece.num_rows() == 1 || piece.get_array_memory_size() <= 10_000,
                "{} bytes",
                piece.get_array_memory_size()
            );
        }
        assert_eq!(pieces.iter().map(RecordBatch::num_rows).sum::<usize>(), 64);
    }
}
