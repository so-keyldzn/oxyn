//! The statements a stateless session refuses before sending them.
//!
//! Each request carries everything that decides its meaning (ADR-0060), and
//! nothing a request does outlives it. Three statements exist only to change
//! what the **next** statements mean — and over HTTP without a session they
//! would succeed and change nothing, so the user would believe a setting or a
//! database held while it did not. They are refused, for every actor, with a
//! fixed text that names the way out and never echoes what was typed
//! ([I-03](../../../CLAUDE.md#i-03)):
//!
//! * `USE db`: the database is the session's, changed by the context selector
//!   ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md));
//! * `SET name = value`: a setting is written in the statement's own
//!   `SETTINGS` clause;
//! * `CREATE TEMPORARY TABLE`: a temporary table lives in an HTTP session, and
//!   would be gone before the next statement.
//!
//! A statement's own `SETTINGS` clause is refused too when it assigns a
//! setting the driver pins on every request: the clause applies after the
//! request's settings, and would undo what the driver relies on to read the
//! answer and to stop the statement.
//!
//! The server refuses a second statement in one request itself
//! (`Multi-statements are not allowed`, observed on 26.3), so the lexical
//! check reads every statement only to refuse before anything is sent.

use oxyn_core::{OxynError, Result, SqlDialect};
use oxyn_query::TokenKind;

const DIALECT: SqlDialect = SqlDialect::ClickHouse;

/// Why a typed `USE` is refused.
pub(crate) const CONTEXT_REFUSED: &str = "a typed `USE`: every statement is sent on its own, \
    with the database the session declares, so change it with the context selector";

/// Why a typed `SET` is refused.
pub(crate) const SET_REFUSED: &str = "a typed `SET`: every statement is sent on its own, so a \
    setting would not reach the next one; write it in the statement's `SETTINGS` clause \
    (`SELECT … SETTINGS max_execution_time = 60`)";

/// Why a typed `CREATE TEMPORARY TABLE` is refused.
pub(crate) const TEMPORARY_REFUSED: &str = "a temporary table: it lives in an HTTP session, \
    which this driver does not keep, so it would be gone before the next statement; create a \
    table with the `Memory` engine instead";

/// Why a statement that changes a setting the driver pins is refused.
pub(crate) const PINNED_REFUSED: &str = "a setting the driver pins to read the answer or to \
    stop the statement (`output_format_binary_*`, `send_progress_in_http_headers`, \
    `http_headers_progress_interval_ms`, `wait_end_of_query`, \
    `cancel_http_readonly_queries_on_client_close`, `readonly`): remove it from the \
    statement's `SETTINGS` clause";

/// The settings a statement's own `SETTINGS` clause may not change, since it
/// applies after the request's own:
///
/// * the decoder reads the header and the rows `output_format_binary_*`
///   shape, and a 200 sent with the progress headers comes before the
///   statement ends;
/// * `wait_end_of_query` is what makes a server that marks no exception in
///   its body send a late error as a status rather than as rows;
/// * `cancel_http_readonly_queries_on_client_close` and `readonly` are what
///   stop a read whose cursor is dropped, for an account that may not kill.
///
/// `max_execution_time` is not one of them: it is the user's bound to
/// choose, and the executor's own deadline still ends the execution.
fn pinned(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("output_format_binary_")
        || [
            "send_progress_in_http_headers",
            "http_headers_progress_interval_ms",
            "wait_end_of_query",
            "cancel_http_readonly_queries_on_client_close",
            "readonly",
        ]
        .contains(&name.as_str())
}

/// Whether a statement assigns a pinned setting: a name, bare or quoted,
/// followed by `=`, after a `SETTINGS` keyword. A column of the same name
/// before the clause — `SELECT readonly FROM system.settings` — is not one,
/// nor is the clause of `CREATE USER … SETTINGS readonly = 1`, which sets
/// the account's profile, not this statement.
fn assigns_pinned(fragment: &str) -> bool {
    if configures_access(fragment) {
        return false;
    }
    let tokens = oxyn_query::tokens(fragment, DIALECT);
    let mut in_settings = false;
    let mut previous = "";
    for (token, next) in tokens.iter().zip(tokens.iter().skip(1)) {
        // `system.settings` names a table, not the clause.
        let qualified = std::mem::replace(&mut previous, token.text) == ".";
        let name = match token.kind {
            TokenKind::Word => token.text,
            // `SETTINGS \`readonly\` = 0` assigns it too.
            TokenKind::Quoted => token
                .text
                .get(1..token.text.len().saturating_sub(1))
                .unwrap_or_default(),
            _ => continue,
        };
        if token.kind == TokenKind::Word && !qualified && name.eq_ignore_ascii_case("settings") {
            in_settings = true;
        } else if in_settings && next.text == "=" && pinned(name) {
            return true;
        }
    }
    false
}

/// `CREATE`/`ALTER` of a user, a role or a settings profile: its `SETTINGS`
/// clause is what the entity carries for later sessions.
fn configures_access(fragment: &str) -> bool {
    let words = oxyn_query::words(fragment, DIALECT);
    let mut leading = words
        .iter()
        .filter(|word| !word.call)
        .map(|word| word.text)
        .filter(|word| !word.eq_ignore_ascii_case("or") && !word.eq_ignore_ascii_case("replace"));
    matches!(
        (leading.next(), leading.next()),
        (Some(verb), Some(entity))
            if (verb.eq_ignore_ascii_case("create") || verb.eq_ignore_ascii_case("alter"))
                && ["user", "role", "settings", "profile"]
                    .iter()
                    .any(|kind| entity.eq_ignore_ascii_case(kind))
    )
}

