//! The ClickHouse → Arrow mapping, and what it loses (ADR-0060).
//!
//! | ClickHouse | Arrow | Loss |
//! |---|---|---|
//! | `Bool` | `Boolean` | none |
//! | `Int8` … `Int64`, `UInt8` … `UInt64` | the same width and sign | none: a `UInt64` above 2^53 stays an integer |
//! | `Int128`, `UInt128` | `Decimal256(39, 0)` | none: every value fits 39 digits |
//! | `Int256`, `UInt256` | `Utf8`, the exact decimal digits | the column sorts as text; 77 and 78 digits exceed `Decimal256`'s 76 |
//! | `Float32`, `Float64` | the same | none |
//! | `BFloat16` | `Float32` | none: widening is exact |
//! | `Decimal(P, S)` | `Decimal128(P, S)` up to P = 38, `Decimal256(P, S)` up to 76 | none, never a float |
//! | `String`, `LowCardinality(String)` | `Utf8` | a value that is not UTF-8 fails the result, naming the column |
//! | `FixedString(N)` | `FixedSizeBinary(N)` | none: the server's padding is kept |
//! | `JSON` | `Utf8`, the server's own JSON text | the server's typed paths become text |
//! | `UUID` | `Utf8`, canonical lowercase | the column sorts as text |
//! | `IPv4`, `IPv6` | `Utf8`, the address as the server prints it | the column sorts as text |
//! | `Enum8`, `Enum16` | `Utf8`, the label | the numeric value |
//! | `Date`, `Date32` | `Date32` | none |
//! | `DateTime[(tz)]` | `Timestamp(Second, tz)` | none: see below for the zone |
//! | `DateTime64(P[, tz])` | `Timestamp(unit, tz)`, unit the smallest of s/ms/µs/ns holding P digits | none: a value scaled past `i64` fails the result |
//! | `Time`, `Time64(P)` | `Duration(unit)` | none: a `Time` is ±999:59:59, not a time of day |
//! | `Interval…` | `Int64`, the unit in the column's metadata | the unit is only in the metadata |
//! | `Nullable(T)` | `T`, nullable | none |
//! | `Array(T)`, `Nested(…)` | `List` | none |
//! | `Tuple(…)` | `Struct`, unnamed elements named `1`, `2`… as ClickHouse names them | none |
//! | `Map(K, V)` | `Map` | none |
//! | `Point` … `MultiPolygon` | the tuples and arrays they stand for | the geometry's name is only in the metadata |
//! | `Variant`, `Dynamic`, `AggregateFunction`, `Object`, any other | **refused** | see [`crate::types`]: the wire gives no length to skip them |
//!
//! **Time zones are the server's, never the workstation's.** A column declaring
//! a zone carries it. A `DateTime` that declares none is still an instant — the
//! server stores seconds since the epoch — and ClickHouse renders it in the
//! zone of the query, which the HTTP response names in `X-ClickHouse-Timezone`:
//! the column carries that zone, so the grid shows what `clickhouse-client`
//! shows ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)). A
//! `DateTime64` with no zone is the same instant type, and gets the same rule.

use std::collections::HashMap;
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Fields, Schema, SchemaRef, TimeUnit};
use oxyn_catalog::model::LogicalType;

use crate::types::ChType;

/// The field metadata key carrying the server's own type name.
pub const META_CLICKHOUSE_TYPE: &str = "oxyn:clickhouse_type";

/// A result column: its name, the server's type text, and the parsed type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The name the server gives the column. Hostile text.
    pub name: String,
    /// The type as the server spelled it, kept for the metadata and messages.
    pub raw_type: String,
    /// The type the decoder walks.
    pub ty: ChType,
}

