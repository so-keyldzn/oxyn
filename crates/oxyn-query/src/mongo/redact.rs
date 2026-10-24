//! The credentials a MongoDB command carries, redacted before its text is
//! persisted ([ADR-0053](../../../../docs/adr/0053-redact-sql-passwords-before-persistence.md)).
//!
//! Three places hold one (MongoDB manual, consulted on 2026-10-24, see
//! RESEARCH-NOTES):
//!
//! - the value of **every** `pwd` key, at any depth: `createUser` and
//!   `updateUser` take the new user's password there;
//! - the `payload` of `saslStart` and `saslContinue`: the SASL conversation,
//!   which carries the password itself under `PLAIN` and a proof derived from
//!   it under SCRAM;
//! - the password of a connection string, `scheme://user:password@host`, in a
//!   string value.
//!
//! # Decided on the decoded command
//!
//! What the server reads decides, not the text as written — the rule the
//! Redis redaction holds on its argv. Every key and every string is decoded
//! as JSON decodes it: `"pwd"` is `pwd`, `"mongodb:\/\/"` is a
//! connection string, and a command's name is the decoded first key of a
//! top-level document. The replacement is made on the text as written, every
//! other byte kept. A string whose decoded content itself holds a command — a
//! tool call's rendering quoting it, a document stored as text — is read again,
//! so a level of quoting never hides a key.
//!
//! The reading is lexical, never a parse: a console text is often several
//! commands, one being typed, or the shell syntax the console refuses but a
//! history keeps — unquoted keys and single-quoted strings are read too. A
//! value that does not end is redacted through the end of the text. Like the
//! SQL and Redis redactions, this is not a secret detector: a password under
//! any other key is outside the grammar it reads.

use std::borrow::Cow;
use std::ops::Range;

use super::skip_string;

/// The key whose value is a password.
const CREDENTIAL_FIELD: &str = "pwd";

/// The commands whose top-level `payload` is a SASL message.
const SASL_COMMANDS: [&str; 2] = ["saslStart", "saslContinue"];

/// The key of a SASL message.
const SASL_PAYLOAD: &str = "payload";

/// The JSON string kept in place of a password value.
pub const REDACTED_VALUE: &str = "\"<redacted>\"";

/// What replaces the password of a connection string.
const REDACTED_PASSWORD: &str = "<redacted>";

/// How many levels of quoting are read through. A rendering quotes a command
/// once, and a re-encoded record twice; beyond, the cost of reading again
/// grows with each level for a text nobody writes.
const MAX_NESTING: usize = 4;

/// Replaces the credentials of the MongoDB commands in `text`, preserving every
/// other byte: the value of every `pwd` key and of the `payload` of
/// `saslStart`/`saslContinue` with [`REDACTED_VALUE`], and the password of a
/// connection string inside a string with `<redacted>`.
///
/// Strings are skipped whole, so a `"pwd":` inside a string value is text,
/// not a key — unless that string's decoded content is itself a command.
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    redact_nested(text, MAX_NESTING).into_owned()
}

fn redact_nested(text: &str, nesting: usize) -> Cow<'_, str> {
    if !may_hold_secret(text) {
        return Cow::Borrowed(text);
    }
    let mut replacements = Vec::new();
    scan(text, nesting, &mut replacements);
    if replacements.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut redacted = String::with_capacity(text.len());
    let mut copied = 0;
    for (span, replacement) in replacements {
        if span.start < copied {
            continue;
        }
        redacted.push_str(text.get(copied..span.start).unwrap_or_default());
        redacted.push_str(&replacement);
        copied = span.end;
    }
    redacted.push_str(text.get(copied..).unwrap_or_default());
    Cow::Owned(redacted)
}

/// Whether `text` can hold anything [`scan`] replaces: a key or a scheme
/// written plainly, or an escape that may spell one.
fn may_hold_secret(text: &str) -> bool {
    [CREDENTIAL_FIELD, SASL_PAYLOAD, "://", "\\"]
        .iter()
        .any(|needle| text.contains(needle))
}

/// A container open at the scanned position.
enum Frame<'a> {
    Object {
        /// The decoded key whose value comes next.
        key: Option<Cow<'a, str>>,
        expecting_key: bool,
    },
    Array,
}