/// Refuses what the session cannot carry from one statement to the next,
/// and what would change a setting the driver pins.
///
/// # Errors
/// [`OxynError::NotSupported`] with one of the fixed texts above.
pub fn refuse_session_state(text: &str) -> Result<()> {
    if oxyn_query::changes_context(text, DIALECT) {
        return Err(refusal(CONTEXT_REFUSED));
    }
    for fragment in oxyn_query::split(text, DIALECT) {
        if assigns_pinned(fragment.text) {
            return Err(refusal(PINNED_REFUSED));
        }
        let words = oxyn_query::words(fragment.text, DIALECT);
        let mut leading = words.iter().filter(|word| !word.call).map(|word| word.text);
        match (leading.next(), leading.next()) {
            (Some(first), _) if first.eq_ignore_ascii_case("set") => {
                return Err(refusal(SET_REFUSED));
            }
            (Some(first), Some(second))
                if first.eq_ignore_ascii_case("create")
                    && second.eq_ignore_ascii_case("temporary") =>
            {
                return Err(refusal(TEMPORARY_REFUSED));
            }
            _ => {}
        }
    }
    Ok(())
}

fn refusal(text: &str) -> OxynError {
    OxynError::NotSupported {
        capability: text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("USE shop", CONTEXT_REFUSED)]
    #[case("  use `a``; DROP`", CONTEXT_REFUSED)]
    #[case("/* x */ USE shop", CONTEXT_REFUSED)]
    #[case("SELECT 1; USE shop", CONTEXT_REFUSED)]
    #[case("SET max_threads = 1", SET_REFUSED)]
    #[case("set readonly = 0", SET_REFUSED)]
    #[case("-- pick\nSET ROLE admin", SET_REFUSED)]
    #[case("SELECT 1; SET x = 1", SET_REFUSED)]
    #[case("CREATE TEMPORARY TABLE t (a UInt8)", TEMPORARY_REFUSED)]
    #[case("create temporary table if not exists t (a UInt8)", TEMPORARY_REFUSED)]
    #[case(
        "SELECT j FROM t SETTINGS output_format_binary_write_json_as_string = 0",
        PINNED_REFUSED
    )]
    #[case(
        "INSERT INTO t SELECT * FROM s SETTINGS SEND_PROGRESS_IN_HTTP_HEADERS = 1",
        PINNED_REFUSED
    )]
    #[case(
        "SELECT 1 SETTINGS max_threads = 2, http_headers_progress_interval_ms = 10",
        PINNED_REFUSED
    )]
    #[case("SELECT throwIf(1) SETTINGS wait_end_of_query = 0", PINNED_REFUSED)]
    #[case(
        "SELECT count() FROM numbers(1e12) SETTINGS cancel_http_readonly_queries_on_client_close=0",
        PINNED_REFUSED
    )]
    #[case("SELECT 1 SETTINGS max_threads = 2, readonly = 0", PINNED_REFUSED)]
    #[case("SELECT 1 SETTINGS `wait_end_of_query` = 0", PINNED_REFUSED)]
    #[case("SELECT 1 SETTINGS \"Readonly\" = 0", PINNED_REFUSED)]
    #[case(
        "SELECT * FROM (SELECT 1 SETTINGS wait_end_of_query = 0) SETTINGS max_threads = 1",
        PINNED_REFUSED
    )]
    #[case(
        "INSERT INTO t SETTINGS wait_end_of_query = 0 VALUES (1)",
        PINNED_REFUSED
    )]
    fn session_state_is_refused_before_anything_is_sent(
        #[case] text: &str,
        #[case] expected: &str,
    ) {
        let error = refuse_session_state(text).expect_err(text);
        let OxynError::NotSupported { capability } = &error else {
            panic!("a refusal of capability: {error:?}");
        };
        assert_eq!(capability, expected, "{text}");
        for typed in ["shop", "DROP", "max_threads", "admin"] {
            assert!(!capability.contains(typed), "`{text}` echoes `{typed}`");
        }
    }

    #[rstest]
    #[case("SELECT 'use', 'set'")]
    #[case("SELECT `set` FROM t")]
    #[case("SELECT 1 SETTINGS max_threads = 8")]
    #[case("ALTER TABLE t UPDATE x = 1 WHERE 1")]
    #[case("SELECT * FROM t -- USE x")]
    #[case("CREATE TABLE t (a UInt8) ENGINE = Memory")]
    #[case("SELECT * FROM used")]
    #[case("SELECT 'send_progress_in_http_headers' AS name")]
    #[case("SELECT * FROM system.settings -- output_format_binary_x")]
    #[case("SELECT name, readonly FROM system.settings WHERE readonly = 1")]
    #[case("SELECT 1 SETTINGS max_execution_time = 60")]
    #[case("CREATE USER OR REPLACE u IDENTIFIED BY 'p' SETTINGS readonly = 1")]
    #[case("ALTER ROLE r SETTINGS wait_end_of_query = 1")]
    #[case("CREATE SETTINGS PROFILE p SETTINGS readonly = 2 TO u")]
    #[case("SELECT * FROM t SETTINGS max_threads = 2 -- wait_end_of_query = 0")]
    fn what_only_mentions_them_goes_through(#[case] text: &str) {
        refuse_session_state(text).expect(text);
    }
}
