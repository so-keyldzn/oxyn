//! A catalog column's DuckDB type, as Oxyn's logical type.
//!
//! The catalog reports `data_type` as DuckDB writes it (`DECIMAL(18,3)`,
//! `INTEGER[]`, `STRUCT(a INTEGER)`, `TIMESTAMP WITH TIME ZONE`). This mapping
//! only labels a column for the tree, completion and the AI's schema: values
//! are converted by [`crate::convert`] from the engine's own type, never from
//! this text. The raw text always travels next to it (`Field::raw_type`).

use oxyn_catalog::model::LogicalType;

/// Maps DuckDB's written type to a logical type; `Unknown` for what has no
/// counterpart.
#[must_use]
pub fn logical_type(raw: &str, precision: Option<u16>, scale: Option<i16>) -> LogicalType {
    let upper = raw.trim().to_ascii_uppercase();
    if let Some(element) = upper.strip_suffix("[]") {
        return LogicalType::Array(Box::new(logical_type(element, None, None)));
    }
    if upper.ends_with(']')
        && let Some((element, _)) = upper.rsplit_once('[')
    {
        return LogicalType::Array(Box::new(logical_type(element, None, None)));
    }
    let head = upper.split('(').next().unwrap_or_default().trim();
    match head {
        "BOOLEAN" | "BOOL" | "LOGICAL" => LogicalType::Boolean,
        "TINYINT" | "INT1" | "UTINYINT" => LogicalType::Integer { bits: 8 },
        "SMALLINT" | "INT2" | "SHORT" | "USMALLINT" => LogicalType::Integer { bits: 16 },
        "INTEGER" | "INT4" | "INT" | "SIGNED" | "UINTEGER" => LogicalType::Integer { bits: 32 },
        "BIGINT" | "INT8" | "LONG" | "UBIGINT" => LogicalType::Integer { bits: 64 },
        "HUGEINT" | "UHUGEINT" => LogicalType::Integer { bits: 128 },
        "FLOAT" | "FLOAT4" | "REAL" => LogicalType::Float { bits: 32 },
        "DOUBLE" | "FLOAT8" => LogicalType::Float { bits: 64 },
        "DECIMAL" | "NUMERIC" => LogicalType::Decimal { precision, scale },
        "VARCHAR" | "CHAR" | "BPCHAR" | "TEXT" | "STRING" | "ENUM" => LogicalType::Text,
        "BLOB" | "BYTEA" | "BINARY" | "VARBINARY" | "BIT" | "BITSTRING" | "BIGNUM" => {
            LogicalType::Bytes
        }
        "UUID" => LogicalType::Uuid,
        "DATE" => LogicalType::Date,
        "TIME" | "TIME_NS" | "TIME WITH TIME ZONE" | "TIMETZ" => LogicalType::Time,
        "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" => LogicalType::Timestamp { tz: true },
        "TIMESTAMP" | "DATETIME" | "TIMESTAMP_S" | "TIMESTAMP_MS" | "TIMESTAMP_NS" => {
            LogicalType::Timestamp { tz: false }
        }
        "INTERVAL" => LogicalType::Interval,
        "JSON" => LogicalType::Json,
        "GEOMETRY" => LogicalType::Geometry,
        _ => LogicalType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_types_map_to_their_logical_type() {
        assert_eq!(logical_type("INTEGER", None, None), LogicalType::INT32);
        assert_eq!(
            logical_type("DECIMAL(18,3)", Some(18), Some(3)),
            LogicalType::Decimal {
                precision: Some(18),
                scale: Some(3)
            }
        );
        assert_eq!(
            logical_type("TIMESTAMP WITH TIME ZONE", None, None),
            LogicalType::Timestamp { tz: true }
        );
        assert_eq!(
            logical_type("TIMESTAMP", None, None),
            LogicalType::Timestamp { tz: false }
        );
        assert_eq!(
            logical_type("VARCHAR[]", None, None),
            LogicalType::Array(Box::new(LogicalType::Text))
        );
        assert_eq!(
            logical_type("DOUBLE[3]", None, None),
            LogicalType::Array(Box::new(LogicalType::Float { bits: 64 }))
        );
        assert_eq!(
            logical_type("STRUCT(a INTEGER)", None, None),
            LogicalType::Unknown
        );
        assert_eq!(
            logical_type("HUGEINT", None, None),
            LogicalType::Integer { bits: 128 }
        );
    }
}
