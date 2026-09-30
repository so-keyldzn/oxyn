//! The MySQL → Arrow mapping, and its losses (ADR-0050 §10).
//!
//! Decided from the column definition the server sends with every result —
//! type code, flags, length, `decimals`, character set — before the first row.
//! **No type fails a result**: a code the driver does not know becomes
//! `Binary` with the code in [`META_MYSQL_TYPE`].
//!
//! | MySQL / MariaDB | Arrow | Loss, or why |
//! |---|---|---|
//! | `TINYINT` … `BIGINT` | `Int8` … `Int64`, `UInt8` … `UInt64` when `UNSIGNED` | none. `TINYINT(1)` stays `Int8`: it holds up to 127 |
//! | `MEDIUMINT` | `Int32` / `UInt32` | none |
//! | `FLOAT`, `DOUBLE` | `Float32`, `Float64` | none |
//! | `DECIMAL(p, s)` | `Decimal128(p, s)` up to 38, `Decimal256(p, s)` beyond | none; a definition the driver cannot read back into `(p, s)` is `Utf8`, marked |
//! | `DATE` | `Date32` | zero and partial dates refused, see [`crate::decode`] |
//! | `DATETIME` | `Timestamp(µs, None)` | none; no time zone invented |
//! | `TIMESTAMP` | `Timestamp(µs, "UTC")` | none; the session runs at `+00:00` |
//! | `TIME` | `Duration(µs)` | none; ±838:59:59 is not a time of day |
//! | `YEAR` | `UInt16` | none |
//! | `BIT(n)` | `UInt64` | none |
//! | text types, `ENUM`, `SET` | `Utf8`, or `Binary` in the `binary` character set | none |
//! | `JSON` | `Utf8` | none, although MySQL announces it in `binary` |
//! | `BINARY`, `VARBINARY`, `BLOB` | `Binary` | none |
//! | `GEOMETRY` | `Binary`, marked | the server's SRID then WKB, not decoded |
//! | `VECTOR` | `Binary`, marked on MySQL | packed `f32`, not decoded; MariaDB's cannot be told from `VARBINARY` |
//! | anything else | `Binary`, marked with the code | the bytes as sent |
//!
//! In the other direction — bound parameters — see `crate::params`.

use std::collections::HashMap;
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use mysql_async::Column;
use mysql_async::consts::{ColumnFlags, ColumnType};

/// The field metadata key carrying the server's type.
pub const META_MYSQL_TYPE: &str = "oxyn:mysql_type";
/// The field metadata key saying the column is shown as the server's text
/// rather than in its own type — the key the PostgreSQL driver uses too.
pub const META_FALLBACK: &str = "oxyn:fallback";

/// MySQL's `binary` character set: the one that makes a string type bytes.
const BINARY_CHARSET: u16 = 63;
/// The largest precision `Decimal128` holds.
const DECIMAL128_MAX_PRECISION: u8 = 38;
/// The largest precision `Decimal256` holds.
const DECIMAL256_MAX_PRECISION: u8 = 76;

/// How a column's values become Arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MyDecoding {
    /// Signed integers.
    Int8,
    /// Signed 16 bits.
    Int16,
    /// Signed 32 bits.
    Int32,
    /// Signed 64 bits.
    Int64,
    /// Unsigned 8 bits.
    UInt8,
    /// Unsigned 16 bits; also `YEAR`.
    UInt16,
    /// Unsigned 32 bits.
    UInt32,
    /// Unsigned 64 bits.
    UInt64,
    /// `BIT(n)`: big-endian bytes into an unsigned integer.
    Bit,
    /// `FLOAT`.
    Float32,
    /// `DOUBLE`.
    Float64,
    /// `DECIMAL` up to 38 digits.
    Decimal128 {
        /// Precision.
        precision: u8,
        /// Scale.
        scale: i8,
    },
    /// `DECIMAL` beyond 38 digits.
    Decimal256 {
        /// Precision.
        precision: u8,
        /// Scale.
        scale: i8,
    },
    /// `DATE`.
    Date,
    /// `DATETIME`: no time zone.
    DateTime,
    /// `TIMESTAMP`: UTC, the session's time zone.
    Timestamp,
    /// `TIME`: a signed duration.
    Time,
    /// Text in the connection's character set, `JSON` included.
    Text,
    /// Bytes as sent.
    Binary,
}

