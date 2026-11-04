//! The values a caller binds, as Hrana types them.
//!
//! The values go to the server as typed `args` of the statement, **never**
//! written into its text ([I-10](../../../CLAUDE.md#i-10)): the server binds
//! them to the statement's positional placeholders (`?`, `?NNN`). The
//! conversions are SQLite's own, since a bound value lands in a SQLite column
//! (the SQLite driver binds the same way):
//!
//! | Value | Sent as | Why |
//! |---|---|---|
//! | boolean | integer 0 or 1 | SQLite has no boolean |
//! | decimal | text | no `f64` holds `0.10`; a rounded amount is silent corruption |
//! | date, time, naive timestamp | text, as `datetime()` writes it | no time zone is added |
//! | instant | text, RFC 3339 in UTC | the machine's zone would shift the data |
//! | UUID, JSON | text | |
//! | interval, array | refused | SQLite reads neither back |
//!
//! A decimal is exact on the wire and not always in the column: SQLite applies
//! the column's affinity on storage, so `"0.10"` into a `NUMERIC` or
//! `DECIMAL(10,2)` column is stored as the real 0.1, and a 25-digit decimal
//! loses its tail. That loss is SQLite's own, and the SQLite driver has it too.
//!
//! A refusal names the value's **rank** and type, never the value
//! ([I-03](../../../CLAUDE.md#i-03)).

use oxyn_core::{OxynError, Result, ScalarValue};

use crate::wire::Arg;

/// The values of a request, in placeholder order.
///
/// # Errors
/// [`OxynError::NotSupported`] for a value SQLite has no storage for.
pub(crate) fn bind(params: &[ScalarValue]) -> Result<Vec<Arg>> {
    params
        .iter()
        .enumerate()
        .map(|(index, value)| to_arg(value, index.saturating_add(1)))
        .collect()
}

fn to_arg(value: &ScalarValue, rank: usize) -> Result<Arg> {
    let refuse = || OxynError::NotSupported {
        capability: format!(
            "binding a {} as parameter {rank}: SQLite has no storage for it",
            value.type_name()
        ),
    };
    Ok(match value {
        ScalarValue::Null => Arg::Null,
        ScalarValue::Bool(flag) => Arg::Integer(i64::from(*flag)),
        ScalarValue::Int64(number) => Arg::Integer(*number),
        ScalarValue::Float64(number) => Arg::Float(*number),
        ScalarValue::Decimal(text) | ScalarValue::Text(text) => Arg::Text(text.clone()),
        ScalarValue::Bytes(bytes) => Arg::Blob(bytes.clone()),
        ScalarValue::Uuid(id) => Arg::Text(id.to_string()),
        ScalarValue::Date(date) => Arg::Text(date.to_string()),
        ScalarValue::Time(time) => Arg::Text(time.to_string()),
        ScalarValue::Timestamp(instant) => Arg::Text(instant.to_rfc3339()),
        ScalarValue::TimestampNaive(instant) => Arg::Text(instant.to_string()),
        ScalarValue::Json(json) => Arg::Text(json.to_string()),
        ScalarValue::Interval { .. } | ScalarValue::Array(_) => return Err(refuse()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_keep_their_storage_class() {
        let args = bind(&[
            ScalarValue::Null,
            ScalarValue::Bool(true),
            ScalarValue::Int64(i64::MIN),
            ScalarValue::Float64(0.5),
            ScalarValue::Decimal("0.10".to_owned()),
            ScalarValue::Text("x".to_owned()),
            ScalarValue::Bytes(vec![1, 2]),
            ScalarValue::Json(serde_json::json!({"a": 1})),
        ])
        .expect("bound");
        assert_eq!(
            args,
            [
                Arg::Null,
                Arg::Integer(1),
                Arg::Integer(i64::MIN),
                Arg::Float(0.5),
                Arg::Text("0.10".to_owned()),
                Arg::Text("x".to_owned()),
                Arg::Blob(vec![1, 2]),
                Arg::Text(r#"{"a":1}"#.to_owned()),
            ]
        );
    }

    #[test]
    fn a_value_with_no_sqlite_storage_is_refused_without_being_shown() {
        let error = bind(&[
            ScalarValue::Int64(1),
            ScalarValue::Array(vec![ScalarValue::Text("hunter2".to_owned())]),
        ])
        .expect_err("an array");
        let shown = error.to_string();
        assert!(shown.contains("parameter 2"), "{shown}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }

    #[test]
    fn an_instant_is_utc_and_a_naive_timestamp_gets_no_zone() {
        let instant = chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00+02:00")
            .expect("an instant")
            .with_timezone(&chrono::Utc);
        let naive = chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
            .and_then(|date| date.and_hms_opt(10, 0, 0))
            .expect("a timestamp");
        let args = bind(&[
            ScalarValue::Timestamp(instant),
            ScalarValue::TimestampNaive(naive),
        ])
        .expect("bound");
        assert_eq!(
            args,
            [
                Arg::Text("2024-01-15T10:00:00+00:00".to_owned()),
                Arg::Text("2024-01-15 10:00:00".to_owned()),
            ]
        );
    }
}
