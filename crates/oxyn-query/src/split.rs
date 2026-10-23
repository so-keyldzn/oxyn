//! Splitting a SQL text into statements, and reading its bare words.
//!
//! An editor holds a *batch*: several statements separated by semicolons. To
//! classify that batch it must first be split, and a naive split on `';'` goes
//! wrong at the first apostrophe:
//!
//! ```text
//! SELECT ';' ;                       -- a semicolon inside a string
//! SELECT 1; -- then ; in a comment
//! CREATE FUNCTION f() ... $$ BEGIN ... ; ... END $$;
//! ```
//!
//! Each of these lines is **one** statement. The scanner of this module
//! therefore recognizes, without building an AST: strings, quoted identifiers,
//! `--`, `#`, `/* */` comments, and PostgreSQL `$$ … $$` bodies. What MySQL
//! adds on top — `BEGIN … END` bodies of stored programs and the `mysql`
//! client's `DELIMITER` line — depends on words, not bytes, and lives in the
//! private `mysql` submodule.
//!
//! # The direction of the error
//!
//! A scanner can be wrong. When it is wrong, it **merges**: an unclosed string
//! swallows the end of the text, and so does an unclosed comment. The
//! resulting fragment no longer parses, so [`classify`](crate::classify())
//! classifies it [`Unknown`](oxyn_core::StatementIntent::Unknown), so the
//! `PolicyGate` asks for an approval. A splitting error costs one confirmation
//! too many, never one write too few.

use std::{borrow::Cow, ops::Range};

use oxyn_core::SqlDialect;

use crate::{QueryError, validate};

mod mysql;

/// A statement isolated in a batch.
///
/// The text is **borrowed** from the original batch and trimmed of its
/// whitespace: it is exactly what is given to the parser, and
/// [`span`](Self::span) leads back to the spot in the editor's buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment<'a> {
    /// Text of the statement, without the final semicolon or edge whitespace.
    pub text: &'a str,
    /// Byte bounds of [`text`](Self::text) in the original batch.
    pub span: Range<usize>,
    /// The fragment contains at least one comment.
    ///
    /// Used by [`format`](crate::format()): the `Display` of the `sqlparser`
    /// AST does not keep comments, so a fragment that carries some is not
    /// reformatted.
    pub has_comment: bool,
    /// The fragment ended with an explicit `;` in the batch — or, in MySQL,
    /// with the delimiter a `DELIMITER` line set.
    pub terminated: bool,
    /// A line comment in this fragment holds a lone `\r` followed by text, in a
    /// dialect whose lexer was not checked ([`LineCommentEnd::Unverified`]).
    ///
    /// Where that comment ends depends on the server, so which statements the
    /// fragment holds cannot be known: [`classify`](crate::classify()) makes it
    /// `Unknown` and [`validate`](crate::validate()) refuses it.
    pub unreadable_comment: bool,
}

/// Which bytes end a `--` (or `#`) comment, as the server's own lexer reads it.
///
/// Neither choice is safe by default. Ending too late hides a statement the
/// server runs — PostgreSQL runs `-- x\rDROP TABLE audit`. Ending too early
/// exposes a `/*` or a quote the server reads as comment text, and the
/// statement behind it vanishes into what the splitter takes for a block
/// comment — SQLite runs the `DROP` in `SELECT 1; -- x\r/*\nDROP TABLE audit -- */`.
/// Sources: docs/RESEARCH-NOTES.md, "End of a `--` comment".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineCommentEnd {
    /// `\n` only: SQLite (`tokenize.c`), ClickHouse (observed on 26.3).
    LineFeed,
    /// `\n` or `\r`: PostgreSQL (`scan.l`, `newline [\n\r]`).
    LineFeedOrCarriageReturn,
    /// Not checked against the server's lexer. The comment runs to `\n`, and a
    /// lone `\r` followed by anything but whitespace marks the fragment
    /// [`unreadable_comment`](Fragment::unreadable_comment).
    Unverified,
}

/// A bare word of the text: not in a string, not in a quoted identifier, not
/// in a comment.
///
/// It is the material of the keyword safety net of
/// [`classify`](crate::classify()): recognizing `DELETE` in
/// `SELECT * FROM t WHERE note = 'DELETE'` would be a false positive, and this
/// type exists to make that false positive impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word<'a> {
    /// The word, as written (case is kept).
    pub text: &'a str,
    /// Byte bounds in the original text.
    pub span: Range<usize>,
    /// The word is immediately followed by an opening parenthesis: it is a
    /// function call, not a statement keyword.
    ///
    /// `TRUNCATE(x, 2)` and `INSERT('abc', 1, 1, 'z')` are MySQL functions;
    /// confusing them with `TRUNCATE TABLE` and `INSERT INTO` would ask for an
    /// approval for a `SELECT`.
    pub call: bool,
}

/// The lexical particularities of a dialect.
///
/// It is not a grammar: only what one must know to go through a text without
/// understanding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitProfile {
    /// `\` escapes the next character inside a string (MySQL, ClickHouse,
    /// Snowflake, BigQuery). PostgreSQL does **not**: with
    /// `standard_conforming_strings` on, `'a\'` is a complete string.
    pub backslash_escapes: bool,
    /// An `E` prefix turns on `\` escapes for that string (PostgreSQL's
    /// `E'\''`).
    pub escape_string_prefix: bool,
    /// Backticks quote an identifier (MySQL, ClickHouse, BigQuery).
    pub backtick_quotes: bool,
    /// Square brackets quote an identifier (SQL Server, SQLite).
    pub bracket_quotes: bool,
    /// `$tag$ … $tag$` delimits a literal body (PostgreSQL, Redshift, DuckDB,
    /// ClickHouse).
    pub dollar_quotes: bool,
    /// `#` opens an end-of-line comment (MySQL).
    pub hash_line_comments: bool,
    /// `/* /* */ */` nests (PostgreSQL, DuckDB, ClickHouse).
    pub nested_block_comments: bool,
    /// Where a line comment ends.
    pub line_comment_end: LineCommentEnd,
    /// `\` escapes the next character inside a backtick identifier
    /// (ClickHouse 26.3 reads `` `a\`b` `` as `` a`b ``). MySQL does not: its
    /// `` `a\` `` is a complete identifier.
    pub backtick_backslash_escapes: bool,
}

