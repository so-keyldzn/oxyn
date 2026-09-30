//! Rendering a duration the way MySQL prints a `TIME`.
//!
//! A MySQL `TIME` is not a time of day: it runs from `-838:59:59` to
//! `838:59:59`, and the MySQL driver maps it to `Duration(Microsecond)`
//! ([ADR-0050](../../../docs/adr/0050-mysql-driver-on-mysql-async-prepared-first.md)).
//! Arrow's formatter writes a duration as ISO 8601 — `-PT3023999S` for the
//! lower bound — which nobody who wrote the column recognises, and which the
//! server does not read back from a pasted `INSERT`.
//!
//! The grid and the export both go through [`write`]: the module rule of
//! [`crate::cell`] — the file says what the screen says — holds for durations
//! only if a single function writes them.

use std::fmt;

use arrow::array::{Array, AsArray, StringArray, StringBuilder};
use arrow::datatypes::{
    DataType, DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType,
    DurationSecondType, TimeUnit,
};

/// Bytes of `-838:59:59.000000`, the widest MySQL `TIME`: the capacity that
/// spares a reallocation for every value a MySQL server can send.
const TYPICAL_LEN: usize = 17;

/// Writes `value`, counted in `unit`, as `[-]HH:MM:SS[.fraction]`.
///
/// Hours take at least two digits and as many as they need — `838` for the
/// bound of a MySQL `TIME`, far more for a `Duration(Second)` near
/// `i64::MAX`. The fraction has the unit's natural width — none, 3, 6 or 9
/// digits — and is always written: the Arrow type does not carry the column's
/// declared precision, and a width that varied from row to row would misalign
/// a column the reader compares at a glance.
///
/// Never panics: the magnitude goes through `unsigned_abs`, so `i64::MIN` has
/// no negation to overflow ([I-09](../../../CLAUDE.md#i-09)).
///
/// # Errors
///
/// Only those of `out`; writing into a `String` does not fail.
pub(crate) fn write(out: &mut impl fmt::Write, value: i64, unit: TimeUnit) -> fmt::Result {
    let (per_second, width): (u64, usize) = match unit {
        TimeUnit::Second => (1, 0),
        TimeUnit::Millisecond => (1_000, 3),
        TimeUnit::Microsecond => (1_000_000, 6),
        TimeUnit::Nanosecond => (1_000_000_000, 9),
    };
    let magnitude = value.unsigned_abs();
    let seconds = magnitude / per_second;
    let fraction = magnitude % per_second;
    let (hours, minutes, secs) = (seconds / 3600, seconds / 60 % 60, seconds % 60);

    if value < 0 {
        out.write_char('-')?;
    }
    write!(out, "{hours:02}:{minutes:02}:{secs:02}")?;
    if width > 0 {
        write!(out, ".{fraction:0width$}")?;
    }
    Ok(())
}

/// The value at `row` of a duration array, with its unit.
///
/// `None` when the array is not a duration or `row` is out of range: the
/// caller falls back rather than panics.
pub(crate) fn value_at(array: &dyn Array, row: usize) -> Option<(i64, TimeUnit)> {
    let DataType::Duration(unit) = array.data_type() else {
        return None;
    };
    let value = match unit {
        TimeUnit::Second => array
            .as_primitive_opt::<DurationSecondType>()?
            .values()
            .get(row),
        TimeUnit::Millisecond => array
            .as_primitive_opt::<DurationMillisecondType>()?
            .values()
            .get(row),
        TimeUnit::Microsecond => array
            .as_primitive_opt::<DurationMicrosecondType>()?
            .values()
            .get(row),
        TimeUnit::Nanosecond => array
            .as_primitive_opt::<DurationNanosecondType>()?
            .values()
            .get(row),
    };
    Some((*value?, *unit))
}

/// The duration column `array` rewritten as text, nulls kept as nulls.
///
/// For the text exports, whose Arrow writers offer no hook on how a duration
/// is written. One buffer for the whole column: the values are written into
/// the builder, never into a `String` per row.
///
/// `None` when `array` is not a duration.
pub(crate) fn as_text_column(array: &dyn Array) -> Option<StringArray> {
    let DataType::Duration(_) = array.data_type() else {
        return None;
    };
    let rows = array.len();
    let mut builder = StringBuilder::with_capacity(rows, rows.saturating_mul(TYPICAL_LEN));
    for row in 0..rows {
        if array.is_null(row) {
            builder.append_null();
            continue;
        }
        let (value, unit) = value_at(array, row)?;
        // `StringBuilder` accumulates what `fmt::Write` writes into the
        // pending value; `append_value("")` closes it.
        write(&mut builder, value, unit).ok()?;
        builder.append_value("");
    }
    Some(builder.finish())
}

#[cfg(test)]
mod tests {
    use arrow::array::{DurationMicrosecondArray, DurationSecondArray};

    use super::*;

    fn text(value: i64, unit: TimeUnit) -> String {
        let mut out = String::new();
        write(&mut out, value, unit).expect("writing into a String does not fail");
        out
    }

    /// 838:59:59 in microseconds, the bound of a MySQL `TIME`.
    const TIME_MAX: i64 = 3_020_399_000_000;

    #[test]
    fn a_mysql_time_renders_as_mysql_prints_it() {
        let unit = TimeUnit::Microsecond;
        assert_eq!(text(TIME_MAX, unit), "838:59:59.000000");
        assert_eq!(text(-TIME_MAX, unit), "-838:59:59.000000");
        assert_eq!(text(0, unit), "00:00:00.000000");
        assert_eq!(text(43_384_500_000, unit), "12:03:04.500000");
        // Under a second: the sign is not lost with the whole part.
        assert_eq!(text(-1, unit), "-00:00:00.000001");
    }

    #[test]
    fn the_extreme_values_do_not_panic() {
        assert_eq!(
            text(i64::MIN, TimeUnit::Microsecond),
            "-2562047788:00:54.775808"
        );
        assert_eq!(
            text(i64::MAX, TimeUnit::Microsecond),
            "2562047788:00:54.775807"
        );
        assert_eq!(text(i64::MIN, TimeUnit::Second), "-2562047788015215:30:08");
    }

    #[test]
    fn every_unit_has_its_natural_fraction() {
        assert_eq!(text(3_661, TimeUnit::Second), "01:01:01");
        assert_eq!(text(3_661_250, TimeUnit::Millisecond), "01:01:01.250");
        assert_eq!(
            text(3_661_000_000_007, TimeUnit::Nanosecond),
            "01:01:01.000000007"
        );
    }

    #[test]
    fn a_column_becomes_text_with_its_nulls() {
        let array = DurationMicrosecondArray::from(vec![Some(-TIME_MAX), None, Some(0)]);
        let column = as_text_column(&array).expect("a duration column");
        assert_eq!(column.value(0), "-838:59:59.000000");
        assert!(column.is_null(1));
        assert_eq!(column.value(2), "00:00:00.000000");

        assert!(as_text_column(&arrow::array::Int64Array::from(vec![1])).is_none());
        assert_eq!(
            value_at(&DurationSecondArray::from(vec![5]), 1),
            None,
            "out of range is None, not a panic"
        );
    }
}
