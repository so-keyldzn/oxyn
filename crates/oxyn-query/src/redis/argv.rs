//! A Redis command line, as `redis-cli` reads it.
//!
//! # The grammar is `redis-cli`'s
//!
//! [`parse`] follows `sdssplitargs` of Redis 8.10.2 (`src/sds.c`): arguments
//! separated by blanks; inside double quotes `\n`, `\r`, `\t`, `\b`, `\a`,
//! `\xHH` and `\<any>` are escapes; inside single quotes only `\'` is; a
//! closing quote must be followed by a blank or the end. A professional pastes
//! what works in `redis-cli`, and it means the same here.
//!
//! Two departures, both refusals, never a different reading:
//!
//! * **one command per execution.** `redis-cli` reads a line as a command; a
//!   text with a second line outside quotes would here become extra arguments
//!   of the first command — `GET a` then `DEL b` sent as `GET a DEL b`. It is
//!   refused, naming the line;
//! * **a NUL character is refused**: the C parser stops there, so what follows
//!   it would be read differently by `redis-cli` and by Oxyn. `"\x00"` writes
//!   the byte.
//!
//! # One decoder for what runs and for what is kept
//!
//! The driver builds the argv it sends with [`parse`]; the redaction of
//! credentials reads a console text with [`commands`]. Both go through the
//! same private function that reads one argument, so a password is recognized on the bytes the
//! server receives: `"\x3ehunter2"` is the rule `>hunter2` for `ACL SETUSER`,
//! and `"A\x55TH"` is `AUTH`. Two decoders would disagree on exactly the
//! escape nobody tested, and the secret would be kept
//! ([ADR-0061](../../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md)).

use std::ops::Range;

/// Why a command line cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ArgvError {
    /// Nothing but blanks.
    #[error("the command is empty")]
    Empty,
    /// A quote opened and never closed.
    #[error("a quote opened at byte {0} is never closed")]
    Unterminated(usize),
    /// A closing quote glued to the next character, which `redis-cli` refuses.
    #[error("a closing quote at byte {0} must be followed by a space or the end of the command")]
    GluedQuote(usize),
    /// A second command after a line break.
    #[error(
        "one command per execution: line {0} starts another one; run it on its own, or wrap a \
         line break that belongs to a value in double quotes"
    )]
    SecondCommand(usize),
    /// A NUL character typed in the text.
    #[error("a NUL character cannot be typed; write it \"\\x00\" inside double quotes")]
    Nul,
}

/// One argument of a command: the bytes the server receives, and where it is
/// written in the text, quotes included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    /// The decoded bytes.
    pub bytes: Vec<u8>,
    /// Its byte span in the text, always on character boundaries.
    pub span: Range<usize>,
}

/// Splits a command line into its arguments, binary-safe.
///
/// # Errors
/// [`ArgvError`] when the line is empty, a quote is left open or glued to the
/// next character, a second command follows a line break, or a NUL is typed.
pub fn parse(line: &str) -> Result<Vec<Vec<u8>>, ArgvError> {
    let bytes = line.as_bytes();
    if bytes.contains(&0) {
        return Err(ArgvError::Nul);
    }
    let mut argv = Vec::new();
    let mut i = 0;
    let mut line_number = 1;
    loop {
        let (next, breaks) = skip_blanks(bytes, i);
        i = next;
        line_number += breaks;
        if i >= bytes.len() {
            break;
        }
        if line_number > 1 && !argv.is_empty() {
            return Err(ArgvError::SecondCommand(line_number));
        }
        let (argument, next) = token(bytes, i, Reading::Strict)?;
        argv.push(argument);
        i = next;
    }
    if argv.is_empty() {
        return Err(ArgvError::Empty);
    }
    Ok(argv)
}

/// Every command of a console text, one per line outside quotes, as the
/// arguments [`parse`] would decode.
///
/// Never fails: what [`parse`] refuses is still read, the way that keeps the
/// most bytes inside one argument — a quote left open runs to the end of the
/// text, a quote glued to the next character continues the argument, a NUL is
/// a byte. A text the driver refuses is still kept in the history, and its
/// password with it if this reading stopped where the parser does.
#[must_use]
pub fn commands(text: &str) -> Vec<Vec<Argument>> {
    let bytes = text.as_bytes();
    let mut commands = Vec::new();
    let mut current = Vec::new();
    let mut i = 0;
    loop {
        let (next, breaks) = skip_blanks(bytes, i);
        if breaks > 0 && !current.is_empty() {
            commands.push(std::mem::take(&mut current));
        }
        i = next;
        if i >= bytes.len() {
            break;
        }
        // The lenient reading has no error to return.
        let Ok((argument, end)) = token(bytes, i, Reading::Lenient) else {
            break;
        };
        current.push(Argument {
            bytes: argument,
            span: i..end,
        });
        i = end;
    }
    if !current.is_empty() {
        commands.push(current);
    }
    commands
}

/// Whether a token stops at what `redis-cli` refuses, or reads on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    Strict,
    Lenient,
}

/// Skips the blanks from `start`; returns where the next argument starts and
/// how many line breaks were crossed.
fn skip_blanks(bytes: &[u8], start: usize) -> (usize, usize) {
    let mut i = start;
    let mut breaks = 0;
    while let Some(&byte) = bytes.get(i) {
        if !is_space(byte) {
            break;
        }
        if byte == b'\n' {
            breaks += 1;
        }
        i += 1;
    }
    (i, breaks)
}

