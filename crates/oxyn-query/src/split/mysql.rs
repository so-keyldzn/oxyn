//! The MySQL batch: stored program bodies and the `DELIMITER` directive.
//!
//! Two things a lexical profile cannot carry, because they depend on words and
//! not on bytes:
//!
//! * the body of a stored program — `CREATE PROCEDURE p() BEGIN SELECT 1;
//!   DELETE FROM t; END` — is **one** statement for the server, semicolons
//!   included. Cut at its semicolons, it becomes three fragments, and the
//!   middle one is a `DELETE` Oxyn would send on its own;
//! * the `mysql` client's `DELIMITER` line is a directive of the batch, not
//!   SQL: the server never sees it, and it changes what ends a statement.
//!
//! # The direction of the error
//!
//! The parent module's, applied to block depth. A closing word that does not
//! match the innermost open block means the reading went wrong: the fragment
//! then runs to the end of the text. A body closed too early exposes one of
//! its statements as a statement of the batch; a body closed too late only
//! merges, and a merged fragment is approved or refused as a whole.

use std::ops::Range;

use oxyn_core::SqlDialect;

use super::{Flags, Fragment, Scanner, SplitProfile, Tok, push_fragment};

/// Splits a MySQL batch.
pub(super) fn split(sql: &str) -> Vec<Fragment<'_>> {
    let mut fragments = Vec::new();
    let mut scanner = Scanner::new(sql, SplitProfile::for_dialect(SqlDialect::MySql));
    let mut pending = Pending::at(0);
    while let Some((token, span)) = scanner.next() {
        match token {
            Tok::Comment => pending.has_comment = true,
            // Never dropped as « comments only »: what it hides may run.
            Tok::UnreadableComment => {
                pending.has_comment = true;
                pending.has_code = true;
                pending.unreadable = true;
            }
            Tok::Semicolon if pending.body.is_open() => pending.body.separator(),
            Tok::Semicolon => {
                pending.flush(sql, span.start, true, &mut fragments);
                pending = Pending::at(span.end);
            }
            Tok::Word => {
                if let Some(directive) = directive_at(sql, span.clone()) {
                    // The `mysql` client sends what it holds before obeying.
                    pending.flush(sql, span.start, false, &mut fragments);
                    scanner.terminator = directive.delimiter;
                    scanner.at = directive.end;
                    pending = Pending::at(directive.end);
                    continue;
                }
                pending.has_code = true;
                // Under a custom delimiter, the author said where statements
                // end: a body needs no reading, and `;` is no separator anyway.
                if scanner.terminator == b";" {
                    pending.body.word(span, &mut scanner);
                }
            }
            Tok::Quoted | Tok::SingleQuoted | Tok::Symbol => {
                pending.has_code = true;
                pending.body.other(is_colon(sql, token, &span));
            }
        }
    }
    pending.flush(sql, sql.len(), false, &mut fragments);
    fragments
}

/// Is the fragment a stored program, and is its body closed where it ends?
///
/// `None`: not a stored program. `Some(false)`: a block is still open, the
/// reading went wrong, or the text holds a second statement after the body.
pub(super) fn program_is_complete(text: &str) -> Option<bool> {
    let mut scanner = Scanner::new(text, SplitProfile::for_dialect(SqlDialect::MySql));
    let mut body = Body::default();
    let mut single = true;
    while let Some((token, span)) = scanner.next() {
        match token {
            Tok::Semicolon if body.is_open() => body.separator(),
            Tok::Semicolon => single = false,
            Tok::Word => body.word(span, &mut scanner),
            Tok::Quoted | Tok::SingleQuoted | Tok::Symbol => {
                body.other(is_colon(text, token, &span));
            }
            Tok::Comment | Tok::UnreadableComment => {}
        }
    }
    body.is_program().then_some(single && !body.is_open())
}

/// `lbl:` puts what follows at the start of a statement.
fn is_colon(sql: &str, token: Tok, span: &Range<usize>) -> bool {
    token == Tok::Symbol && sql.as_bytes().get(span.start) == Some(&b':')
}

/// The statement being read.
struct Pending {
    start: usize,
    has_comment: bool,
    has_code: bool,
    unreadable: bool,
    body: Body,
}

impl Pending {
    fn at(start: usize) -> Self {
        Self {
            start,
            has_comment: false,
            has_code: false,
            unreadable: false,
            body: Body::default(),
        }
    }

