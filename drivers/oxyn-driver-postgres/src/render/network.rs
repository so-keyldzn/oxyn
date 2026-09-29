//! `inet`, `cidr`, `macaddr` and `macaddr8`.
//!
//! These values were once treated as UTF-8 text: their binary layout is not
//! text, so `10.0.0.1` came out as control characters and `192.168.0.1` failed
//! the whole query. The output reproduces `network_out` and `inet_net_ntop.c`
//! rather than `std::net`'s `Display`, because the two disagree on IPv6: Rust
//! never prints an embedded IPv4 (`::1.2.3.4`), and PostgreSQL does.

use std::fmt::Write as _;

use super::Rendered;
use crate::numeric::Reader;

/// `PGSQL_AF_INET`: `AF_INET + 0`, fixed by the protocol, not by the platform.
const FAMILY_V4: u8 = 2;
/// `PGSQL_AF_INET6`: `AF_INET + 1`.
const FAMILY_V6: u8 = 3;
/// Address bytes of an IPv4.
const V4_LEN: usize = 4;
/// Address bytes of an IPv6.
const V6_LEN: usize = 16;

/// An `inet` or a `cidr`: the binary carries its own `is_cidr` flag, set by
/// `cidr_send`, so one renderer serves both types.
pub(crate) fn inet(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let family = reader.u8().ok_or("truncated inet header")?;
    let bits = reader.u8().ok_or("truncated inet header")?;
    let is_cidr = reader.u8().ok_or("truncated inet header")?;
    let length = reader.u8().ok_or("truncated inet header")?;
    let address = reader
        .take(usize::from(length))
        .ok_or("truncated inet address")?;
    if !reader.rest().is_empty() {
        return Err("trailing bytes after an inet address");
    }

    let full = match family {
        FAMILY_V4 => {
            let [a, b, c, d]: [u8; V4_LEN] = address
                .try_into()
                .map_err(|_| "IPv4 inet whose address is not four bytes")?;
            if bits > 32 {
                return Err("IPv4 inet mask longer than 32 bits");
            }
            let _ = write!(out, "{a}.{b}.{c}.{d}");
            32
        }
        FAMILY_V6 => {
            let octets: [u8; V6_LEN] = address
                .try_into()
                .map_err(|_| "IPv6 inet whose address is not sixteen bytes")?;
            if bits > 128 {
                return Err("IPv6 inet mask longer than 128 bits");
            }
            write_ipv6(&octets, out);
            128
        }
        _ => return Err("unknown inet address family"),
    };

    // `inet_net_ntop` omits a full-length mask; `cidr_out` then adds it back,
    // because a `cidr` always names its network length.
    if bits != full || is_cidr != 0 {
        let _ = write!(out, "/{bits}");
    }
    Ok(())
}

/// An IPv6 address as `inet_net_ntop_ipv6` writes it.
fn write_ipv6(octets: &[u8; V6_LEN], out: &mut String) {
    let mut words = [0_u16; 8];
    let (pairs, _) = octets.as_chunks::<2>();
    for (word, pair) in words.iter_mut().zip(pairs) {
        *word = u16::from_be_bytes(*pair);
    }

    // The longest run of zero words becomes `::`; on a tie, the **first** run
    // wins (strict `>`), and a lone zero word is not compressed.
    let mut best: Option<(usize, usize)> = None;
    let mut current: Option<(usize, usize)> = None;
    for (index, word) in words.iter().enumerate() {
        if *word == 0 {
            current = Some(current.map_or((index, 1), |(base, len)| (base, len + 1)));
        } else if let Some(run) = current.take()
            && best.is_none_or(|(_, len)| run.1 > len)
        {
            best = Some(run);
        }
    }
    if let Some(run) = current
        && best.is_none_or(|(_, len)| run.1 > len)
    {
        best = Some(run);
    }
    let best = best.filter(|&(_, len)| len >= 2);

    let [.., fifth, _, last] = words;
    for (index, word) in words.iter().enumerate() {
        if let Some((base, len)) = best
            && index >= base
            && index < base + len
        {
            if index == base {
                out.push(':');
            }
            continue;
        }
        if index != 0 {
            out.push(':');
        }
        // An IPv4 compatible (`::a.b.c.d`, but not `::1`) or mapped
        // (`::ffff:a.b.c.d`) address prints its last 32 bits as an IPv4.
        let embedded = index == 6
            && best.is_some_and(|(base, len)| {
                base == 0
                    && (len == 6 || (len == 7 && last != 0x0001) || (len == 5 && fifth == 0xffff))
            });
        if embedded {
            let [.., a, b, c, d] = octets;
            let _ = write!(out, "{a}.{b}.{c}.{d}");
            return;
        }
        let _ = write!(out, "{word:x}");
    }
    if best.is_some_and(|(base, len)| base + len == words.len()) {
        out.push(':');
    }
}