/// Reads one argument starting at `start`; returns it and where reading
/// continues.
fn token(bytes: &[u8], start: usize, reading: Reading) -> Result<(Vec<u8>, usize), ArgvError> {
    let mut current = Vec::new();
    let mut i = start;
    let mut quote: Option<(u8, usize)> = None;
    loop {
        let byte = bytes.get(i).copied();
        match (quote, byte) {
            (Some(_), None) if reading == Reading::Lenient => return Ok((current, i)),
            (Some((_, opened)), None) => return Err(ArgvError::Unterminated(opened)),
            (None, None | Some(b' ' | b'\n' | b'\r' | b'\t')) => return Ok((current, i)),
            (None, Some(open @ (b'"' | b'\''))) => quote = Some((open, i)),
            (None, Some(other)) => current.push(other),
            (Some((b'"', _)), Some(b'\\')) => {
                let escaped = bytes.get(i + 1).copied();
                let hex = (bytes.get(i + 2).copied(), bytes.get(i + 3).copied());
                match (escaped, hex) {
                    (Some(b'x'), (Some(high), Some(low)))
                        if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() =>
                    {
                        current.push(hex_value(high) * 16 + hex_value(low));
                        i += 3;
                    }
                    (Some(next), _) => {
                        current.push(match next {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            b'b' => 0x08,
                            b'a' => 0x07,
                            other => other,
                        });
                        i += 1;
                    }
                    (None, _) => current.push(b'\\'),
                }
            }
            (Some((b'\'', _)), Some(b'\\')) if bytes.get(i + 1) == Some(&b'\'') => {
                current.push(b'\'');
                i += 1;
            }
            (Some((open, _)), Some(close)) if close == open => {
                if bytes.get(i + 1).is_some_and(|&next| !is_space(next)) {
                    if reading == Reading::Strict {
                        return Err(ArgvError::GluedQuote(i));
                    }
                    quote = None;
                } else {
                    return Ok((current, i + 1));
                }
            }
            (Some(_), Some(other)) => current.push(other),
        }
        i += 1;
    }
}

/// `isspace` in the C locale.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

const fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn words(line: &str) -> Vec<Vec<u8>> {
        parse(line).unwrap_or_else(|error| panic!("{line:?}: {error}"))
    }

    #[rstest]
    #[case("GET k", &[b"GET".as_slice(), b"k"])]
    #[case("  SET   k   v  ", &[b"SET".as_slice(), b"k", b"v"])]
    #[case(r#"SET "a b" 'c d'"#, &[b"SET".as_slice(), b"a b", b"c d"])]
    #[case(r#"SET "\x00\xff\n\"" 'it\'s'"#, &[b"SET".as_slice(), b"\x00\xff\n\"", b"it's"])]
    #[case(r#"SET k "\q""#, &[b"SET".as_slice(), b"k", b"q"])]
    #[case(r#"SET k "\xZZ""#, &[b"SET".as_slice(), b"k", b"xZZ"])]
    #[case(r#"SET k 'a\nb'"#, &[b"SET".as_slice(), b"k", br"a\nb"])]
    #[case(r#"SET a"b c" e"#, &[b"SET".as_slice(), b"ab c", b"e"])]
    #[case("SET k \"line\nbreak\"", &[b"SET".as_slice(), b"k", b"line\nbreak"])]
    #[case("\n\n  PING  \n\n", &[b"PING".as_slice()])]
    // No octal escape in `sdssplitargs`: `\0` is the character `0`.
    #[case(r#"SET k "\076""#, &[b"SET".as_slice(), b"k", b"076"])]
    fn a_line_splits_as_redis_cli_splits_it(#[case] line: &str, #[case] expected: &[&[u8]]) {
        assert_eq!(
            words(line),
            expected.iter().map(|w| w.to_vec()).collect::<Vec<_>>()
        );
    }

    #[rstest]
    #[case("", ArgvError::Empty)]
    #[case("   \n\t ", ArgvError::Empty)]
    #[case(r#"GET "k"#, ArgvError::Unterminated(4))]
    #[case("GET 'k", ArgvError::Unterminated(4))]
    #[case(r#"GET "k"x"#, ArgvError::GluedQuote(6))]
    #[case("GET a\nDEL a", ArgvError::SecondCommand(2))]
    #[case("GET a\r\nFLUSHALL", ArgvError::SecondCommand(2))]
    #[case("GET a\u{0}b", ArgvError::Nul)]
    fn what_redis_cli_would_read_otherwise_is_refused(
        #[case] line: &str,
        #[case] expected: ArgvError,
    ) {
        assert_eq!(parse(line), Err(expected));
    }

    #[rstest]
    #[case("GET k")]
    #[case(r#"SET "a b" 'c d'"#)]
    #[case(r#"SET "\x00\xff\n\"" 'it\'s'"#)]
    #[case("SET k \"line\nbreak\"")]
    #[case(r#"ACL SETUSER u "\x3ehunter2""#)]
    fn the_lenient_reading_decodes_what_the_parser_decodes(#[case] line: &str) {
        let read: Vec<Vec<u8>> = commands(line)
            .into_iter()
            .flatten()
            .map(|argument| argument.bytes)
            .collect();
        assert_eq!(read, words(line), "{line}");
    }

    #[test]
    fn the_lenient_reading_keeps_spans_and_reads_on() {
        let text = "GET 'a b'\nAUTH \"hun\"ter2 \"open";
        let read = commands(text);
        assert_eq!(read.len(), 2);
        let spans: Vec<&str> = read
            .iter()
            .flatten()
            .map(|argument| text.get(argument.span.clone()).unwrap_or_default())
            .collect();
        assert_eq!(spans, ["GET", "'a b'", "AUTH", "\"hun\"ter2", "\"open"]);
        let auth = read.get(1).and_then(|command| command.get(1));
        assert_eq!(
            auth.map(|a| a.bytes.as_slice()),
            Some(b"hunter2".as_slice())
        );
    }
}
