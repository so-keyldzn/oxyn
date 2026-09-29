//! The geometric types: `point`, `line`, `lseg`, `box`, `path`, `polygon`, `circle`.
//!
//! Every coordinate is a big-endian `float8`, printed like any `float8`
//! ([`super::float::float8`]): the server does the same, so `1e20` comes out as
//! `1e+20` in a point as in a column of its own.

use super::Rendered;
use super::float::float8;
use crate::numeric::Reader;

/// Bytes of one point on the wire: two `float8`.
const POINT_LEN: usize = 16;

/// `(x,y)`.
pub(crate) fn point(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    write_point(&mut reader, out).ok_or("truncated point")?;
    finished(&reader)
}

/// `{A,B,C}`: the coefficients of `Ax + By + C = 0`.
pub(crate) fn line(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let a = reader.f64().ok_or("truncated line")?;
    let b = reader.f64().ok_or("truncated line")?;
    let c = reader.f64().ok_or("truncated line")?;
    finished(&reader)?;
    out.push('{');
    float8(a, out);
    out.push(',');
    float8(b, out);
    out.push(',');
    float8(c, out);
    out.push('}');
    Ok(())
}

/// `[(x1,y1),(x2,y2)]`.
pub(crate) fn lseg(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    out.push('[');
    write_point(&mut reader, out).ok_or("truncated lseg")?;
    out.push(',');
    write_point(&mut reader, out).ok_or("truncated lseg")?;
    out.push(']');
    finished(&reader)
}

/// `(high.x,high.y),(low.x,low.y)` — the `box` type, named `rect` because
/// `box` is a Rust keyword.
///
/// The server sends and prints the upper-right corner first; reordering the
/// corners would print a box other than the one the user stored.
pub(crate) fn rect(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    write_point(&mut reader, out).ok_or("truncated box")?;
    out.push(',');
    write_point(&mut reader, out).ok_or("truncated box")?;
    finished(&reader)
}

/// `((x,y),…)` when closed, `[(x,y),…]` when open.
pub(crate) fn path(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let closed = reader.u8().ok_or("truncated path")? != 0;
    let (open_mark, close_mark) = if closed { ('(', ')') } else { ('[', ']') };
    out.push(open_mark);
    points(&mut reader, out, "truncated path")?;
    out.push(close_mark);
    finished(&reader)
}

/// `((x,y),…)`.
pub(crate) fn polygon(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    out.push('(');
    points(&mut reader, out, "truncated polygon")?;
    out.push(')');
    finished(&reader)
}

/// `<(x,y),r>`.
pub(crate) fn circle(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    out.push('<');
    write_point(&mut reader, out).ok_or("truncated circle")?;
    let radius = reader.f64().ok_or("truncated circle")?;
    out.push(',');
    float8(radius, out);
    out.push('>');
    finished(&reader)
}

/// A point count, then as many points, comma-separated.
///
/// The count is checked against the bytes actually received before anything
/// is written: a hostile count of two billion must fail at once, not after
/// reading as far as the buffer goes.
fn points(reader: &mut Reader<'_>, out: &mut String, truncated: &'static str) -> Rendered {
    let count = reader.i32().ok_or(truncated)?;
    let count = usize::try_from(count).map_err(|_| "negative point count")?;
    if count.checked_mul(POINT_LEN) != Some(reader.rest().len()) {
        return Err("point count inconsistent with the buffer received");
    }
    for rank in 0..count {
        if rank > 0 {
            out.push(',');
        }
        write_point(reader, out).ok_or(truncated)?;
    }
    Ok(())
}

/// Reads one point and appends `(x,y)`.
fn write_point(reader: &mut Reader<'_>, out: &mut String) -> Option<()> {
    let x = reader.f64()?;
    let y = reader.f64()?;
    out.push('(');
    float8(x, out);
    out.push(',');
    float8(y, out);
    out.push(')');
    Some(())
}