/// The server's column type, and the decoding plan, of a result.
#[must_use]
pub fn schema_for(columns: &[Column]) -> (SchemaRef, Vec<MyDecoding>) {
    let mut fields = Vec::with_capacity(columns.len());
    let mut decodings = Vec::with_capacity(columns.len());
    for column in columns {
        let (field, decoding) = field_for(column);
        fields.push(field);
        decodings.push(decoding);
    }
    (Arc::new(Schema::new(fields)), decodings)
}

/// One column's field and decoding.
fn field_for(column: &Column) -> (Field, MyDecoding) {
    let column_type = column.column_type();
    let flags = column.flags();
    let unsigned = flags.contains(ColumnFlags::UNSIGNED_FLAG);
    let binary_charset = column.character_set() == BINARY_CHARSET;
    let decoding = decoding_for(
        column_type,
        unsigned,
        binary_charset,
        column.column_length(),
        column.decimals(),
    );
    let mut metadata = HashMap::from([(META_MYSQL_TYPE.to_owned(), type_name(column_type, flags))]);
    if matches!(decoding, MyDecoding::Text)
        && matches!(
            column_type,
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL
        )
    {
        // A decimal whose definition did not read back into `(p, s)`: shown as
        // the server's text, and said so.
        metadata.insert(META_FALLBACK.to_owned(), "text".to_owned());
    }
    let field = Field::new(column.name_str(), arrow_type(decoding), true).with_metadata(metadata);
    (field, decoding)
}

/// The decoding a column definition calls for.
///
/// Pure, so the whole table is tested without a server.
#[must_use]
pub fn decoding_for(
    column_type: ColumnType,
    unsigned: bool,
    binary_charset: bool,
    length: u32,
    decimals: u8,
) -> MyDecoding {
    use ColumnType as T;
    match column_type {
        T::MYSQL_TYPE_TINY => pick(unsigned, MyDecoding::UInt8, MyDecoding::Int8),
        T::MYSQL_TYPE_SHORT => pick(unsigned, MyDecoding::UInt16, MyDecoding::Int16),
        T::MYSQL_TYPE_INT24 | T::MYSQL_TYPE_LONG => {
            pick(unsigned, MyDecoding::UInt32, MyDecoding::Int32)
        }
        T::MYSQL_TYPE_LONGLONG => pick(unsigned, MyDecoding::UInt64, MyDecoding::Int64),
        T::MYSQL_TYPE_YEAR => MyDecoding::UInt16,
        T::MYSQL_TYPE_BIT => MyDecoding::Bit,
        T::MYSQL_TYPE_FLOAT => MyDecoding::Float32,
        T::MYSQL_TYPE_DOUBLE => MyDecoding::Float64,
        T::MYSQL_TYPE_DECIMAL | T::MYSQL_TYPE_NEWDECIMAL => decimal(unsigned, length, decimals),
        T::MYSQL_TYPE_DATE | T::MYSQL_TYPE_NEWDATE => MyDecoding::Date,
        T::MYSQL_TYPE_DATETIME => MyDecoding::DateTime,
        T::MYSQL_TYPE_TIMESTAMP => MyDecoding::Timestamp,
        T::MYSQL_TYPE_TIME => MyDecoding::Time,
        // MySQL announces JSON in the `binary` character set; it is text.
        T::MYSQL_TYPE_JSON => MyDecoding::Text,
        T::MYSQL_TYPE_STRING
        | T::MYSQL_TYPE_VAR_STRING
        | T::MYSQL_TYPE_VARCHAR
        | T::MYSQL_TYPE_TINY_BLOB
        | T::MYSQL_TYPE_MEDIUM_BLOB
        | T::MYSQL_TYPE_LONG_BLOB
        | T::MYSQL_TYPE_BLOB
        | T::MYSQL_TYPE_ENUM
        | T::MYSQL_TYPE_SET => {
            // The character set decides, not the `BINARY` flag: MariaDB sets
            // it on `utf8mb4` text (ADR-0050 §10).
            if binary_charset {
                MyDecoding::Binary
            } else {
                MyDecoding::Text
            }
        }
        // `GEOMETRY`, `VECTOR`, and every code the driver does not decode.
        _ => MyDecoding::Binary,
    }
}

