//! Exact decoding of a PostgreSQL `NUMERIC`.
//!
//! `sqlx` 0.9 can decode `NUMERIC` only through `bigdecimal` or
//! `rust_decimal`, neither of which is in this crate's dependency contract.
//! Rather than add a dependency to rebuild a string, this module reads the
//! server's binary format — fifty lines, no loss.
//!
//! **Conversion to `f64` does not exist here, and that is the point of the
//! module.** A `NUMERIC(38, 10)` fits in no 64-bit float: converting changes
//! amounts by a fraction of a cent, which nobody sees before an accounting
//! reconciliation.
//!
//! # The format
//!
//! As written by `numeric_send`:
//!
//! | Field | Size | Meaning |
//! |---|---|---|
//! | `ndigits` | `i16` | number of base-10,000 groups |
//! | `weight` | `i16` | rank of the first group, in powers of 10,000 |
//! | `sign` | `u16` | `0x0000` positive, `0x4000` negative, `0xC000` NaN, `0xD000` +∞, `0xF000` −∞ |
//! | `dscale` | `u16` | digits to display after the decimal point |
//! | `digits` | `i16` × `ndigits` | groups, each in `0..=9999` |

use std::fmt::Write as _;

/// Beyond this, the value does not come from a `NUMERIC`: PostgreSQL bounds its
/// precision to 1,000 decimal digits, i.e. 250 groups.
///
/// The bound is not an optimization: `weight` is an `i16`, and a hostile value
/// of 32,767 would allocate 130 KB of zeros per cell.
const MAX_GROUPES: usize = 4_096;

/// Bound on the display scale. PostgreSQL limits it to 16,383.
const MAX_DSCALE: usize = 16_384;

/// Groups of four decimal digits.
const BASE: i16 = 10_000;

/// Decodes a binary-format `NUMERIC` into its exact decimal representation.
///
/// Returns `None` — never a panic, never an approximate value — if the bytes do
/// not form a readable `NUMERIC`. The caller then turns it into an opaque
/// fallback: the bytes come from the network and are not trustworthy
/// ([I-09](../../../CLAUDE.md#i-09)).
///
/// Special values are rendered as PostgreSQL writes them: `NaN`, `Infinity`,
/// `-Infinity`.
#[must_use]
pub(crate) fn render_binary(bytes: &[u8]) -> Option<String> {
    let mut lecteur = Lecteur::new(bytes);
    let ndigits = lecteur.i16()?;
    let weight = lecteur.i16()?;
    let sign = lecteur.u16()?;
    let dscale = lecteur.u16()?;

    match sign {
        0xC000 => return Some("NaN".to_owned()),
        0xD000 => return Some("Infinity".to_owned()),
        0xF000 => return Some("-Infinity".to_owned()),
        0x0000 | 0x4000 => {}
        _ => return None,
    }

    let compte = usize::try_from(ndigits).ok()?;
    if compte > MAX_GROUPES {
        return None;
    }
    let dscale = usize::from(dscale);
    if dscale > MAX_DSCALE {
        return None;
    }

    let mut groupes = Vec::with_capacity(compte);
    for _ in 0..compte {
        let groupe = lecteur.i16()?;
        if !(0..BASE).contains(&groupe) {
            return None;
        }
        groupes.push(groupe);
    }

    let poids = i32::from(weight);
    if poids > i32::try_from(MAX_GROUPES).ok()? {
        return None;
    }

    let mut sortie = String::with_capacity(dscale + 8);
    if sign == 0x4000 {
        sortie.push('-');
    }

    // Integer part: the first group is written without padding, the next ones
    // on four digits — otherwise 1 then 0002 would read 12.
    if poids < 0 {
        sortie.push('0');
    } else {
        for rang in 0..=poids {
            let groupe = groupe_at(&groupes, rang);
            if rang == 0 {
                let _ = write!(sortie, "{groupe}");
            } else {
                let _ = write!(sortie, "{groupe:04}");
            }
        }
    }

    if dscale > 0 {
        let mut fraction = String::with_capacity(dscale + 4);
        let mut rang = poids.checked_add(1)?;
        while fraction.len() < dscale {
            let _ = write!(fraction, "{:04}", groupe_at(&groupes, rang));
            rang = rang.checked_add(1)?;
        }
        fraction.truncate(dscale);
        sortie.push('.');
        sortie.push_str(&fraction);
    }

    Some(sortie)
}

