//! A `record` or a composite value, the way `record_out` prints it: `(1,abc,)`.

use super::Rendered;
use super::value::{by_oid, is_pg_space};
use crate::numeric::Reader;

/// Appends a record. Each field is rendered according to the type OID it
/// carries on the wire; a `NULL` field prints nothing.
pub(crate) fn record(bytes: &[u8], out: &mut String, depth: usize) -> Rendered {
    let mut reader = Reader::new(bytes);
    let count = reader.i32().ok_or("truncated record")?;
    let count = usize::try_from(count).map_err(|_| "negative record field count")?;
    // Each field costs at least eight bytes (OID and length): a larger count is
    // hostile, and must be refused before the loop runs that long.
    if count > reader.rest().len() / 8 {
        return Err("record field count inconsistent with the buffer received");
    }

    let mut scratch = String::new();
    out.push('(');
    for rank in 0..count {
        if rank > 0 {
            out.push(',');
        }
        let type_oid = reader.u32().ok_or("truncated record field")?;
        let length = reader.i32().ok_or("truncated record field")?;
        if length == -1 {
            continue;
        }
        let length = usize::try_from(length).map_err(|_| "negative record field length")?;
        let encoded = reader.take(length).ok_or("truncated record field")?;
        scratch.clear();
        by_oid(type_oid, encoded, &mut scratch, depth)?;
        quote_field(&scratch, out);
    }
    out.push(')');
    Ok(())
}

/// Quotes a field when `record_out` would: empty, or holding a character of the
/// record syntax. Unlike a range bound, brackets do not force quotes.
fn quote_field(text: &str, out: &mut String) {
    let needs_quotes = text.is_empty()
        || text
            .chars()
            .any(|c| matches!(c, '"' | '\\' | '(' | ')' | ',') || is_pg_space(c));
    if !needs_quotes {
        out.push_str(text);
        return;
    }
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('"');
}
