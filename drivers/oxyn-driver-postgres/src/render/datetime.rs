//! Dates, times and intervals as PostgreSQL's default `DateStyle` (ISO) and `IntervalStyle` (postgres) print them.
//!
//! These renderings serve values nested in a range, a record or a
//! multirange, where the server itself would print text; a top-level `date`
//! or `timestamp` column stays a typed Arrow column. The rules are those of
//! `datetime.c`, `timestamp.c` and `date.c`, recorded in
//! [RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md#postgresql-binary-wire-formats--checked-on-2026-09-28).
//!
//! `timestamptz` is printed as the server prints it with `TimeZone = 'UTC'`:
//! the driver never invents the workstation's time zone
//! ([DRIVER-CONTRACT §7](../../../../docs/DRIVER-CONTRACT.md#7-it-treats-time-zones-and-temporal-types-as-data-not-as-text)).

use std::fmt::Write as _;

use super::Rendered;

/// Julian day of 2000-01-01, PostgreSQL's epoch (`POSTGRES_EPOCH_JDATE`).
const POSTGRES_EPOCH_JDATE: i64 = 2_451_545;
const USECS_PER_DAY: i64 = 86_400_000_000;
const USECS_PER_HOUR: i64 = 3_600_000_000;
const USECS_PER_MINUTE: i64 = 60_000_000;
const USECS_PER_SEC: i64 = 1_000_000;
const MONTHS_PER_YEAR: i32 = 12;
/// Fraction digits of `AppendSeconds` for every type rendered here
/// (`MAX_TIMESTAMP_PRECISION`, `MAX_TIME_PRECISION`, `MAX_INTERVAL_PRECISION`).
const FRACTION_DIGITS: usize = 6;

/// A calendar date, with PostgreSQL's convention: year 0 is 1 BC.
struct Ymd {
    year: i64,
    month: u64,
    day: u64,
}

/// `date`: days since 2000-01-01; `i32::MIN` and `i32::MAX` are the infinities.
pub(crate) fn date(days_since_2000: i32, out: &mut String) -> Rendered {
    if let Some(special) = special_i32(days_since_2000) {
        out.push_str(special);
        return Ok(());
    }
    let ymd = julian_to_date(i64::from(days_since_2000) + POSTGRES_EPOCH_JDATE)
        .ok_or("date out of range")?;
    push_date(&ymd, out);
    push_era(&ymd, out);
    Ok(())
}

/// `time`: microseconds since midnight, `24:00:00` included.
pub(crate) fn time(micros: i64, out: &mut String) -> Rendered {
    if !(0..=USECS_PER_DAY).contains(&micros) {
        return Err("time outside the bounds of a day");
    }
    push_clock(micros, out);
    Ok(())
}

/// `timestamp`: microseconds since 2000-01-01, no time zone.
pub(crate) fn timestamp(micros_since_2000: i64, out: &mut String) -> Rendered {
    timestamp_with(micros_since_2000, false, out)
}

/// `timestamptz`: microseconds since 2000-01-01 UTC, printed in UTC (`+00`).
pub(crate) fn timestamptz(micros_since_2000: i64, out: &mut String) -> Rendered {
    timestamp_with(micros_since_2000, true, out)
}

/// `interval`, in the `postgres` style: `1 year 2 mons 3 days 04:05:06.5`.
///
/// Each field keeps its own sign — `1 day -00:00:01` is not `23:59:59`, since
/// a day is not always 24 hours — and a positive field that follows a
/// negative one is printed with its `+`, as `AddPostgresIntPart` does.
pub(crate) fn interval(micros: i64, days: i32, months: i32, out: &mut String) -> Rendered {
    // PostgreSQL 17+: all three fields at their minimum, or all at their
    // maximum, mean an infinite interval. A single extreme field is finite.
    if micros == i64::MIN && days == i32::MIN && months == i32::MIN {
        out.push_str("-infinity");
        return Ok(());
    }
    if micros == i64::MAX && days == i32::MAX && months == i32::MAX {
        out.push_str("infinity");
        return Ok(());
    }

    // `interval2itm`: truncating divisions, the remainder keeps the sign of the
    // dividend — Rust's `/` and `%` on signed integers, like C's.
    let years = months / MONTHS_PER_YEAR;
    let months = months % MONTHS_PER_YEAR;
    let hours = micros / USECS_PER_HOUR;
    let rest = micros % USECS_PER_HOUR;
    let minutes = rest / USECS_PER_MINUTE;
    let rest = rest % USECS_PER_MINUTE;
    let seconds = rest / USECS_PER_SEC;
    let fraction = rest % USECS_PER_SEC;

    let mut is_zero = true;
    let mut is_before = false;
    for (value, unit) in [
        (i64::from(years), "year"),
        (i64::from(months), "mon"),
        (i64::from(days), "day"),
    ] {
        if value == 0 {
            continue;
        }
        if !is_zero {
            out.push(' ');
        }
        if is_before && value > 0 {
            out.push('+');
        }
        let plural = if value == 1 { "" } else { "s" };
        let _ = write!(out, "{value} {unit}{plural}");
        is_before = value < 0;
        is_zero = false;
    }

    if is_zero || hours != 0 || minutes != 0 || seconds != 0 || fraction != 0 {
        if !is_zero {
            out.push(' ');
        }
        if hours < 0 || minutes < 0 || seconds < 0 || fraction < 0 {
            out.push('-');
        } else if is_before {
            out.push('+');
        }
        let _ = write!(
            out,
            "{:02}:{:02}:",
            hours.unsigned_abs(),
            minutes.unsigned_abs()
        );
        push_seconds(seconds.unsigned_abs(), fraction.unsigned_abs(), out);
    }
    Ok(())
}

