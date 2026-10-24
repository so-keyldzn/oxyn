//! `ATTACH` against the one-engine-per-file registry.
//!
//! An `ATTACH` opens a database file inside the session's engine. If that file
//! is open as an Oxyn connection, the process now holds two engines on it,
//! and DuckDB's record lock, owned by the process, does not see the second:
//! writes through both corrupt the file with no error anywhere
//! (`crate::open`). The other direction — opening a connection on a file an
//! engine has attached — is refused when the connection opens.
//!
//! The file is read from the statement before DuckDB prepares it. A file the
//! driver cannot read there — not a plain string literal — is refused rather
//! than let through: the guard holds only if it sees every target.

use std::path::Path;

use oxyn_core::{ErrorClass, OxynError, Result};

use crate::error::{self, DuckDbError};
use crate::open;
use crate::stream::code_of;

/// Refuses an `ATTACH` of a file open as an Oxyn connection; any other
/// statement passes.
///
/// # Errors
/// [`DuckDbError::OpenAsConnection`], [`DuckDbError::AttachTargetUnread`], or
/// a configuration error for a path DuckDB would respell.
pub(crate) fn check(text: &str) -> Result<()> {
    let Some(rest) = word(code_of(text), "attach") else {
        return Ok(());
    };
    let Some(target) = target(rest) else {
        return Err(error::driver(
            DuckDbError::AttachTargetUnread,
            ErrorClass::Permanent,
        ));
    };
    if target.is_empty() || target == ":memory:" {
        return Ok(());
    }
    if crate::driver::rewritten_by_the_engine(&target) {
        return Err(OxynError::Config(
            "the file of an ATTACH starts with `~` or `file:`, which DuckDB rewrites itself; \
             write its full path"
                .to_owned(),
        ));
    }
    if open::is_open(Path::new(&target)) {
        return Err(error::driver(
            DuckDbError::OpenAsConnection,
            ErrorClass::Permanent,
        ));
    }
    Ok(())
}

/// The file of `ATTACH [OR REPLACE] [DATABASE] [IF NOT EXISTS] '<file>' …`,
/// read after the `ATTACH` keyword; `None` when it is not a plain string
/// literal.
fn target(text: &str) -> Option<String> {
    let mut rest = text;
    for optional in [
        &["or", "replace"][..],
        &["database"],
        &["if", "not", "exists"],
    ] {
        let mut after = Some(rest);
        for keyword in optional {
            after = after.and_then(|text| word(code_of(text), keyword));
        }
        if let Some(after) = after {
            rest = after;
        }
    }
    let quoted = code_of(rest).strip_prefix('\'')?;
    let mut value = String::new();
    let mut chars = quoted.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' {
            value.push(c);
        } else if chars.peek() == Some(&'\'') {
            chars.next();
            value.push('\'');
        } else {
            return Some(value);
        }
    }
    None
}

/// What follows `keyword` at the start of `text`, compared without case and
/// ending there.
fn word<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let head = text.get(..keyword.len())?;
    let rest = text.get(keyword.len()..)?;
    let ends = !rest
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_');
    (head.eq_ignore_ascii_case(keyword) && ends).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_of_an_attach_is_read_from_its_literal() {
        for (statement, file) in [
            ("ATTACH 'sales.duckdb'", "sales.duckdb"),
            ("attach database 'a b.duckdb' AS ab", "a b.duckdb"),
            ("ATTACH IF NOT EXISTS 'x.duckdb'", "x.duckdb"),
            (
                "ATTACH OR REPLACE DATABASE 'y.duckdb' (READ_ONLY)",
                "y.duckdb",
            ),
            ("/* c */ ATTACH /* d */ 'it''s.duckdb'", "it's.duckdb"),
            ("ATTACH ':memory:' AS scratch", ":memory:"),
        ] {
            let rest = word(code_of(statement), "attach").expect("an ATTACH");
            assert_eq!(target(rest).as_deref(), Some(file), "{statement}");
        }
        for unread in ["ATTACH $$a.duckdb$$", "ATTACH E'a.duckdb'", "ATTACH 'open"] {
            let rest = word(code_of(unread), "attach").expect("an ATTACH");
            assert_eq!(target(rest), None, "{unread}");
        }
        assert!(word("ATTACHED", "attach").is_none());
        assert!(check("SELECT 'ATTACH x'").is_ok());
        assert!(check("ATTACH E'x.duckdb'").is_err());
    }
}
