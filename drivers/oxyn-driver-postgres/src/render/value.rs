//! Any decodable value rendered as PostgreSQL's text.
//!
//! A column of `int4` becomes an Arrow `Int32`, but a range bound, a record
//! field or an array element nested in them has no column of its own: it is
//! part of a text, and must read the way the server would print it. This module
//! is that path, shared by [`super::range`] and [`super::record`].

use std::fmt::Write as _;

use super::{Rendered, datetime, float, geometry, network, system, textsearch};
use crate::numeric::{Reader, render_binary};
use crate::types::{PgDecoding, TextFormat, decoding_for_builtin_oid};

/// How deep ranges, records and arrays may nest inside one another.
///
/// The server's own types never come close; a deeper value is hostile, and
/// following it would exhaust the stack — an abort that no `Result` catches.
pub(crate) const MAX_DEPTH: usize = 32;

/// Appends the text of a value of this decoding.
pub(crate) fn value(
    decoding: &PgDecoding,
    bytes: &[u8],
    out: &mut String,
    depth: usize,
) -> Rendered {
    let depth = depth.checked_add(1).ok_or("value nested too deeply")?;
    if depth > MAX_DEPTH {
        return Err("value nested too deeply");
    }
    match decoding {
        PgDecoding::Bool => {
            let byte = bytes.first().ok_or("empty boolean")?;
            out.push(if *byte == 0 { 'f' } else { 't' });
        }
        PgDecoding::Int16 => {
            let _ = write!(out, "{}", Reader::new(bytes).i16().ok_or("truncated int2")?);
        }
        PgDecoding::Int32 => {
            let _ = write!(out, "{}", Reader::new(bytes).i32().ok_or("truncated int4")?);
        }
        PgDecoding::Int64 => {
            let _ = write!(out, "{}", Reader::new(bytes).i64().ok_or("truncated int8")?);
        }
        PgDecoding::UInt32 => {
            let _ = write!(out, "{}", Reader::new(bytes).u32().ok_or("truncated oid")?);
        }
        PgDecoding::UInt64 => {
            let _ = write!(out, "{}", Reader::new(bytes).u64().ok_or("truncated xid8")?);
        }
        PgDecoding::Float32 => {
            let encoded: [u8; 4] = bytes.try_into().map_err(|_| "float4 of unexpected size")?;
            float::float4(f32::from_be_bytes(encoded), out);
        }
        PgDecoding::Float64 => {
            let encoded: [u8; 8] = bytes.try_into().map_err(|_| "float8 of unexpected size")?;
            float::float8(f64::from_be_bytes(encoded), out);
        }
        PgDecoding::Text => {
            out.push_str(std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 text from the server")?);
        }
        PgDecoding::Jsonb => {
            let (version, tail) = bytes.split_first().ok_or("empty jsonb")?;
            if *version != 1 {
                return Err("unknown jsonb version");
            }
            out.push_str(std::str::from_utf8(tail).map_err(|_| "non-UTF-8 jsonb")?);
        }
        PgDecoding::Numeric => out.push_str(&render_binary(bytes).ok_or("unreadable numeric")?),
        PgDecoding::Uuid => {
            let encoded: [u8; 16] = bytes.try_into().map_err(|_| "uuid of unexpected size")?;
            let mut buffer = uuid::Uuid::encode_buffer();
            out.push_str(
                uuid::Uuid::from_bytes(encoded)
                    .hyphenated()
                    .encode_lower(&mut buffer),
            );
        }
        PgDecoding::TimeTz => timetz(bytes, out)?,
        PgDecoding::Bytes | PgDecoding::Opaque => hex(bytes, out),
        PgDecoding::Date => {
            datetime::date(Reader::new(bytes).i32().ok_or("truncated date")?, out)?;
        }
        PgDecoding::Time => {
            datetime::time(Reader::new(bytes).i64().ok_or("truncated time")?, out)?;
        }
        PgDecoding::Timestamp => {
            datetime::timestamp(Reader::new(bytes).i64().ok_or("truncated timestamp")?, out)?;
        }
        PgDecoding::TimestampTz => {
            datetime::timestamptz(
                Reader::new(bytes).i64().ok_or("truncated timestamptz")?,
                out,
            )?;
        }
        PgDecoding::Interval => {
            let mut reader = Reader::new(bytes);
            let micros = reader.i64().ok_or("truncated interval")?;
            let days = reader.i32().ok_or("interval without days")?;
            let months = reader.i32().ok_or("interval without months")?;
            datetime::interval(micros, days, months, out)?;
        }
        PgDecoding::Rendered(format) => rendered(*format, bytes, out)?,
        PgDecoding::Range(element) => super::range::range(element, bytes, out, depth)?,
        PgDecoding::Multirange(element) => super::range::multirange(element, bytes, out, depth)?,
        PgDecoding::Record => super::record::record(bytes, out, depth)?,
        PgDecoding::List(element) => array(element, bytes, out, depth)?,
    }
    Ok(())
}

/// Appends the text of a value known only by its type OID, as a record field is.
///
/// A built-in OID gets its decoding; any other — an enum, a domain, an
/// extension type — has no layout the driver can trust, and keeps its bytes in
/// the `\x` form `bytea` uses, rather than a guess.
pub(crate) fn by_oid(type_oid: u32, bytes: &[u8], out: &mut String, depth: usize) -> Rendered {
    let decoding = decoding_for_builtin_oid(type_oid).unwrap_or(PgDecoding::Opaque);
    value(&decoding, bytes, out, depth)
}

/// Appends the text of one of the built-in layouts.
pub(crate) fn rendered(format: TextFormat, bytes: &[u8], out: &mut String) -> Rendered {
    match format {
        TextFormat::Inet => network::inet(bytes, out),
        TextFormat::MacAddr => network::macaddr(bytes, out),
        TextFormat::Bits => system::bits(bytes, out),
        TextFormat::Point => geometry::point(bytes, out),
        TextFormat::Line => geometry::line(bytes, out),
        TextFormat::Lseg => geometry::lseg(bytes, out),
        TextFormat::Box => geometry::rect(bytes, out),
        TextFormat::Path => geometry::path(bytes, out),
        TextFormat::Polygon => geometry::polygon(bytes, out),
        TextFormat::Circle => geometry::circle(bytes, out),
        TextFormat::Lsn => system::lsn(bytes, out),
        TextFormat::Tid => system::tid(bytes, out),
        TextFormat::Snapshot => system::snapshot(bytes, out),
        TextFormat::TsVector => textsearch::tsvector(bytes, out),
        TextFormat::TsQuery => textsearch::tsquery(bytes, out),
        TextFormat::Void => Ok(()),
    }
}

/// `\x` then two lowercase hex digits per byte: `bytea`'s default output.
fn hex(bytes: &[u8], out: &mut String) {
    out.reserve(bytes.len().saturating_mul(2).saturating_add(2));
    out.push_str("\\x");
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
}

/// A `timetz` the way `timetz_out` prints it: the time, then the offset east of
/// UTC in `±HH`, with `:MM` and `:SS` only when they are not zero.
fn timetz(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let micros = reader.i64().ok_or("truncated timetz")?;
    // The wire counts the offset in seconds **west** of UTC; the text counts
    // east. Missing the flip shifts the value by twice its offset.
    let west = reader.i32().ok_or("timetz without an offset")?;
    datetime::time(micros, out)?;
    let east = west.checked_neg().ok_or("nonsensical timetz offset")?;
    out.push(if east < 0 { '-' } else { '+' });
    let absolute = east.unsigned_abs();
    let (hours, minutes, seconds) = (absolute / 3_600, (absolute % 3_600) / 60, absolute % 60);
    let _ = write!(out, "{hours:02}");
    if minutes != 0 || seconds != 0 {
        let _ = write!(out, ":{minutes:02}");
    }
    if seconds != 0 {
        let _ = write!(out, ":{seconds:02}");
    }
    Ok(())
}

/// The whitespace C's `isspace` sees in the `C` locale, which is what the
/// server's `*_out` functions test before quoting. Rust's
/// `is_ascii_whitespace` leaves out the vertical tab, and an unquoted `\v`
/// would not read back.
pub(crate) const fn is_pg_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C')
}

/// A one-dimensional array the way `array_out` prints it: `{a,b,NULL}`.
fn array(element: &PgDecoding, bytes: &[u8], out: &mut String, depth: usize) -> Rendered {
    let elements = crate::decode::split_array(bytes)?;
    let mut scratch = String::new();
    out.push('{');
    for (rank, item) in elements.iter().enumerate() {
        if rank > 0 {
            out.push(',');
        }
        match item {
            None => out.push_str("NULL"),
            Some(item) => {
                scratch.clear();
                value(element, item, &mut scratch, depth)?;
                quote_array_element(&scratch, out);
            }
        }
    }
    out.push('}');
    Ok(())
}

/// Quotes an array element when `array_out` would: empty, the word `NULL`, or
/// a character that the array syntax reads — braces, quote, comma, backslash,
/// whitespace.
fn quote_array_element(text: &str, out: &mut String) {
    let needs_quotes = text.is_empty()
        || text.eq_ignore_ascii_case("NULL")
        || text
            .chars()
            .any(|c| matches!(c, '{' | '}' | '"' | ',' | '\\') || is_pg_space(c));
    if !needs_quotes {
        out.push_str(text);
        return;
    }
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}