    fn flush<'a>(&self, sql: &'a str, end: usize, terminated: bool, out: &mut Vec<Fragment<'a>>) {
        if self.has_code {
            let flags = Flags::new(self.has_comment, terminated, self.unreadable);
            push_fragment(sql, self.start..end, flags, out);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The `DELIMITER` directive
// ─────────────────────────────────────────────────────────────────────────────

/// A `DELIMITER` line: the new terminator, and where the line ends.
struct Directive<'a> {
    delimiter: &'a [u8],
    end: usize,
}

/// The directive whose `DELIMITER` word spans `word`, if it is one.
///
/// As the `mysql` client reads it: first on its line, case-insensitive,
/// followed by a blank, then the delimiter up to the next blank; the rest of
/// the line goes with the directive. A delimiter holding a `\` is refused by
/// the client, so the line stays text — and text the server will refuse.
fn directive_at(sql: &str, word: Range<usize>) -> Option<Directive<'_>> {
    if !sql.get(word.clone())?.eq_ignore_ascii_case("delimiter") {
        return None;
    }
    let before = sql.get(..word.start)?;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    if !before
        .get(line_start..)?
        .bytes()
        .all(|c| c.is_ascii_whitespace())
    {
        return None;
    }
    let rest = sql.get(word.end..)?;
    let line = rest.split('\n').next().unwrap_or(rest);
    if !line.starts_with([' ', '\t']) {
        return None;
    }
    let delimiter = line.split_ascii_whitespace().next()?;
    if delimiter.contains('\\') {
        return None;
    }
    Some(Directive {
        delimiter: delimiter.as_bytes(),
        end: word.end.checked_add(line.len())?,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Stored program bodies
// ─────────────────────────────────────────────────────────────────────────────

/// What the first words of the statement say about it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Header {
    #[default]
    Start,
    /// `CREATE` or `ALTER` (`ALTER EVENT … DO` carries a body), no program
    /// keyword yet.
    Create,
    /// A stored program, or MariaDB's anonymous `BEGIN NOT ATOMIC` block:
    /// blocks are counted.
    Program,
    Other,
}

/// A block open in a body, and what closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// Closed by `END`.
    Begin,
    /// A `CASE` statement, closed by `END CASE`.
    CaseStatement,
    /// A `CASE` expression, closed by `END` in the middle of an expression.
    CaseExpression,
    If,
    Loop,
    While,
    Repeat,
    /// MariaDB's `FOR … DO … END FOR`.
    For,
}

impl Frame {
    const fn closed_by(self, qualifier: Keyword) -> bool {
        matches!(
            (self, qualifier),
            (Self::CaseStatement | Self::CaseExpression, Keyword::Case)
                | (Self::If, Keyword::If)
                | (Self::Loop, Keyword::Loop)
                | (Self::While, Keyword::While)
                | (Self::Repeat, Keyword::Repeat)
                | (Self::For, Keyword::For)
        )
    }
}

/// The words that open, close or delimit a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keyword {
    Begin,
    Case,
    If,
    Loop,
    While,
    Repeat,
    For,
    End,
    Then,
    Else,
    Do,
}

impl Keyword {
    const TABLE: [(&'static str, Self); 11] = [
        ("begin", Self::Begin),
        ("case", Self::Case),
        ("if", Self::If),
        ("loop", Self::Loop),
        ("while", Self::While),
        ("repeat", Self::Repeat),
        ("for", Self::For),
        ("end", Self::End),
        ("then", Self::Then),
        ("else", Self::Else),
        ("do", Self::Do),
    ];

    fn of(word: &str) -> Option<Self> {
        Self::TABLE
            .iter()
            .find(|(text, _)| word.eq_ignore_ascii_case(text))
            .map(|&(_, keyword)| keyword)
    }

    /// The word after `END` that names the block it closes.
    const fn is_qualifier(self) -> bool {
        matches!(
            self,
            Self::Case | Self::If | Self::Loop | Self::While | Self::Repeat | Self::For
        )
    }
}

const PROGRAMS: [&str; 4] = ["procedure", "function", "trigger", "event"];

/// Block depth of a statement's body.
#[derive(Debug, Default)]
struct Body {
    header: Header,
    frames: Vec<Frame>,
    /// A closing word did not match the open block: nothing is closed any
    /// more, the fragment runs to the end of the text.
    confused: bool,
    /// The next word starts a statement of the body. `IF` there is a
    /// statement, not the `IF()` function; `END` there closes a block, and
    /// elsewhere may be a column named `end`.
    statement_start: bool,
}

impl Body {
    fn is_open(&self) -> bool {
        self.confused || !self.frames.is_empty()
    }

    fn is_program(&self) -> bool {
        self.header == Header::Program
    }

    fn separator(&mut self) {
        self.statement_start = true;
    }

    fn other(&mut self, colon: bool) {
        if self.header == Header::Start {
            self.header = Header::Other;
        }
        self.statement_start = colon;
    }

    fn word(&mut self, span: Range<usize>, scanner: &mut Scanner<'_>) {
        let Some(word) = scanner.sql.get(span) else {
            return;
        };
        match self.header {
            Header::Start => self.first_word(word, scanner),
            // Any program keyword after `CREATE` counts, `CREATE TABLE event`
            // included: taking a statement for a program only merges, taking a
            // program for a statement splits its body.
            Header::Create if PROGRAMS.iter().any(|p| word.eq_ignore_ascii_case(p)) => {
                self.header = Header::Program;
            }
            Header::Program => self.body_word(word, scanner),
            Header::Create | Header::Other => {}
        }
    }