/// Collects, in text order, the spans of `text` to replace and what replaces
/// each.
fn scan<'a>(text: &'a str, nesting: usize, out: &mut Vec<(Range<usize>, String)>) {
    let bytes = text.as_bytes();
    let mut stack: Vec<Frame<'a>> = Vec::new();
    // The decoded first key of the top-level document being read.
    let mut command: Option<Cow<'a, str>> = None;
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let depth = stack.len();
        match byte {
            b'{' => {
                if depth == 0 {
                    command = None;
                }
                stack.push(Frame::Object {
                    key: None,
                    expecting_key: true,
                });
                index = index.saturating_add(1);
            }
            b'[' => {
                stack.push(Frame::Array);
                index = index.saturating_add(1);
            }
            b'}' | b']' => {
                stack.pop();
                index = index.saturating_add(1);
            }
            b',' => {
                if let Some(Frame::Object { key, expecting_key }) = stack.last_mut() {
                    *key = None;
                    *expecting_key = true;
                }
                index = index.saturating_add(1);
            }
            b':' => {
                index = index.saturating_add(1);
                let Some(Frame::Object { key, expecting_key }) = stack.last_mut() else {
                    continue;
                };
                *expecting_key = false;
                let secret = key.as_deref().is_some_and(|key| {
                    key == CREDENTIAL_FIELD
                        || (depth == 1
                            && key == SASL_PAYLOAD
                            && command
                                .as_deref()
                                .is_some_and(|name| SASL_COMMANDS.contains(&name)))
                });
                if secret {
                    let start = skip_whitespace(bytes, index);
                    let end = skip_value(bytes, start);
                    out.push((start..end, REDACTED_VALUE.to_owned()));
                    index = end;
                }
            }
            b'"' | b'\'' => {
                let end = skip_quoted(bytes, index);
                let literal = text.get(index..end).unwrap_or_default();
                if let Some(Frame::Object {
                    key,
                    expecting_key: true,
                }) = stack.last_mut()
                {
                    let decoded = decode(literal);
                    if depth == 1 && command.is_none() {
                        command = Some(decoded.clone());
                    }
                    *key = Some(decoded);
                } else if let Some(replacement) = redact_string(literal, nesting) {
                    out.push((index..end, replacement));
                }
                index = end;
            }
            byte if is_identifier_start(byte) => {
                let end = skip_identifier(bytes, index);
                if let Some(Frame::Object {
                    key,
                    expecting_key: true,
                }) = stack.last_mut()
                {
                    let name = Cow::Borrowed(text.get(index..end).unwrap_or_default());
                    if depth == 1 && command.is_none() {
                        command = Some(name.clone());
                    }
                    *key = Some(name);
                }
                index = end;
            }
            _ => index = index.saturating_add(1),
        }
    }
}

/// What replaces the string `literal` — quotes included —, or `None` when it
/// holds no credential once decoded. The replacement is written as a JSON
/// string.
fn redact_string(literal: &str, nesting: usize) -> Option<String> {
    if !may_hold_secret(literal) {
        return None;
    }
    let decoded = decode(literal);
    let inner = match nesting.checked_sub(1) {
        Some(below) => redact_nested(&decoded, below),
        None => Cow::Borrowed(decoded.as_ref()),
    };
    let redacted = redact_uri_passwords(&inner);
    if redacted == decoded {
        return None;
    }
    Some(serde_json::to_string(redacted.as_ref()).unwrap_or_else(|_| REDACTED_VALUE.to_owned()))
}

/// Replaces the password of every `scheme://user:password@…` in `text`.
///
/// The connection string runs to the next blank, and its user information to
/// its last `@`: a password written with a raw `@` or `/` is still covered,
/// at the cost of a host redacted with it when a query option holds an `@`.
fn redact_uri_passwords(text: &str) -> Cow<'_, str> {
    let mut redacted = String::new();
    let mut copied = 0;
    let mut search = 0;
    while let Some(found) = text.get(search..).and_then(|rest| rest.find("://")) {
        let authority = search.saturating_add(found).saturating_add(3);
        let rest = text.get(authority..).unwrap_or_default();
        let length = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let uri = rest.get(..length).unwrap_or_default();
        if let Some(at) = uri.rfind('@')
            && let Some(colon) = uri.get(..at).and_then(|user| user.find(':'))
            && uri.get(colon.saturating_add(1)..at) != Some(REDACTED_PASSWORD)
        {
            let start = authority.saturating_add(colon).saturating_add(1);
            redacted.push_str(text.get(copied..start).unwrap_or_default());
            redacted.push_str(REDACTED_PASSWORD);
            copied = authority.saturating_add(at);
        }
        search = authority.saturating_add(length);
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    redacted.push_str(text.get(copied..).unwrap_or_default());
    Cow::Owned(redacted)
}

