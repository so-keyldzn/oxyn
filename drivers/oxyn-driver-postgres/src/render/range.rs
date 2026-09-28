//! Ranges and multiranges, the way `range_out` and `multirange_out` print them.

use super::Rendered;
use super::value::{bounded, is_pg_space, value};
use crate::numeric::Reader;
use crate::types::PgDecoding;

/// The range is empty.
const EMPTY: u8 = 0x01;
/// The lower bound is included.
const LOWER_INCLUSIVE: u8 = 0x02;
/// The upper bound is included.
const UPPER_INCLUSIVE: u8 = 0x04;
/// There is no lower bound.
const LOWER_INFINITE: u8 = 0x08;
/// There is no upper bound.
const UPPER_INFINITE: u8 = 0x10;

/// Appends a range: `[1,5)`, `(,10]`, `empty`.
pub(crate) fn range(
    element: &PgDecoding,
    bytes: &[u8],
    out: &mut String,
    depth: usize,
) -> Rendered {
    let mut reader = Reader::new(bytes);
    let flags = reader.u8().ok_or("empty range")?;
    if flags & EMPTY != 0 {
        out.push_str("empty");
        return Ok(());
    }

    out.push(if flags & LOWER_INCLUSIVE != 0 {
        '['
    } else {
        '('
    });
    let mut scratch = String::new();
    if flags & LOWER_INFINITE == 0 {
        bound(element, &mut reader, &mut scratch, out, depth)?;
    }
    out.push(',');
    if flags & UPPER_INFINITE == 0 {
        bound(element, &mut reader, &mut scratch, out, depth)?;
    }
    out.push(if flags & UPPER_INCLUSIVE != 0 {
        ']'
    } else {
        ')'
    });
    Ok(())
}

/// Appends a multirange: `{[1,5),[7,9)}`, `{}`.
pub(crate) fn multirange(
    element: &PgDecoding,
    bytes: &[u8],
    out: &mut String,
    depth: usize,
) -> Rendered {
    let mut reader = Reader::new(bytes);
    let count = reader.i32().ok_or("truncated multirange")?;
    let count = usize::try_from(count).map_err(|_| "negative multirange count")?;
    out.push('{');
    for rank in 0..count {
        if rank > 0 {
            out.push(',');
        }
        let length = reader.i32().ok_or("truncated multirange")?;
        let length = usize::try_from(length).map_err(|_| "negative range length")?;
        let encoded = reader.take(length).ok_or("truncated multirange")?;
        range(element, encoded, out, depth)?;
        bounded(out)?;
    }
    out.push('}');
    Ok(())
}

/// Reads one bound and appends it, quoted when `range_out` would quote it.
fn bound(
    element: &PgDecoding,
    reader: &mut Reader<'_>,
    scratch: &mut String,
    out: &mut String,
    depth: usize,
) -> Rendered {
    let length = reader.i32().ok_or("truncated range bound")?;
    let length = usize::try_from(length).map_err(|_| "negative range bound length")?;
    let encoded = reader.take(length).ok_or("truncated range bound")?;
    scratch.clear();
    value(element, encoded, scratch, depth)?;
    quote_bound(scratch, out);
    bounded(out)
}

/// Quotes a bound when it is empty or holds a character of the range syntax —
/// which a timestamp's space is enough to trigger.
fn quote_bound(text: &str, out: &mut String) {
    let needs_quotes = text.is_empty()
        || text
            .chars()
            .any(|c| matches!(c, '"' | '\\' | '(' | ')' | '[' | ']' | ',') || is_pg_space(c));
    if !needs_quotes {
        out.push_str(text);
        return;
    }
    out.push('"');
    for c in text.chars() {
        // `range_out` doubles the quote and the backslash instead of
        // escaping them with a backslash, unlike `array_out`.
        if matches!(c, '"' | '\\') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('"');
}