/// Bytes left after a fixed-size value mean it is not the type announced.
fn finished(reader: &Reader<'_>) -> Rendered {
    if reader.rest().is_empty() {
        Ok(())
    } else {
        Err("geometric value longer than its type")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(hex.get(i..i + 2).expect("even"), 16).expect("hex"))
            .collect()
    }

    type Renderer = fn(&[u8], &mut String) -> Rendered;

    fn render(renderer: Renderer, hex: &str) -> Result<String, &'static str> {
        let mut out = String::new();
        renderer(&bytes(hex), &mut out)?;
        Ok(out)
    }

    /// `SELECT v::text, encode(<type>_send(v), 'hex')` on PostgreSQL 17.11.
    #[test]
    fn every_type_matches_the_server() {
        let cases: [(Renderer, &str, &str); 16] = [
            (point, "3ff00000000000004000000000000000", "(1,2)"),
            (point, "bff80000000000003fd0000000000000", "(-1.5,0.25)"),
            (point, "4415af1d78b58c408000000000000000", "(1e+20,-0)"),
            (
                line,
                "3ff0000000000000bff00000000000000000000000000000",
                "{1,-1,0}",
            ),
            (
                line,
                "3fe00000000000004000000000000000c00a000000000000",
                "{0.5,2,-3.25}",
            ),
            (
                lseg,
                "000000000000000000000000000000003ff00000000000003ff0000000000000",
                "[(0,0),(1,1)]",
            ),
            (
                lseg,
                "bff800000000000040000000000000004008000000000000c011000000000000",
                "[(-1.5,2),(3,-4.25)]",
            ),
            (
                rect,
                "3ff00000000000003ff000000000000000000000000000000000000000000000",
                "(1,1),(0,0)",
            ),
            (
                rect,
                "40080000000000004000000000000000bff0000000000000c012000000000000",
                "(3,2),(-1,-4.5)",
            ),
            (
                path,
                "0000000002000000000000000000000000000000003ff00000000000003ff0000000000000",
                "[(0,0),(1,1)]",
            ),
            (
                path,
                "0100000003000000000000000000000000000000003ff00000000000003ff000000000000040000000000000003fe0000000000000",
                "((0,0),(1,1),(2,0.5))",
            ),
            (
                path,
                "0000000001bff0000000000000c000000000000000",
                "[(-1,-2)]",
            ),
            (
                polygon,
                "00000003000000000000000000000000000000003ff00000000000003ff00000000000003ff00000000000000000000000000000",
                "((0,0),(1,1),(1,0))",
            ),
            (
                polygon,
                "00000001bff80000000000004000000000000000",
                "((-1.5,2))",
            ),
            (
                circle,
                "000000000000000000000000000000003ff0000000000000",
                "<(0,0),1>",
            ),
            (
                circle,
                "bff800000000000040020000000000003fe0000000000000",
                "<(-1.5,2.25),0.5>",
            ),
        ];
        for (renderer, hex, expected) in cases {
            assert_eq!(render(renderer, hex), Ok(expected.to_owned()), "{hex}");
        }
    }

    #[test]
    fn a_truncated_value_is_an_error() {
        assert!(render(point, "3ff0000000000000").is_err());
        assert!(render(line, "3ff0000000000000bff0000000000000").is_err());
        assert!(render(circle, "").is_err());
        assert!(render(path, "").is_err());
        assert!(render(rect, "3ff00000000000003ff0000000000000").is_err());
    }

    #[test]
    fn a_hostile_point_count_fails_without_reading_on() {
        // Two billion points announced, one received.
        assert!(render(polygon, "7fffffff3ff00000000000004000000000000000").is_err());
        assert!(render(path, "017fffffff3ff00000000000004000000000000000").is_err());
        assert!(render(polygon, "ffffffff").is_err());
    }

    #[test]
    fn trailing_bytes_are_an_error() {
        assert!(render(point, "3ff0000000000000400000000000000000").is_err());
    }
}
