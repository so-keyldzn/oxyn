//! A console's bound values, matched to ClickHouse's `{name:Type}`
//! placeholders.
//!
//! The console binds values by position ([UX-SPEC](../../../docs/UX-SPEC.md),
//! "Bound values of a console"); ClickHouse binds them by name, server-side,
//! and parses each one as the type its placeholder declares. The `n`-th value
//! goes to the `n`-th **distinct** placeholder name, in the order the text
//! first names them, so `{id:UInt64}` used twice takes one value. Nothing is
//! written into the statement: each value travels as a `param_<name>` of the
//! request ([I-10](../../../CLAUDE.md#i-10)).
//!
//! The placeholders are found with ClickHouse's own lexical rules — a `{` in a
//! string, a quoted name, a comment or a heredoc is not one — and a count that
//! differs from the bound values is refused before anything is sent, naming
//! the counts and never a value ([I-03](../../../CLAUDE.md#i-03)).

use oxyn_core::{OxynError, Result, ScalarValue};

/// The most placeholders read from one statement; the console binds at most
/// 128 values (UX-SPEC), so a text naming more cannot be bound anyway.
const MAX_PLACEHOLDERS: usize = 1024;

/// What a request carries for one bound value: its text, or `None` for
/// `NULL`, which the server reads as `\N`.
pub(crate) type Bound = (String, Option<String>);

/// Binds `values` to the placeholders of `text`, in order.
///
/// # Errors
/// [`OxynError::Config`] when the counts differ or a placeholder name is not
/// an identifier; [`OxynError::NotSupported`] for a value ClickHouse cannot
/// receive as text: bytes, an interval, an array, an instant before 1970.
pub(crate) fn bind(text: &str, values: &[ScalarValue]) -> Result<Vec<Bound>> {
    let names = placeholders(text)?;
    if names.len() != values.len() {
        return Err(OxynError::Config(format!(
            "the statement names {} distinct `{{name:Type}}` placeholder(s) and {} value(s) \
             are bound: ClickHouse binds them by name, in the order the statement first \
             names them",
            names.len(),
            values.len()
        )));
    }
    names
        .into_iter()
        .zip(values)
        .enumerate()
        .map(|(rank, (name, value))| Ok((name, text_of(rank.saturating_add(1), value)?)))
        .collect()
}

/// The distinct placeholder names of `text`, in the order it first names
/// them.
fn placeholders(text: &str) -> Result<Vec<String>> {
    let bytes = text.as_bytes();
    let mut names: Vec<String> = Vec::new();
    let mut at = 0_usize;
    while let Some(&c) = bytes.get(at) {
        at = match c {
            b'\'' | b'"' | b'`' => skip_quoted(bytes, at, c),
            b'-' if bytes.get(at + 1) == Some(&b'-') => skip_line(bytes, at),
            b'#' => skip_line(bytes, at),
            b'/' if bytes.get(at + 1) == Some(&b'*') => skip_block(bytes, at),
            b'$' => skip_heredoc(bytes, at),
            b'{' => match placeholder(bytes, at) {
                Some((name, end)) => {
                    if !names.contains(&name) {
                        if names.len() >= MAX_PLACEHOLDERS {
                            return Err(OxynError::Config(
                                "the statement names too many placeholders to bind".to_owned(),
                            ));
                        }
                        names.push(name);
                    }
                    end
                }
                None => at + 1,
            },
            _ => at + 1,
        };
    }
    Ok(names)
}

/// `{name:Type}` starting at `at`: the name and where the placeholder ends.
/// `None` when the text there is not one — a map literal `{'k': 1}`, say.
fn placeholder(bytes: &[u8], at: usize) -> Option<(String, usize)> {
    let start = at + 1;
    let mut end = start;
    while bytes
        .get(end)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        end += 1;
    }
    let name = std::str::from_utf8(bytes.get(start..end)?).ok()?;
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut colon = end;
    while bytes.get(colon).is_some_and(u8::is_ascii_whitespace) {
        colon += 1;
    }
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    // The type may hold parentheses and quoted zones: `DateTime64(3, 'UTC')`.
    let mut depth = 0_usize;
    let mut at = colon + 1;
    loop {
        match *bytes.get(at)? {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'\'' | b'"' | b'`' => {
                at = skip_quoted(bytes, at, *bytes.get(at)?);
                continue;
            }
            b'}' if depth == 0 => return Some((name.to_owned(), at + 1)),
            _ => {}
        }
        at += 1;
    }
}