impl SplitProfile {
    /// The profile of a dialect.
    ///
    /// An unknown dialect gets the most **inclusive** profile: every form of
    /// quoting is recognized. Recognizing a quoting that does not exist in the
    /// real dialect merges two statements at worst, which leads to `Unknown`;
    /// not recognizing it would cut in the middle of a string and produce two
    /// halves, one of which could parse as a valid statement nobody wrote.
    #[must_use]
    pub fn for_dialect(dialect: SqlDialect) -> Self {
        match dialect {
            SqlDialect::Postgres | SqlDialect::Redshift => Self {
                backslash_escapes: false,
                escape_string_prefix: true,
                backtick_quotes: false,
                bracket_quotes: false,
                dollar_quotes: true,
                hash_line_comments: false,
                nested_block_comments: true,
                line_comment_end: if matches!(dialect, SqlDialect::Postgres) {
                    LineCommentEnd::LineFeedOrCarriageReturn
                } else {
                    LineCommentEnd::Unverified
                },
                backtick_backslash_escapes: false,
            },
            SqlDialect::MySql => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: false,
                hash_line_comments: true,
                nested_block_comments: false,
                line_comment_end: LineCommentEnd::Unverified,
                backtick_backslash_escapes: false,
            },
            SqlDialect::Sqlite => Self {
                backslash_escapes: false,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: true,
                dollar_quotes: false,
                hash_line_comments: false,
                nested_block_comments: false,
                line_comment_end: LineCommentEnd::LineFeed,
                backtick_backslash_escapes: false,
            },
            SqlDialect::SqlServer => Self {
                backslash_escapes: false,
                escape_string_prefix: false,
                backtick_quotes: false,
                bracket_quotes: true,
                dollar_quotes: false,
                hash_line_comments: false,
                nested_block_comments: true,
                line_comment_end: LineCommentEnd::Unverified,
                backtick_backslash_escapes: false,
            },
            // `--` and `#` end at `\n` only: ClickHouse 26.3 runs
            // `SELECT 1 -- x\r, 2` as one column, and reads `$t$a;b$t$` as
            // one string (RESEARCH-NOTES, ClickHouse, 2026-10-23).
            SqlDialect::ClickHouse => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: true,
                hash_line_comments: true,
                nested_block_comments: true,
                line_comment_end: LineCommentEnd::LineFeed,
                backtick_backslash_escapes: true,
            },
            SqlDialect::DuckDb => Self {
                backslash_escapes: false,
                escape_string_prefix: true,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: true,
                hash_line_comments: false,
                nested_block_comments: true,
                line_comment_end: LineCommentEnd::Unverified,
                backtick_backslash_escapes: false,
            },
            SqlDialect::Snowflake | SqlDialect::BigQuery => Self {
                backslash_escapes: true,
                escape_string_prefix: false,
                backtick_quotes: true,
                bracket_quotes: false,
                dollar_quotes: matches!(dialect, SqlDialect::Snowflake),
                hash_line_comments: false,
                nested_block_comments: false,
                line_comment_end: LineCommentEnd::Unverified,
                backtick_backslash_escapes: false,
            },
            // `Ansi`, `Oracle`, and any value added later: the inclusive
            // profile.
            _ => Self::permissive(),
        }
    }

    /// The profile that recognizes every known form of quoting.
    #[must_use]
    pub const fn permissive() -> Self {
        Self {
            backslash_escapes: false,
            escape_string_prefix: true,
            backtick_quotes: true,
            bracket_quotes: true,
            dollar_quotes: true,
            hash_line_comments: false,
            nested_block_comments: true,
            line_comment_end: LineCommentEnd::Unverified,
            backtick_backslash_escapes: false,
        }
    }
}

impl Default for SplitProfile {
    fn default() -> Self {
        Self::permissive()
    }
}

/// Splits a batch into statements.
///
/// Empty fragments — a run of `;;`, a text made of comments only — are
/// dropped: they have nothing to run. A comment that **precedes** a statement
/// stays attached to it; it is indeed its text.
///
/// In [`SqlDialect::MySql`], the `BEGIN … END` body of a stored program stays
/// in its `CREATE` statement, and a `DELIMITER` line is honored as the `mysql`
/// client does: it changes what ends a statement and belongs to no fragment.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::split;
///
/// let batch = "SELECT ';' ; DELETE FROM t; -- ; not a separator";
/// let fragments = split(batch, SqlDialect::Postgres);
/// assert_eq!(fragments.len(), 2);
/// assert_eq!(fragments[0].text, "SELECT ';'");
/// assert_eq!(fragments[1].text, "DELETE FROM t");
/// ```
#[must_use]
pub fn split(sql: &str, dialect: SqlDialect) -> Vec<Fragment<'_>> {
    if dialect == SqlDialect::MySql {
        mysql::split(sql)
    } else {
        split_with(sql, SplitProfile::for_dialect(dialect))
    }
}

/// Returns the complete statement under the cursor, or nothing in a separator.
///
/// The cursor is a UTF-8 byte index. A position in the middle of a character,
/// in a separator or in whitespace outside a statement selects nothing. The
/// fragment is validated before being returned: an incomplete text cannot be
/// run as the "current statement". A MySQL stored program, which the parser
/// cannot read, is returned when its body is closed and refused otherwise.
pub fn current_statement(
    sql: &str,
    dialect: SqlDialect,
    cursor_byte: usize,
) -> Result<Option<Fragment<'_>>, QueryError> {
    if cursor_byte > sql.len() || !sql.is_char_boundary(cursor_byte) {
        return Err(QueryError::Selection {
            cursor: cursor_byte,
            message: "cursor is not on a UTF-8 boundary".into(),
        });
    }
    let fragments = if dialect == SqlDialect::Sqlite {
        sqlite_statements(sql, cursor_byte)?
    } else {
        split(sql, dialect)
    };
    let fragment = fragments
        .iter()
        .find(|fragment| {
            (fragment.span.start <= cursor_byte && cursor_byte < fragment.span.end)
                || (fragment.span.end == sql.len() && cursor_byte == sql.len())
        })
        .filter(|fragment| {
            cursor_byte != fragment.span.start || !starts_with_comment(fragment.text, dialect)
        })
        .or_else(|| {
            fragments.iter().rev().find(|fragment| {
                fragment.terminated
                    && terminator_after(sql, fragment.span.end).is_some_and(|separator| {
                        cursor_byte == separator || cursor_byte == separator + 1
                    })
            })
        });
    let Some(fragment) = fragment else {
        return Ok(None);
    };
    if dialect == SqlDialect::Sqlite && is_sqlite_trigger(fragment.text) {
        if sqlite_trigger_is_ambiguous(fragment.text) {
            return Err(QueryError::Selection {
                cursor: cursor_byte,
                message: "SQLite trigger contains an ambiguous unquoted keyword".into(),
            });
        }
        if !words(fragment.text, SqlDialect::Sqlite)
            .last()
            .is_some_and(|word| word.text.eq_ignore_ascii_case("end"))
        {
            return Err(QueryError::Selection {
                cursor: cursor_byte,
                message: "SQLite trigger body is incomplete".into(),
            });
        }
    } else if let Some(complete) = mysql_program(fragment, dialect) {
        if !complete {
            return Err(QueryError::Selection {
                cursor: cursor_byte,
                message: "MySQL stored program body is not closed where the statement ends".into(),
            });
        }
    } else {
        validate(fragment.text, dialect).map_err(|error| match error {
            QueryError::Syntax { message, span } => QueryError::Syntax {
                message,
                span: fragment.span.start.saturating_add(span.start)
                    ..fragment.span.start.saturating_add(span.end),
            },
            other => other,
        })?;
    }
    Ok(Some(fragment.clone()))
}

