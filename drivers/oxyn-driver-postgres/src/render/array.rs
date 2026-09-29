//! An array nested in a rendered value, the way `array_out` prints it:
//! `{{1,2},{3,4}}`, `[0:1]={a,b}`, `{(1,1),(0,0);(2,2),(1,1)}`.
//!
//! A top-level array column is an Arrow list, one-dimensional. Inside a record
//! or a range there is no Arrow type to keep, so every shape the server sends
//! is rendered: several dimensions, lower bounds other than 1, the `box`
//! delimiter.

use std::fmt::Write as _;

use super::Rendered;
use super::value::{bounded, is_pg_space, value};
use crate::numeric::Reader;
use crate::types::{PgDecoding, TextFormat};

/// `MAXDIM` of `utils/array.h`: the server never sends more dimensions.
const MAX_DIMENSIONS: usize = 6;

/// Appends an array. The element type announced in the header is ignored, as
/// in [`split_array`](crate::decode::split_array): the decoding comes from the
/// description.
pub(crate) fn array(
    element: &PgDecoding,
    bytes: &[u8],
    out: &mut String,
    depth: usize,
) -> Rendered {
    let mut reader = Reader::new(bytes);
    let dimensions = reader.i32().ok_or("truncated array header")?;
    let _flags = reader.i32().ok_or("truncated array header")?;
    let _element_type = reader.u32().ok_or("truncated array header")?;
    let dimensions = usize::try_from(dimensions).map_err(|_| "negative array dimensions")?;
    if dimensions > MAX_DIMENSIONS {
        return Err("array with more dimensions than PostgreSQL allows");
    }

    let mut lengths = [0_usize; MAX_DIMENSIONS];
    let mut lower_bounds = [0_i32; MAX_DIMENSIONS];
    let mut count = 1_usize;
    for (length, lower_bound) in lengths.iter_mut().zip(&mut lower_bounds).take(dimensions) {
        let raw = reader.i32().ok_or("truncated array dimension")?;
        *length = usize::try_from(raw).map_err(|_| "negative array length")?;
        *lower_bound = reader.i32().ok_or("truncated array dimension")?;
        count = count.checked_mul(*length).ok_or("array length overflows")?;
    }
    let lengths = lengths.get(..dimensions).ok_or("array dimensions")?;
    let lower_bounds = lower_bounds.get(..dimensions).ok_or("array dimensions")?;
    if dimensions == 0 || count == 0 {
        out.push_str("{}");
        return Ok(());
    }
    // Each element costs at least its four length bytes: a larger count is
    // hostile, and must be refused before the loop runs that long.
    if count > reader.rest().len() / 4 {
        return Err("array length inconsistent with the buffer received");
    }

    if lower_bounds.iter().any(|lower_bound| *lower_bound != 1) {
        for (length, lower_bound) in lengths.iter().zip(lower_bounds) {
            let length = i32::try_from(*length).map_err(|_| "array length overflows")?;
            let upper_bound = lower_bound
                .checked_add(length - 1)
                .ok_or("array bound overflows")?;
            let _ = write!(out, "[{lower_bound}:{upper_bound}]");
        }
        out.push('=');
    }

    let delimiter = delimiter(element);
    let mut scratch = String::new();
    let mut level = Level {
        element,
        delimiter,
        reader: &mut reader,
        scratch: &mut scratch,
        depth,
    };
    level.render(lengths, out)
}

/// `typdelim` of the element type: `;` for `box`, whose text holds commas,
/// `,` for every other built-in type (`pg_type.dat`, `REL_18_STABLE`).
const fn delimiter(element: &PgDecoding) -> char {
    match element {
        PgDecoding::Rendered(TextFormat::Box) => ';',
        _ => ',',
    }
}

/// What every dimension of one array shares while it is rendered.
struct Level<'a, 'b> {
    element: &'a PgDecoding,
    delimiter: char,
    reader: &'a mut Reader<'b>,
    scratch: &'a mut String,
    depth: usize,
}

