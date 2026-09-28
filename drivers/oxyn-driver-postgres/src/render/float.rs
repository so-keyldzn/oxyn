//! `float4` and `float8` as `float8out_internal` prints them.
//!
//! With the default `extra_float_digits` (1), PostgreSQL prints the shortest
//! digits that read back to the same value, in fixed notation while the decimal
//! exponent stays in a window, in exponential notation outside it. Rust's `{:e}`
//! already produces the shortest round-trip digits; only the layout differs.
//! The window is not the same for both widths: `[-4, 15)` for `float8`,
//! `[-4, 6)` for `float4` — `1234567::real` prints `1.234567e+06`.

use std::fmt::Write as _;

/// Smallest decimal exponent still printed in fixed notation.
const FIXED_MIN_EXPONENT: i32 = -4;
/// First decimal exponent printed in exponential notation, for `float8`.
const FLOAT8_FIXED_END: i32 = 15;
/// First decimal exponent printed in exponential notation, for `float4`.
const FLOAT4_FIXED_END: i32 = 6;

/// Appends a `float8` as PostgreSQL prints it.
pub(crate) fn float8(value: f64, out: &mut String) {
    if value.is_nan() {
        out.push_str("NaN");
    } else if value.is_infinite() {
        out.push_str(if value < 0.0 { "-Infinity" } else { "Infinity" });
    } else {
        layout(&format!("{value:e}"), FLOAT8_FIXED_END, out);
    }
}

/// Appends a `float4` as PostgreSQL prints it.
///
/// Formatted as an `f32`, not widened: the shortest digits of `0.1_f32` are
/// `1`, those of the same value widened to `f64` are seventeen.
pub(crate) fn float4(value: f32, out: &mut String) {
    if value.is_nan() {
        out.push_str("NaN");
    } else if value.is_infinite() {
        out.push_str(if value < 0.0 { "-Infinity" } else { "Infinity" });
    } else {
        layout(&format!("{value:e}"), FLOAT4_FIXED_END, out);
    }
}

/// Lays out Rust's shortest scientific form (`-1.25e-3`) the way PostgreSQL
/// does.
///
/// The input is produced by `{:e}` on a finite value, never by the server; a
/// shape this function does not recognize is still written as is rather than
/// dropped.
fn layout(scientific: &str, fixed_end: i32, out: &mut String) {
    let (negative, unsigned) = match scientific.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, scientific),
    };
    let Some((mantissa, exponent)) = unsigned.split_once('e') else {
        out.push_str(scientific);
        return;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        out.push_str(scientific);
        return;
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();

    if negative {
        out.push('-');
    }
    if (FIXED_MIN_EXPONENT..fixed_end).contains(&exponent) {
        fixed(&digits, exponent, out);
    } else {
        exponential(&digits, exponent, out);
    }
}

/// `digits` × 10^(`exponent` − len + 1), in fixed notation.
fn fixed(digits: &str, exponent: i32, out: &mut String) {
    if let Ok(integer_len) = usize::try_from(exponent) {
        // Integer part: the first `exponent + 1` digits, padded with zeros.
        let split = integer_len.saturating_add(1);
        match (digits.get(..split), digits.get(split..)) {
            (Some(integer), Some(fraction)) if !fraction.is_empty() => {
                let _ = write!(out, "{integer}.{fraction}");
            }
            _ => {
                out.push_str(digits);
                let zeros = split.saturating_sub(digits.len());
                out.extend(std::iter::repeat_n('0', zeros));
            }
        }
    } else {
        // Negative exponent: `0.` then the leading zeros of the fraction.
        let zeros = usize::try_from(exponent.unsigned_abs().saturating_sub(1)).unwrap_or(0);
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', zeros));
        out.push_str(digits);
    }
}

/// `d.ddde+XX`, with at least two exponent digits.
fn exponential(digits: &str, exponent: i32, out: &mut String) {
    let mut chars = digits.chars();
    if let Some(first) = chars.next() {
        out.push(first);
    }
    let rest = chars.as_str();
    if !rest.is_empty() {
        out.push('.');
        out.push_str(rest);
    }
    let sign = if exponent < 0 { '-' } else { '+' };
    let _ = write!(out, "e{sign}{:02}", exponent.unsigned_abs());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eight(hex: &str) -> String {
        let bits = u64::from_str_radix(hex, 16).expect("test vector");
        let mut out = String::new();
        float8(f64::from_bits(bits), &mut out);
        out
    }

    fn four(hex: &str) -> String {
        let bits = u32::from_str_radix(hex, 16).expect("test vector");
        let mut out = String::new();
        float4(f32::from_bits(bits), &mut out);
        out
    }

    /// `SELECT v::text, encode(float8send(v), 'hex')` on PostgreSQL 17.11.
    #[test]
    fn float8_matches_the_server() {
        for (hex, expected) in [
            ("0000000000000000", "0"),
            ("8000000000000000", "-0"),
            ("3ff0000000000000", "1"),
            ("3ff8000000000000", "1.5"),
            ("3fb999999999999a", "0.1"),
            ("430c6bf526340000", "1e+15"),
            ("3ee4f8b588e368f1", "1e-05"),
            ("437b69b4ba630f35", "1.2345678901234568e+17"),
            ("7fefffffffffffff", "1.7976931348623157e+308"),
            ("0000000000000001", "5e-324"),
            ("7ff8000000000000", "NaN"),
            ("7ff0000000000000", "Infinity"),
            ("fff0000000000000", "-Infinity"),
            ("42d6bcc41e900000", "100000000000000"),
            ("42dc12218377de66", "123456789012345.6"),
            ("3f1a36e2eb1c432d", "0.0001"),
            ("3ee9e0fcaf9380fc", "1.234e-05"),
            ("be90c6f7a0b5ed8d", "-2.5e-07"),
            ("4341c37937e08000", "1e+16"),
            ("4059000000000000", "100"),
        ] {
            assert_eq!(eight(hex), expected, "{hex}");
        }
    }

    /// `SELECT v::text, encode(float4send(v), 'hex')` on PostgreSQL 17.11.
    #[test]
    fn float4_matches_the_server() {
        for (hex, expected) in [
            ("00000000", "0"),
            ("80000000", "-0"),
            ("3f800000", "1"),
            ("3fc00000", "1.5"),
            ("3dcccccd", "0.1"),
            ("501502f9", "1e+10"),
            ("3727c5ac", "1e-05"),
            ("7f7fffff", "3.4028235e+38"),
            ("00000001", "1e-45"),
            ("7fc00000", "NaN"),
            ("7f800000", "Infinity"),
            ("ff800000", "-Infinity"),
            ("49742400", "1e+06"),
            ("47c35000", "100000"),
            ("47f12000", "123456"),
            ("4996b438", "1.234567e+06"),
            ("4b3c614e", "1.2345678e+07"),
            ("38d1b717", "0.0001"),
        ] {
            assert_eq!(four(hex), expected, "{hex}");
        }
    }
}