const fn pick(unsigned: bool, if_unsigned: MyDecoding, if_signed: MyDecoding) -> MyDecoding {
    if unsigned { if_unsigned } else { if_signed }
}

/// `DECIMAL(p, s)` from the column definition.
///
/// The server sends `s` as `decimals`, and as length `p`, plus one for the sign
/// unless `UNSIGNED`, plus one for the point when `s > 0` — checked on four
/// definitions against four servers (RESEARCH-NOTES, 2026-09-30). A definition
/// that does not read back into a valid `(p, s)` is shown as text rather than
/// given an invented precision.
fn decimal(unsigned: bool, length: u32, decimals: u8) -> MyDecoding {
    let sign = u32::from(!unsigned);
    let point = u32::from(decimals > 0);
    let Some(digits) = length
        .checked_sub(sign)
        .and_then(|rest| rest.checked_sub(point))
    else {
        return MyDecoding::Text;
    };
    let (Ok(precision), Ok(scale)) = (u8::try_from(digits), i8::try_from(decimals)) else {
        return MyDecoding::Text;
    };
    if precision == 0 || decimals > precision {
        return MyDecoding::Text;
    }
    if precision <= DECIMAL128_MAX_PRECISION {
        MyDecoding::Decimal128 { precision, scale }
    } else if precision <= DECIMAL256_MAX_PRECISION {
        MyDecoding::Decimal256 { precision, scale }
    } else {
        MyDecoding::Text
    }
}

/// The Arrow type of a decoding.
#[must_use]
pub fn arrow_type(decoding: MyDecoding) -> DataType {
    match decoding {
        MyDecoding::Int8 => DataType::Int8,
        MyDecoding::Int16 => DataType::Int16,
        MyDecoding::Int32 => DataType::Int32,
        MyDecoding::Int64 => DataType::Int64,
        MyDecoding::UInt8 => DataType::UInt8,
        MyDecoding::UInt16 => DataType::UInt16,
        MyDecoding::UInt32 => DataType::UInt32,
        MyDecoding::UInt64 | MyDecoding::Bit => DataType::UInt64,
        MyDecoding::Float32 => DataType::Float32,
        MyDecoding::Float64 => DataType::Float64,
        MyDecoding::Decimal128 { precision, scale } => DataType::Decimal128(precision, scale),
        MyDecoding::Decimal256 { precision, scale } => DataType::Decimal256(precision, scale),
        MyDecoding::Date => DataType::Date32,
        MyDecoding::DateTime => DataType::Timestamp(TimeUnit::Microsecond, None),
        MyDecoding::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        MyDecoding::Time => DataType::Duration(TimeUnit::Microsecond),
        MyDecoding::Text => DataType::Utf8,
        MyDecoding::Binary => DataType::Binary,
    }
}

