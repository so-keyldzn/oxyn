//! The session settings the driver pins, and the typed `SET` it refuses.
//!
//! `sqlx` sends `TimeZone = UTC`, `DateStyle = ISO, MDY` and `client_encoding =
//! UTF8` in the startup message of every connection, with no way to leave them
//! out (sqlx-postgres 0.9.0, `connection/establish.rs`; RESEARCH-NOTES). A
//! startup parameter outranks `postgresql.conf`, `ALTER DATABASE` and `ALTER
//! ROLE`, and it is also what `RESET ALL` returns to. The driver therefore
//! declares them as **pinned** ([DRIVER-CONTRACT §7]) and rests on them: the
//! decoders carry a `timestamptz` in UTC, and read text as UTF-8.
//!
//! A user's `SET TIME ZONE 'Europe/Paris'` would be accepted, then silently
//! undone: by the `ROLLBACK` that closes a read-only execution, by the `RESET
//! ALL` after a write, or by landing on another pooled connection. That is the
//! "invisible session state" the contract forbids, seen from the other side.
//! The statement is refused before anything is sent, like a typed `USE` or a
//! MySQL `SET time_zone`, and the message names the way out (`AT TIME ZONE`,
//! `to_char`).
//!
//! [DRIVER-CONTRACT §7]: ../../../docs/DRIVER-CONTRACT.md
//!
//! # What it covers
//!
//! The statements of the text, split as the server would, that:
//!
//! * lead with `SET` (optionally `SESSION` or `LOCAL`) and then name a pinned
//!   parameter, `TIME ZONE`, or `NAMES` (PostgreSQL's spelling of `SET
//!   client_encoding`). The parameter may be quoted: `SET "TimeZone" = …`;
//! * call `set_config('<pinned>', …)` anywhere, `SELECT` included.
//!
//! `RESET`, `SHOW`, and `ALTER DATABASE … SET` are not refused: the first two
//! read or restore the pinned value, and the last changes a default that the
//! startup parameter outranks anyway.
//!
//! # What it cannot cover
//!
//! A text the driver cannot read is not read: the body of a `DO` block or of a
//! function, `EXECUTE` of a string built at run time, and a `set_config` whose
//! first argument is not a literal. They change the setting behind the refusal;
//! the next reset undoes it, as it always did.

use oxyn_core::SqlDialect;
use oxyn_query::{Token, TokenKind};

/// The parameters `sqlx` imposes at startup, and the decoders rest on.
const PINNED: [&str; 3] = ["timezone", "datestyle", "client_encoding"];

/// Does a statement of the text re-point a setting the driver pinned?
pub(crate) fn changes_pinned_setting(text: &str) -> bool {
    oxyn_query::split(text, SqlDialect::Postgres)
        .iter()
        .any(|fragment| statement_changes_pinned_setting(fragment.text))
}

fn statement_changes_pinned_setting(statement: &str) -> bool {
    let tokens = oxyn_query::tokens(statement, SqlDialect::Postgres);
    leads_with_pinned_set(&tokens) || calls_set_config_on_pinned(&tokens)
}

/// `SET [SESSION | LOCAL] <pinned> …`, `SET … TIME ZONE …`, `SET … NAMES …`.
fn leads_with_pinned_set(tokens: &[Token<'_>]) -> bool {
    let Some((verb, mut rest)) = tokens.split_first() else {
        return false;
    };
    if !is_word(verb, "set") {
        return false;
    }
    if let Some((scope, after)) = rest.split_first()
        && (is_word(scope, "session") || is_word(scope, "local"))
    {
        rest = after;
    }
    let Some((first, after)) = rest.split_first() else {
        return false;
    };
    if is_word(first, "names") {
        return true;
    }
    if is_word(first, "time") {
        return after.first().is_some_and(|zone| is_word(zone, "zone"));
    }
    names_pinned(first)
}

/// `set_config('timezone', …)`: a call whose first argument is a literal
/// naming a pinned parameter.
fn calls_set_config_on_pinned(tokens: &[Token<'_>]) -> bool {
    tokens.windows(3).any(|window| {
        let [function, open, argument] = window else {
            return false;
        };
        is_word(function, "set_config")
            && open.text == "("
            && argument.kind == TokenKind::String
            && unquote(argument.text, '\'').is_some_and(is_pinned)
    })
}

/// Is the token a bare or double-quoted name of a pinned parameter?
fn names_pinned(token: &Token<'_>) -> bool {
    match token.kind {
        TokenKind::Word => is_pinned(token.text),
        TokenKind::Quoted => unquote(token.text, '"').is_some_and(is_pinned),
        _ => false,
    }
}

fn is_pinned(name: &str) -> bool {
    PINNED
        .iter()
        .any(|pinned| name.trim().eq_ignore_ascii_case(pinned))
}

/// The text inside a pair of `quote` characters. A doubled quote inside is left
/// as written: no pinned name holds one.
fn unquote(quoted: &str, quote: char) -> Option<&str> {
    quoted.strip_prefix(quote)?.strip_suffix(quote)
}

fn is_word(token: &Token<'_>, expected: &str) -> bool {
    token.kind == TokenKind::Word && token.text.eq_ignore_ascii_case(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_set_of_a_pinned_setting_is_refused() {
        for text in [
            "SET TIME ZONE 'Europe/Paris'",
            "set time zone local",
            "SET SESSION TIME ZONE 'UTC'",
            "SET LOCAL TIME ZONE 'UTC'",
            "SET TimeZone = 'Europe/Paris'",
            "SET timezone TO 'Europe/Paris'",
            "SET SESSION timezone TO DEFAULT",
            "SET datestyle = 'ISO, DMY'",
            "SET DateStyle TO ISO, DMY",
            "SET LOCAL datestyle = dmy",
            "SET \"TimeZone\" = 'Europe/Paris'",
            "SET client_encoding = 'LATIN1'",
            "SET NAMES 'LATIN1'",
            "  /* now */ SET TIME ZONE 'UTC'",
            // A second statement does not hide it.
            "SELECT 1; SET TIME ZONE 'Europe/Paris'",
            "SET search_path = public;\nSET datestyle = 'SQL, DMY'",
            "SELECT set_config('TimeZone', 'Europe/Paris', false)",
            "SELECT set_config( 'datestyle', 'ISO, DMY', false)",
            "SELECT 1, pg_catalog.set_config('client_encoding', 'LATIN1', true)",
        ] {
            assert!(changes_pinned_setting(text), "must be refused: {text}");
        }
    }

    #[test]
    fn what_reads_or_restores_a_pinned_setting_is_sent() {
        for text in [
            "SHOW TimeZone",
            "SHOW DateStyle",
            "RESET TimeZone",
            "RESET ALL",
            "SET search_path = public",
            "SET statement_timeout = '5s'",
            "SET application_name = 'x'",
            "SET ROLE reporting",
            "SET TRANSACTION READ ONLY",
            "SELECT now() AT TIME ZONE 'Europe/Paris'",
            "SELECT current_setting('TimeZone')",
            "SELECT set_config('search_path', 'public', false)",
            "SELECT 'SET TIME ZONE ''UTC'''",
            "-- SET TIME ZONE 'UTC'\nSELECT 1",
            "SELECT * FROM settings WHERE name = 'timezone'",
            "ALTER DATABASE shop SET timezone = 'Europe/Paris'",
            "CREATE TABLE timezone (name text)",
            "",
        ] {
            assert!(!changes_pinned_setting(text), "must be sent: {text}");
        }
    }
}