/// The group of rank `rang`, or zero: the groups missing before and after
/// those the server sends are worth zero, which is what allows only the
/// significant ones to be transmitted.
fn groupe_at(groupes: &[i16], rang: i32) -> i16 {
    if rang < 0 {
        return 0;
    }
    usize::try_from(rang)
        .ok()
        .and_then(|index| groupes.get(index).copied())
        .unwrap_or(0)
}

/// Big-endian reader over a slice, without range indexing.
///
/// Written by hand for a specific reason: `bytes::Buf::get_i16` **panics** on a
/// buffer that is too short, and this buffer comes from the network
/// ([I-09](../../../CLAUDE.md#i-09)).
pub(crate) struct Lecteur<'a> {
    reste: &'a [u8],
}

impl<'a> Lecteur<'a> {
    /// A reader positioned at the start of the slice.
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { reste: bytes }
    }

    /// The bytes not read yet.
    pub(crate) const fn reste(&self) -> &'a [u8] {
        self.reste
    }

    /// Is the slice exhausted?
    ///
    /// `#[cfg(test)]`: the decoder does not use it — it reads a fixed number of
    /// groups announced by the header. The tests are what check it left nothing
    /// behind.
    #[cfg(test)]
    pub(crate) const fn est_vide(&self) -> bool {
        self.reste.is_empty()
    }

    /// `n` bytes, or `None` if there are not enough.
    pub(crate) fn prendre(&mut self, n: usize) -> Option<&'a [u8]> {
        let (debut, suite) = self.reste.split_at_checked(n)?;
        self.reste = suite;
        Some(debut)
    }

    /// A signed 16-bit integer, big-endian.
    pub(crate) fn i16(&mut self) -> Option<i16> {
        let octets: [u8; 2] = self.prendre(2)?.try_into().ok()?;
        Some(i16::from_be_bytes(octets))
    }

    /// An unsigned 16-bit integer, big-endian.
    pub(crate) fn u16(&mut self) -> Option<u16> {
        let octets: [u8; 2] = self.prendre(2)?.try_into().ok()?;
        Some(u16::from_be_bytes(octets))
    }

    /// A signed 32-bit integer, big-endian.
    pub(crate) fn i32(&mut self) -> Option<i32> {
        let octets: [u8; 4] = self.prendre(4)?.try_into().ok()?;
        Some(i32::from_be_bytes(octets))
    }

    /// An unsigned 32-bit integer, big-endian.
    pub(crate) fn u32(&mut self) -> Option<u32> {
        let octets: [u8; 4] = self.prendre(4)?.try_into().ok()?;
        Some(u32::from_be_bytes(octets))
    }

    /// A signed 64-bit integer, big-endian.
    pub(crate) fn i64(&mut self) -> Option<i64> {
        let octets: [u8; 8] = self.prendre(8)?.try_into().ok()?;
        Some(i64::from_be_bytes(octets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the binary encoding of a `NUMERIC`, as the server would.
    fn encoder(weight: i16, sign: u16, dscale: u16, groupes: &[i16]) -> Vec<u8> {
        let mut octets = Vec::new();
        let ndigits = i16::try_from(groupes.len()).expect("the test cases are short");
        octets.extend_from_slice(&ndigits.to_be_bytes());
        octets.extend_from_slice(&weight.to_be_bytes());
        octets.extend_from_slice(&sign.to_be_bytes());
        octets.extend_from_slice(&dscale.to_be_bytes());
        for groupe in groupes {
            octets.extend_from_slice(&groupe.to_be_bytes());
        }
        octets
    }

    #[test]
    fn a_simple_integer_reads_back() {
        // 1234
        let octets = encoder(0, 0x0000, 0, &[1234]);
        assert_eq!(render_binary(&octets).as_deref(), Some("1234"));
    }

    #[test]
    fn the_first_group_is_not_padded_and_the_next_ones_are() {
        // 1 0002 = 10002, not 12.
        let octets = encoder(1, 0x0000, 0, &[1, 2]);
        assert_eq!(render_binary(&octets).as_deref(), Some("10002"));
    }

    #[test]
    fn an_amount_with_two_decimals_stays_exact() {
        // 1234.56: weight 0, groups [1234, 5600], dscale 2.
        let octets = encoder(0, 0x0000, 2, &[1234, 5600]);
        assert_eq!(render_binary(&octets).as_deref(), Some("1234.56"));
    }

    #[test]
    fn the_negative_sign_carries_over() {
        let octets = encoder(0, 0x4000, 2, &[1234, 5600]);
        assert_eq!(render_binary(&octets).as_deref(), Some("-1234.56"));
    }

    #[test]
    fn a_purely_fractional_value_gets_its_leading_zero() {
        // 0.1234: weight -1.
        let octets = encoder(-1, 0x0000, 4, &[1234]);
        assert_eq!(render_binary(&octets).as_deref(), Some("0.1234"));
    }

    #[test]
    fn groups_missing_before_the_significant_ones_are_zero() {
        // 0.00001234: weight -2, a single group transmitted.
        let octets = encoder(-2, 0x0000, 8, &[1234]);
        assert_eq!(render_binary(&octets).as_deref(), Some("0.00001234"));
    }

    #[test]
    fn missing_decimals_are_padded_with_zeros() {
        // 12.5 declared with four decimals.
        let octets = encoder(0, 0x0000, 4, &[12, 5000]);
        assert_eq!(render_binary(&octets).as_deref(), Some("12.5000"));
    }

    #[test]
    fn zero_renders_zero() {
        let octets = encoder(0, 0x0000, 0, &[]);
        assert_eq!(render_binary(&octets).as_deref(), Some("0"));
    }

    #[test]
    fn a_precision_that_does_not_fit_in_an_f64_stays_exact() {
        // 12,345,678,901,234,567,890.12345678: 26 significant digits, well
        // beyond the 15 to 17 of an f64. It is the case that motivates this
        // module.
        let octets = encoder(4, 0x0000, 8, &[1234, 5678, 9012, 3456, 7890, 1234, 5678]);
        assert_eq!(
            render_binary(&octets).as_deref(),
            Some("12345678901234567890.12345678")
        );
    }

    #[test]
    fn special_values_are_named() {
        assert_eq!(
            render_binary(&encoder(0, 0xC000, 0, &[])).as_deref(),
            Some("NaN")
        );
        assert_eq!(
            render_binary(&encoder(0, 0xD000, 0, &[])).as_deref(),
            Some("Infinity")
        );
        assert_eq!(
            render_binary(&encoder(0, 0xF000, 0, &[])).as_deref(),
            Some("-Infinity")
        );
    }

    #[test]
    fn a_truncated_input_returns_none_instead_of_panicking() {
        // The bytes come from the network: a server may send fewer than its
        // header announces.
        assert_eq!(render_binary(&[]), None);
        assert_eq!(render_binary(&[0, 1, 0, 0]), None);
        let mut tronque = encoder(0, 0x0000, 0, &[1234]);
        tronque.pop();
        assert_eq!(render_binary(&tronque), None);
    }

    #[test]
    fn an_unknown_sign_is_refused() {
        assert_eq!(render_binary(&encoder(0, 0x1234, 0, &[1])), None);
    }

    #[test]
    fn an_out_of_bounds_group_is_refused() {
        // 10,000 is not a valid base-10,000 group.
        assert_eq!(render_binary(&encoder(0, 0x0000, 0, &[10_000])), None);
        assert_eq!(render_binary(&encoder(0, 0x0000, 0, &[-1])), None);
    }

    #[test]
    fn an_absurd_weight_does_not_allocate_130_kb() {
        assert_eq!(render_binary(&encoder(i16::MAX, 0x0000, 0, &[1])), None);
    }

    #[test]
    fn the_reader_never_overflows() {
        let mut lecteur = Lecteur::new(&[0x00, 0x01, 0x02]);
        assert_eq!(lecteur.i16(), Some(1));
        assert_eq!(
            lecteur.i16(),
            None,
            "one remaining byte does not make an i16"
        );
        assert_eq!(lecteur.reste(), &[0x02]);
        assert!(!lecteur.est_vide());
        assert_eq!(lecteur.prendre(1), Some(&[0x02][..]));
        assert!(lecteur.est_vide());
    }
}