impl Level<'_, '_> {
    /// One dimension in braces; the recursion is bounded by
    /// [`MAX_DIMENSIONS`].
    fn render(&mut self, lengths: &[usize], out: &mut String) -> Rendered {
        let Some((length, inner)) = lengths.split_first() else {
            return self.item(out);
        };
        out.push('{');
        for rank in 0..*length {
            if rank > 0 {
                out.push(self.delimiter);
            }
            self.render(inner, out)?;
        }
        out.push('}');
        Ok(())
    }

    /// One element, quoted when `array_out` would.
    fn item(&mut self, out: &mut String) -> Rendered {
        let size = self.reader.i32().ok_or("truncated array element")?;
        if size == -1 {
            out.push_str("NULL");
            return Ok(());
        }
        let size = usize::try_from(size).map_err(|_| "negative element size")?;
        let item = self.reader.take(size).ok_or("truncated array element")?;
        self.scratch.clear();
        value(self.element, item, self.scratch, self.depth)?;
        quote_element(self.scratch, self.delimiter, out);
        bounded(out)
    }
}

/// Quotes an element when `array_out` would: empty, the word `NULL`, or a
/// character that the array syntax reads — braces, quote, backslash, the
/// delimiter, whitespace.
fn quote_element(text: &str, delimiter: char, out: &mut String) {
    let needs_quotes = text.is_empty()
        || text.eq_ignore_ascii_case("NULL")
        || text
            .chars()
            .any(|c| matches!(c, '{' | '}' | '"' | '\\') || c == delimiter || is_pg_space(c));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An `int4` array as `array_send` writes it.
    fn int4_array(dimensions: &[(i32, i32)], items: &[i32]) -> Vec<u8> {
        let mut bytes = i32::try_from(dimensions.len())
            .expect("few")
            .to_be_bytes()
            .to_vec();
        bytes.extend_from_slice(&0_i32.to_be_bytes());
        bytes.extend_from_slice(&23_u32.to_be_bytes());
        for (length, lower_bound) in dimensions {
            bytes.extend_from_slice(&length.to_be_bytes());
            bytes.extend_from_slice(&lower_bound.to_be_bytes());
        }
        for item in items {
            bytes.extend_from_slice(&4_i32.to_be_bytes());
            bytes.extend_from_slice(&item.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn several_dimensions_nest_their_braces() {
        let bytes = int4_array(&[(2, 1), (2, 1)], &[1, 2, 3, 4]);
        let mut out = String::new();
        array(&PgDecoding::Int32, &bytes, &mut out, 0).expect("a valid array");
        assert_eq!(out, "{{1,2},{3,4}}");
    }

    #[test]
    fn a_lower_bound_other_than_one_is_printed() {
        let bytes = int4_array(&[(2, 0)], &[7, 8]);
        let mut out = String::new();
        array(&PgDecoding::Int32, &bytes, &mut out, 0).expect("a valid array");
        assert_eq!(out, "[0:1]={7,8}");
    }

    #[test]
    fn an_empty_array_has_no_dimension() {
        let bytes = int4_array(&[], &[]);
        let mut out = String::new();
        array(&PgDecoding::Int32, &bytes, &mut out, 0).expect("a valid array");
        assert_eq!(out, "{}");
    }

    #[test]
    fn a_hostile_shape_is_refused_before_looping() {
        let mut out = String::new();
        let huge = int4_array(&[(i32::MAX, 1), (i32::MAX, 1)], &[]);
        assert!(array(&PgDecoding::Int32, &huge, &mut out, 0).is_err());
        let deep = int4_array(&[(1, 1); 7], &[1]);
        assert!(array(&PgDecoding::Int32, &deep, &mut out, 0).is_err());
        let bound = int4_array(&[(2, i32::MAX)], &[1, 2]);
        assert!(array(&PgDecoding::Int32, &bound, &mut out, 0).is_err());
    }

    #[test]
    fn the_box_delimiter_leaves_commas_unquoted() {
        let mut out = String::new();
        quote_element("(1,1),(0,0)", ';', &mut out);
        assert_eq!(out, "(1,1),(0,0)");
        out.clear();
        quote_element("a,b", ',', &mut out);
        assert_eq!(out, "\"a,b\"");
    }
}