/// The Arrow schema of a result.
///
/// `zone` is the zone of the query, from the response's
/// `X-ClickHouse-Timezone`, given to every instant column that declares none.
#[must_use]
pub fn schema_for(columns: &[Column], zone: Option<&str>) -> SchemaRef {
    let fields: Vec<Field> = columns
        .iter()
        .map(|column| {
            let metadata =
                HashMap::from([(META_CLICKHOUSE_TYPE.to_owned(), column.raw_type.clone())]);
            field(&column.name, &column.ty, zone).with_metadata(metadata)
        })
        .collect();
    Arc::new(Schema::new(fields))
}

/// The Arrow field of one value of `ty`.
pub(crate) fn field(name: &str, ty: &ChType, zone: Option<&str>) -> Field {
    let (inner, nullable) = match ty {
        ChType::Nullable(inner) => (&**inner, true),
        other => (other, other.is_nullable()),
    };
    Field::new(name, data_type(inner, zone), nullable)
}

/// The Arrow type of a non-`Nullable` ClickHouse type.
pub(crate) fn data_type(ty: &ChType, zone: Option<&str>) -> DataType {
    match ty {
        ChType::Bool => DataType::Boolean,
        ChType::Int(8) => DataType::Int8,
        ChType::Int(16) => DataType::Int16,
        ChType::Int(32) => DataType::Int32,
        ChType::Int(64) => DataType::Int64,
        ChType::UInt(8) => DataType::UInt8,
        ChType::UInt(16) => DataType::UInt16,
        ChType::UInt(32) => DataType::UInt32,
        ChType::UInt(64) => DataType::UInt64,
        ChType::Int(128) | ChType::UInt(128) => DataType::Decimal256(39, 0),
        ChType::Float32 | ChType::BFloat16 => DataType::Float32,
        ChType::Float64 => DataType::Float64,
        ChType::Decimal { precision, scale } => decimal(*precision, *scale),
        ChType::FixedString(width) => {
            i32::try_from(*width).map_or(DataType::Null, DataType::FixedSizeBinary)
        }
        ChType::Date | ChType::Date32 => DataType::Date32,
        ChType::DateTime(declared) => {
            DataType::Timestamp(TimeUnit::Second, instant_zone(declared.as_deref(), zone))
        }
        ChType::DateTime64 {
            precision,
            zone: declared,
        } => DataType::Timestamp(unit_of(*precision), instant_zone(declared.as_deref(), zone)),
        ChType::Time => DataType::Duration(TimeUnit::Second),
        ChType::Time64(precision) => DataType::Duration(unit_of(*precision)),
        ChType::Interval => DataType::Int64,
        ChType::Nothing => DataType::Null,
        ChType::Array(inner) => DataType::List(Arc::new(field("item", inner, zone))),
        ChType::Tuple(elements) => DataType::Struct(tuple_fields(elements, zone)),
        ChType::Map(key, value) => DataType::Map(
            Arc::new(Field::new(
                "entries",
                DataType::Struct(Fields::from(vec![
                    field("keys", key, zone),
                    field("values", value, zone),
                ])),
                false,
            )),
            false,
        ),
        // Text: `String`, `JSON`, and every type rendered as text — the
        // 256-bit integers, `UUID`, the addresses, the enum labels.
        ChType::String
        | ChType::Json
        | ChType::Int(_)
        | ChType::UInt(_)
        | ChType::Uuid
        | ChType::Ipv4
        | ChType::Ipv6
        | ChType::Enum { .. } => DataType::Utf8,
        // A nested `Nullable` (inside an array or a tuple) is handled by
        // `field`; an unsupported type never reaches a schema, the result is
        // refused first.
        ChType::Nullable(inner) => data_type(inner, zone),
        ChType::Unsupported(_) => DataType::Null,
    }
}

/// The struct fields of a tuple: its names, or `1`, `2`… as ClickHouse names
/// the elements of an unnamed tuple.
fn tuple_fields(elements: &[(Option<String>, ChType)], zone: Option<&str>) -> Fields {
    elements
        .iter()
        .enumerate()
        .map(|(rank, (name, ty))| {
            let name = name
                .clone()
                .unwrap_or_else(|| rank.saturating_add(1).to_string());
            field(&name, ty, zone)
        })
        .collect()
}

