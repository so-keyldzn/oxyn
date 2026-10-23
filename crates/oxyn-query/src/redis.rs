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
const SECRET_PARAMETERS: [&str; 4] = [
    "requirepass",
    "masterauth",
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
/// `CONFIG SET requirepass | masterauth | tls-key-file-pass |
/// tls-client-key-file-pass`; the passwords of `MIGRATE … AUTH` and
/// `AUTH2`. Every other byte is kept. Like the SQL redaction, this is not a
/// secret detector: a password inside a script's body or a value is outside
/// the grammar it reads.
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let mut replacements: Vec<(std::ops::Range<usize>, &'static str)> = Vec::new();
    for command in arguments(text) {
        secrets_of(text, &command, &mut replacements);
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
fn secrets_of(
    text: &str,
    command: &[std::ops::Range<usize>],
    out: &mut Vec<(std::ops::Range<usize>, &'static str)>,
) {
    let word = |index: usize| command.get(index).map(|span| unquoted(text, span));
    let Some(name) = word(0) else { return };
    match name.as_str() {
        "auth" => {
            if let Some(last) = command.get(1..).and_then(<[_]>::last) {
                out.push((last.clone(), "<redacted>"));
            }
        }
        "hello" => {
            if let Some(at) = (1..command.len()).find(|&i| word(i).as_deref() == Some("auth"))
                && let Some(password) = command.get(at.saturating_add(2))
            {
                out.push((password.clone(), "<redacted>"));
            }
        }
        "acl" if word(1).as_deref() == Some("setuser") => {
            for span in command.iter().skip(3) {
                let rule = text.get(span.clone()).unwrap_or_default();
                let marker = match rule.trim_start_matches(['"', '\'']).chars().next() {
                    Some('>') => "><redacted>",
                    Some('<') => "<<redacted>",
                    Some('#') => "#<redacted>",
                    Some('!') => "!<redacted>",
                    _ => continue,
                };
                out.push((span.clone(), marker));
            }
        }
        "config" if word(1).as_deref() == Some("set") => {
            for pair in command.get(2..).unwrap_or_default().chunks(2) {
                if let [parameter, value] = pair
                    && SECRET_PARAMETERS.contains(&unquoted(text, parameter).as_str())
                {
                    out.push((value.clone(), "<redacted>"));
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
                    out.push((password.clone(), "<redacted>"));
                }
            }
        }
        _ => {}
    }
}

/// A word of the command, lowercase and without its quotes, to recognize a
/// keyword: escapes are left as they are, since no keyword holds one.
fn unquoted(text: &str, span: &std::ops::Range<usize>) -> String {
    text.get(span.clone())
        .unwrap_or_default()
        .replace(['"', '\''], "")
        .to_ascii_lowercase()
}

/// Every command of `text` as the spans of its arguments, quotes included. A
/// quote left open runs to the end of the text, as the driver would refuse it.
fn arguments(text: &str) -> Vec<Vec<std::ops::Range<usize>>> {
    let bytes = text.as_bytes();
    let mut commands = Vec::new();
    let mut current: Vec<std::ops::Range<usize>> = Vec::new();
    let mut token: Option<usize> = None;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        match (quote, byte) {
            (Some(b'"'), b'\\') => i = i.saturating_add(1),
            (Some(b'\''), b'\\') if bytes.get(i.saturating_add(1)) == Some(&b'\'') => {
                i = i.saturating_add(1);
            }
            (Some(open), close) if close == open => quote = None,
            (Some(_), _) => {}
            (None, b' ' | b'\t' | b'\r' | b'\n') => {
                if let Some(start) = token.take() {
                    current.push(start..i);
                }
                if byte == b'\n' && !current.is_empty() {
                    commands.push(std::mem::take(&mut current));
                }
            }
            (None, other) => {
                token.get_or_insert(i);
                if matches!(other, b'"' | b'\'') {
                    quote = Some(other);
                }
            }
        }
        i = i.saturating_add(1);
    }
    if let Some(start) = token {
        current.push(start..bytes.len());
    }
    if !current.is_empty() {
        commands.push(current);
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_under_the_cursor_is_the_command() {
        let text = "GET a\nDEL b\nSET c 1";
        assert_eq!(current_command(text, 0), Ok(Some("GET a")));
        // Right after the last character of the first line.
        assert_eq!(current_command(text, 5), Ok(Some("GET a")));
        assert_eq!(current_command(text, 8), Ok(Some("DEL b")));
        assert_eq!(current_command(text, text.len()), Ok(Some("SET c 1")));
    }

    #[test]
    fn a_blank_line_runs_nothing() {
        assert_eq!(current_command("GET a\n\nDEL b", 6), Ok(None));
        assert_eq!(current_command("GET a\n     \nDEL b", 9), Ok(None));
        assert_eq!(current_command("", 0), Ok(None));
    }

    #[test]
    fn a_line_break_inside_quotes_belongs_to_the_value() {
        let text = "SET k \"two\nlines\"\nDEL b";
        assert_eq!(current_command(text, 12), Ok(Some("SET k \"two\nlines\"")));
        assert_eq!(current_command(text, text.len()), Ok(Some("DEL b")));
        let single = "SET k 'it\\'s\nfine'\nGET k";
        assert_eq!(current_command(single, 2), Ok(Some("SET k 'it\\'s\nfine'")));
    }

    #[test]
    fn an_escaped_quote_does_not_close_the_argument() {
        let text = "SET k \"a\\\"\nb\"\nGET k";
        assert_eq!(current_command(text, 0), Ok(Some("SET k \"a\\\"\nb\"")));
    }

    #[test]
    fn a_quote_never_closed_asks_for_a_selection() {
        let text = "GET a\nSET k \"open\nDEL b";
        assert!(matches!(
            current_command(text, 8),
            Err(QueryError::Selection { .. })
        ));
        // A command before the open quote is still found.
        assert_eq!(current_command(text, 2), Ok(Some("GET a")));
    }

    #[test]
    fn a_cursor_inside_a_character_is_refused() {
        assert!(matches!(
            current_command("GET é", 5),
            Err(QueryError::Selection { .. })
        ));
    }

    #[test]
    fn carriage_returns_stay_out_of_the_command() {
        assert_eq!(current_command("GET a\r\nDEL b", 1), Ok(Some("GET a")));
    }

    #[test]
    fn every_credential_the_grammar_carries_is_redacted() {
        for (line, expected) in [
            ("AUTH hunter2", "AUTH <redacted>"),
            ("auth app hunter2", "auth app <redacted>"),
            (
                "HELLO 3 AUTH app hunter2 SETNAME x",
                "HELLO 3 AUTH app <redacted> SETNAME x",
            ),
            (
                "ACL SETUSER alan on >hunter2 +@read <old \"#c3ab8f\" !d1e2",
                "ACL SETUSER alan on ><redacted> +@read <<redacted> #<redacted> !<redacted>",
            ),
            (
                "CONFIG SET maxmemory 1gb requirepass hunter2 masterauth 'two words'",
                "CONFIG SET maxmemory 1gb requirepass <redacted> masterauth <redacted>",
            ),
            (
                "config set TLS-KEY-FILE-PASS hunter2",
                "config set TLS-KEY-FILE-PASS <redacted>",
            ),
            (
                "MIGRATE h 6379 k 0 5000 AUTH hunter2",
                "MIGRATE h 6379 k 0 5000 AUTH <redacted>",
            ),
            (
                "MIGRATE h 6379 \"\" 0 5000 AUTH2 app hunter2 KEYS a b",
                "MIGRATE h 6379 \"\" 0 5000 AUTH2 app <redacted> KEYS a b",
            ),
        ] {
            assert_eq!(redact_secrets(line), expected, "{line}");
        }
    }

    #[test]
    fn a_command_without_credentials_is_kept_byte_for_byte() {
        for line in [
            "SET password hunter2",
            "GET \"a > b\"",
            "ACL WHOAMI",
            "CONFIG SET maxmemory 1gb",
            "EVAL \"return 1\" 0",
            "",
        ] {
            assert_eq!(redact_secrets(line), line);
        }
    }

    #[test]
    fn each_line_of_a_console_is_redacted_as_its_own_command() {
        assert_eq!(
            redact_secrets("GET a\nAUTH hunter2\nCONFIG SET requirepass \"x\ny\""),
            "GET a\nAUTH <redacted>\nCONFIG SET requirepass <redacted>"
        );
    }
}