/// Is the fragment a MySQL stored program, and is its body complete?
///
/// `None` sends the fragment to the parser's validation. So does an unreadable
/// comment: [`validate`] refuses it, and skipping the parser must not skip that.
fn mysql_program(fragment: &Fragment<'_>, dialect: SqlDialect) -> Option<bool> {
    if dialect != SqlDialect::MySql || fragment.unreadable_comment {
        return None;
    }
    mysql::program_is_complete(fragment.text)
}

fn starts_with_comment(sql: &str, dialect: SqlDialect) -> bool {
    sql.starts_with("--")
        || sql.starts_with("/*")
        || (dialect == SqlDialect::MySql && sql.starts_with('#'))
}

fn terminator_after(sql: &str, from: usize) -> Option<usize> {
    let tail = sql.get(from..)?;
    let whitespace = tail.len() - tail.trim_start().len();
    let position = from.checked_add(whitespace)?;
    (sql.as_bytes().get(position) == Some(&b';')).then_some(position)
}

fn is_sqlite_trigger(sql: &str) -> bool {
    let words = words(sql, SqlDialect::Sqlite);
    let Some(first) = words.first() else {
        return false;
    };
    if !first.text.eq_ignore_ascii_case("create") {
        return false;
    }
    let second = words.get(1).map(|word| word.text);
    let trigger = match second {
        Some(word)
            if word.eq_ignore_ascii_case("temp") || word.eq_ignore_ascii_case("temporary") =>
        {
            words.get(2).map(|word| word.text)
        }
        word => word,
    };
    trigger.is_some_and(|word| word.eq_ignore_ascii_case("trigger"))
}

fn sqlite_trigger_is_ambiguous(sql: &str) -> bool {
    let words = words(sql, SqlDialect::Sqlite);
    let Some(trigger) = words
        .iter()
        .position(|word| word.text.eq_ignore_ascii_case("trigger"))
    else {
        return false;
    };
    if words
        .get(trigger + 1)
        .is_some_and(|word| word.text.eq_ignore_ascii_case("begin"))
    {
        return true;
    }
    words
        .iter()
        .filter(|word| word.text.eq_ignore_ascii_case("case"))
        .any(|word| {
            sql.get(word.span.end..)
                .is_some_and(|tail| tail.trim_start().starts_with('='))
        })
}

fn sqlite_statements(sql: &str, cursor: usize) -> Result<Vec<Fragment<'_>>, QueryError> {
    let mut fragments = Vec::new();
    let mut start = 0usize;
    let mut has_comment = false;
    let mut has_code = false;
    let mut prefix = 0u8;
    let mut trigger = false;
    let mut seen_on = false;
    let mut relation_seen = false;
    let mut body = false;
    let mut body_statement_start = false;
    let mut ended = false;
    scan(
        sql,
        SplitProfile::for_dialect(SqlDialect::Sqlite),
        &mut |token, span| match token {
            // The SQLite profile ends comments at `\n`, as `tokenize.c` does:
            // an unreadable comment cannot occur here.
            Tok::Comment | Tok::UnreadableComment => has_comment = true,
            Tok::Word => {
                let word = sql.get(span.clone()).unwrap_or_default();
                let mut began_body = false;
                if !has_code {
                    prefix = u8::from(word.eq_ignore_ascii_case("create"));
                } else if prefix == 1 {
                    if word.eq_ignore_ascii_case("temp") || word.eq_ignore_ascii_case("temporary") {
                        prefix = 2;
                    } else if word.eq_ignore_ascii_case("trigger") {
                        prefix = 3;
                        trigger = true;
                    } else {
                        prefix = 0;
                    }
                } else if prefix == 2 {
                    if word.eq_ignore_ascii_case("trigger") {
                        prefix = 3;
                        trigger = true;
                    } else {
                        prefix = 0;
                    }
                }
                if trigger && word.eq_ignore_ascii_case("on") {
                    seen_on = true;
                } else if seen_on && !relation_seen {
                    relation_seen = true;
                } else if trigger && relation_seen && word.eq_ignore_ascii_case("begin") {
                    body = true;
                    body_statement_start = true;
                    began_body = true;
                } else if body
                    && !ended
                    && word.eq_ignore_ascii_case("end")
                    && body_statement_start
                    && end_closes_trigger(sql, span.end)
                {
                    ended = true;
                }
                if body && !ended && !began_body {
                    body_statement_start = false;
                }
                has_code = true;
            }
            Tok::Semicolon if !(body && !ended) => {
                if has_code {
                    let flags = Flags::new(has_comment, true, false);
                    push_fragment(sql, start..span.start, flags, &mut fragments);
                }
                start = span.end;
                has_comment = false;
                has_code = false;
                prefix = 0;
                trigger = false;
                seen_on = false;
                relation_seen = false;
                body = false;
                body_statement_start = false;
                ended = false;
            }
            Tok::Quoted | Tok::SingleQuoted | Tok::Symbol => {
                has_code = true;
                if body && !ended {
                    body_statement_start = false;
                }
            }
            Tok::Semicolon => body_statement_start = true,
        },
    );
    if trigger && !body {
        return Err(QueryError::Selection {
            cursor,
            message: "SQLite trigger body cannot be established".into(),
        });
    }
    if has_code {
        let flags = Flags::new(has_comment, false, false);
        push_fragment(sql, start..sql.len(), flags, &mut fragments);
    }
    Ok(fragments)
}

fn end_closes_trigger(sql: &str, from: usize) -> bool {
    let mut tail = sql.get(from..).unwrap_or_default().trim_start();
    loop {
        if let Some(rest) = tail.strip_prefix("--") {
            tail = rest
                .split_once('\n')
                .map_or("", |(_, after)| after)
                .trim_start();
        } else if let Some(rest) = tail.strip_prefix("/*") {
            let Some((_, after)) = rest.split_once("*/") else {
                return false;
            };
            tail = after.trim_start();
        } else {
            break;
        }
    }
    tail.is_empty() || tail.starts_with(';')
}

