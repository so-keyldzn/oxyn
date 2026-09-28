//! Bound parameters: `ScalarValue` → SQLite storage class.
//!
//! **Values are bound, not concatenated**
//! ([`DRIVER-CONTRACT` §6](../../../docs/DRIVER-CONTRACT.md)). Nothing in this
//! module produces query text: every value goes through `sqlite3_bind_*`.
//!
//! # The mapping table, and what it loses
//!
//! SQLite has only five storage classes — NULL, INTEGER, REAL, TEXT, BLOB.
//! Everything else is an **encoding convention**, and a convention is
//! documented:
//!
//! | `ScalarValue` | SQLite class | What is lost |
//! |---|---|---|
//! | `Null` | NULL | nothing |
//! | `Bool` | INTEGER `0`/`1` | the type: `1` and `true` are indistinguishable when read back |
//! | `Int64` | INTEGER | nothing |
//! | `Float64` | REAL | nothing |
//! | `Decimal` | TEXT | ordering: comparison becomes lexicographic. The **value** is exact, which is the point: no float represents `0.10` |
//! | `Text` | TEXT | nothing |
//! | `Bytes` | BLOB | nothing |
//! | `Uuid` | canonical hyphenated TEXT | the type; it is the usual SQLite convention |
//! | `Date` | TEXT `YYYY-MM-DD` | the type; format of SQLite's `date()` functions |
//! | `Time` | TEXT `HH:MM:SS[.fff]` | the type |
//! | `Timestamp` | TEXT RFC 3339 **in UTC** | the type; the time zone is kept, never converted to the machine's |
//! | `TimestampNaive` | TEXT `YYYY-MM-DD HH:MM:SS` | the type; **no time zone is invented** |
//! | `Json` | TEXT | the type; it is what the `json1` extension expects |
//! | `Interval` | **refused** | — |
//! | `Array` | **refused** | — |
//!
//! The two refusals are not gaps to fill: SQLite has neither interval nor array,
//! and encoding them as text would produce a value no query could read back. Not
//! knowing how is an acceptable answer; pretending is not.

use oxyn_core::ScalarValue;
use rusqlite::Statement;
use rusqlite::types::Value;

use crate::error::{Bound, SqliteError};

/// Binds the positional parameters of a prepared statement.
///
/// SQLite slots are numbered **from 1**; `params[0]` therefore feeds `?1`.
///
/// # Errors
/// [`SqliteError::ParameterCount`] if the count does not match what the
/// statement expects, [`SqliteError::Parameter`] for a value SQLite cannot
/// store, or the engine error if binding fails.
pub(crate) fn bind(
    statement: &mut Statement<'_>,
    params: &[ScalarValue],
) -> Result<(), SqliteError> {
    let expected = statement.parameter_count();
    if expected != params.len() {
        return Err(SqliteError::ParameterCount {
            expected,
            given: params.len(),
        });
    }
    for (position, value) in params.iter().enumerate() {
        // The count is checked above, and `enumerate` does not overflow:
        // `position + 1` fits in a `usize` since `position < len`.
        let index = position.saturating_add(1);
        // The choice is made here rather than swallowed by a `?`: this is the
        // function that binds the caller's values, hence the only place in the
        // driver where the engine could quote one (I-03).
        statement
            .raw_bind_parameter(index, to_storage(value, index)?)
            .map_err(|error| {
                let code = match &error {
                    rusqlite::Error::SqliteFailure(inner, _) => Some(*inner),
                    _ => None,
                };
                crate::error::hide(error, code, Bound::Caller)
            })?;
    }
    Ok(())
}

