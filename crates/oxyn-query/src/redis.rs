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
            // Pairs shifted by a missing value or an unquoted value.
            (
                "CONFIG SET maxmemory requirepass hunter2",
                "CONFIG SET maxmemory requirepass <redacted>",
            ),
            (
                "CONFIG SET save 3600 1 requirepass hunter2",
                "CONFIG SET save 3600 1 requirepass <redacted>",
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

    /// What the server receives decides, not the text as written: every line
    /// below runs as a credential command once decoded.
    #[test]
    fn a_credential_is_recognized_on_the_decoded_argv() {
        for (line, expected) in [
            // Escaped command names and keywords.
            ("\"A\\x55TH\" hunter2", "\"A\\x55TH\" <redacted>"),
            ("\"\\x61uth\" app 'hunter2'", "\"\\x61uth\" app <redacted>"),
            ("aUtH \"hunter 2\"", "aUtH <redacted>"),
            (
                "\"\\x41CL\" \"set\\x75ser\" u on \"\\x3ehunter2\"",
                "\"\\x41CL\" \"set\\x75ser\" u on ><redacted>",
            ),
            (
                "ACL SETUSER u \"\\x3ehunter2\" '<old' \"\\x23c3ab8f\"",
                "ACL SETUSER u ><redacted> <<redacted> #<redacted>",
            ),
            (
                "hello 3 \"\\x41UTH\" app \"hun\\x74er2\"",
                "hello 3 \"\\x41UTH\" app <redacted>",
            ),
            (
                "CONFIG \"S\\x45T\" \"requirep\\x61ss\" hunter2 MasterAuth \"x\" masteruser bob",
                "CONFIG \"S\\x45T\" \"requirep\\x61ss\" <redacted> MasterAuth <redacted> \
                 masteruser <redacted>",
            ),
            (
                "MIGRATE h 6379 k 0 5000 \"\\x61uth2\" app hunter2",
                "MIGRATE h 6379 k 0 5000 \"\\x61uth2\" app <redacted>",
            ),
            (
                "migrate h 6379 k 0 5000 Auth 'hunter2'",
                "migrate h 6379 k 0 5000 Auth <redacted>",
            ),
            // A quote glued to the next character is refused by the driver,
            // but kept in the history: the argument runs on.
            ("AUTH \"hun\"ter2", "AUTH <redacted>"),
            ("AUTH \"hunter2", "AUTH <redacted>"),
        ] {
            assert_eq!(redact_secrets(line), expected, "{line}");
        }
    }

    #[test]
    fn an_escape_the_server_does_not_decode_stays_what_it_is() {
        // `sdssplitargs` has no octal escape: `"\076"` is `076`, not `>`,
        // and outside quotes a backslash is a byte — neither is a rule.
        for line in [
            "ACL SETUSER u on \"\\076hunter2\"",
            "ACL SETUSER u on \\x3ehunter2",
            "A\\x55TH hunter2",
        ] {
            assert_eq!(redact_secrets(line), line, "{line}");
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