/// Splits a batch with an explicit lexical profile.
///
/// A profile carries lexical facts only: the MySQL stored program bodies and
/// `DELIMITER` lines are honored by [`split`] with [`SqlDialect::MySql`], not
/// here.
#[must_use]
pub fn split_with(sql: &str, profile: SplitProfile) -> Vec<Fragment<'_>> {
    let mut fragments = Vec::new();
    let mut start = 0usize;
    let mut has_comment = false;
    let mut has_code = false;
    let mut unreadable = false;

    scan(sql, profile, &mut |token, span| match token {
        Tok::Comment => has_comment = true,
        // Never dropped as « comments only »: what it hides may run.
        Tok::UnreadableComment => {
            has_comment = true;
            has_code = true;
            unreadable = true;
        }
        Tok::Semicolon => {
            if has_code {
                let flags = Flags::new(has_comment, true, unreadable);
                push_fragment(sql, start..span.start, flags, &mut fragments);
            }
            start = span.end;
            has_comment = false;
            has_code = false;
            unreadable = false;
        }
        Tok::Word | Tok::Quoted | Tok::SingleQuoted | Tok::Symbol => has_code = true,
    });

    if has_code {
        let flags = Flags::new(has_comment, false, unreadable);
        push_fragment(sql, start..sql.len(), flags, &mut fragments);
    }
    fragments
}

/// Does the text contain at least one comment?
///
/// A comment that belongs to no statement — a batch made of a single
/// `-- note` — appears in no [`Fragment`]: this function is the only way to
/// know it is there.
#[must_use]
pub fn contains_comment(sql: &str, dialect: SqlDialect) -> bool {
    let mut seen = false;
    scan(
        sql,
        SplitProfile::for_dialect(dialect),
        &mut |token, _span| {
            if matches!(token, Tok::Comment | Tok::UnreadableComment) {
                seen = true;
            }
        },
    );
    seen
}

/// The bare words of the text, in order.
///
/// Everything quoted or commented is absent from the result.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::split::words;
///
/// let tokens = words("SELECT x /* DROP */ FROM 'DELETE'", SqlDialect::Ansi);
/// let texts: Vec<&str> = tokens.iter().map(|m| m.text).collect();
/// assert_eq!(texts, ["SELECT", "x", "FROM"]);
/// ```
#[must_use]
pub fn words(sql: &str, dialect: SqlDialect) -> Vec<Word<'_>> {
    words_with(sql, SplitProfile::for_dialect(dialect))
}

/// The bare words of the text, with an explicit lexical profile.
#[must_use]
pub fn words_with(sql: &str, profile: SplitProfile) -> Vec<Word<'_>> {
    let mut out: Vec<Word<'_>> = Vec::new();
    scan(sql, profile, &mut |token, span| {
        if token != Tok::Word {
            return;
        }
        let Some(text) = sql.get(span.clone()) else {
            return;
        };
        out.push(Word {
            text,
            span,
            call: false,
        });
    });

    let bytes = sql.as_bytes();
    for word in &mut out {
        word.call = next_visible_byte(bytes, word.span.end) == Some(b'(');
    }
    out
}

/// What the scanner tells a [`Token`] is. It does not understand SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenKind {
    /// A bare word (keyword, unquoted identifier).
    Word,
    /// A quoted identifier, or a double-quoted string: which of the two
    /// depends on the server's mode, not on the text.
    Quoted,
    /// A single-quoted string literal.
    String,
    /// One byte of punctuation, operator or number.
    Symbol,
    /// The statement separator.
    Separator,
}

/// A lexical element of the text, comments left out.
///
/// For a reader that must see more than [`words`]: the `@@`, the `.` and the
/// `=` around a word, or a quoted identifier, decide whether a `SET` assigns
/// it or reads it.
///
/// `Debug` leaves the text out: a token is the user's text, and `SET PASSWORD =
/// 'x'` holds a secret in one (I-03).
#[derive(Clone, PartialEq, Eq)]
pub struct Token<'a> {
    /// What the token is.
    pub kind: TokenKind,
    /// The token, as written, quotes included.
    pub text: &'a str,
    /// Byte bounds in the original text.
    pub span: Range<usize>,
}

impl std::fmt::Debug for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("kind", &self.kind)
            .field("span", &self.span)
            .finish_non_exhaustive()
    }
}

/// The elements of the text, in order, as the scanner reads them for
/// [`words`]: one reading, so the two cannot disagree on where a string ends.
///
/// Comments are absent, whatever their form — including a MySQL executable
/// comment, which [`runnable_text`](crate::runnable_text()) turns into code
/// first when the caller wants it read.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::split::{TokenKind, tokens};
///
/// let read = tokens("SET @@`x` = 'a' /* b */", SqlDialect::MySql);
/// let kinds: Vec<TokenKind> = read.iter().map(|t| t.kind).collect();
/// assert_eq!(kinds, [
///     TokenKind::Word, TokenKind::Symbol, TokenKind::Symbol,
///     TokenKind::Quoted, TokenKind::Symbol, TokenKind::String,
/// ]);
/// ```
#[must_use]
pub fn tokens(sql: &str, dialect: SqlDialect) -> Vec<Token<'_>> {
    let mut out: Vec<Token<'_>> = Vec::new();
    scan(
        sql,
        SplitProfile::for_dialect(dialect),
        &mut |token, span| {
            let kind = match token {
                Tok::Word => TokenKind::Word,
                Tok::Quoted => TokenKind::Quoted,
                Tok::SingleQuoted => TokenKind::String,
                Tok::Symbol => TokenKind::Symbol,
                Tok::Semicolon => TokenKind::Separator,
                Tok::Comment | Tok::UnreadableComment => return,
            };
            if let Some(text) = sql.get(span.clone()) {
                out.push(Token { kind, text, span });
            }
        },
    );
    out
}