/// The server type's name, as [`META_MYSQL_TYPE`] carries it.
///
/// `ENUM` and `SET` reach the client as `STRING` with a flag; the flag is what
/// names them. A code the table does not name is written as its number.
#[must_use]
pub fn type_name(column_type: ColumnType, flags: ColumnFlags) -> String {
    use ColumnType as T;
    if flags.contains(ColumnFlags::ENUM_FLAG) {
        return "ENUM".to_owned();
    }
    if flags.contains(ColumnFlags::SET_FLAG) {
        return "SET".to_owned();
    }
    let name = match column_type {
        T::MYSQL_TYPE_DECIMAL | T::MYSQL_TYPE_NEWDECIMAL => "DECIMAL",
        T::MYSQL_TYPE_TINY => "TINYINT",
        T::MYSQL_TYPE_SHORT => "SMALLINT",
        T::MYSQL_TYPE_INT24 => "MEDIUMINT",
        T::MYSQL_TYPE_LONG => "INT",
        T::MYSQL_TYPE_LONGLONG => "BIGINT",
        T::MYSQL_TYPE_FLOAT => "FLOAT",
        T::MYSQL_TYPE_DOUBLE => "DOUBLE",
        T::MYSQL_TYPE_NULL => "NULL",
        T::MYSQL_TYPE_TIMESTAMP => "TIMESTAMP",
        T::MYSQL_TYPE_DATE | T::MYSQL_TYPE_NEWDATE => "DATE",
        T::MYSQL_TYPE_TIME => "TIME",
        T::MYSQL_TYPE_DATETIME => "DATETIME",
        T::MYSQL_TYPE_YEAR => "YEAR",
        T::MYSQL_TYPE_VARCHAR | T::MYSQL_TYPE_VAR_STRING => "VARCHAR",
        T::MYSQL_TYPE_STRING => "CHAR",
        T::MYSQL_TYPE_BIT => "BIT",
        T::MYSQL_TYPE_JSON => "JSON",
        T::MYSQL_TYPE_ENUM => "ENUM",
        T::MYSQL_TYPE_SET => "SET",
        T::MYSQL_TYPE_TINY_BLOB
        | T::MYSQL_TYPE_MEDIUM_BLOB
        | T::MYSQL_TYPE_LONG_BLOB
        | T::MYSQL_TYPE_BLOB => "BLOB",
        T::MYSQL_TYPE_GEOMETRY => "GEOMETRY",
        T::MYSQL_TYPE_VECTOR => "VECTOR",
        other => return format!("type code {}", u8::from(other)),
    };
    name.to_owned()
}

