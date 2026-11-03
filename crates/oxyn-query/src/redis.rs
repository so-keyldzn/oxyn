//! Where a Redis command ends in a console text.
//!
//! A Redis command is one line, as `redis-cli` reads it (`sdssplitargs`,
//! [ADR-0061](../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md)):
//! blanks separate its arguments, a double-quoted argument takes escapes, a
//! single-quoted one only `\'`, and a line break **inside** quotes belongs to
//! the argument. A console holds several commands, one per line; `⌘↵` without
//! a selection runs the one under the cursor.
//!
//! # The direction of the error
//!
//! The SQL splitter's: when the reading is in doubt, it does not cut. A quote
//! left open swallows the rest of the text, so the boundary is unknown, and
//! the console asks for a selection rather than send half of a value as a
//! command of its own. The driver reads the text sent with the same grammar
//! and refuses a second line outside quotes, so a wrong cut here could only
//! cost a refusal, never a different command.

pub mod argv;

use crate::QueryError;

/// The text of the Redis command under `cursor_byte`, or `None` on a line
/// holding nothing but blanks.
///
/// `cursor_byte` is a UTF-8 byte index into `text`; a cursor right after the
/// last character of a line is on that line.
///
/// # Errors
/// [`QueryError::Selection`] when the cursor is not on a character boundary,
/// or when a quote opened before or on the cursor's command is never closed.
pub fn current_command(text: &str, cursor_byte: usize) -> Result<Option<&str>, QueryError> {
    if !text.is_char_boundary(cursor_byte) {
        return Err(QueryError::Selection {
            cursor: cursor_byte,
            message: "the cursor is not on a character boundary".to_owned(),
        });
    }
    for (start, end) in commands(text, cursor_byte)? {
        if (start..=end).contains(&cursor_byte) {
            let command = text.get(start..end).unwrap_or_default().trim();
            return Ok((!command.is_empty()).then_some(command));
        }
        if start > cursor_byte {
            break;
        }
    }
    Ok(None)
}

/// The byte spans of the commands of `text`, each from its line's start to
/// its closing line break excluded, up to the one holding `cursor_byte`.
fn commands(text: &str, cursor_byte: usize) -> Result<Vec<(usize, usize)>, QueryError> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut quote: Option<(u8, usize)> = None;
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        match (quote, byte) {
            (Some((b'"', _)), b'\\') => i = i.saturating_add(1),
            (Some((b'\'', _)), b'\\') if bytes.get(i.saturating_add(1)) == Some(&b'\'') => {
                i = i.saturating_add(1);
            }
            (Some((open, _)), close) if close == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some((byte, i)),
            (None, b'\n') => {
                spans.push((start, i));
                start = i.saturating_add(1);
                if start > cursor_byte {
                    return Ok(spans);
                }
            }
            (None, _) => {}
        }
        i = i.saturating_add(1);
    }
    if let Some((_, opened)) = quote {
        return Err(QueryError::Selection {
            cursor: cursor_byte,
            message: format!(
                "a quote opened at byte {opened} is never closed, so where the command ends is \
                 unknown: select the command to run"
            ),
        });
    }
    spans.push((start, bytes.len()));
    Ok(spans)
}

/// The configuration parameters whose value is a credential, as `redis.conf`
/// of Redis 8.10.2 names them ([RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md)).
/// `masteruser` is the account `masterauth` authenticates: half of the
/// credential, kept out with the other half.
const SECRET_PARAMETERS: [&str; 5] = [
    "requirepass",
    "masterauth",
    "masteruser",
    "tls-key-file-pass",
    "tls-client-key-file-pass",
];

/// Replaces the credentials a Redis command line carries with `<redacted>`,
/// before it reaches the history, the audit journal or a conversation record
/// ([ADR-0053](../../../docs/adr/0053-redact-sql-passwords-before-persistence.md)).
///
/// Recognized: the password of `AUTH` and of `HELLO … AUTH user password`;
/// the `>password`, `<password`, `#hash` and `!hash` rules of
/// `ACL SETUSER`, whose first character is kept; the value of
/// `CONFIG SET requirepass | masterauth | masteruser | tls-key-file-pass |
/// tls-client-key-file-pass`; the passwords of `MIGRATE … AUTH` and
/// `AUTH2`. Every other byte is kept. Like the SQL redaction, this is not a
/// secret detector: a password inside a script's body or a value is outside
/// the grammar it reads.
///
/// The decision is taken on the decoded argv — the bytes the server
/// receives, read by the driver's own decoder ([`argv::commands`]) —, and the
/// replacement on the text as written: `"A\x55TH" hunter2` is `AUTH`, and
/// `"\x3ehunter2"` is a `>` rule.
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let mut replacements: Vec<(std::ops::Range<usize>, &'static str)> = Vec::new();
    for command in argv::commands(text) {
        secrets_of(&command, &mut replacements);
    }
    if replacements.is_empty() {
        return text.to_owned();
    }
    replacements.sort_unstable_by_key(|(span, _)| span.start);
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (span, marker) in replacements {
        if span.start < copied {
            continue;
        }
        out.push_str(text.get(copied..span.start).unwrap_or_default());
        out.push_str(marker);
        copied = span.end;
    }
    out.push_str(text.get(copied..).unwrap_or_default());
    out
}

/// The spans of the secrets of one command, and what replaces each.
fn secrets_of(command: &[argv::Argument], out: &mut Vec<(std::ops::Range<usize>, &'static str)>) {
    let word = |index: usize| command.get(index).map(keyword);
    let Some(name) = word(0) else { return };
    match name.as_str() {
        "auth" => {
            if let Some(last) = command.get(1..).and_then(<[_]>::last) {
                out.push((last.span.clone(), "<redacted>"));
            }
        }
        "hello" => {
            if let Some(at) = (1..command.len()).find(|&i| word(i).as_deref() == Some("auth"))
                && let Some(password) = command.get(at.saturating_add(2))
            {
                out.push((password.span.clone(), "<redacted>"));
            }
        }
        "acl" if word(1).as_deref() == Some("setuser") => {
            for rule in command.iter().skip(3) {
                let marker = match rule.bytes.first() {
                    Some(b'>') => "><redacted>",
                    Some(b'<') => "<<redacted>",
                    Some(b'#') => "#<redacted>",
                    Some(b'!') => "!<redacted>",
                    _ => continue,
                };
                out.push((rule.span.clone(), marker));
            }
        }
        "config" if word(1).as_deref() == Some("set") => {
            // Not by pairs: a value left out or a multi-word value typed
            // without quotes shifts them, and the password would land in a
            // name's place. Whatever follows a secret's name is redacted.
            for (i, parameter) in command.iter().enumerate().skip(2) {
                if SECRET_PARAMETERS.contains(&keyword(parameter).as_str())
                    && let Some(value) = command.get(i.saturating_add(1))
                {
                    out.push((value.span.clone(), "<redacted>"));
                }
            }
        }
        "migrate" => {
            for (i, _) in command.iter().enumerate() {
                let offset = match word(i).as_deref() {
                    Some("auth") => 1,
                    Some("auth2") => 2,
                    _ => continue,
                };
                if let Some(password) = command.get(i.saturating_add(offset)) {
                    out.push((password.span.clone(), "<redacted>"));
                }
            }
        }
        _ => {}
    }
}

/// An argument as the server compares a keyword: decoded, ASCII-lowercase.
fn keyword(argument: &argv::Argument) -> String {
    String::from_utf8_lossy(&argument.bytes).to_ascii_lowercase()
}

#[cfg(test)]
mod tests;