/// The text as a MySQL or MariaDB server runs it: the body of an executable
/// comment is code.
///
/// `/*!50000 USE x */` and, on MariaDB, `/*M!100000 USE x */` are comments to
/// every other server and statements to these two. The scanner reads them as
/// comments, since it cannot know the server's version, so `split` and `words`
/// never see what hides there: a `USE`, a `COMMIT`, a `DELETE`, or the `;` that
/// ends the statement before it. A reader built on them is told "nothing
/// here", and the check it implements is skipped (issue #223).
///
/// This view blanks, with spaces, the opening marker, the run of digits that
/// gates the version, and the closing `*/` of each such comment, and leaves
/// every other byte alone. The length is unchanged, so a byte offset in the view
/// is the same offset in the original. Only ASCII bytes are replaced, so the
/// result is valid UTF-8 whatever the body holds.
///
/// It reads **more** code than a given server version would run, never less: a
/// body gated on a version above the server's is a comment to it, and reading
/// it as code costs a confirmation at worst. Comments the scanner does not
/// tokenize as such — inside a string, behind `--` — are not touched, and the
/// profile decides what a string is.
///
/// The view is for *reading*. What is sent to the server stays the user's text.
pub(crate) fn executable_sql(sql: &str, profile: SplitProfile) -> Cow<'_, str> {
    let mut unmasked: Option<Vec<u8>> = None;
    scan(sql, profile, &mut |token, span| {
        let Some(comment) = sql.get(span.clone()) else {
            return;
        };
        if token != Tok::Comment {
            return;
        }
        let prefix_len = if comment.starts_with("/*!") {
            3
        } else if comment.starts_with("/*M!") {
            4
        } else {
            return;
        };
        let gate_len = comment.as_bytes().get(prefix_len..).map_or(0, |body| {
            body.iter().take_while(|b| b.is_ascii_digit()).count()
        });
        let bytes = unmasked.get_or_insert_with(|| sql.as_bytes().to_vec());
        if let Some(opening) = bytes.get_mut(span.start..span.start + prefix_len + gate_len) {
            opening.fill(b' ');
        }
        if comment.ends_with("*/")
            && let Some(closing) = bytes.get_mut(span.end.saturating_sub(2)..span.end)
        {
            closing.fill(b' ');
        }
    });
    match unmasked {
        Some(bytes) => Cow::Owned(String::from_utf8_lossy(&bytes).into_owned()),
        None => Cow::Borrowed(sql),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The scanner
// ─────────────────────────────────────────────────────────────────────────────

/// What the scanner can tell apart. It does not understand SQL: it only knows
/// where it is allowed to look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    /// A bare word (keyword, unquoted identifier).
    Word,
    /// A string, a quoted identifier, a `$$ … $$` body.
    Quoted,
    /// A single-quoted string literal.
    SingleQuoted,
    /// A comment, whatever its form.
    Comment,
    /// A line comment whose end depends on a lexer nobody checked
    /// ([`LineCommentEnd::Unverified`]): it may hold a statement, so it counts
    /// as code and makes its fragment unreadable.
    UnreadableComment,
    /// The statement separator: `;`, or what a MySQL `DELIMITER` line chose.
    Semicolon,
    /// Everything else: punctuation, operators, numbers.
    Symbol,
}

/// Goes through the text once and reports each significant element.
///
/// A single scanner serves splitting, comment detection and word reading:
/// three copies of this loop would end up diverging, and it would be the one
/// holding the safety net that diverged.
pub(crate) fn scan(sql: &str, profile: SplitProfile, on: &mut dyn FnMut(Tok, Range<usize>)) {
    for (token, span) in Scanner::new(sql, profile) {
        on(token, span);
    }
}

/// The scanner, as an iterator a caller can steer.
///
/// The MySQL batch needs what a callback cannot give: looking one word ahead
/// (`END IF` closes what `END` alone does not), and changing what ends a
/// statement in the middle of the text (`DELIMITER`). Both are done by the
/// [`mysql`] module on this same scanner, not on a copy of its loop.
#[derive(Clone)]
struct Scanner<'a> {
    sql: &'a str,
    /// Byte index of the next element to read.
    at: usize,
    profile: SplitProfile,
    /// What ends a statement: `;`, unless a MySQL `DELIMITER` line chose
    /// otherwise. Never empty: an empty terminator would match forever.
    terminator: &'a [u8],
}

impl<'a> Scanner<'a> {
    const fn new(sql: &'a str, profile: SplitProfile) -> Self {
        Self {
            sql,
            at: 0,
            profile,
            terminator: b";",
        }
    }

    fn at_terminator(&self) -> bool {
        !self.terminator.is_empty()
            && self
                .sql
                .as_bytes()
                .get(self.at..)
                .is_some_and(|rest| rest.starts_with(self.terminator))
    }
}

impl Iterator for Scanner<'_> {
    type Item = (Tok, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let b = self.sql.as_bytes();
        while let Some(&c) = b.get(self.at) {
            let start = self.at;
            if c.is_ascii_whitespace() {
                self.at += 1;
                continue;
            }
            // The terminator is looked for before anything else, as the `mysql`
            // client does: with `DELIMITER //`, `//` ends the statement.
            let (token, end) = if self.at_terminator() {
                (Tok::Semicolon, start + self.terminator.len())
            } else {
                lex(b, start, c, self.profile)
            };
            self.at = end;
            return Some((token, start..end));
        }
        None
    }
}

/// The element that starts at `i`, whose first byte is `c`, and where it ends.
fn lex(b: &[u8], i: usize, c: u8, profile: SplitProfile) -> (Tok, usize) {
    match c {
        b'-' if b.get(i + 1) == Some(&b'-') => {
            let (end, token) = skip_line_comment(b, i + 2, profile.line_comment_end);
            (token, end)
        }
        b'#' if profile.hash_line_comments => {
            let (end, token) = skip_line_comment(b, i + 1, profile.line_comment_end);
            (token, end)
        }
        b'/' if b.get(i + 1) == Some(&b'*') => (
            Tok::Comment,
            skip_block_comment(b, i, profile.nested_block_comments),
        ),
        b'\'' => {
            let escapes = profile.backslash_escapes
                || (profile.escape_string_prefix && has_escape_prefix(b, i));
            (Tok::SingleQuoted, skip_quoted(b, i, b'\'', escapes))
        }
        b'"' => (
            Tok::Quoted,
            skip_quoted(b, i, b'"', profile.backslash_escapes),
        ),
        b'`' if profile.backtick_quotes => (
            Tok::Quoted,
            skip_quoted(b, i, b'`', profile.backtick_backslash_escapes),
        ),
        b'[' if profile.bracket_quotes => (Tok::Quoted, skip_bracket(b, i)),
        // A lone `$` is a `$1` parameter placeholder, or an operator.
        b'$' if profile.dollar_quotes => {
            skip_dollar_quoted(b, i).map_or((Tok::Symbol, i + 1), |end| (Tok::Quoted, end))
        }
        _ if is_word_start(c) => (Tok::Word, skip_word(b, i)),
        // A `;` the terminator did not match is only punctuation: under
        // `DELIMITER $$`, it separates the statements of a body.
        _ => (Tok::Symbol, i + 1),
    }
}

/// First byte of a bare word. Bytes ≥ `0x80` belong to it: an accented
/// identifier is an identifier.
const fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// `$` is deliberately **excluded**: `AS$$body$$` must open a dollar body,
/// not produce an `AS$$` word that would let the body be cut on its
/// semicolons.
const fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn skip_word(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while let Some(&c) = b.get(i) {
        if !is_word_byte(c) {
            break;
        }
        i += 1;
    }
    i
}

