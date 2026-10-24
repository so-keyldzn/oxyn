//! Where one MongoDB command ends in a console holding several.
//!
//! A MongoDB console holds JSON documents one after another, each a command.
//! The boundary is lexical: a top-level `{` opens a command, its matching `}`
//! closes it, and braces inside strings do not count. What lies between two
//! commands is not run, so the cursor there runs nothing — never the
//! neighbour, which may be a write.

/// The command under the cursor: the top-level JSON object whose span holds
/// `cursor_byte`, closing brace included.
///
/// An object left open at the end of the text runs to the end of the text, so
/// the parser reports what is missing instead of the console saying nothing.
/// `None` when the cursor is outside every object.
#[must_use]
pub fn command_at(text: &str, cursor_byte: usize) -> Option<&str> {
    command_spans(text)
        .find(|span| span.start <= cursor_byte && cursor_byte <= span.end)
        .and_then(|span| text.get(span))
}

/// The spans of the top-level objects of `text`, in order.
fn command_spans(text: &str) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    let bytes = text.as_bytes();
    let mut index = 0_usize;
    std::iter::from_fn(move || {
        let start = index.saturating_add(bytes.get(index..)?.iter().position(|b| *b == b'{')?);
        let mut depth = 0_usize;
        let mut cursor = start;
        while let Some(&byte) = bytes.get(cursor) {
            match byte {
                b'"' => {
                    cursor = super::skip_string(bytes, cursor.saturating_add(1));
                    continue;
                }
                b'{' | b'[' => depth = depth.saturating_add(1),
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        index = cursor.saturating_add(1);
                        return Some(start..index);
                    }
                }
                _ => {}
            }
            cursor = cursor.saturating_add(1);
        }
        index = bytes.len();
        Some(start..bytes.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "{\"find\": \"users\"}\n\n{\"delete\": \"users\", \"deletes\": []}";

    #[test]
    fn the_cursor_inside_a_command_runs_that_command_only() {
        assert_eq!(command_at(TWO, 3), Some("{\"find\": \"users\"}"));
        let second = TWO
            .find("{\"delete\"")
            .expect("the second command is in the text");
        assert_eq!(
            command_at(TWO, second.saturating_add(4)),
            Some("{\"delete\": \"users\", \"deletes\": []}")
        );
    }

    #[test]
    fn the_cursor_just_after_the_closing_brace_still_runs_it() {
        let end = TWO.find('\n').expect("a newline separates the commands");
        assert_eq!(command_at(TWO, end), Some("{\"find\": \"users\"}"));
    }

    #[test]
    fn between_two_commands_nothing_runs() {
        let blank = TWO.find("\n\n").expect("a blank line").saturating_add(1);
        assert_eq!(command_at(TWO, blank), None);
    }

    #[test]
    fn braces_inside_strings_are_not_boundaries() {
        let text = "{\"find\": \"a}b{\", \"filter\": {\"k\": \"}\"}}\n{\"drop\": \"a\"}";
        assert_eq!(
            command_at(text, 1),
            Some("{\"find\": \"a}b{\", \"filter\": {\"k\": \"}\"}}")
        );
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_string() {
        let text = r#"{"find": "a\"}"} {"drop": "a"}"#;
        assert_eq!(command_at(text, 1), Some(r#"{"find": "a\"}"}"#));
    }

    #[test]
    fn an_object_left_open_runs_to_the_end_for_the_parser_to_refuse() {
        let text = "{\"find\": \"users\", \"filter\": {";
        assert_eq!(command_at(text, 2), Some(text));
    }

    #[test]
    fn a_cursor_beyond_the_text_or_inside_a_character_does_not_panic() {
        assert_eq!(
            command_at("{\"find\": \"é\"}", 11),
            Some("{\"find\": \"é\"}")
        );
        assert_eq!(command_at("{}", 99), None);
        assert_eq!(command_at("", 0), None);
    }
}
