//! Recognizing, without parsing it, a text that opens a transaction.
//!
//! # Why the driver looks at the text here
//!
//! An Oxyn session relies on a pool: each execution borrows a connection and
//! returns it. A `BEGIN` typed in a console leaves its connection **inside a
//! transaction**; returned to the pool, it would be inherited by the next
//! borrower — introspection, or the next query of another console, which would
//! then write inside a transaction nobody will commit. Undoing it with an
//! implicit `ROLLBACK` would silently cancel what the user wanted to keep. The
//! driver therefore closes the connection.
//!
//! That requires knowing that the execution opened a transaction, and
//! sqlx-postgres 0.9.0 does not expose it: the state carried by `ReadyForQuery`
//! stays `pub(crate)` (`PgConnection::in_transaction`). The text is enough:
//!
//! * the extended protocol prepares **one** statement per execution, so the
//!   first word of the text is the statement's;
//! * in PostgreSQL, only `BEGIN` and `START TRANSACTION` open a transaction
//!   block at statement level. A `BEGIN` in the body of a `DO` or of a function
//!   is dollar-quoted, never leading; a procedure that commits leaves no open
//!   block behind it.
//!
//! This is **not** a classifier: `oxyn-query` decides whether a statement
//! writes. This module answers a single question, and errs on the cautious
//! side — a false positive closes one connection too many.

/// Is the text a transaction control statement?
///
/// `BEGIN`, `START TRANSACTION`, `COMMIT`, `END`, `ROLLBACK`, `ABORT`,
/// `SAVEPOINT`, `RELEASE`, `PREPARE TRANSACTION` — read on the first word, like
/// [`opens_transaction`]. A session without
/// [`Capabilities::TRANSACTIONS`](oxyn_core::Capabilities::TRANSACTIONS)
/// refuses them **before sending**: on a pool, each would go out on whatever
/// connection is borrowed at the time, and a `ROLLBACK` without a transaction
/// "succeeds" on the server — a mere `WARNING` — while the write it was meant to
/// cancel stays committed. Not knowing how is acceptable; letting the user
/// believe otherwise is not ([DRIVER-CONTRACT §5](../../../docs/DRIVER-CONTRACT.md)).
///
/// `PREPARE name AS …` is **not** concerned: only `PREPARE TRANSACTION` is.
#[must_use]
pub(crate) fn controls_transaction(text: &str) -> bool {
    let mut words = Words::new(text);
    let Some(first) = words.next() else {
        return false;
    };
    let is_word = |expected: &str| first.eq_ignore_ascii_case(expected);
    if [
        "begin",
        "commit",
        "end",
        "rollback",
        "abort",
        "savepoint",
        "release",
    ]
    .into_iter()
    .any(is_word)
    {
        return true;
    }
    if is_word("start") || is_word("prepare") {
        return words
            .next()
            .is_some_and(|next_word| next_word.eq_ignore_ascii_case("transaction"));
    }
    false
}

/// Does the text open a transaction block?
///
/// Skips leading whitespace and comments — `--` to the end of the line, `/* */`
/// nested as the server nests them —, then reads the first word. Panics on no
/// input ([I-09](../../../CLAUDE.md#i-09)).
#[must_use]
pub(crate) fn opens_transaction(text: &str) -> bool {
    let mut words = Words::new(text);
    match words.next() {
        Some(word) if word.eq_ignore_ascii_case("begin") => true,
        Some(word) if word.eq_ignore_ascii_case("start") => words
            .next()
            .is_some_and(|next_word| next_word.eq_ignore_ascii_case("transaction")),
        _ => false,
    }
}

/// The leading words of an SQL text, comments and whitespace skipped.
///
/// Stops at the first character that starts neither a word, nor whitespace, nor
/// a comment: beyond it, the structure no longer serves the question asked.
struct Words<'a> {
    rest: &'a str,
}

impl<'a> Words<'a> {
    const fn new(text: &'a str) -> Self {
        Self { rest: text }
    }

    /// Advances past whitespace and comments. Returns `false` if a comment is
    /// not terminated.
    fn skip_trivia(&mut self) -> bool {
        loop {
            let trimmed = self
                .rest
                .trim_start_matches([' ', '\t', '\n', '\r', '\u{000B}', '\u{000C}']);
            if let Some(after) = trimmed.strip_prefix("--") {
                self.rest = after
                    .find(['\n', '\r'])
                    .map_or("", |end| after.get(end..).unwrap_or(""));
            } else if trimmed.starts_with("/*") {
                match block_comment_end(trimmed) {
                    Some(after) => self.rest = after,
                    None => {
                        self.rest = "";
                        return false;
                    }
                }
            } else {
                self.rest = trimmed;
                return true;
            }
        }
    }
}

