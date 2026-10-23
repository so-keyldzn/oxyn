//! Where a `RowBinary` row ends, before anything is decoded.
//!
//! The HTTP body arrives in chunks that cut rows anywhere. A row is decoded
//! only once it is **whole** in the buffer: [`measure`] walks it without
//! building anything and says where it ends, or that more bytes are needed.
//! An Arrow builder cannot take back half a row, so this order is what keeps a
//! batch consistent.
//!
//! Everything here reads server bytes: no indexing, no `unwrap`, no unchecked
//! arithmetic ([I-09](../../../CLAUDE.md#i-09)). A row larger than
//! [`MAX_ROW_BYTES`] is refused before it is buffered further
//! ([I-06](../../../CLAUDE.md#i-06)).

use crate::schema::Column;
use crate::types::ChType;

/// The largest row the driver buffers before refusing it.
///
/// A row must be whole before it is decoded, so this is the most the driver
/// holds for one row besides the batch being built. 64 MiB lets a large text
/// through and stops a single `String` the size of the machine's memory.
pub const MAX_ROW_BYTES: usize = 64 << 20;

/// Why a value could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The value does not match its announced type.
    #[error("column `{column}`: {reason}")]
    Value {
        /// The column, as the server names it.
        column: String,
        /// What was wrong, in fixed words: the value is never repeated.
        reason: &'static str,
    },
    /// A row larger than [`MAX_ROW_BYTES`].
    #[error(
        "a row exceeds {} MiB: read large values in parts (`substring`, `arraySlice`)",
        MAX_ROW_BYTES >> 20
    )]
    RowTooLarge,
    /// The response ended in the middle of a row.
    #[error("the response ended in the middle of a row")]
    Truncated,
    /// Arrow refused the assembled arrays.
    #[error("assembling the batch: {0}")]
    Assembly(String),
}

/// A NULL marker that is neither 0 nor 1.
pub(crate) const BAD_MARKER: &str = "a NULL marker is neither 0 nor 1";
const TOO_LONG: &str = "a length that does not fit 64 bits";

pub(crate) fn value_error(column: &str, reason: &'static str) -> DecodeError {
    DecodeError::Value {
        column: column.to_owned(),
        reason,
    }
}

/// Where a row ends, or `None` when the buffer holds only part of it.
///
/// # Errors
/// [`DecodeError::RowTooLarge`] when the row already exceeds
/// [`MAX_ROW_BYTES`], [`DecodeError::Value`] for a marker or length no valid
/// row carries.
pub fn measure(columns: &[Column], buf: &[u8]) -> Result<Option<usize>, DecodeError> {
    let limit = buf.len().min(MAX_ROW_BYTES);
    let mut at = 0_usize;
    for column in columns {
        match skip(&column.ty, buf, at, limit, &column.name)? {
            Some(end) => at = end,
            None if buf.len() >= MAX_ROW_BYTES => return Err(DecodeError::RowTooLarge),
            None => return Ok(None),
        }
    }
    Ok(Some(at))
}

/// The end of one value starting at `at`, within `limit`.
fn skip(
    ty: &ChType,
    buf: &[u8],
    at: usize,
    limit: usize,
    column: &str,
) -> Result<Option<usize>, DecodeError> {
    let fixed = |width: usize| Ok(at.checked_add(width).filter(|end| *end <= limit));
    match ty {
        ChType::Bool | ChType::Enum { bits: 8, .. } => fixed(1),
        ChType::Enum { .. } | ChType::BFloat16 | ChType::Date => fixed(2),
        ChType::Int(bits) | ChType::UInt(bits) => fixed(usize::from(*bits / 8)),
        ChType::Float32 | ChType::Date32 | ChType::DateTime(_) | ChType::Time | ChType::Ipv4 => {
            fixed(4)
        }
        ChType::Float64 | ChType::DateTime64 { .. } | ChType::Time64(_) | ChType::Interval => {
            fixed(8)
        }
        ChType::Uuid | ChType::Ipv6 => fixed(16),
        ChType::Decimal { precision, .. } => fixed(decimal_width(*precision)),
        ChType::FixedString(width) if *width > MAX_ROW_BYTES => Err(DecodeError::RowTooLarge),
        ChType::FixedString(width) => fixed(*width),
        // `Nullable(Nothing)` is the type of a bare `NULL`, always a marker
        // of 1; a bare `Nothing` only appears in an empty array. Counting one
        // byte for it bounds a hostile count of them by the bytes received.
        ChType::Nothing => {
            let Some(&marker) = buf.get(at).filter(|_| at < limit) else {
                return Ok(None);
            };
            if marker == 1 {
                Ok(Some(at + 1))
            } else {
                Err(value_error(column, BAD_MARKER))
            }
        }
        ChType::String | ChType::Json => {
            let Some((length, start)) = varint(buf, at, limit, column)? else {
                return Ok(None);
            };
            match usize::try_from(length) {
                Ok(length) if length <= MAX_ROW_BYTES => {
                    Ok(start.checked_add(length).filter(|end| *end <= limit))
                }
                _ => Err(DecodeError::RowTooLarge),
            }
        }
        ChType::Nullable(inner) => {
            let Some(&marker) = buf.get(at).filter(|_| at < limit) else {
                return Ok(None);
            };
            match marker {
                1 => Ok(Some(at + 1)),
                0 => skip(inner, buf, at + 1, limit, column),
                _ => Err(value_error(column, BAD_MARKER)),
            }
        }
        ChType::Array(inner) => repeated(&[&**inner], buf, at, limit, column),
        ChType::Map(key, value) => repeated(&[&**key, &**value], buf, at, limit, column),
        ChType::Tuple(elements) => {
            let mut end = at;
            for (_, ty) in elements {
                match skip(ty, buf, end, limit, column)? {
                    Some(next) => end = next,
                    None => return Ok(None),
                }
            }
            Ok(Some(end))
        }
        // Refused before the first row; reaching it here is a driver bug, and
        // a refusal rather than a panic.
        ChType::Unsupported(_) => Err(value_error(column, "a type this driver does not decode")),
    }
}