fn decimal(precision: u8, scale: u8) -> DataType {
    // `scale <= precision <= 76` is checked by the parser, so both casts to
    // `i8` hold.
    let scale = i8::try_from(scale).unwrap_or(i8::MAX);
    if precision <= 38 {
        DataType::Decimal128(precision, scale)
    } else {
        DataType::Decimal256(precision, scale)
    }
}

/// The zone of an instant column: the one it declares, else the query's.
fn instant_zone(declared: Option<&str>, query: Option<&str>) -> Option<Arc<str>> {
    declared.or(query).map(Arc::from)
}

/// The Arrow unit holding `precision` fractional digits.
pub(crate) const fn unit_of(precision: u8) -> TimeUnit {
    match precision {
        0 => TimeUnit::Second,
        1..=3 => TimeUnit::Millisecond,
        4..=6 => TimeUnit::Microsecond,
        _ => TimeUnit::Nanosecond,
    }
}

/// By how much a tick of `precision` digits is multiplied to reach
/// [`unit_of`]'s unit: `DateTime64(2)` counts hundredths, Arrow milliseconds.
pub(crate) const fn tick_factor(precision: u8) -> i64 {
    match precision {
        1 | 4 | 7 => 100,
        2 | 5 | 8 => 10,
        _ => 1,
    }
}

