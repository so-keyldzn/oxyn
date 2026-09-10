//! Bounded stored SQL reads and lexical spans for SQLite schema declarations.
//!
//! This is not a SQL validator. The engine has already accepted the declaration;
//! spans let metadata retain its original clauses without rewriting expressions.

use oxyn_catalog::{QuoteStyle, quote_identifier};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::{Connection, OptionalExtension as _};

use crate::error::{self, Effect};

pub(crate) const MAX_SCHEMA_SQL_BYTES: usize = 1_048_576;

pub(crate) fn stored_sql(
    connection: &Connection,
    database: &str,
    name: &str,
) -> Result<(String, String)> {
    let database = quote_identifier(database, QuoteStyle::Double);
    let query = format!(
        "SELECT type, CASE WHEN length(CAST(sql AS BLOB)) <= ?2 THEN sql END \
         FROM {database}.sqlite_schema WHERE name = ?1 AND type IN ('table', 'view')"
    );
    let row = connection
        .query_row(&query, (name, MAX_SCHEMA_SQL_BYTES), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(|error| error::engine(error, Effect::ReadOnly))?;
    let Some((kind, sql)) = row else {
        return Err(OxynError::CatalogUnavailable(
            "object has no stored table or view definition".into(),
        ));
    };
    let sql = sql.ok_or_else(|| {
        OxynError::CatalogUnavailable("stored object definition is missing or exceeds 1 MiB".into())
    })?;
    Ok((kind, sql))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Word,
    Quoted(u8),
    Open,
    Close,
    Comma,
    Other,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Token<'a> {
    pub raw: &'a str,
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

impl Token<'_> {
    pub fn keyword(&self, keyword: &str) -> bool {
        self.kind == TokenKind::Word && self.raw.eq_ignore_ascii_case(keyword)
    }

    pub fn identifier(&self) -> Result<String> {
        match self.kind {
            TokenKind::Word => Ok(self.raw.to_owned()),
            TokenKind::Quoted(open) => {
                let inner = self
                    .raw
                    .get(1..self.raw.len().saturating_sub(1))
                    .ok_or_else(invalid_sql)?;
                Ok(match open {
                    b'\'' => inner.replace("''", "'"),
                    b'"' => inner.replace("\"\"", "\""),
                    b'`' => inner.replace("``", "`"),
                    b'[' => inner.to_owned(),
                    _ => return Err(invalid_sql()),
                })
            }
            _ => Err(invalid_sql()),
        }
    }
}

pub(crate) fn invalid_sql() -> OxynError {
    OxynError::CatalogUnavailable(
        "stored SQLite declaration could not be interpreted safely".into(),
    )
}

fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') || !byte.is_ascii()
}

pub(crate) fn tokens<'a>(sql: &'a str, cancel: &CancelToken) -> Result<Vec<Token<'a>>> {
    if sql.len() > MAX_SCHEMA_SQL_BYTES {
        return Err(invalid_sql());
    }
    let bytes = sql.as_bytes();
    let mut at = 0;
    let mut tokens = Vec::new();
    while let Some(&byte) = bytes.get(at) {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if byte == b'-' && bytes.get(at + 1) == Some(&b'-') {
            while bytes.get(at).is_some_and(|b| *b != b'\n') {
                at += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while !(bytes.get(at) == Some(&b'*') && bytes.get(at + 1) == Some(&b'/')) {
                if at >= bytes.len() {
                    return Err(invalid_sql());
                }
                at += 1;
            }
            at += 2;
            continue;
        }
        let start = at;
        let kind = match byte {
            b'\'' | b'"' | b'`' | b'[' => {
                let end = if byte == b'[' { b']' } else { byte };
                at += 1;
                loop {
                    let Some(&current) = bytes.get(at) else {
                        return Err(invalid_sql());
                    };
                    at += 1;
                    if current == end {
                        if byte != b'[' && bytes.get(at) == Some(&end) {
                            at += 1;
                        } else {
                            break;
                        }
                    }
                }
                TokenKind::Quoted(byte)
            }
            b'(' => {
                at += 1;
                TokenKind::Open
            }
            b')' => {
                at += 1;
                TokenKind::Close
            }
            b',' => {
                at += 1;
                TokenKind::Comma
            }
            b if word_byte(b) => {
                while bytes.get(at).is_some_and(|b| word_byte(*b)) {
                    at += 1;
                }
                TokenKind::Word
            }
            _ => {
                at += 1;
                TokenKind::Other
            }
        };
        let raw = sql.get(start..at).ok_or_else(invalid_sql)?;
        tokens.push(Token {
            raw,
            start,
            end: at,
            kind,
        });
    }
    Ok(tokens)
}

/// Splits the first parenthesized list without treating nested commas as separators.
pub(crate) fn list_parts<'a>(tokens: &'a [Token<'a>]) -> Result<Vec<&'a [Token<'a>]>> {
    let open = tokens
        .iter()
        .position(|token| token.kind == TokenKind::Open)
        .ok_or_else(invalid_sql)?;
    let mut start = open + 1;
    let mut depth = 1usize;
    let mut parts = Vec::new();
    for (index, token) in tokens.iter().enumerate().skip(start) {
        match token.kind {
            TokenKind::Open => {
                depth += 1;
                if depth > 512 {
                    return Err(invalid_sql());
                }
            }
            TokenKind::Close => {
                depth = depth.checked_sub(1).ok_or_else(invalid_sql)?;
                if depth == 0 {
                    if start < index {
                        parts.push(tokens.get(start..index).ok_or_else(invalid_sql)?);
                    }
                    return Ok(parts);
                }
            }
            TokenKind::Comma if depth == 1 => {
                if start == index {
                    return Err(invalid_sql());
                }
                parts.push(tokens.get(start..index).ok_or_else(invalid_sql)?);
                start = index + 1;
            }
            _ => {}
        }
    }
    Err(invalid_sql())
}