    fn first_word(&mut self, word: &str, scanner: &mut Scanner<'_>) {
        self.header = if word.eq_ignore_ascii_case("create") || word.eq_ignore_ascii_case("alter") {
            Header::Create
        } else if word.eq_ignore_ascii_case("begin") && skip_not_atomic(scanner) {
            self.frames.push(Frame::Begin);
            self.statement_start = true;
            Header::Program
        } else {
            // `BEGIN` alone, `BEGIN WORK`: a transaction.
            Header::Other
        };
    }

    fn body_word(&mut self, word: &str, scanner: &mut Scanner<'_>) {
        let at_start = std::mem::replace(&mut self.statement_start, false);
        let Some(keyword) = Keyword::of(word) else {
            return;
        };
        match keyword {
            Keyword::Begin => self.open(Frame::Begin, true),
            Keyword::Case if at_start => self.frames.push(Frame::CaseStatement),
            Keyword::Case => self.frames.push(Frame::CaseExpression),
            Keyword::If if opens_block(at_start, scanner, true) => self.open(Frame::If, false),
            Keyword::Repeat if opens_block(at_start, scanner, false) => {
                self.open(Frame::Repeat, true);
            }
            Keyword::Loop => self.open(Frame::Loop, true),
            Keyword::While => self.open(Frame::While, false),
            // `FOR EACH ROW`, `HANDLER FOR`, `CURSOR FOR` are not loops.
            Keyword::For if at_start => self.open(Frame::For, false),
            Keyword::End => self.close(at_start, scanner),
            // Inside a `CASE` expression, `THEN 1` is no statement.
            Keyword::Then | Keyword::Else => {
                self.statement_start = self.frames.last() != Some(&Frame::CaseExpression);
            }
            Keyword::Do => self.statement_start = true,
            Keyword::If | Keyword::Repeat | Keyword::For => {}
        }
    }

    fn open(&mut self, frame: Frame, body_follows: bool) {
        self.frames.push(frame);
        self.statement_start = body_follows;
    }

    fn close(&mut self, at_start: bool, scanner: &mut Scanner<'_>) {
        let mut ahead = scanner.clone();
        let qualifier = match ahead.next() {
            Some((Tok::Word, span)) => scanner
                .sql
                .get(span)
                .and_then(Keyword::of)
                .filter(|keyword| keyword.is_qualifier()),
            _ => None,
        };
        let top = self.frames.last().copied();
        let closes = match qualifier {
            Some(keyword) => {
                *scanner = ahead;
                top.is_some_and(|frame| frame.closed_by(keyword))
            }
            None if at_start => top == Some(Frame::Begin),
            // Mid-expression: the end of a `CASE` expression, or a column
            // named `end`, which closes nothing.
            None if top == Some(Frame::CaseExpression) => true,
            None => return,
        };
        if closes {
            self.frames.pop();
        } else {
            self.confused = true;
        }
    }
}

/// Does an `IF` or a `REPEAT` open a block here?
///
/// At the start of a statement, always. Elsewhere — the first statement of a
/// body without `BEGIN` — unless it is the `IF()` or `REPEAT()` function, or
/// the `IF [NOT] EXISTS` of a DDL statement. A block missed here is caught by
/// its `END IF`, which then closes nothing and stops the reading.
fn opens_block(at_start: bool, scanner: &Scanner<'_>, exists_clause: bool) -> bool {
    if at_start {
        return true;
    }
    match scanner.clone().next() {
        Some((Tok::Symbol, span)) => scanner.sql.as_bytes().get(span.start) != Some(&b'('),
        Some((Tok::Word, span)) => {
            let next = scanner.sql.get(span).unwrap_or_default();
            !(exists_clause
                && (next.eq_ignore_ascii_case("exists") || next.eq_ignore_ascii_case("not")))
        }
        _ => true,
    }
}

/// MariaDB's `BEGIN NOT ATOMIC` opens a block outside any program; `BEGIN`
/// alone opens a transaction. Moves the scanner past `NOT ATOMIC` when found.
fn skip_not_atomic(scanner: &mut Scanner<'_>) -> bool {
    let mut ahead = scanner.clone();
    let sql = scanner.sql;
    let mut expect = |keyword: &str| match ahead.next() {
        Some((Tok::Word, span)) => sql
            .get(span)
            .is_some_and(|word| word.eq_ignore_ascii_case(keyword)),
        _ => false,
    };
    let found = expect("not") && expect("atomic");
    if found {
        *scanner = ahead;
    }
    found
}

#[cfg(test)]
mod tests;