/// The logical type the catalog shows for a column type.
#[must_use]
pub fn logical_type(ty: &ChType) -> LogicalType {
    match ty {
        ChType::Bool => LogicalType::Boolean,
        ChType::Int(bits) | ChType::UInt(bits) => match u8::try_from(*bits) {
            Ok(bits) if bits <= 64 => LogicalType::Integer { bits },
            _ => LogicalType::Decimal {
                precision: None,
                scale: Some(0),
            },
        },
        ChType::Float32 | ChType::BFloat16 => LogicalType::Float { bits: 32 },
        ChType::Float64 => LogicalType::FLOAT64,
        ChType::Decimal { precision, scale } => LogicalType::Decimal {
            precision: Some(u16::from(*precision)),
            scale: Some(i16::from(*scale)),
        },
        ChType::String | ChType::Enum { .. } | ChType::Ipv4 | ChType::Ipv6 => LogicalType::Text,
        ChType::FixedString(_) => LogicalType::Bytes,
        ChType::Json => LogicalType::Json,
        ChType::Uuid => LogicalType::Uuid,
        ChType::Date | ChType::Date32 => LogicalType::Date,
        ChType::DateTime(_) | ChType::DateTime64 { .. } => LogicalType::TIMESTAMPTZ,
        ChType::Time | ChType::Time64(_) | ChType::Interval => LogicalType::Interval,
        ChType::Nullable(inner) => logical_type(inner),
        ChType::Array(inner) => LogicalType::Array(Box::new(logical_type(inner))),
        ChType::Tuple(_) | ChType::Map(..) => LogicalType::Struct(Vec::new()),
        ChType::Nothing | ChType::Unsupported(_) => LogicalType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::types::parse;

    fn mapped(text: &str, zone: Option<&str>) -> DataType {
        data_type(&parse(text).expect("a valid type"), zone)
    }

    #[rstest]
    #[case("UInt64", DataType::UInt64)]
    #[case("Int128", DataType::Decimal256(39, 0))]
    #[case("UInt256", DataType::Utf8)]
    #[case("Decimal(38, 10)", DataType::Decimal128(38, 10))]
    #[case("Decimal(76, 30)", DataType::Decimal256(76, 30))]
    #[case("Decimal32(2)", DataType::Decimal128(9, 2))]
    #[case("BFloat16", DataType::Float32)]
    #[case("UUID", DataType::Utf8)]
    #[case("Enum8('a' = 1)", DataType::Utf8)]
    #[case("FixedString(4)", DataType::FixedSizeBinary(4))]
    #[case("Date", DataType::Date32)]
    #[case("Time", DataType::Duration(TimeUnit::Second))]
    #[case("Time64(3)", DataType::Duration(TimeUnit::Millisecond))]
    #[case("IntervalDay", DataType::Int64)]
    fn scalar_types_map_without_a_float_or_an_invented_value(
        #[case] text: &str,
        #[case] expected: DataType,
    ) {
        assert_eq!(mapped(text, None), expected, "{text}");
    }

    #[test]
    fn an_instant_takes_its_declared_zone_else_the_query_zone_never_the_workstation() {
        assert_eq!(
            mapped("DateTime('Asia/Tokyo')", Some("UTC")),
            DataType::Timestamp(TimeUnit::Second, Some("Asia/Tokyo".into()))
        );
        assert_eq!(
            mapped("DateTime", Some("Europe/Paris")),
            DataType::Timestamp(TimeUnit::Second, Some("Europe/Paris".into()))
        );
        assert_eq!(
            mapped("DateTime64(9)", Some("UTC")),
            DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into()))
        );
        // Without a zone from the response, no zone is made up.
        assert_eq!(
            mapped("DateTime", None),
            DataType::Timestamp(TimeUnit::Second, None)
        );
    }

    #[rstest]
    #[case(0, TimeUnit::Second, 1)]
    #[case(1, TimeUnit::Millisecond, 100)]
    #[case(2, TimeUnit::Millisecond, 10)]
    #[case(3, TimeUnit::Millisecond, 1)]
    #[case(5, TimeUnit::Microsecond, 10)]
    #[case(9, TimeUnit::Nanosecond, 1)]
    fn fractional_ticks_scale_to_the_arrow_unit(
        #[case] precision: u8,
        #[case] unit: TimeUnit,
        #[case] factor: i64,
    ) {
        assert_eq!(unit_of(precision), unit);
        assert_eq!(tick_factor(precision), factor);
    }

    #[test]
    fn nullability_and_nesting_follow_the_type() {
        let ty = parse("Array(Nullable(Int32))").expect("valid");
        let field = field("a", &ty, None);
        assert!(!field.is_nullable(), "an array is never NULL in ClickHouse");
        let DataType::List(item) = field.data_type() else {
            panic!("a list: {field:?}");
        };
        assert!(item.is_nullable());
        assert_eq!(item.data_type(), &DataType::Int32);

        let DataType::Struct(elements) = mapped("Tuple(UInt8, s String)", None) else {
            panic!("a struct");
        };
        let names: Vec<&str> = elements.iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, ["1", "s"]);

        assert!(matches!(
            mapped("Map(String, UInt64)", None),
            DataType::Map(..)
        ));
    }

    #[test]
    fn the_server_type_travels_in_the_field_metadata() {
        let columns = [Column {
            name: "u".into(),
            raw_type: "LowCardinality(Nullable(String))".into(),
            ty: parse("LowCardinality(Nullable(String))").expect("valid"),
        }];
        let schema = schema_for(&columns, None);
        let field = schema.field(0);
        assert!(field.is_nullable());
        assert_eq!(
            field
                .metadata()
                .get(META_CLICKHOUSE_TYPE)
                .map(String::as_str),
            Some("LowCardinality(Nullable(String))")
        );
    }

    #[test]
    fn the_catalog_never_calls_a_wide_integer_a_float() {
        assert_eq!(
            logical_type(&ChType::UInt(64)),
            LogicalType::Integer { bits: 64 }
        );
        assert!(matches!(
            logical_type(&ChType::Int(256)),
            LogicalType::Decimal { .. }
        ));
        assert_eq!(
            logical_type(&parse("Nullable(DateTime64(3))").expect("valid")),
            LogicalType::TIMESTAMPTZ
        );
    }
}