/// The content of a quoted string, decoded as JSON decodes it. A single-quoted
/// string — shell syntax — is read with the same escapes. What does not decode,
/// an unterminated string or an escape JSON refuses, is its raw content: the
/// server would refuse the command, and the text still says what it holds.
fn decode(literal: &str) -> Cow<'_, str> {
    let Some(quote) = literal.chars().next() else {
        return Cow::Borrowed(literal);
    };
    let content = literal.get(1..).unwrap_or_default();
    let content = content.strip_suffix(quote).unwrap_or(content);
    if !content.contains('\\') {
        return Cow::Borrowed(content);
    }
    let decoded = if quote == '"' {
        serde_json::from_str::<String>(literal)
    } else {
        let mut json = String::with_capacity(literal.len().saturating_add(2));
        json.push('"');
        let mut chars = content.chars();
        while let Some(character) = chars.next() {
            match character {
                '\\' => match chars.next() {
                    Some('\'') => json.push('\''),
                    Some(escaped) => {
                        json.push('\\');
                        json.push(escaped);
                    }
                    None => json.push_str("\\\\"),
                },
                '"' => json.push_str("\\\""),
                other => json.push(other),
            }
        }
        json.push('"');
        serde_json::from_str::<String>(&json)
    };
    decoded.map_or(Cow::Borrowed(content), Cow::Owned)
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    index
}

/// The index just after the string whose opening quote is at `index`.
fn skip_quoted(bytes: &[u8], index: usize) -> usize {
    let content = index.saturating_add(1);
    let end = if bytes.get(index) == Some(&b'\'') {
        let mut cursor = content;
        loop {
            match bytes.get(cursor) {
                None => break cursor,
                Some(b'\\') => cursor = cursor.saturating_add(2),
                Some(b'\'') => break cursor.saturating_add(1),
                Some(_) => cursor = cursor.saturating_add(1),
            }
        }
    } else {
        skip_string(bytes, content)
    };
    end.min(bytes.len())
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn skip_identifier(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
    {
        index = index.saturating_add(1);
    }
    index
}

/// The index just after the value starting at `index`: a string, an object or
/// an array balanced with the strings inside skipped, or a scalar up to the
/// next delimiter — a shell constructor's parentheses included, so that
/// `BinData(0, "…")` is one value. Never past the end of `bytes`.
fn skip_value(bytes: &[u8], index: usize) -> usize {
    let end = match bytes.get(index) {
        Some(b'"' | b'\'') => skip_quoted(bytes, index),
        Some(b'{' | b'[') => skip_balanced(bytes, index, b"{[", b"}]"),
        _ => {
            let mut cursor = index;
            loop {
                match bytes.get(cursor) {
                    None => break cursor,
                    Some(b'(') => cursor = skip_balanced(bytes, cursor, b"(", b")"),
                    Some(b'"' | b'\'') => cursor = skip_quoted(bytes, cursor),
                    Some(byte)
                        if matches!(byte, b',' | b'}' | b']') || byte.is_ascii_whitespace() =>
                    {
                        break cursor;
                    }
                    Some(_) => cursor = cursor.saturating_add(1),
                }
            }
        }
    };
    end.min(bytes.len())
}

/// The index just after the bracket at `index` is closed, strings skipped.
fn skip_balanced(bytes: &[u8], index: usize, open: &[u8], close: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut cursor = index;
    loop {
        match bytes.get(cursor) {
            None => break cursor,
            Some(b'"' | b'\'') => cursor = skip_quoted(bytes, cursor),
            Some(byte) if open.contains(byte) => {
                depth = depth.saturating_add(1);
                cursor = cursor.saturating_add(1);
            }
            Some(byte) if close.contains(byte) => {
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

#[cfg(test)]
mod tests;
