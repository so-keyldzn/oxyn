//! The passwords a MongoDB command carries, redacted before its text is
//! persisted ([ADR-0053](../../../../docs/adr/0053-redact-sql-passwords-before-persistence.md)).
//!
//! `createUser` and `updateUser` take the new user's password as `pwd`
//! (MongoDB manual, consulted on 2026-10-24, see RESEARCH-NOTES). The value of
//! **every** `pwd` key, at any depth, is replaced with [`REDACTED_VALUE`]; every
//! other byte is kept. The driver withholds a server error by the same rule:
//! what the text holds decides, not the name of the command around it.
//!
//! The scan is lexical, never a parse: a console text is often not one valid
//! document — several commands one after the other, one being typed. A key
//! spelled with escapes (`"pwd"`) is the same key to the server, so it is
//! decoded before it is compared. A value that does not end is redacted
//! through the end of the text.

use super::skip_string;

/// The key whose value is a password.
const CREDENTIAL_FIELD: &str = "pwd";

/// The JSON string kept in place of a password value.
pub const REDACTED_VALUE: &str = "\"<redacted>\"";

/// Replaces the value of every `pwd` key in `text` with [`REDACTED_VALUE`],
/// preserving every other byte. Strings are skipped whole, so a `"pwd":` inside
/// a string value is text, not a key.
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut redacted = String::with_capacity(text.len());
    let mut copied = 0;
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte != b'"' {
            index = index.saturating_add(1);
            continue;
        }
        let end = skip_string(bytes, index.saturating_add(1)).min(bytes.len());
        let colon = skip_whitespace(bytes, end);
        let is_credential = bytes.get(colon) == Some(&b':')
            && text
                .get(index..end)
                .and_then(|key| serde_json::from_str::<String>(key).ok())
                .is_some_and(|key| key == CREDENTIAL_FIELD);
        if !is_credential {
            index = end;
            continue;
        }
        let start = skip_whitespace(bytes, colon.saturating_add(1));
        let value_end = skip_value(bytes, start);
        redacted.push_str(text.get(copied..start).unwrap_or_default());
        redacted.push_str(REDACTED_VALUE);
        copied = value_end;
        index = value_end;
    }
    redacted.push_str(text.get(copied..).unwrap_or_default());
    redacted
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    index
}

/// The index just after the JSON value starting at `index`: a string, an
/// object or an array balanced with the strings inside skipped, or a scalar up
/// to the next delimiter. Never past the end of `bytes`.
fn skip_value(bytes: &[u8], index: usize) -> usize {
    let end = match bytes.get(index) {
        Some(b'"') => skip_string(bytes, index.saturating_add(1)),
        Some(b'{' | b'[') => {
            let mut depth = 0_usize;
            let mut cursor = index;
            loop {
                match bytes.get(cursor) {
                    None => break cursor,
                    Some(b'"') => cursor = skip_string(bytes, cursor.saturating_add(1)),
                    Some(b'{' | b'[') => {
                        depth = depth.saturating_add(1);
                        cursor = cursor.saturating_add(1);
                    }
                    Some(b'}' | b']') => {
                        depth = depth.saturating_sub(1);
                        cursor = cursor.saturating_add(1);
                        if depth == 0 {
                            break cursor;
                        }
                    }
                    Some(_) => cursor = cursor.saturating_add(1),
                }
            }
        }
        _ => {
            let mut cursor = index;
            while bytes.get(cursor).is_some_and(|byte| {
                !matches!(byte, b',' | b'}' | b']') && !byte.is_ascii_whitespace()
            }) {
                cursor = cursor.saturating_add(1);
            }
            cursor
        }
    };
    end.min(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::{REDACTED_VALUE, redact_secrets};

    const SECRET: &str = "witness-secret";

    #[test]
    fn the_password_of_create_and_update_user_is_redacted_and_nothing_else() {
        for (text, expected) in [
            (
                r#"{"createUser": "app", "pwd": "witness-secret", "roles": ["read"]}"#,
                r#"{"createUser": "app", "pwd": "<redacted>", "roles": ["read"]}"#,
            ),
            (
                "{\n  \"updateUser\": \"app\",\n  \"pwd\" :\t\"witness-secret\"\n}",
                "{\n  \"updateUser\": \"app\",\n  \"pwd\" :\t\"<redacted>\"\n}",
            ),
        ] {
            assert_eq!(redact_secrets(text), expected);
        }
    }

    #[test]
    fn a_password_is_found_at_any_depth_spelled_any_way_and_of_any_type() {
        for text in [
            r#"{"bulkWrite": 1, "x": [{"y": {"pwd": "witness-secret"}}]}"#,
            r#"{"createUser": "app", "pwd": "witness-secret"}"#,
            r#"{"createUser": "app", "pwd": "witness-\"secret\\"}"#,
            r#"{"createUser": "app", "pwd": {"$secret": ["witness-secret", {"a": "}"}]}}"#,
            r#"{"createUser": "app", "pwd": 0witness-secret}"#,
            r#"{"createUser": "a"}{"updateUser": "b", "pwd": "witness-secret"}"#,
        ] {
            let redacted = redact_secrets(text);
            assert!(!redacted.contains("witness"), "{text} → {redacted}");
            assert!(redacted.contains(REDACTED_VALUE), "{redacted}");
        }
    }

    #[test]
    fn an_unterminated_password_is_redacted_through_the_end() {
        for text in [
            r#"{"createUser": "app", "pwd": "witness-secret"#,
            r#"{"createUser": "app", "pwd": {"a": "witness-secret"#,
            r#"{"createUser": "app", "pwd": "witness-secret\"#,
        ] {
            assert!(!redact_secrets(text).contains(SECRET), "{text}");
        }
    }

    #[test]
    fn text_that_only_mentions_the_key_is_kept() {
        for text in [
            r#"{"find": "users", "filter": {"note": "pwd: hunter"}}"#,
            r#"{"find": "users", "filter": {"x": "\"pwd\": 1"}}"#,
            r#"{"find": "pwd", "projection": {"password": 0}}"#,
            r#"{"insert": "c", "documents": [{"pwds": "kept"}]}"#,
            "",
            "\"",
            "pwd",
        ] {
            assert_eq!(redact_secrets(text), text);
        }
    }
}
