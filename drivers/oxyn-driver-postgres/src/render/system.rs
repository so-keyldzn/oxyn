//! Bit strings, `pg_lsn`, `tid` and transaction snapshots.
//!
//! Layouts and outputs are those of
//! [RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md#postgresql-binary-wire-formats--checked-on-2026-09-28).

use std::fmt::Write as _;

use super::Rendered;
use crate::numeric::Reader;

/// Bytes a transaction ID takes in a snapshot.
const XID8_LEN: usize = 8;

/// `bit` and `varbit`: a bit count, then the bits, most significant first.
///
/// The last byte may carry padding bits beyond the count: the server zeroes
/// them, but a hostile or buggy one could not, and they are not part of the
/// value — they are ignored rather than printed.
pub(crate) fn bits(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let length = reader.i32().ok_or("truncated bit string")?;
    let length = usize::try_from(length).map_err(|_| "negative bit string length")?;
    let payload = reader.rest();
    if payload.len() != length.div_ceil(8) {
        return Err("bit string length inconsistent with its bytes");
    }

    out.reserve(length);
    let mut remaining = length;
    for byte in payload {
        let in_this_byte = remaining.min(8);
        for shift in 0..in_this_byte {
            let mask = 0x80_u8 >> shift;
            out.push(if byte & mask == 0 { '0' } else { '1' });
        }
        remaining -= in_this_byte;
    }
    Ok(())
}

/// `pg_lsn`: 64 bits, printed as two unpadded uppercase hexadecimal halves.
pub(crate) fn lsn(bytes: &[u8], out: &mut String) -> Rendered {
    let encoded: [u8; 8] = bytes.try_into().map_err(|_| "pg_lsn of unexpected size")?;
    let value = u64::from_be_bytes(encoded);
    let high = value >> 32;
    let low = value & 0xFFFF_FFFF;
    let _ = write!(out, "{high:X}/{low:X}");
    Ok(())
}

/// `tid`: a block number and an offset in that block.
pub(crate) fn tid(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let block = reader.u32().ok_or("truncated tid")?;
    let offset = reader.u16().ok_or("truncated tid")?;
    if !reader.rest().is_empty() {
        return Err("tid of unexpected size");
    }
    let _ = write!(out, "({block},{offset})");
    Ok(())
}

/// `pg_snapshot` and `txid_snapshot`: `xmin:xmax:xip1,xip2`.
///
/// On the wire the count of in-progress IDs comes **first**, before `xmin`
/// and `xmax`; in the text it is implicit. The count is checked against the
/// buffer before anything is read: a hostile count would otherwise announce
/// more IDs than the value holds.
pub(crate) fn snapshot(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let count = reader.i32().ok_or("truncated snapshot")?;
    let count = usize::try_from(count).map_err(|_| "negative snapshot count")?;
    let xmin = reader.u64().ok_or("truncated snapshot")?;
    let xmax = reader.u64().ok_or("truncated snapshot")?;
    if Some(reader.rest().len()) != count.checked_mul(XID8_LEN) {
        return Err("snapshot count inconsistent with its bytes");
    }

    let _ = write!(out, "{xmin}:{xmax}:");
    for rank in 0..count {
        let xip = reader.u64().ok_or("truncated snapshot")?;
        if rank > 0 {
            out.push(',');
        }
        let _ = write!(out, "{xip}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors read from PostgreSQL 17.11: `v::text` and `*_send(v)`.
    fn render(
        renderer: fn(&[u8], &mut String) -> Rendered,
        hex: &str,
    ) -> Result<String, &'static str> {
        let bytes = decode_hex(hex);
        let mut out = String::new();
        renderer(&bytes, &mut out).map(|()| out)
    }

    fn decode_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(hex.get(i..i + 2).expect("even length"), 16).expect("hex"))
            .collect()
    }

    #[test]
    fn bit_strings_match_the_server() {
        for (hex, text) in [
            ("00000000", ""),
            ("0000000180", "1"),
            ("00000003a0", "101"),
            ("00000008aa", "10101010"),
            ("00000009aa80", "101010101"),
            ("000000100001", "0000000000000001"),
            ("0000000a8000", "1000000000"),
        ] {
            assert_eq!(render(bits, hex).as_deref(), Ok(text), "{hex}");
        }
    }

    #[test]
    fn padding_bits_are_ignored() {
        // `101` followed by five set padding bits.
        assert_eq!(render(bits, "00000003bf").as_deref(), Ok("101"));
    }

    #[test]
    fn a_bit_length_inconsistent_with_the_bytes_is_refused() {
        for hex in ["", "000000", "00000009aa", "00000003a0a0", "ffffffffaa"] {
            assert!(render(bits, hex).is_err(), "{hex}");
        }
    }

    #[test]
    fn lsns_match_the_server() {
        for (hex, text) in [
            ("0000000000000000", "0/0"),
            ("00000016b374d848", "16/B374D848"),
            ("ffffffffffffffff", "FFFFFFFF/FFFFFFFF"),
            ("0000000000000001", "0/1"),
            ("0000000100000000", "1/0"),
        ] {
            assert_eq!(render(lsn, hex).as_deref(), Ok(text), "{hex}");
        }
        assert!(render(lsn, "00000016").is_err());
    }

    #[test]
    fn tids_match_the_server() {
        for (hex, text) in [
            ("000000000001", "(0,1)"),
            ("ffffffffffff", "(4294967295,65535)"),
            ("0000000c0000", "(12,0)"),
        ] {
            assert_eq!(render(tid, hex).as_deref(), Ok(text), "{hex}");
        }
        assert!(render(tid, "0000000c00").is_err());
        assert!(render(tid, "0000000c000000").is_err());
    }

    #[test]
    fn snapshots_match_the_server() {
        for (hex, text) in [
            ("00000000000000000000000a0000000000000014", "10:20:"),
            (
                "00000003000000000000000a0000000000000014000000000000000c000000000000000f0000000000000012",
                "10:20:12,15,18",
            ),
            (
                "00000000ffffffffffffffffffffffffffffffff",
                "18446744073709551615:18446744073709551615:",
            ),
            (
                "00000002000000000000000a0000000000000014000000000000000c000000000000000f",
                "10:20:12,15",
            ),
        ] {
            assert_eq!(render(snapshot, hex).as_deref(), Ok(text), "{hex}");
        }
    }

    #[test]
    fn a_hostile_snapshot_count_is_refused_before_reading() {
        for hex in [
            // Announces two billion IDs, carries none.
            "7fffffff000000000000000a0000000000000014",
            "ffffffff000000000000000a0000000000000014",
            // Announces one, carries half of it.
            "00000001000000000000000a000000000000001400000000",
            "0000000000000000",
        ] {
            assert!(render(snapshot, hex).is_err(), "{hex}");
        }
    }
}
