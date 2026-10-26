use super::*;

// Every expected text below was printed by PostgreSQL 17.11 with
// `TimeZone = 'UTC'`, `DateStyle = 'ISO, MDY'`, `IntervalStyle = 'postgres'`,
// and every input read back through `*_send`.

fn rendered(render: impl FnOnce(&mut String) -> Rendered) -> Result<String, &'static str> {
    let mut out = String::new();
    render(&mut out).map(|()| out)
}

/// The two's-complement value of a big-endian hexadecimal `*_send` output.
fn i64_of(hex: &str) -> i64 {
    let bits = u64::from_str_radix(hex, 16).expect("hex");
    i64::from_be_bytes(bits.to_be_bytes())
}

#[test]
fn dates_match_the_server() {
    for (days, text) in [
        (0, "2000-01-01"),
        (8_825, "2024-02-29"),
        (-730_119, "0001-01-01"),
        (-730_120, "0001-12-31 BC"),
        (-2_451_545, "4714-11-24 BC"),
        (2_145_031_948, "5874897-12-31"),
        (-1, "1999-12-31"),
        (-36_465, "1900-03-01"),
        (-730_791, "0002-03-01 BC"),
        (i32::MAX, "infinity"),
        (i32::MIN, "-infinity"),
    ] {
        assert_eq!(
            rendered(|out| date(days, out)).as_deref(),
            Ok(text),
            "{days}"
        );
    }
}

#[test]
fn a_date_before_the_julian_origin_is_refused() {
    assert!(rendered(|out| date(-2_451_546, out)).is_err());
    assert!(rendered(|out| date(i32::MIN + 1, out)).is_err());
}

#[test]
fn times_match_the_server() {
    for (hex, text) in [
        ("0000000000000000", "00:00:00"),
        ("0000000c27646b20", "14:30:00.5"),
        ("000000141dd75fff", "23:59:59.999999"),
        ("000000141dd76000", "24:00:00"),
        ("0000000000000001", "00:00:00.000001"),
        ("000000079d420280", "09:05:03.12"),
    ] {
        assert_eq!(
            rendered(|out| time(i64_of(hex), out)).as_deref(),
            Ok(text),
            "{hex}"
        );
    }
    assert!(rendered(|out| time(-1, out)).is_err());
    assert!(rendered(|out| time(USECS_PER_DAY + 1, out)).is_err());
}

#[test]
fn timestamps_match_the_server() {
    for (hex, text) in [
        ("0000000000000000", "2000-01-01 00:00:00"),
        ("7fffff5bb3b29fff", "294276-12-31 23:59:59.999999"),
        ("fd0f7cc1411fa000", "4714-11-24 00:00:00 BC"),
        ("fffc96188bb035c0", "1969-07-20 20:17:40.12"),
        ("ff1fe2ffc590ee50", "0001-12-31 23:59:59.25 BC"),
        ("0002b58cd363bfff", "2024-02-29 23:59:59.999999"),
    ] {
        assert_eq!(
            rendered(|out| timestamp(i64_of(hex), out)).as_deref(),
            Ok(text),
            "{hex}"
        );
    }
    assert_eq!(
        rendered(|out| timestamp(-999_999, out)).as_deref(),
        Ok("1999-12-31 23:59:59.000001")
    );
    assert_eq!(
        rendered(|out| timestamp(i64::MAX, out)).as_deref(),
        Ok("infinity")
    );
    assert_eq!(
        rendered(|out| timestamp(i64::MIN, out)).as_deref(),
        Ok("-infinity")
    );
}

#[test]
fn timestamptz_prints_utc_and_the_era_last() {
    for (micros, text) in [
        (0x0002_fdba_4feb_8200, "2026-09-05 12:30:00+00"),
        (
            i64_of("7fffff5bb3b29fff"),
            "294276-12-31 23:59:59.999999+00",
        ),
        (-63_082_281_600_750_000, "0001-12-31 23:59:59.25+00 BC"),
        (-211_813_488_000_000_000, "4714-11-24 00:00:00+00 BC"),
        (-3_094_168_448_000_000, "1901-12-13 20:45:52+00"),
    ] {
        assert_eq!(
            rendered(|out| timestamptz(micros, out)).as_deref(),
            Ok(text),
            "{micros}"
        );
    }
    assert_eq!(
        rendered(|out| timestamptz(i64::MIN, out)).as_deref(),
        Ok("-infinity")
    );
}

#[test]
fn a_timestamp_before_the_julian_origin_is_refused() {
    assert!(rendered(|out| timestamp(-211_813_488_000_000_001, out)).is_err());
    assert!(rendered(|out| timestamp(i64::MIN + 1, out)).is_err());
}

#[test]
fn intervals_match_the_server() {
    // (time, day, month) as `interval_send` orders them.
    for (micros, days, months, text) in [
        (0, 0, 0, "00:00:00"),
        (
            0x0000_0003_6c93_61a0,
            3,
            14,
            "1 year 2 mons 3 days 04:05:06.5",
        ),
        (
            i64_of("fffffffc93743f80"),
            3,
            -14,
            "-1 years -2 mons +3 days -04:05:06",
        ),
        (i64_of("fffffffffff0bdc0"), 1, 0, "1 day -00:00:01"),
        (0x0000_0000_d693_a400, -1, 0, "-1 days +01:00:00"),
        (0, -1, 1, "1 mon -1 days"),
        (-1, 0, 0, "-00:00:00.000001"),
        (0x0000_0053_d1ac_1000, 0, 0, "100:00:00"),
        (0, 0, 12, "1 year"),
        (0, -1, 0, "-1 days"),
        (0, 2, 0, "2 days"),
        (0, 0, 1, "1 mon"),
        (0, 0, 13, "1 year 1 mon"),
        (i64_of("fffffffe52d8b800"), -3, 0, "-3 days -02:00:00"),
        (0x0000_000a_8be6_2608, 0, 0, "12:34:56.789"),
        (0, 0, i32::MIN + 1, "-178956970 years -7 mons"),
        (i64::MAX, 0, 0, "2562047788:00:54.775807"),
        (i64::MAX, i32::MAX, i32::MAX, "infinity"),
        (i64::MIN, i32::MIN, i32::MIN, "-infinity"),
    ] {
        assert_eq!(
            rendered(|out| interval(micros, days, months, out)).as_deref(),
            Ok(text),
            "{micros} {days} {months}"
        );
    }
}

#[test]
fn a_single_extreme_interval_field_is_finite() {
    assert_eq!(
        rendered(|out| interval(i64::MIN, 0, 0, out)).as_deref(),
        Ok("-2562047788:00:54.775808")
    );
}