/// Converts a domain value into an SQLite storage class.
///
/// The `match` is **exhaustive on purpose**: [`ScalarValue`] is a closed
/// enumeration precisely so that adding a scalar type fails the compilation of
/// every driver, and asks "and this one, how do I return it?" rather than a
/// `_ =>` that would decide silently.
fn to_storage(value: &ScalarValue, index: usize) -> Result<Value, SqliteError> {
    let refuse = || SqliteError::Parameter {
        index,
        type_name: value.type_name(),
    };
    Ok(match value {
        ScalarValue::Null => Value::Null,
        ScalarValue::Bool(b) => Value::Integer(i64::from(*b)),
        ScalarValue::Int64(i) => Value::Integer(*i),
        ScalarValue::Float64(x) => Value::Real(*x),
        // Text, not a float: no `f64` represents `0.10`, and a monetary value
        // rounded along the way is silent corruption.
        ScalarValue::Decimal(d) => Value::Text(d.clone()),
        ScalarValue::Text(t) => Value::Text(t.clone()),
        ScalarValue::Bytes(b) => Value::Blob(b.clone()),
        ScalarValue::Uuid(u) => Value::Text(u.to_string()),
        ScalarValue::Date(d) => Value::Text(d.to_string()),
        ScalarValue::Time(t) => Value::Text(t.to_string()),
        // RFC 3339, in UTC. Converting to the machine's time zone would shift the
        // data invisibly and permanently (DRIVER-CONTRACT §7).
        ScalarValue::Timestamp(ts) => Value::Text(ts.to_rfc3339()),
        // `YYYY-MM-DD HH:MM:SS`, the form SQLite's `datetime()` functions return.
        // No time zone is added.
        ScalarValue::TimestampNaive(ts) => Value::Text(ts.to_string()),
        ScalarValue::Json(v) => Value::Text(v.to_string()),
        // SQLite has neither interval nor array. Encoding them as text would
        // produce a value no SQLite query could read back.
        ScalarValue::Interval { .. } | ScalarValue::Array(_) => return Err(refuse()),
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn stored(value: &ScalarValue) -> Result<Value, SqliteError> {
        to_storage(value, 1)
    }

    #[test]
    fn a_boolean_becomes_an_integer() {
        // SQLite has no boolean type: `0` and `1` are the convention.
        assert_eq!(
            stored(&ScalarValue::Bool(true)).expect("bound"),
            Value::Integer(1)
        );
        assert_eq!(
            stored(&ScalarValue::Bool(false)).expect("bound"),
            Value::Integer(0)
        );
    }

    #[test]
    fn a_decimal_does_not_go_through_a_float() {
        let value = stored(&ScalarValue::Decimal("0.10".to_owned())).expect("bound");
        assert_eq!(
            value,
            Value::Text("0.10".to_owned()),
            "trailing zeros are significant, and no f64 holds 0.10"
        );
    }

    #[test]
    fn bytes_stay_bytes() {
        // A BLOB is not rendered as text "at best": it stays opaque.
        let value = stored(&ScalarValue::Bytes(vec![0x00, 0xff, 0x80])).expect("bound");
        assert_eq!(value, Value::Blob(vec![0x00, 0xff, 0x80]));
    }

    #[test]
    fn an_interval_and_an_array_are_refused_not_encoded() {
        // SQLite has neither; encoding them would produce a value no query could
        // read back.
        for sample in [
            ScalarValue::Interval {
                months: 1,
                days: 0,
                nanos: 0,
            },
            ScalarValue::Array(vec![ScalarValue::Int64(1)]),
        ] {
            let err = stored(&sample).expect_err("expected refusal");
            assert!(
                matches!(err, SqliteError::Parameter { .. }),
                "{err:?} for {}",
                sample.type_name()
            );
        }
    }

    #[test]
    fn a_wrong_parameter_count_is_refused_before_any_execution() {
        let conn = Connection::open_in_memory().expect("in-memory database");
        let mut stmt = conn.prepare("SELECT ?1, ?2").expect("preparation");

        let err = bind(&mut stmt, &[ScalarValue::Int64(1)]).expect_err("expected refusal");
        let SqliteError::ParameterCount { expected, given } = err else {
            panic!("wrong variant: {err:?}");
        };
        assert_eq!((expected, given), (2, 1));
    }

    #[test]
    fn a_bound_value_never_crosses_the_query_text() {
        // The test that matters: a hostile bound value stays a value.
        let conn = Connection::open_in_memory().expect("in-memory database");
        conn.execute_batch("CREATE TABLE audit(note TEXT); CREATE TABLE t(v TEXT);")
            .expect("schema");

        let mut stmt = conn
            .prepare("INSERT INTO t(v) VALUES (?1)")
            .expect("preparation");
        bind(
            &mut stmt,
            &[ScalarValue::Text("'); DROP TABLE audit; --".to_owned())],
        )
        .expect("binding");
        stmt.raw_execute().expect("insertion");

        let remaining: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(remaining, 1, "the audit table was dropped");
    }

    // The temporal variants, `Uuid` and `Json` of `ScalarValue` are not covered
    // here: building them would require `chrono`, `uuid` and `serde_json`, which
    // are not in this crate's dependency contract. Their rendering is described
    // in the module table and uses only each type's `Display` — so nothing that
    // could diverge silently.
}
