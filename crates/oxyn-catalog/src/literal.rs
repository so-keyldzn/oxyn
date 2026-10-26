//! SQL string literals composed from a value, per dialect.
//!
//! The counterpart of [`quote_identifier`](crate::quote_identifier) for
//! **values**: a text Oxyn writes for the user to run — a copied `INSERT`, an
//! `IN (…)` list — carries cells of the user's data, and a cell is as hostile
//! as an object name ([I-10], SECURITY "Input surface"). A value
//! `'); DROP TABLE x; --` must come out as one literal and nothing else.
//!
//! # Why the dialect, and not [`QuoteStyle`](crate::QuoteStyle)
//!
//! Identifier quoting and literal escaping do not split the dialects the same
//! way. Snowflake quotes identifiers like PostgreSQL but reads a backslash in a
//! string as an escape; PostgreSQL itself reads it literally only while
//! `standard_conforming_strings` is on. Keyed on the quote style, one of these
//! would be escaped wrong — and a backslash escaped wrong is the one mistake
//! here that lets a value close its literal.
//!
//! The rules per dialect are sourced and dated in
//! [RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md), "SQL string literals"
//! ([I-12]).
//!
//! # Control characters
//!
//! Tab, line feed and carriage return are ordinary data and stay in the
//! value: inside a literal they cannot end it in any dialect. BigQuery writes
//! them as escapes, since its quoted strings cannot hold a raw line break.
//! NUL and every other control character (`char::is_control`: C0, DEL, C1)
//! are **refused**:
//!
//! * NUL cannot be stored in a PostgreSQL text value at all, and cuts the
//!   statement short in any client that hands it to a C string;
//! * the others have no portable literal form, and the text is headed for the
//!   clipboard: pasted into a terminal client (`psql`, `mysql`), an escape
//!   sequence can end bracketed paste and have what follows read as typed
//!   keys. A copy that fails and says why is better than one that does that.
//!
//! An **identifier** written into copied SQL gets the same refusal through
//! [`check_identifier`], tab and line breaks included: quoting keeps it one
//! identifier, but not off the terminal.
//!
//! [I-10]: ../../../CLAUDE.md
//! [I-12]: ../../../CLAUDE.md

use oxyn_core::SqlDialect;

/// Why a value or a name could not be written into copied SQL.
///
/// Never repeats the text: a value is a cell of the user's data, a name may
/// hold the very sequence refused, and an error message ends up in a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum LiteralError {
    /// The text holds a NUL character.
    #[error("it holds a NUL character, which copied SQL never carries")]
    Nul,
    /// The text holds a control character the copy refuses.
    #[error("it holds the control character U+{code:04X}, which copied SQL never carries")]
    ControlCharacter {
        /// The character's code point.
        code: u32,
    },
}

/// How a dialect reads a single-quoted string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escaping {
    /// Only `'` is special, doubled; a backslash is itself: ANSI, SQLite,
    /// DuckDB, Oracle.
    Standard,
    /// `N'…'`: SQL Server evaluates a plain literal through a code page, a
    /// national one as Unicode.
    National,
    /// PostgreSQL and Redshift. `'…'` while the value has no backslash; with
    /// one, `E'…'` with backslashes doubled, which reads the same whatever
    /// `standard_conforming_strings` says — with the setting off, a plain
    /// `'a\'` would let the backslash swallow the closing quote. For Redshift
    /// this is **unverified**: if it lacks `E'…'`, the text fails to parse,
    /// which is the safe way to be wrong.
    PostgresEscape,
    /// MySQL, Snowflake, ClickHouse: the backslash escapes, so it is doubled;
    /// `'` is doubled too — MySQL under `NO_BACKSLASH_ESCAPES` reads `\'` as a
    /// backslash followed by the end of the literal, `''` in either mode as a
    /// quote.
    Backslash,
    /// BigQuery: the backslash escapes, and `''` is not a quote but two
    /// literals concatenated. `\'` it is, safe there because no mode turns
    /// backslash escapes off; line breaks and tabs become `\n`, `\r`, `\t`.
    BackslashQuote,
}

impl Escaping {
    const fn of(dialect: SqlDialect) -> Self {
        match dialect {
            SqlDialect::SqlServer => Self::National,
            SqlDialect::Postgres | SqlDialect::Redshift => Self::PostgresEscape,
            SqlDialect::MySql | SqlDialect::Snowflake | SqlDialect::ClickHouse => Self::Backslash,
            SqlDialect::BigQuery => Self::BackslashQuote,
            // `SqlDialect` is `#[non_exhaustive]`. The fallback doubles the
            // quote only: for a dialect that also reads backslashes, a value
            // ending in one then fails to parse — loudly, never silently.
            _ => Self::Standard,
        }
    }
}

/// Appends `value` to `out` as one single-quoted SQL string literal of
/// `dialect`.
///
/// Appends rather than returns so that a caller writing thousands of values —
/// a copied `INSERT` — reuses one buffer instead of allocating per value.
///
/// # Errors
///
/// [`LiteralError`] when `value` holds NUL or a control character other than
/// tab, line feed or carriage return (see the module documentation). `out` is
/// then left exactly as it was: nothing half-written reaches the caller.
pub fn push_string_literal(
    out: &mut String,
    value: &str,
    dialect: SqlDialect,
) -> Result<(), LiteralError> {
    refuse_controls(value, |c| matches!(c, '\t' | '\n' | '\r'))?;
    let escaping = Escaping::of(dialect);
    let doubles_backslash = match escaping {
        Escaping::Backslash | Escaping::BackslashQuote => true,
        Escaping::PostgresEscape => value.contains('\\'),
        Escaping::Standard | Escaping::National => false,
    };
    out.reserve(value.len().saturating_add(3));
    match escaping {
        Escaping::National => out.push('N'),
        Escaping::PostgresEscape if doubles_backslash => out.push('E'),
        _ => {}
    }
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' if escaping == Escaping::BackslashQuote => out.push_str("\\'"),
            '\'' => out.push_str("''"),
            '\\' if doubles_backslash => out.push_str("\\\\"),
            '\n' if escaping == Escaping::BackslashQuote => out.push_str("\\n"),
            '\r' if escaping == Escaping::BackslashQuote => out.push_str("\\r"),
            '\t' if escaping == Escaping::BackslashQuote => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    Ok(())
}

/// Checks that a name can be quoted into copied SQL: no NUL, no control
/// character at all — tab and line breaks included.
///
/// Quoting keeps a hostile name one identifier; it does not keep an escape
/// sequence off the terminal the text is pasted into. Call before
/// [`quote_identifier`](crate::quote_identifier) on any name that reaches the
/// clipboard as SQL. A [`CatalogPath`](crate::CatalogPath) segment already
/// passes this check; a result's column name has not been through one.
///
/// # Errors
///
/// [`LiteralError`], naming the code point, never the name.
pub fn check_identifier(name: &str) -> Result<(), LiteralError> {
    refuse_controls(name, |_| false)
}

/// The first control character of `text` that `allowed` does not accept.
fn refuse_controls(text: &str, allowed: impl Fn(char) -> bool) -> Result<(), LiteralError> {
    match text.chars().find(|c| c.is_control() && !allowed(*c)) {
        None => Ok(()),
        Some('\0') => Err(LiteralError::Nul),
        Some(refused) => Err(LiteralError::ControlCharacter {
            code: u32::from(refused),
        }),
    }
}

#[cfg(test)]
mod tests;
