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
//! The server refuses a second statement in one request itself
//! (`Multi-statements are not allowed`, observed on 26.3), so the lexical
//! check reads every statement only to refuse before anything is sent.

use oxyn_core::{OxynError, Result, SqlDialect};

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

/// Refuses what the session cannot carry from one statement to the next.
///
/// # Errors
/// [`OxynError::NotSupported`] with one of the fixed texts above.
pub fn refuse_session_state(text: &str) -> Result<()> {
    if oxyn_query::changes_context(text, DIALECT) {
        return Err(refusal(CONTEXT_REFUSED));
    }
    for fragment in oxyn_query::split(text, DIALECT) {
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
    fn what_only_mentions_them_goes_through(#[case] text: &str) {
        refuse_session_state(text).expect(text);
    }
}