fn timestamp_with(micros: i64, utc_suffix: bool, out: &mut String) -> Rendered {
    if let Some(special) = special_i64(micros) {
        out.push_str(special);
        return Ok(());
    }
    // `TMODULO` then the negative-time correction of `timestamp2tm`: exactly a
    // Euclidean division.
    let days = micros.div_euclid(USECS_PER_DAY);
    let clock = micros.rem_euclid(USECS_PER_DAY);
    let julian = days
        .checked_add(POSTGRES_EPOCH_JDATE)
        .ok_or("timestamp out of range")?;
    let ymd = julian_to_date(julian).ok_or("timestamp out of range")?;

    push_date(&ymd, out);
    out.push(' ');
    push_clock(clock, out);
    if utc_suffix {
        // `EncodeTimezone` with a zero offset prints the hours only.
        out.push_str("+00");
    }
    push_era(&ymd, out);
    Ok(())
}

/// `j2date`, over PostgreSQL's whole Julian range (`0..=i32::MAX`).
///
/// The C code works in 32-bit unsigned integers; every intermediate value
/// here fits in `u64`, and the domain check keeps the input where the
/// algorithm is valid.
fn julian_to_date(julian_day: i64) -> Option<Ymd> {
    if !(0..=i64::from(i32::MAX)).contains(&julian_day) {
        return None;
    }
    let mut julian = u64::try_from(julian_day).ok()? + 32_044;
    let quad = julian / 146_097;
    let extra = (julian - quad * 146_097) * 4 + 3;
    julian += 60 + quad * 3 + extra / 146_097;
    let quad = julian / 1_461;
    julian -= quad * 1_461;
    let y = julian * 4 / 1_461;
    julian = if y == 0 {
        (julian + 306) % 366
    } else {
        (julian + 305) % 365
    } + 123;
    let year = i64::try_from(y + quad * 4).ok()? - 4_800;
    let quad = julian * 2_141 / 65_536;
    let day = julian - 7_834 * quad / 256;
    let month = (quad + 10) % 12 + 1;
    Some(Ymd { year, month, day })
}

/// `YYYY-MM-DD`: a year before 1 is printed as its BC number, the era comes last.
fn push_date(ymd: &Ymd, out: &mut String) {
    let printed_year = if ymd.year > 0 { ymd.year } else { 1 - ymd.year };
    let _ = write!(out, "{printed_year:04}-{:02}-{:02}", ymd.month, ymd.day);
}

fn push_era(ymd: &Ymd, out: &mut String) {
    if ymd.year <= 0 {
        out.push_str(" BC");
    }
}

/// `HH:MM:SS[.ffffff]` from microseconds since midnight (already in `0..=24h`).
fn push_clock(micros: i64, out: &mut String) {
    let hours = micros / USECS_PER_HOUR;
    let minutes = micros % USECS_PER_HOUR / USECS_PER_MINUTE;
    let seconds = micros % USECS_PER_MINUTE / USECS_PER_SEC;
    let fraction = micros % USECS_PER_SEC;
    let _ = write!(out, "{hours:02}:{minutes:02}:");
    push_seconds(seconds.unsigned_abs(), fraction.unsigned_abs(), out);
}

/// `AppendSeconds` with `fillzeros`: two digits, then the fraction without its
/// trailing zeros.
fn push_seconds(seconds: u64, fraction: u64, out: &mut String) {
    let _ = write!(out, "{seconds:02}");
    if fraction != 0 {
        // Trailing zeros dropped by arithmetic, not by trimming a formatted
        // copy: a cell must not cost an allocation.
        let (mut fraction, mut width) = (fraction, FRACTION_DIGITS);
        while fraction % 10 == 0 && width > 1 {
            fraction /= 10;
            width -= 1;
        }
        let _ = write!(out, ".{fraction:0width$}");
    }
}

fn special_i32(value: i32) -> Option<&'static str> {
    match value {
        i32::MIN => Some("-infinity"),
        i32::MAX => Some("infinity"),
        _ => None,
    }
}

fn special_i64(value: i64) -> Option<&'static str> {
    match value {
        i64::MIN => Some("-infinity"),
        i64::MAX => Some("infinity"),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
