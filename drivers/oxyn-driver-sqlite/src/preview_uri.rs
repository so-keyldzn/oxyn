//! Narrow URI access without changing SQLite's filename or other VFS options.

use crate::worker::{OpenSpec, OpenTarget};

pub(crate) fn read_only(mut spec: OpenSpec) -> OpenSpec {
    spec.read_only = true;
    if let OpenTarget::File(path) = &mut spec.target
        && let Some(uri) = path.to_str().filter(|value| value.starts_with("file:"))
    {
        *path = rewrite_mode(uri).into();
    }
    spec
}

/// An empty main filename can only be reopened when it has a shared identity.
pub(crate) fn shares_memory(spec: &OpenSpec) -> bool {
    let OpenTarget::File(path) = &spec.target else {
        return true;
    };
    let Some(uri) = path.to_str().and_then(|path| path.strip_prefix("file:")) else {
        return false;
    };
    let address = uri.split('#').next().unwrap_or_default();
    let Some((base, query)) = address.split_once('?') else {
        return false;
    };
    let name = match base.strip_prefix("//") {
        Some(authority) => authority
            .find('/')
            .and_then(|at| authority.get(at..))
            .unwrap_or_default(),
        None => base,
    };
    let named = next_byte(&mut name.bytes()).is_some_and(|byte| byte != 0);
    let memory = decoded_eq(name, b":memory:") || option_is(query, b"mode", b"memory");
    named && memory && option_is(query, b"cache", b"shared")
}

fn option_is(query: &str, name: &[u8], value: &[u8]) -> bool {
    let mut matches = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(key, _)| decoded_eq(key, name));
    matches
        .next()
        .is_some_and(|(_, found)| decoded_eq(found, value))
        && matches.next().is_none()
}

fn rewrite_mode(uri: &str) -> String {
    let (address, fragment) = uri
        .split_once('#')
        .map_or((uri, None), |(a, f)| (a, Some(f)));
    let Some((base, query)) = address.split_once('?') else {
        return uri.to_owned();
    };
    let mut result = String::with_capacity(uri.len());
    result.push_str(base);
    result.push('?');
    for (position, pair) in query.split('&').enumerate() {
        if position > 0 {
            result.push('&');
        }
        match pair.split_once('=') {
            Some((key, value))
                if decoded_eq(key, b"mode")
                    && (decoded_eq(value, b"rw") || decoded_eq(value, b"rwc")) =>
            {
                result.push_str(key);
                result.push_str("=ro");
            }
            _ => result.push_str(pair),
        }
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    result
}

// SQLite decodes percent escapes, but does not use HTML form '+' semantics.
// Match only its access option; do not canonicalize relative file URIs or VFS
// parameters through a general URL parser and accidentally change the target.
fn decoded_eq(raw: &str, expected: &[u8]) -> bool {
    let mut bytes = raw.bytes();
    expected
        .iter()
        .all(|expected| next_byte(&mut bytes) == Some(*expected))
        && bytes.next().is_none()
}

fn next_byte(bytes: &mut std::str::Bytes<'_>) -> Option<u8> {
    match bytes.next()? {
        b'%' => hex(bytes.next()?)?
            .checked_mul(16)?
            .checked_add(hex(bytes.next()?)?),
        byte => Some(byte),
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_mode;

    #[test]
    fn only_writable_mode_values_change_and_other_options_remain_byte_exact() {
        assert_eq!(
            rewrite_mode("file:relative%3f.sqlite?cache=shared&mode=rwc&vfs=unix#mode=rw"),
            "file:relative%3f.sqlite?cache=shared&mode=ro&vfs=unix#mode=rw"
        );
        assert_eq!(
            rewrite_mode("file:/db?%6d%6Fde=%72%77%63&immutable=1"),
            "file:/db?%6d%6Fde=ro&immutable=1"
        );
        for uri in [
            "file:/db",
            "file:memory?mode=memory&cache=shared",
            "file:/db?mode=ro",
            "file:/db?modeof=rw&name=a+b",
            "file:/db?mo+de=rw&mode=%Q0",
            "file:/db#ignored?mode=rw",
        ] {
            assert_eq!(rewrite_mode(uri), uri);
        }
    }
}
