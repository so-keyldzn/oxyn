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
}
