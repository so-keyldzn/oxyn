//! Standard base64 (RFC 4648 §4), both ways.
//!
//! Hrana carries a blob as base64 in JSON and says no more than "base64"
//! (RESEARCH-NOTES, 2026-11-04). Whether a server writes it padded is not
//! known, so the decoder accepts both forms and the encoder writes the
//! standard, padded one. About forty lines are not worth a dependency on the
//! external boundary.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The text of `bytes`, padded with `=`.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().unwrap_or(0);
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        let word = (u32::from(first) << 16)
            | (u32::from(second.unwrap_or(0)) << 8)
            | u32::from(third.unwrap_or(0));
        let at = |shift: u32| {
            char::from(ALPHABET[usize::try_from((word >> shift) & 0x3f).unwrap_or(0) % 64])
        };
        out.push(at(18));
        out.push(at(12));
        out.push(if second.is_some() { at(6) } else { '=' });
        out.push(if third.is_some() { at(0) } else { '=' });
    }
    out
}

/// The bytes of `text`, or `None` when it is not base64.
///
/// Padding is optional, but when present it must complete the last group;
/// the unused bits of the last character must be zero, so two texts never
/// decode to the same bytes. Whitespace is refused: Hrana sends none.
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let trimmed = text
        .strip_suffix("==")
        .or_else(|| text.strip_suffix('='))
        .unwrap_or(text);
    let padding = text.len() - trimmed.len();
    if padding > 0 && !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() / 4 * 3 + 2);
    let mut word: u32 = 0;
    let mut bits: u32 = 0;
    for byte in trimmed.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        word = (word << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((word >> bits) & 0xff).ok()?);
            word &= (1 << bits) - 1;
        }
    }
    // One character alone carries 6 bits: not even a byte.
    if trimmed.len() % 4 == 1 || word != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(b"" as &[u8], "")]
    #[case(b"f", "Zg==")]
    #[case(b"fo", "Zm8=")]
    #[case(b"foo", "Zm9v")]
    #[case(b"foob", "Zm9vYg==")]
    #[case(b"fooba", "Zm9vYmE=")]
    #[case(b"foobar", "Zm9vYmFy")]
    #[case(&[0xff, 0xfe, 0xfd], "//79")]
    fn the_rfc_vectors_round_trip(#[case] bytes: &[u8], #[case] text: &str) {
        assert_eq!(encode(bytes), text);
        assert_eq!(decode(text).as_deref(), Some(bytes));
    }

    #[test]
    fn unpadded_text_is_read() {
        assert_eq!(decode("Zg").as_deref(), Some(&b"f"[..]));
        assert_eq!(decode("Zm9vYmE").as_deref(), Some(&b"fooba"[..]));
    }

    #[rstest]
    #[case("Z")]
    #[case("Zg=")]
    #[case("Zg===")]
    #[case("Zh==")]
    #[case("Zm9 v")]
    #[case("Zm9v\n")]
    #[case("Zm9v!")]
    #[case("=")]
    fn what_is_not_base64_is_refused(#[case] text: &str) {
        assert_eq!(decode(text), None, "{text:?}");
    }

    #[test]
    fn every_byte_value_survives() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
    }
}