impl<'a> Iterator for Words<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if !self.skip_trivia() {
            return None;
        }
        let mut characters = self.rest.char_indices();
        let (_, first) = characters.next()?;
        if !(first.is_alphabetic() || first == '_') {
            return None;
        }
        let end = characters
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
            .map_or(self.rest.len(), |(index, _)| index);
        let (word, rest) = self.rest.split_at_checked(end)?;
        self.rest = rest;
        Some(word)
    }
}

/// What follows a `/* … */` comment at the head of `text`, nesting included.
fn block_comment_end(text: &str) -> Option<&str> {
    let mut depth = 0_usize;
    let mut rest = text;
    loop {
        let opening = rest.find("/*");
        let closing = rest.find("*/");
        match (opening, closing) {
            (Some(o), Some(f)) if o < f => {
                depth = depth.saturating_add(1);
                rest = rest.get(o.saturating_add(2)..)?;
            }
            (Some(o), None) => {
                depth = depth.saturating_add(1);
                rest = rest.get(o.saturating_add(2)..)?;
            }
            (_, Some(f)) => {
                depth = depth.checked_sub(1)?;
                rest = rest.get(f.saturating_add(2)..)?;
                if depth == 0 {
                    return Some(rest);
                }
            }
            (None, None) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{controls_transaction, opens_transaction};

    #[test]
    fn transaction_control_is_recognized() {
        for text in [
            "BEGIN",
            "start transaction",
            "COMMIT",
            "commit and chain",
            "END",
            "end transaction",
            "ROLLBACK",
            "rollback to savepoint s",
            "ABORT",
            "SAVEPOINT s",
            "RELEASE SAVEPOINT s",
            "release s",
            "PREPARE TRANSACTION 'x'",
            "COMMIT PREPARED 'x'",
            "ROLLBACK PREPARED 'x'",
            "/* annuler */ ROLLBACK",
            "-- valider\r\ncommit;",
        ] {
            assert!(controls_transaction(text), "{text:?}");
        }
    }

    #[test]
    fn transaction_control_false_friends_pass() {
        for text in [
            "SELECT 'BEGIN'",
            "SELECT 'ROLLBACK'",
            "DO $$ BEGIN PERFORM 1; END $$",
            "PREPARE read_request AS SELECT 1",
            "START",
            "ENDING",
            "commit_log",
            "SELECT 1 -- COMMIT",
            "/* ROLLBACK */ SELECT 1",
            "\"rollback\"",
            "",
        ] {
            assert!(!controls_transaction(text), "{text:?}");
        }
    }

    #[test]
    fn transaction_openings_are_recognized() {
        for text in [
            "BEGIN",
            "begin;",
            "Begin Work",
            "BEGIN TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "START TRANSACTION",
            "start   transaction read write",
            "  \n\t BEGIN",
            "-- open\nBEGIN",
            "-- open\r\nSTART TRANSACTION",
            "/* comment */ begin",
            "/* a /* nested */ still inside */ BEGIN",
            "START /* between */ TRANSACTION",
            "START\n-- line\nTRANSACTION",
        ] {
            assert!(opens_transaction(text), "{text:?}");
        }
    }

    #[test]
    fn everything_else_is_not_an_opening() {
        for text in [
            "",
            "   ",
            "SELECT 'BEGIN'",
            "SELECT 1 -- BEGIN",
            "COMMIT",
            "ROLLBACK",
            "END",
            "BEGINNING",
            "START",
            "START TRANSACTIONS",
            "DO $$ BEGIN PERFORM 1; END $$",
            "/* BEGIN */ SELECT 1",
            "-- BEGIN",
            "\"begin\"",
            "/* never closed BEGIN",
            "/* a /* b */ BEGIN",
            "*/ BEGIN",
            "(BEGIN)",
            "café BEGIN",
        ] {
            assert!(!opens_transaction(text), "{text:?}");
        }
    }

    #[test]
    fn no_input_makes_it_panic() {
        // I-09: the text comes from the user or from an agent. Cuts in the
        // middle of a multi-byte character and orphan markers are the minimal
        // corpus.
        for text in [
            "/",
            "-",
            "*",
            "/*",
            "*/",
            "--",
            "é",
            "/*é*/é",
            "--é\né",
            "\u{0}",
            "𝔅EGIN",
            "/**/",
            "/*/",
            "/*/*/",
            "BEGIN\u{0}",
        ] {
            let _ = opens_transaction(text);
        }
    }
}