/// A count, then that many groups of `types`: an array or a map.
///
/// Every value takes at least one byte, so the loop runs at most once per
/// byte received: a hostile count of 2^63 ends at the buffer's end.
fn repeated(
    types: &[&ChType],
    buf: &[u8],
    at: usize,
    limit: usize,
    column: &str,
) -> Result<Option<usize>, DecodeError> {
    let Some((count, mut end)) = varint(buf, at, limit, column)? else {
        return Ok(None);
    };
    for _ in 0..count {
        for ty in types {
            match skip(ty, buf, end, limit, column)? {
                Some(next) => end = next,
                None => return Ok(None),
            }
        }
    }
    Ok(Some(end))
}

/// The bytes of a `Decimal(P, _)` on the wire.
pub(crate) const fn decimal_width(precision: u8) -> usize {
    match precision {
        0..=9 => 4,
        10..=18 => 8,
        19..=38 => 16,
        _ => 32,
    }
}

/// A LEB128 unsigned integer at `at`: its value and where it ends.
pub(crate) fn varint(
    buf: &[u8],
    at: usize,
    limit: usize,
    column: &str,
) -> Result<Option<(u64, usize)>, DecodeError> {
    let mut value = 0_u64;
    for (shift, offset) in (0_u32..70).step_by(7).zip(at..) {
        if offset >= limit {
            return Ok(None);
        }
        let Some(&byte) = buf.get(offset) else {
            return Ok(None);
        };
        let bits = u64::from(byte & 0x7f);
        let shifted = bits
            .checked_shl(shift)
            .filter(|shifted| shifted.checked_shr(shift) == Some(bits))
            .ok_or_else(|| value_error(column, TOO_LONG))?;
        value |= shifted;
        if byte & 0x80 == 0 {
            return Ok(Some((value, offset + 1)));
        }
    }
    Err(value_error(column, TOO_LONG))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::parse;

    fn columns(types: &[&str]) -> Vec<Column> {
        types
            .iter()
            .enumerate()
            .map(|(rank, text)| Column {
                name: format!("c{rank}"),
                raw_type: (*text).to_owned(),
                ty: parse(text).expect("a valid type"),
            })
            .collect()
    }

    #[test]
    fn a_row_is_measured_whole_or_not_at_all() {
        let columns = columns(&["UInt32", "String", "Nullable(Int8)"]);
        let row = [1, 0, 0, 0, 2, b'h', b'i', 0, 7];
        assert_eq!(measure(&columns, &row), Ok(Some(row.len())));
        for cut in 0..row.len() {
            assert_eq!(
                measure(&columns, row.get(..cut).expect("in range")),
                Ok(None),
                "{cut}"
            );
        }
        let null = [1, 0, 0, 0, 0, 1];
        assert_eq!(measure(&columns, &null), Ok(Some(null.len())));
    }

    #[test]
    fn a_marker_that_is_neither_zero_nor_one_is_refused() {
        let columns = columns(&["Nullable(Int8)"]);
        assert!(matches!(
            measure(&columns, &[2, 0]),
            Err(DecodeError::Value { reason, .. }) if reason == BAD_MARKER
        ));
    }

    #[test]
    fn a_hostile_length_never_reserves_memory_or_loops() {
        // A string announcing 2^62 bytes.
        let columns = columns(&["String"]);
        let huge = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x40];
        assert_eq!(measure(&columns, &huge), Err(DecodeError::RowTooLarge));
        // An array of 2^63 empty arrays: the count runs into the end of the
        // buffer, not into an endless loop.
        let columns = self::columns(&["Array(Array(UInt8))"]);
        let mut count = vec![0xff; 8];
        count.push(0x7f);
        assert_eq!(measure(&columns, &count), Ok(None));
        // A varint longer than ten bytes.
        let columns = self::columns(&["String"]);
        assert!(matches!(
            measure(&columns, &[0xff; 11]),
            Err(DecodeError::Value { .. })
        ));
    }

    #[test]
    fn a_fixed_string_wider_than_a_row_is_refused_before_buffering() {
        let columns = columns(&["FixedString(100000000)"]);
        assert_eq!(measure(&columns, &[0; 8]), Err(DecodeError::RowTooLarge));
    }

    #[test]
    fn decimal_widths_follow_the_precision() {
        assert_eq!(decimal_width(9), 4);
        assert_eq!(decimal_width(18), 8);
        assert_eq!(decimal_width(38), 16);
        assert_eq!(decimal_width(76), 32);
    }
}