/// A `macaddr` (six bytes) or a `macaddr8` (eight).
pub(crate) fn macaddr(bytes: &[u8], out: &mut String) -> Rendered {
    if bytes.len() != 6 && bytes.len() != 8 {
        return Err("MAC address of unexpected size");
    }
    for (index, byte) in bytes.iter().enumerate() {
        if index != 0 {
            out.push(':');
        }
        let _ = write!(out, "{byte:02x}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("test hex"))
            .collect()
    }

    fn render_inet(encoded: &str) -> Result<String, &'static str> {
        let mut out = String::new();
        inet(&hex(encoded), &mut out).map(|()| out)
    }

    /// Pairs taken from a PostgreSQL 17.11 server on 2026-09-28:
    /// `format('%s', v)` (the `*_out` function, unlike `v::text`, which always
    /// shows the mask) and `encode(inet_send(v), 'hex')`.
    const SERVER_CASES: &[(&str, &str)] = &[
        ("02200004c0a80001", "192.168.0.1"),
        ("02180004c0a80001", "192.168.0.1/24"),
        ("0200000400000000", "0.0.0.0/0"),
        (
            "0380001020010db885a3000100028a2e03707334",
            "2001:db8:85a3:1:2:8a2e:370:7334",
        ),
        ("0380001000000000000000000000000000000000", "::"),
        ("0380001000000000000000000000000000000001", "::1"),
        ("0380001000000000000000000000ffff01020304", "::ffff:1.2.3.4"),
        ("0380001000000000000000000000000001020304", "::1.2.3.4"),
        ("0380001000010000000000020000000000030004", "1::2:0:0:3:4"),
        ("0340001020010db8000000000000000000000000", "2001:db8::/64"),
        ("03400010fe800000000000000000000000010002", "fe80::1:2/64"),
        (
            "0380001000010002000300040005000600070000",
            "1:2:3:4:5:6:7:0",
        ),
        // `cidr_send`: the `is_cidr` byte forces the mask.
        ("020801040a000000", "10.0.0.0/8"),
        ("02200104c0a80001", "192.168.0.1/32"),
        ("0320011020010db8000000000000000000000000", "2001:db8::/32"),
        ("0380011000000000000000000000000000000001", "::1/128"),
        ("0200010400000000", "0.0.0.0/0"),
    ];

    #[test]
    fn inet_and_cidr_match_the_server_output() {
        for (encoded, expected) in SERVER_CASES {
            assert_eq!(render_inet(encoded).as_deref(), Ok(*expected), "{encoded}");
        }
    }

    #[test]
    fn a_malformed_inet_is_refused_without_panicking() {
        for encoded in [
            "",
            "0220",
            "02200004c0a800",
            "02200010c0a80001",
            "02210004c0a80001",
            "038100100000000000000000000000000000000000",
            "09200004c0a80001",
            "02200004c0a8000100",
        ] {
            assert!(render_inet(encoded).is_err(), "{encoded}");
        }
    }

    #[test]
    fn mac_addresses_match_the_server_output() {
        let mut out = String::new();
        macaddr(&hex("08002b010203"), &mut out).expect("macaddr");
        assert_eq!(out, "08:00:2b:01:02:03");

        let mut out = String::new();
        macaddr(&hex("08002b0102030405"), &mut out).expect("macaddr8");
        assert_eq!(out, "08:00:2b:01:02:03:04:05");

        assert!(macaddr(&hex("08002b0102"), &mut String::new()).is_err());
    }
}
