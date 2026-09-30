//! Bound parameters: the "write" direction of the type table.
//!
//! What has no MySQL equivalent is **refused**, never converted by guesswork: a
//! UUID has no MySQL type — `CHAR(36)` or `BINARY(16)` depends on the schema —
//! and an interval has no column type at all. The refusal names the
//! parameter's rank, never its value ([I-03](../../../CLAUDE.md#i-03)).
//!
//! | `ScalarValue` | Sent as | Why |
//! |---|---|---|
//! | `Null` | `NULL` | |
//! | `Bool` | integer 0 / 1 | MySQL's `BOOLEAN` is `TINYINT(1)` |
//! | `Int64`, `Float64` | `BIGINT`, `DOUBLE` | |
//! | `Decimal` | its text | the server converts a string to `DECIMAL` exactly; a float would not |
//! | `Text`, `Bytes`, `Json` | bytes | `JSON` as its serialized text |
//! | `Date`, `Time`, `Timestamp`, `TimestampNaive` | the binary protocol's date and time | a `Timestamp` in UTC, the session's time zone |
//! | `Uuid`, `Interval`, `Array` | refused | no MySQL type to send them as |

use chrono::{Datelike as _, NaiveDateTime, Timelike as _};
use mysql_async::{Params, Value};
use oxyn_core::{OxynError, Result, ScalarValue};

/// Encodes a request's parameters for `exec_iter`.
///
/// # Errors
/// [`OxynError::NotSupported`] for a value MySQL has no type to receive.
pub(crate) fn bind_params(params: &[ScalarValue]) -> Result<Params> {
    if params.is_empty() {
        return Ok(Params::Empty);
    }
    let mut values = Vec::with_capacity(params.len());
    for (rank, value) in params.iter().enumerate() {
        values.push(match value {
            ScalarValue::Null => Value::NULL,
            ScalarValue::Bool(v) => Value::Int(i64::from(*v)),
            ScalarValue::Int64(v) => Value::Int(*v),
            ScalarValue::Float64(v) => Value::Double(*v),
            ScalarValue::Decimal(text) | ScalarValue::Text(text) => {
                Value::Bytes(text.clone().into_bytes())
            }
            ScalarValue::Bytes(bytes) => Value::Bytes(bytes.clone()),
            ScalarValue::Json(json) => Value::Bytes(json.to_string().into_bytes()),
            ScalarValue::Date(date) => {
                datetime(rank, date.and_hms_opt(0, 0, 0).unwrap_or_default())?
            }
            ScalarValue::Time(time) => Value::Time(
                false,
                0,
                u8::try_from(time.hour()).map_err(|_| unsupported(rank, "this time"))?,
                u8::try_from(time.minute()).map_err(|_| unsupported(rank, "this time"))?,
                u8::try_from(time.second()).map_err(|_| unsupported(rank, "this time"))?,
                time.nanosecond() / 1_000,
            ),
            ScalarValue::Timestamp(instant) => datetime(rank, instant.naive_utc())?,
            ScalarValue::TimestampNaive(local) => datetime(rank, *local)?,
            ScalarValue::Uuid(_) => {
                return Err(unsupported(
                    rank,
                    "a UUID (bind its text and let the query say CHAR or UUID_TO_BIN)",
                ));
            }
            ScalarValue::Interval { .. } => {
                return Err(unsupported(
                    rank,
                    "an interval, which MySQL has no type for",
                ));
            }
            ScalarValue::Array(_) => return Err(unsupported(rank, "an array")),
        });
    }
    Ok(Params::Positional(values))
}

/// A date and time in the binary protocol's parts.
fn datetime(rank: usize, value: NaiveDateTime) -> Result<Value> {
    let invalid = || unsupported(rank, "a date outside MySQL's years 0 to 9999");
    let year = u16::try_from(value.year()).map_err(|_| invalid())?;
    let part = |n: u32| u8::try_from(n).map_err(|_| invalid());
    Ok(Value::Date(
        year,
        part(value.month())?,
        part(value.day())?,
        part(value.hour())?,
        part(value.minute())?,
        part(value.second())?,
        value.nanosecond() / 1_000,
    ))
}

/// The refusal of a parameter, named by its rank.
fn unsupported(rank: usize, what: &str) -> OxynError {
    OxynError::NotSupported {
        capability: format!(
            "binding {what} as parameter {} — cast it in the query instead",
            rank.saturating_add(1)
        ),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};

    use super::*;

    #[test]
    fn the_usual_types_bind() {
        let params = vec![
            ScalarValue::Null,
            ScalarValue::Bool(true),
            ScalarValue::Int64(-3),
            ScalarValue::Float64(1.5),
            ScalarValue::Decimal("12345678901234567890.12".to_owned()),
            ScalarValue::Text("shop".to_owned()),
            ScalarValue::Bytes(vec![1, 2]),
            ScalarValue::Date(NaiveDate::from_ymd_opt(2026, 9, 30).expect("valid test date")),
            ScalarValue::Time(NaiveTime::from_hms_micro_opt(14, 30, 0, 5).expect("valid time")),
        ];
        let Params::Positional(values) = bind_params(&params).expect("all these bind") else {
            panic!("positional parameters expected");
        };
        assert_eq!(values.len(), params.len());
        assert_eq!(values[1], Value::Int(1));
        assert_eq!(
            values[4],
            Value::Bytes(b"12345678901234567890.12".to_vec()),
            "an exact decimal travels as text, never as a float"
        );
        assert_eq!(values[7], Value::Date(2026, 9, 30, 0, 0, 0, 0));
        assert_eq!(values[8], Value::Time(false, 0, 14, 30, 0, 5));
    }

    #[test]
    fn no_parameter_is_no_parameter() {
        assert_eq!(bind_params(&[]).expect("empty"), Params::Empty);
    }

    #[test]
    fn what_mysql_cannot_receive_is_refused_by_rank_never_by_value() {
        let secret = uuid_like();
        let params = vec![ScalarValue::Int64(1), ScalarValue::Uuid(secret)];
        let error = bind_params(&params).expect_err("refusal expected");
        assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
        assert!(error.to_string().contains("parameter 2"), "{error}");
        assert!(!error.to_string().contains(&secret.to_string()), "{error}");
        let interval = vec![ScalarValue::Interval {
            months: 1,
            days: 0,
            nanos: 0,
        }];
        assert!(bind_params(&interval).is_err());
    }

    fn uuid_like() -> uuid::Uuid {
        uuid::Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef)
    }
}