/// End of a line comment starting at `i`, and how it reads.
///
/// The end must be the server's: a comment that runs past it hides a statement
/// the server executes, from the splitter and from the keyword sweep alike.
fn skip_line_comment(b: &[u8], mut i: usize, end: LineCommentEnd) -> (usize, Tok) {
    let mut lone_carriage_return = false;
    let mut unreadable = false;
    while let Some(&c) = b.get(i) {
        match (c, end) {
            (b'\n', _) | (b'\r', LineCommentEnd::LineFeedOrCarriageReturn) => break,
            (b'\r', LineCommentEnd::Unverified) => lone_carriage_return = true,
            // Only text after the `\r` makes the two readings differ; trailing
            // blanks read the same either way.
            (_, LineCommentEnd::Unverified) if lone_carriage_return && !c.is_ascii_whitespace() => {
                unreadable = true;
            }
            _ => {}
        }
        i += 1;
    }
    let token = if unreadable {
        Tok::UnreadableComment
    } else {
        Tok::Comment
    };
    (i, token)
}

/// An unclosed block comment swallows the end of the text: it is what a real
/// server does, and it leads to an unreadable fragment, hence `Unknown`.
fn skip_block_comment(b: &[u8], start: usize, nested: bool) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    while let Some(&c) = b.get(i) {
        if nested && c == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
            continue;
        }
        if c == b'*' && b.get(i + 1) == Some(&b'/') {
            i += 2;
            depth -= 1;
            if depth == 0 {
                return i;
            }
            continue;
        }
        i += 1;
    }
    b.len()
}