/// Past a quoted string or name: a backslash escapes, a doubled quote too.
fn skip_quoted(bytes: &[u8], at: usize, quote: u8) -> usize {
    let mut i = at + 1;
    while let Some(&c) = bytes.get(i) {
        if c == b'\\' {
            i += 2;
        } else if c == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Past a `--` or `#` comment, which ends at a line feed only.
fn skip_line(bytes: &[u8], at: usize) -> usize {
    bytes
        .get(at..)
        .and_then(|rest| rest.iter().position(|c| *c == b'\n'))
        .map_or(bytes.len(), |offset| at + offset + 1)
}

/// Past a block comment, which nests.
fn skip_block(bytes: &[u8], at: usize) -> usize {
    let mut depth = 0_usize;
    let mut i = at;
    while i < bytes.len() {
        if bytes.get(i..i + 2) == Some(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes.get(i..i + 2) == Some(b"*/") {
            depth = depth.saturating_sub(1);
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Past a `$tag$ … $tag$` string, or one byte when `$` opens none.
fn skip_heredoc(bytes: &[u8], at: usize) -> usize {
    let mut end = at + 1;
    while bytes
        .get(end)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        end += 1;
    }
    if bytes.get(end) != Some(&b'$') {
        return at + 1;
    }
    let Some(tag) = bytes.get(at..=end) else {
        return at + 1;
    };
    let body = end + 1;
    bytes
        .get(body..)
        .and_then(|rest| rest.windows(tag.len()).position(|window| window == tag))
        .map_or(bytes.len(), |offset| body + offset + tag.len())
}

/// The text the server parses as the placeholder's type; `None` is `NULL`.
fn text_of(rank: usize, value: &ScalarValue) -> Result<Option<String>> {
    Ok(Some(match value {
        ScalarValue::Null => return Ok(None),
        // `1`/`0` parse as a `Bool` and as any integer type (observed, 26.3).
        ScalarValue::Bool(value) => if *value { "1" } else { "0" }.to_owned(),
        ScalarValue::Int64(value) => value.to_string(),
        ScalarValue::Float64(value) => value.to_string(),
        ScalarValue::Decimal(text) | ScalarValue::Text(text) => text.clone(),
        ScalarValue::Json(json) => json.to_string(),
        ScalarValue::Uuid(uuid) => uuid.hyphenated().to_string(),
        ScalarValue::Date(date) => date.format("%Y-%m-%d").to_string(),
        ScalarValue::Time(time) => time.format("%H:%M:%S%.f").to_string(),
        // A wall-clock time: the server reads it in the placeholder's zone,
        // else its own. No zone is invented here.
        ScalarValue::TimestampNaive(local) => local.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
        // An instant: seconds since the epoch, which `DateTime` and
        // `DateTime64` read without a zone (observed, 26.3).
        ScalarValue::Timestamp(instant) => {
            let seconds = instant.timestamp();
            if seconds < 0 {
                return Err(refusal(rank, "an instant before 1970"));
            }
            match instant.timestamp_subsec_nanos() {
                0 => seconds.to_string(),
                nanos => format!("{seconds}.{nanos:09}"),
            }
        }
        ScalarValue::Bytes(_) => {
            return Err(refusal(
                rank,
                "bytes; bind their hex text and read it with `unhex()`",
            ));
        }
        ScalarValue::Interval { .. } => return Err(refusal(rank, "an interval")),
        ScalarValue::Array(_) => return Err(refusal(rank, "an array")),
    }))
}

fn refusal(rank: usize, what: &str) -> OxynError {
    OxynError::NotSupported {
        capability: format!("bound value {rank}: ClickHouse cannot receive {what} as a parameter"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("SELECT {id:UInt64}", &["id"])]
    #[case("SELECT {a:UInt8} + {b:UInt8} + {a:UInt8}", &["a", "b"])]
    #[case("SELECT {t:DateTime64(3, 'Europe/Paris')}, {u: String}", &["t", "u"])]
    #[case("SELECT '{x:UInt8}', `{y:UInt8}`, \"{z:UInt8}\"", &[])]
    #[case("SELECT 1 -- {x:UInt8}\n, {y:UInt8}", &["y"])]
    #[case("SELECT 1 # {x:UInt8}\n", &[])]
    #[case("SELECT /* {x:UInt8} /* nested */ {y:UInt8} */ 1", &[])]
    #[case("SELECT $t${x:UInt8}$t$, {y:UInt8}", &["y"])]
    #[case("SELECT {'k': 1}, {1:UInt8}", &[])]
    #[case(r"SELECT 'it\'s {x:UInt8}', {y:UInt8}", &["y"])]
    fn placeholders_are_read_where_clickhouse_reads_them(
        #[case] text: &str,
        #[case] expected: &[&str],
    ) {
        assert_eq!(placeholders(text).expect("readable"), expected, "{text}");
    }

    #[test]
    fn a_count_that_differs_is_refused_without_echoing_a_value() {
        let error = bind(
            "SELECT {a:UInt8}",
            &[
                ScalarValue::Text("secret-value".to_owned()),
                ScalarValue::Null,
            ],
        )
        .expect_err("refused");
        assert!(matches!(error, OxynError::Config(_)), "{error:?}");
        assert!(!error.to_string().contains("secret-value"));
    }

    #[test]
    fn values_become_the_text_the_server_parses() {
        let instant = DateTime::from_timestamp(1_700_000_000, 123_456_789).expect("valid");
        let bound = bind(
            "SELECT {n:Nullable(UInt8)}, {b:Bool}, {i:Int64}, {d:Decimal(38, 2)}, \
             {day:Date}, {at:DateTime64(9)}, {whole:DateTime}",
            &[
                ScalarValue::Null,
                ScalarValue::Bool(true),
                ScalarValue::Int64(i64::MIN),
                ScalarValue::Decimal("12345678901234567890.25".to_owned()),
                ScalarValue::Date(NaiveDate::from_ymd_opt(2024, 2, 29).expect("valid")),
                ScalarValue::Timestamp(instant),
                ScalarValue::Timestamp(DateTime::from_timestamp(5, 0).expect("valid")),
            ],
        )
        .expect("bound");
        let texts: Vec<(&str, Option<&str>)> = bound
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_deref()))
            .collect();
        assert_eq!(
            texts,
            [
                ("n", None),
                ("b", Some("1")),
                ("i", Some("-9223372036854775808")),
                ("d", Some("12345678901234567890.25")),
                ("day", Some("2024-02-29")),
                ("at", Some("1700000000.123456789")),
                ("whole", Some("5")),
            ]
        );
    }

    #[test]
    fn what_the_server_cannot_read_as_text_is_refused_by_rank() {
        for value in [
            ScalarValue::Bytes(vec![0, 1]),
            ScalarValue::Array(vec![ScalarValue::Int64(1)]),
            ScalarValue::Timestamp(DateTime::from_timestamp(-1, 0).expect("valid")),
        ] {
            let error = bind("SELECT {a:String}", &[value]).expect_err("refused");
            assert!(error.to_string().contains("bound value 1"), "{error}");
        }
    }

    #[test]
    fn an_unterminated_text_reads_without_panicking() {
        for text in [
            "SELECT '{x:UInt8}",
            "SELECT {x:",
            "SELECT {x:Array(",
            "/* {x:UInt8}",
            "$t$",
        ] {
            assert!(placeholders(text).is_ok(), "{text}");
        }
    }
}