/// Would `mysql_common` 0.37.3 **panic** decoding this type in the binary
/// protocol?
///
/// Its `Value::deserialize_bin` ends in `unimplemented!` for the server's
/// internal codes — `TIMESTAMP2`, `DATETIME2`, `TIME2`, `TYPED_ARRAY` and
/// `UNKNOWN` —, which a well-behaved server never sends to a client. A hostile
/// or broken one could, and the release profile aborts on panic: the whole
/// application would die on a result ([I-09](../../../CLAUDE.md#i-09)). The
/// driver reads each result set's column definitions **before** its first row
/// and refuses such a result instead of decoding it.
#[must_use]
pub const fn panics_in_binary_protocol(column_type: ColumnType) -> bool {
    matches!(
        column_type,
        ColumnType::MYSQL_TYPE_TIMESTAMP2
            | ColumnType::MYSQL_TYPE_DATETIME2
            | ColumnType::MYSQL_TYPE_TIME2
            | ColumnType::MYSQL_TYPE_TYPED_ARRAY
            | ColumnType::MYSQL_TYPE_UNKNOWN
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::decimal_10_2(false, 12, 2, 10, 2)]
    #[case::decimal_65_30(false, 67, 30, 65, 30)]
    #[case::decimal_10_0_unsigned(true, 10, 0, 10, 0)]
    #[case::decimal_5_5(false, 7, 5, 5, 5)]
    fn decimal_precision_is_read_back_from_length_and_decimals(
        #[case] unsigned: bool,
        #[case] length: u32,
        #[case] decimals: u8,
        #[case] precision: u8,
        #[case] scale: i8,
    ) {
        // The four definitions observed on 2026-09-30 (RESEARCH-NOTES).
        let decoding = decoding_for(
            ColumnType::MYSQL_TYPE_NEWDECIMAL,
            unsigned,
            true,
            length,
            decimals,
        );
        let expected = if precision <= 38 {
            MyDecoding::Decimal128 { precision, scale }
        } else {
            MyDecoding::Decimal256 { precision, scale }
        };
        assert_eq!(decoding, expected);
    }

    #[test]
    fn an_unreadable_decimal_definition_is_text_not_an_invented_precision() {
        for (length, decimals) in [(0, 0), (1, 5), (200, 2), (3, 250)] {
            assert_eq!(
                decoding_for(
                    ColumnType::MYSQL_TYPE_NEWDECIMAL,
                    false,
                    true,
                    length,
                    decimals
                ),
                MyDecoding::Text,
                "({length}, {decimals})"
            );
        }
    }

    #[test]
    fn the_character_set_decides_between_text_and_bytes() {
        // ADR-0050 §10: MariaDB sets the BINARY flag on utf8mb4 text.
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_VAR_STRING, false, false, 40, 0),
            MyDecoding::Text
        );
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_VAR_STRING, false, true, 12, 0),
            MyDecoding::Binary
        );
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_JSON, false, true, 0, 0),
            MyDecoding::Text,
            "MySQL announces JSON as binary; it is text"
        );
    }

    #[test]
    fn integers_follow_the_unsigned_flag_and_tinyint_1_stays_an_integer() {
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_TINY, false, true, 1, 0),
            MyDecoding::Int8
        );
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_LONGLONG, true, true, 20, 0),
            MyDecoding::UInt64
        );
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_INT24, false, true, 8, 0),
            MyDecoding::Int32
        );
        assert_eq!(
            decoding_for(ColumnType::MYSQL_TYPE_YEAR, true, true, 4, 0),
            MyDecoding::UInt16
        );
    }

    #[test]
    fn temporal_types_carry_no_invented_time_zone() {
        assert_eq!(
            arrow_type(MyDecoding::DateTime),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(
            arrow_type(MyDecoding::Timestamp),
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        assert_eq!(
            arrow_type(MyDecoding::Time),
            DataType::Duration(TimeUnit::Microsecond)
        );
    }

    #[test]
    fn geometry_vector_and_internal_codes_are_bytes_with_their_name() {
        for code in [
            ColumnType::MYSQL_TYPE_GEOMETRY,
            ColumnType::MYSQL_TYPE_VECTOR,
            ColumnType::MYSQL_TYPE_TYPED_ARRAY,
            ColumnType::MYSQL_TYPE_NULL,
        ] {
            assert_eq!(
                decoding_for(code, false, true, 0, 0),
                MyDecoding::Binary,
                "{code:?}"
            );
        }
        assert_eq!(
            type_name(ColumnType::MYSQL_TYPE_VECTOR, ColumnFlags::empty()),
            "VECTOR"
        );
        assert_eq!(
            type_name(ColumnType::MYSQL_TYPE_TYPED_ARRAY, ColumnFlags::empty()),
            "type code 20"
        );
        assert_eq!(
            type_name(ColumnType::MYSQL_TYPE_STRING, ColumnFlags::ENUM_FLAG),
            "ENUM"
        );
    }

    #[test]
    fn the_internal_codes_that_panic_the_library_are_named() {
        assert!(panics_in_binary_protocol(ColumnType::MYSQL_TYPE_UNKNOWN));
        assert!(panics_in_binary_protocol(ColumnType::MYSQL_TYPE_DATETIME2));
        assert!(!panics_in_binary_protocol(ColumnType::MYSQL_TYPE_VECTOR));
        assert!(!panics_in_binary_protocol(ColumnType::MYSQL_TYPE_NULL));
    }
}
