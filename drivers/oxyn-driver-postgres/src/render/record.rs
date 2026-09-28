//! A `record` or a composite value, the way `record_out` prints it: `(1,abc,)`.

use super::Rendered;
use super::value::{bounded, by_oid, is_pg_space, value};
use crate::numeric::Reader;
use crate::types::PgDecoding;

/// Appends a record; a `NULL` field prints nothing. A field is rendered with
/// its resolved decoding when its wire OID matches the one `sqlx` resolved,
/// by the OID alone otherwise: the wire is what the server sent, the
/// resolution what it described.
pub(crate) fn record(
    fields: &[(u32, PgDecoding)],
    bytes: &[u8],
    out: &mut String,
    depth: usize,
) -> Rendered {
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
        match fields.get(rank) {
            Some((resolved, decoding)) if *resolved == type_oid => {
                value(decoding, encoded, &mut scratch, depth)?;
            }
            _ => by_oid(type_oid, encoded, &mut scratch, depth)?,
        }
        quote_field(&scratch, out);
        bounded(out)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A record of one field, as `record_send` writes it.
    fn one_field(type_oid: u32, value: &[u8]) -> Vec<u8> {
        let mut bytes = 1_i32.to_be_bytes().to_vec();
        bytes.extend_from_slice(&type_oid.to_be_bytes());
        bytes.extend_from_slice(&i32::try_from(value.len()).expect("short").to_be_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    #[test]
    fn a_field_is_quoted_like_record_out() {
        // `SELECT ROW('a b', NULL, 1)` prints `("a b",,1)`.
        let mut bytes = 3_i32.to_be_bytes().to_vec();
        for (type_oid, value) in [
            (25_u32, Some(&b"a b"[..])),
            (25, None),
            (23, Some(&[0, 0, 0, 1][..])),
        ] {
            bytes.extend_from_slice(&type_oid.to_be_bytes());
            match value {
                Some(value) => {
                    bytes.extend_from_slice(
                        &i32::try_from(value.len()).expect("short").to_be_bytes(),
                    );
                    bytes.extend_from_slice(value);
                }
                None => bytes.extend_from_slice(&(-1_i32).to_be_bytes()),
            }
        }
        let mut out = String::new();
        record(&[], &bytes, &mut out, 0).expect("a valid record");
        assert_eq!(out, "(\"a b\",,1)");
    }

    #[test]
    fn a_resolved_field_is_rendered_with_its_decoding_only_if_the_oid_matches() {
        // An `enum` has a per-installation OID: only the resolution knows its
        // binary form is its label.
        let bytes = one_field(90_001, b"happy");
        let fields = [(90_001, PgDecoding::Text)];
        let mut out = String::new();
        record(&fields, &bytes, &mut out, 0).expect("a valid record");
        assert_eq!(out, "(happy)");

        // The wire disagrees with the resolution: the wire wins, unknown OID
        // stays bytes.
        let mismatched = [(90_002, PgDecoding::Text)];
        out.clear();
        record(&mismatched, &bytes, &mut out, 0).expect("a valid record");
        assert_eq!(out, "(\"\\\\x6861707079\")");
    }

    #[test]
    fn nested_quoting_is_bounded_not_exponential() {
        // Thirty records around one `"`: a few hundred bytes on the wire, and
        // each level doubles the quotes of the one below. Unbounded, this is
        // gigabytes of text for one cell.
        let mut bytes = one_field(25, b"\"");
        for _ in 0..30 {
            bytes = one_field(2249, &bytes);
        }
        assert!(bytes.len() < 512, "the hostile value is small on the wire");
        let mut out = String::new();
        let issue = record(&[], &bytes, &mut out, 0);
        assert!(issue.is_err(), "a rendering past the bound is refused");
        assert!(out.len() <= 2 * crate::render::value::MAX_RENDERED_BYTES + 2);
    }

    #[test]
    fn a_hostile_field_count_is_refused_before_looping() {
        let mut out = String::new();
        let issue = record(&[], &i32::MAX.to_be_bytes(), &mut out, 0);
        assert!(issue.is_err());
    }
}
