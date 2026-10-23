//! Bound values: [`ScalarValue`] to DuckDB values.
//!
//! Values are bound, never written into the text ([I-10](../../../CLAUDE.md#i-10)).
//! What DuckDB cannot receive exactly is refused or documented here:
//!
//! | `ScalarValue` | DuckDB | Loss |
//! |---|---|---|
//! | `Null`, `Bool`, `Int64`, `Float64` | `NULL`, `BOOLEAN`, `BIGINT`, `DOUBLE` | none |
//! | `Decimal` | `VARCHAR`, cast by DuckDB to the column's `DECIMAL` | none: the digits travel as text |
//! | `Text`, `Bytes` | `VARCHAR`, `BLOB` | none |
//! | `Uuid` | `VARCHAR`, hyphenated, cast by DuckDB | none |
//! | `Json` | `VARCHAR` | none: the document as text |
//! | `Date` | `DATE` | none within DuckDB's range |
//! | `Time` | `TIME` | below the microsecond: DuckDB's `TIME` stops there |
//! | `Timestamp` (UTC) | `TIMESTAMP`, the UTC wall clock | none: DuckDB reads a zone-less value bound to `TIMESTAMPTZ` as UTC |
//! | `TimestampNaive` | `TIMESTAMP` | below the microsecond |
//! | `Interval` | `INTERVAL` | below the microsecond |
//! | `Array` | refused | DuckDB's C binding takes no list value |

use chrono::{NaiveDate, Timelike};
use duckdb::types::{TimeUnit, Value};
use oxyn_core::ScalarValue;

/// Converts the values of a request, in placeholder order.
///
/// # Errors
/// A message naming the position and the reason, for a value DuckDB cannot
/// receive exactly.
pub(crate) fn values(params: &[ScalarValue]) -> Result<Vec<Value>, String> {
    params
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value_of(value).map_err(|reason| format!("bound value {}: {reason}", index + 1))
        })
        .collect()
}

fn value_of(value: &ScalarValue) -> Result<Value, String> {
    Ok(match value {
        ScalarValue::Null => Value::Null,
        ScalarValue::Bool(flag) => Value::Boolean(*flag),
        ScalarValue::Int64(number) => Value::BigInt(*number),
        ScalarValue::Float64(number) => Value::Double(*number),
        ScalarValue::Decimal(digits) => Value::Text(digits.clone()),
        ScalarValue::Text(text) => Value::Text(text.clone()),
        ScalarValue::Bytes(bytes) => Value::Blob(bytes.clone()),
        ScalarValue::Uuid(uuid) => Value::Text(uuid.hyphenated().to_string()),
        ScalarValue::Json(document) => Value::Text(document.to_string()),
        ScalarValue::Date(date) => Value::Date32(days_since_epoch(*date)?),
        ScalarValue::Time(time) => {
            let micros = i64::from(time.num_seconds_from_midnight())
                .checked_mul(1_000_000)
                .and_then(|seconds| seconds.checked_add(i64::from(time.nanosecond() / 1_000)))
                .ok_or_else(|| "a time of day out of range".to_owned())?;
            Value::Time64(TimeUnit::Microsecond, micros)
        }
        ScalarValue::Timestamp(instant) => {
            Value::Timestamp(TimeUnit::Microsecond, instant.timestamp_micros())
        }
        ScalarValue::TimestampNaive(wall) => {
            Value::Timestamp(TimeUnit::Microsecond, wall.and_utc().timestamp_micros())
        }
        ScalarValue::Interval {
            months,
            days,
            nanos,
        } => Value::Interval {
            months: *months,
            days: *days,
            nanos: *nanos,
        },
        ScalarValue::Array(_) => {
            return Err(
                "DuckDB takes no list as a bound value; write the list in the query".to_owned(),
            );
        }
    })
}

/// Days since 1970-01-01, as DuckDB's `DATE` counts them.
fn days_since_epoch(date: NaiveDate) -> Result<i32, String> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).ok_or_else(|| "no epoch".to_owned())?;
    i32::try_from(date.signed_duration_since(epoch).num_days())
        .map_err(|_| "a date out of DuckDB's range".to_owned())
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveTime, TimeZone, Utc};

    use super::*;

    #[test]
    fn dates_and_times_keep_their_value() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 22).expect("date");
        assert!(matches!(
            value_of(&ScalarValue::Date(date)),
            Ok(Value::Date32(20748))
        ));
        let time = NaiveTime::from_hms_micro_opt(13, 5, 7, 250).expect("time");
        assert!(matches!(
            value_of(&ScalarValue::Time(time)),
            Ok(Value::Time64(TimeUnit::Microsecond, 47_107_000_250))
        ));
        let instant = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("instant");
        assert!(matches!(
            value_of(&ScalarValue::Timestamp(instant)),
            Ok(Value::Timestamp(
                TimeUnit::Microsecond,
                1_700_000_000_000_000
            ))
        ));
    }

    #[test]
    fn a_list_is_refused_with_its_position() {
        let refused = values(&[ScalarValue::Int64(1), ScalarValue::Array(Vec::new())])
            .expect_err("a list cannot be bound");
        assert!(refused.starts_with("bound value 2:"), "{refused}");
    }

    #[test]
    fn a_decimal_travels_as_its_digits() {
        let digits = "123456789012345678901234567890.123456789";
        assert!(matches!(
            value_of(&ScalarValue::Decimal(digits.to_owned())),
            Ok(Value::Text(text)) if text == digits
        ));
    }
}