/// Goes through a quoted region. A doubled delimiter (`''`, `""`) is always
/// recognized; escaping with `\` depends on the dialect.
fn skip_quoted(b: &[u8], start: usize, quote: u8, backslash: bool) -> usize {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        if backslash && c == b'\\' {
            i += 2;
            continue;
        }
        if c == quote {
            if b.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// SQL Server `[identifier]`: the closing bracket is doubled to be literal,
/// the opening one is not.
fn skip_bracket(b: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        if c == b']' {
            if b.get(i + 1) == Some(&b']') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// End of the opening `$tag$` delimiter, or `None` if this `$` does not open
/// one.
///
/// `$1` is not a delimiter: a tag does not start with a digit. That is what
/// keeps a function body apart from a PostgreSQL parameter placeholder.
fn dollar_tag_end(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        if c == b'$' {
            return Some(i + 1);
        }
        let acceptable = if i == start + 1 {
            c.is_ascii_alphabetic() || c == b'_'
        } else {
            c.is_ascii_alphanumeric() || c == b'_'
        };
        if !acceptable {
            return None;
        }
        i += 1;
    }
    None
}

fn skip_dollar_quoted(b: &[u8], start: usize) -> Option<usize> {
    let tag_end = dollar_tag_end(b, start)?;
    let tag = b.get(start..tag_end)?;
    let mut i = tag_end;
    while i + tag.len() <= b.len() {
        if b.get(i..i + tag.len()) == Some(tag) {
            return Some(i + tag.len());
        }
        i += 1;
    }
    // Unclosed body: the end is swallowed.
    Some(b.len())
}

/// Is the `'` at `start` preceded by an isolated `E` prefix?
fn has_escape_prefix(b: &[u8], start: usize) -> bool {
    let Some(&previous) = start.checked_sub(1).and_then(|k| b.get(k)) else {
        return false;
    };
    if !matches!(previous, b'e' | b'E') {
        return false;
    }
    match start.checked_sub(2).and_then(|k| b.get(k)) {
        Some(&before) => !is_word_byte(before),
        None => true,
    }
}

fn next_visible_byte(b: &[u8], mut i: usize) -> Option<u8> {
    while let Some(&c) = b.get(i) {
        if !c.is_ascii_whitespace() {
            return Some(c);
        }
        i += 1;
    }
    None
}

/// What the scanner learned about a fragment, beside its bounds.
#[derive(Clone, Copy)]
struct Flags {
    has_comment: bool,
    terminated: bool,
    unreadable_comment: bool,
}

impl Flags {
    const fn new(has_comment: bool, terminated: bool, unreadable_comment: bool) -> Self {
        Self {
            has_comment,
            terminated,
            unreadable_comment,
        }
    }
}

fn push_fragment<'a>(sql: &'a str, range: Range<usize>, flags: Flags, out: &mut Vec<Fragment<'a>>) {
    let start = range.start;
    let Some(raw) = sql.get(range) else {
        return;
    };
    let text = raw.trim();
    if text.is_empty() {
        return;
    }
    let lead = raw.len() - raw.trim_start().len();
    let from = start + lead;
    out.push(Fragment {
        text,
        span: from..from + text.len(),
        has_comment: flags.has_comment,
        terminated: flags.terminated,
        unreadable_comment: flags.unreadable_comment,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str, dialect: SqlDialect) -> Vec<String> {
        split(sql, dialect)
            .into_iter()
            .map(|f| f.text.to_owned())
            .collect()
    }

    fn executable(sql: &str) -> String {
        executable_sql(sql, SplitProfile::for_dialect(SqlDialect::MySql)).into_owned()
    }

    #[test]
    fn the_executable_view_blanks_the_markers_and_the_version_gate_only() {
        assert_eq!(executable("/*!50000 USE x */"), "         USE x   ");
        assert_eq!(executable("/*M!100000 USE x */"), "           USE x   ");
        assert_eq!(executable("/*! USE x */"), "    USE x   ");
        assert_eq!(
            executable("SELECT 1 /*!50000 ; DROP t */"),
            "SELECT 1          ; DROP t   "
        );
        // Only the digits that follow the marker are the gate.
        assert_eq!(
            executable("/*!50000 SELECT 5000 */"),
            "         SELECT 5000   "
        );
    }

    #[test]
    fn the_executable_view_keeps_every_offset_and_every_character() {
        let sql = "SELECT 'é' /*!50000 , 'ü' */ FROM `ñ` -- ça";
        let view = executable(sql);
        assert_eq!(view.len(), sql.len());
        for (index, (original, seen)) in sql.bytes().zip(view.bytes()).enumerate() {
            assert!(
                original == seen || (seen == b' ' && original.is_ascii()),
                "byte {index} changed from {original:#x} to {seen:#x}"
            );
        }
    }

    #[test]
    fn the_executable_view_leaves_every_other_comment_alone() {
        for sql in [
            "SELECT 1 /* USE x */",
            "SELECT /*+ USE x */ 1",
            "SELECT '/*!50000 USE x */'",
            "SELECT `/*!50000 USE x */`",
            "-- /*!50000 USE x */\nSELECT 1",
            "# /*!50000 USE x */\nSELECT 1",
        ] {
            let view = executable_sql(sql, SplitProfile::for_dialect(SqlDialect::MySql));
            assert!(matches!(view, Cow::Borrowed(_)), "{sql}");
        }
    }

    #[test]
    fn an_unclosed_executable_comment_is_read_to_the_end() {
        // The server fails the statement; reading the body as code costs a
        // confirmation at worst.
        assert_eq!(executable("/*!50000 USE x"), "         USE x");
    }

    #[test]
    fn current_statement_syntax_errors_keep_document_offsets() {
        let sql = "SELECT 1; SELECT";
        let error = current_statement(sql, SqlDialect::Postgres, sql.len())
            .expect_err("incomplete second statement");
        let QueryError::Syntax { span, .. } = error else {
            panic!("syntax error");
        };
        assert_eq!(span.start, sql.rfind("SELECT").expect("second SELECT"));
        assert_eq!(span.end, sql.len());
    }

    #[test]
    fn current_statement_keeps_the_sqlite_trigger_body() {
        let sql = "CREATE TRIGGER t AFTER INSERT ON x BEGIN INSERT INTO log VALUES(CASE WHEN NEW.id > 0 THEN 1 ELSE 0 END); UPDATE x SET seen = 1; END; SELECT 2";
        let cursor = sql.find("UPDATE x").expect("trigger body");
        let fragment = current_statement(sql, SqlDialect::Sqlite, cursor)
            .expect("valid trigger")
            .expect("trigger selection");
        assert!(fragment.text.starts_with("CREATE TRIGGER"));
        assert!(fragment.text.contains("UPDATE x SET seen"));
        assert!(!fragment.text.contains("SELECT 2"));
    }

    #[test]
    fn current_statement_refuses_ambiguous_positions_and_texts() {
        let sql = "SELECT 'é';  SELECT 2";
        assert!(current_statement(sql, SqlDialect::Postgres, 9).is_err());
        assert_eq!(
            current_statement(sql, SqlDialect::Postgres, 13).expect("second space"),
            None
        );
        assert!(current_statement("SELECT 'unfinished", SqlDialect::Postgres, 4).is_err());
    }

    #[test]
    fn current_statement_ties_the_separator_to_the_previous_statement() {
        let first = "SELECT 1";
        let trailing = "SELECT 1;";
        assert_eq!(
            current_statement(trailing, SqlDialect::Postgres, trailing.len())
                .expect("trailing separator")
                .expect("statement")
                .text,
            first
        );
        let adjacent = "SELECT 1;SELECT 2";
        assert_eq!(
            current_statement(adjacent, SqlDialect::Postgres, first.len())
                .expect("separator")
                .expect("previous")
                .text,
            first
        );
        assert_eq!(
            current_statement(adjacent, SqlDialect::Postgres, first.len() + 1)
                .expect("next statement")
                .expect("next")
                .text,
            "SELECT 2"
        );
        assert_eq!(
            current_statement("SELECT 1;  SELECT 2", SqlDialect::Postgres, 9)
                .expect("first space")
                .expect("previous")
                .text,
            first
        );
        assert!(
            current_statement("SELECT 1;  SELECT 2", SqlDialect::Postgres, 10)
                .expect("second space")
                .is_none()
        );
        let dollar = "SELECT $$;$$;";
        assert_eq!(
            current_statement(dollar, SqlDialect::Postgres, dollar.len())
                .expect("dollar quote")
                .expect("statement")
                .text,
            "SELECT $$;$$"
        );
    }

    #[test]
    fn current_statement_does_not_jump_past_a_leading_comment() {
        for sql in [
            "SELECT 1; -- comment\nDELETE FROM t;",
            "SELECT 1;-- comment\nDELETE FROM t;",
        ] {
            let cursor = sql.find(';').expect("separator") + 1;
            assert_eq!(
                current_statement(sql, SqlDialect::Postgres, cursor)
                    .expect("selection")
                    .expect("previous statement")
                    .text,
                "SELECT 1"
            );
        }
    }

    #[test]
    fn current_statement_refuses_ambiguous_words_in_a_sqlite_trigger() {
        for sql in [
            "CREATE TRIGGER BEGIN AFTER INSERT ON x BEGIN UPDATE x SET id = 1; END;",
            "CREATE TRIGGER t AFTER INSERT ON x BEGIN UPDATE x SET CASE = 1; END;",
        ] {
            assert!(
                current_statement(sql, SqlDialect::Sqlite, 8).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn current_statement_never_isolates_a_sqlite_trigger_body() {
        let trigger = "CREATE TEMP TRIGGER t AFTER INSERT ON source BEGIN SELECT end FROM source; UPDATE other SET id = CASE WHEN NEW.id > 0 THEN 999 ELSE 1 END; END;";
        for needle in ["SELECT end", "UPDATE other", "CASE WHEN", "END;"] {
            let cursor = trigger.find(needle).expect("body token");
            let result = current_statement(trigger, SqlDialect::Sqlite, cursor);
            assert!(
                result.is_err()
                    || result
                        .expect("selection")
                        .is_some_and(|fragment| fragment.text.starts_with("CREATE TEMP TRIGGER")),
                "{needle}"
            );
        }
    }

    #[test]
    fn create_table_with_reserved_identifiers_is_not_a_trigger() {
        let sql = "CREATE TABLE things (trigger text, begin integer, end integer); UPDATE things SET end = 1;";
        let cursor = sql.find("UPDATE").expect("update");
        assert_eq!(
            current_statement(sql, SqlDialect::Sqlite, cursor)
                .expect("table statements")
                .expect("update")
                .text,
            "UPDATE things SET end = 1"
        );
    }

    #[test]
    fn a_simple_batch_is_split() {
        assert_eq!(
            texts("SELECT 1; SELECT 2", SqlDialect::Ansi),
            ["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn the_last_semicolon_creates_no_empty_fragment() {
        assert_eq!(texts("SELECT 1;", SqlDialect::Ansi), ["SELECT 1"]);
        assert_eq!(texts("SELECT 1;;;", SqlDialect::Ansi), ["SELECT 1"]);
        assert_eq!(texts("  ;  ", SqlDialect::Ansi), Vec::<String>::new());
    }

    #[test]
    fn a_semicolon_inside_a_string_does_not_separate() {
        assert_eq!(texts("SELECT ';'", SqlDialect::Ansi), ["SELECT ';'"]);
        assert_eq!(
            texts("SELECT 'a;b', 'c;d' FROM t", SqlDialect::Ansi),
            ["SELECT 'a;b', 'c;d' FROM t"]
        );
    }

    #[test]
    fn a_doubled_apostrophe_does_not_close_the_string() {
        assert_eq!(
            texts("SELECT 'it''s naïve ; rest'", SqlDialect::Postgres),
            ["SELECT 'it''s naïve ; rest'"]
        );
    }

    /// The trap the specification cites: a comment that contains a semicolon.
    #[test]
    fn a_semicolon_inside_a_comment_does_not_separate() {
        assert_eq!(
            texts("SELECT 1 -- keep ; here\n, 2", SqlDialect::Ansi),
            ["SELECT 1 -- keep ; here\n, 2"]
        );
        assert_eq!(
            texts("SELECT /* ; */ 1", SqlDialect::Ansi),
            ["SELECT /* ; */ 1"]
        );
    }

    #[test]
    fn a_block_comment_nests_in_postgres() {
        assert_eq!(
            texts("SELECT /* a /* ; */ ; */ 1", SqlDialect::Postgres),
            ["SELECT /* a /* ; */ ; */ 1"]
        );
    }

    #[test]
    fn a_quoted_identifier_does_not_separate() {
        assert_eq!(
            texts(r#"SELECT "a;b" FROM t"#, SqlDialect::Postgres),
            [r#"SELECT "a;b" FROM t"#]
        );
        assert_eq!(
            texts("SELECT `a;b` FROM t", SqlDialect::MySql),
            ["SELECT `a;b` FROM t"]
        );
        assert_eq!(
            texts("SELECT [a;b] FROM t", SqlDialect::SqlServer),
            ["SELECT [a;b] FROM t"]
        );
    }

    /// A `$$ … $$` body almost always contains semicolons: it is the case where
    /// a naive split breaks most visibly.
    #[test]
    fn a_dollar_body_does_not_separate() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN DELETE FROM t; RETURN 1; END $$ LANGUAGE plpgsql; SELECT 2";
        let fragments = texts(sql, SqlDialect::Postgres);
        assert_eq!(fragments.len(), 2, "{fragments:?}");
        assert!(fragments[0].starts_with("CREATE FUNCTION"));
        assert_eq!(fragments[1], "SELECT 2");
    }

    #[test]
    fn a_named_dollar_tag_is_recognized() {
        let sql = "DO $corps$ BEGIN ; END $corps$; SELECT 1";
        assert_eq!(
            texts(sql, SqlDialect::Postgres),
            ["DO $corps$ BEGIN ; END $corps$", "SELECT 1"]
        );
    }

    /// `$1` is a parameter placeholder, not the opening of a body: confusing
    /// them would swallow the whole rest of the batch.
    #[test]
    fn a_parameter_placeholder_does_not_open_a_body() {
        assert_eq!(
            texts("SELECT $1; DELETE FROM t", SqlDialect::Postgres),
            ["SELECT $1", "DELETE FROM t"]
        );
    }

    #[test]
    fn mysql_escapes_with_backslash() {
        // `'a\';'` is a single string in MySQL: the backslash protects the
        // apostrophe.
        assert_eq!(
            texts(r"SELECT 'a\';' , 1", SqlDialect::MySql),
            [r"SELECT 'a\';' , 1"]
        );
    }

    #[test]
    fn postgres_does_not_escape_with_backslash_without_prefix() {
        // Without an `E` prefix, `'a\'` is a complete string: the `;` separates.
        assert_eq!(
            texts(r"SELECT 'a\'; SELECT 2", SqlDialect::Postgres),
            [r"SELECT 'a\'", "SELECT 2"]
        );
    }

    #[test]
    fn postgres_escapes_with_the_e_prefix() {
        assert_eq!(
            texts(r"SELECT E'a\';' , 1", SqlDialect::Postgres),
            [r"SELECT E'a\';' , 1"]
        );
        // `table_e` must not be taken for a prefix: the `e` is glued to a
        // word.
        assert_eq!(
            texts(r"SELECT line'a\'; SELECT 2", SqlDialect::Postgres),
            [r"SELECT line'a\'", "SELECT 2"]
        );
    }

    #[test]
    fn mysql_comments_with_the_hash() {
        assert_eq!(
            texts("SELECT 1 # ; nothing\n, 2", SqlDialect::MySql),
            ["SELECT 1 # ; nothing\n, 2"]
        );
    }

    #[test]
    fn an_unclosed_string_merges_rather_than_cuts() {
        // Direction of the error: a single unreadable fragment, hence `Unknown`.
        let fragments = split("SELECT 'oops ; DELETE FROM t", SqlDialect::Ansi);
        assert_eq!(fragments.len(), 1);
    }

    #[test]
    fn the_bounds_designate_the_exact_text() {
        let sql = "  SELECT 1 ;\n  DELETE FROM t  ";
        for fragment in split(sql, SqlDialect::Ansi) {
            assert_eq!(
                sql.get(fragment.span.clone()),
                Some(fragment.text),
                "wrong bounds for {fragment:?}"
            );
        }
    }

    #[test]
    fn the_comment_flag_follows_the_fragment() {
        let fragments = split("SELECT 1; -- note\nSELECT 2", SqlDialect::Ansi);
        assert_eq!(fragments.len(), 2);
        assert!(!fragments[0].has_comment);
        assert!(fragments[1].has_comment);
    }

    #[test]
    fn termination_is_reported() {
        let fragments = split("SELECT 1; SELECT 2", SqlDialect::Ansi);
        assert!(fragments[0].terminated);
        assert!(!fragments[1].terminated);
    }

    #[test]
    fn a_batch_of_comments_only_gives_no_statement() {
        assert!(split("-- nothing\n/* nothing either */", SqlDialect::Ansi).is_empty());
        assert!(contains_comment(
            "-- nothing\n/* nothing either */",
            SqlDialect::Ansi
        ));
        assert!(!contains_comment(
            "SELECT '-- not a comment'",
            SqlDialect::Ansi
        ));
    }

    #[test]
    fn bare_words_ignore_strings_and_comments() {
        let tokens = words(
            "SELECT x /* DROP */ FROM t WHERE note = 'DELETE FROM u' -- TRUNCATE",
            SqlDialect::Ansi,
        );
        let texts: Vec<&str> = tokens.iter().map(|m| m.text).collect();
        assert_eq!(texts, ["SELECT", "x", "FROM", "t", "WHERE", "note"]);
    }

    #[test]
    fn a_function_call_is_flagged() {
        let tokens = words(
            "SELECT TRUNCATE(1.234, 2), truncate  (x)",
            SqlDialect::MySql,
        );
        let seen_tokens: Vec<(&str, bool)> = tokens.iter().map(|m| (m.text, m.call)).collect();
        assert_eq!(
            seen_tokens,
            [
                ("SELECT", false),
                ("TRUNCATE", true),
                ("truncate", true),
                ("x", false),
            ],
            "{tokens:?}"
        );
    }

    #[test]
    fn word_bounds_designate_the_text() {
        let sql = "SELECT café FROM t";
        for word in words(sql, SqlDialect::Ansi) {
            assert_eq!(sql.get(word.span.clone()), Some(word.text));
        }
    }
}
