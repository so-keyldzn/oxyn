//! The statements a stateless session refuses before sending them.
//!
//! Each statement runs on a Hrana stream of its own, closed in the same
//! request ([`crate::wire`]). Four kinds of statement exist only to change
//! what the **next** ones mean — and on a stream that ends with them they
//! would succeed and change nothing, so the user would believe a transaction
//! was open or a setting held while it was not:
//!
//! * `BEGIN`, `COMMIT`, `END`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`;
//! * `PRAGMA name = value`, which sets a connection's state;
//! * `ATTACH` and `DETACH`, which last as long as a connection;
//! * `USE`, which SQLite has not, but which the executor flags as a context
//!   change.
//!
//! They are refused, for every actor, with a fixed text that names the way out
//! and never echoes what was typed ([I-03](../../../CLAUDE.md#i-03)). A
//! `PRAGMA` that only reads (`PRAGMA table_info(t)`) goes through.
//!
//! Only the **first word** of the text is read for the transaction verbs: a
//! trigger's body holds `BEGIN … END` inside a `CREATE TRIGGER`, which is a
//! definition and runs fine.

use oxyn_core::{ExecRequest, OxynError, Result, SqlDialect};
use oxyn_query::TokenKind;

const DIALECT: SqlDialect = SqlDialect::Sqlite;

/// Why a typed transaction verb is refused.
pub(crate) const TRANSACTION_REFUSED: &str = "a typed transaction statement: every statement \
    runs on a stream of its own that ends with it, so a `BEGIN` would be undone before the next \
    statement and a `COMMIT` would find nothing to commit; Turso offers no way to hold one open \
    from Oxyn";

/// Why a `PRAGMA` that sets a value is refused.
pub(crate) const PRAGMA_REFUSED: &str = "a typed `PRAGMA` that sets a value: every statement runs \
    on a stream of its own, so the setting would be gone before the next statement";

/// Why `ATTACH` and `DETACH` are refused.
pub(crate) const ATTACH_REFUSED: &str = "a typed `ATTACH` or `DETACH`: an attached database \
    lasts as long as its stream, which ends with the statement";

/// Why a context change is refused.
pub(crate) const CONTEXT_REFUSED: &str = "a typed `USE`: a Turso connection is one database, \
    named by its URL";

fn refusal(text: &str) -> OxynError {
    OxynError::NotSupported {
        capability: text.to_owned(),
    }
}

/// Refuses what a stream that ends with the statement cannot carry.
///
/// The executor's own flags are honoured first (it classified the text with
/// the same grammar), then the text is read, since a driver used without the
/// executor has no flags.
///
/// # Errors
/// [`OxynError::NotSupported`] with one of the fixed texts above.
pub fn refuse_session_state(request: &ExecRequest) -> Result<()> {
    if request.context_change {
        return Err(refusal(CONTEXT_REFUSED));
    }
    if request.transaction_control {
        return Err(refusal(TRANSACTION_REFUSED));
    }
    let words = oxyn_query::words(&request.text, DIALECT);
    let Some(first) = words.iter().find(|word| !word.call).map(|word| word.text) else {
        return Ok(());
    };
    let is = |verb: &str| first.eq_ignore_ascii_case(verb);
    if ["begin", "commit", "end", "rollback", "savepoint", "release"]
        .iter()
        .any(|verb| is(verb))
    {
        return Err(refusal(TRANSACTION_REFUSED));
    }
    if is("attach") || is("detach") {
        return Err(refusal(ATTACH_REFUSED));
    }
    if is("pragma") && sets_a_pragma(&request.text) {
        return Err(refusal(PRAGMA_REFUSED));
    }
    Ok(())
}

/// The pragmas that take an argument and only read: the object they look at.
/// Any other `PRAGMA name(value)` is the call form of a setting
/// (`PRAGMA foreign_keys(ON)`), which sets exactly what `=` sets.
const READS_WITH_ARGUMENT: [&str; 12] = [
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "foreign_key_check",
    "integrity_check",
    "quick_check",
    "collation_list",
    "database_list",
];

/// Whether a `PRAGMA` statement sets a value: `PRAGMA [schema.]name = value`,
/// or `PRAGMA [schema.]name(value)` for a name that is not a known read.
fn sets_a_pragma(text: &str) -> bool {
    let tokens = oxyn_query::tokens(text, DIALECT);
    if tokens
        .iter()
        .any(|token| token.kind == TokenKind::Symbol && token.text == "=")
    {
        return true;
    }
    let mut after = tokens.iter().skip(1);
    let Some(mut name) = after.next() else {
        return false;
    };
    let mut next = after.next();
    // `PRAGMA main.name(…)`: the schema comes first.
    if next.is_some_and(|token| token.text == ".") {
        let Some(qualified) = after.next() else {
            return false;
        };
        name = qualified;
        next = after.next();
    }
    let unquoted = name
        .text
        .trim_matches(|c| matches!(c, '"' | '`' | '\'' | '[' | ']'));
    next.is_some_and(|token| token.text == "(")
        && !READS_WITH_ARGUMENT
            .iter()
            .any(|read| unquoted.eq_ignore_ascii_case(read))
}

#[cfg(test)]
mod tests {
    use oxyn_core::QueryLanguage;
    use rstest::rstest;

    use super::*;

    fn request(text: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::Sql(DIALECT), text)
    }

    #[rstest]
    #[case("BEGIN", TRANSACTION_REFUSED)]
    #[case("begin immediate transaction", TRANSACTION_REFUSED)]
    #[case("  /* x */ COMMIT", TRANSACTION_REFUSED)]
    #[case("END", TRANSACTION_REFUSED)]
    #[case("ROLLBACK TO sp", TRANSACTION_REFUSED)]
    #[case("SAVEPOINT sp", TRANSACTION_REFUSED)]
    #[case("RELEASE sp", TRANSACTION_REFUSED)]
    #[case("-- note\nBEGIN", TRANSACTION_REFUSED)]
    #[case("ATTACH DATABASE 'x.db' AS other", ATTACH_REFUSED)]
    #[case("detach other", ATTACH_REFUSED)]
    #[case("PRAGMA foreign_keys = ON", PRAGMA_REFUSED)]
    #[case("pragma main.user_version=3", PRAGMA_REFUSED)]
    #[case("PRAGMA foreign_keys(ON)", PRAGMA_REFUSED)]
    #[case("pragma busy_timeout (5000)", PRAGMA_REFUSED)]
    #[case("PRAGMA main.journal_mode(WAL)", PRAGMA_REFUSED)]
    fn session_state_is_refused_before_anything_is_sent(
        #[case] text: &str,
        #[case] expected: &str,
    ) {
        let error = refuse_session_state(&request(text)).expect_err(text);
        let OxynError::NotSupported { capability } = &error else {
            panic!("a refusal of capability: {error:?}");
        };
        assert_eq!(capability, expected, "{text}");
        for typed in ["x.db", "other", "foreign_keys", "user_version", "sp"] {
            assert!(!capability.contains(typed), "`{text}` echoes `{typed}`");
        }
    }

    #[rstest]
    #[case("SELECT 'begin', 'commit'")]
    #[case("SELECT * FROM \"end\"")]
    #[case("PRAGMA table_info(users)")]
    #[case("PRAGMA foreign_key_list('orders')")]
    #[case("PRAGMA user_version")]
    #[case("PRAGMA main.table_info(\"users\")")]
    #[case("PRAGMA index_list(users)")]
    #[case("PRAGMA integrity_check(10)")]
    #[case("SELECT * FROM pragma_table_info('t') WHERE name = 'a'")]
    #[case("INSERT INTO t VALUES ('BEGIN')")]
    #[case("CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET x = 1; END")]
    #[case("UPDATE attachments SET detached = 1")]
    #[case("EXPLAIN QUERY PLAN SELECT 1")]
    #[case("")]
    fn what_only_mentions_them_goes_through(#[case] text: &str) {
        refuse_session_state(&request(text)).expect(text);
    }

    #[test]
    fn the_executors_flags_are_honoured_without_reading_the_text() {
        let mut flagged = request("SELECT 1");
        flagged.transaction_control = true;
        assert!(refuse_session_state(&flagged).is_err());
        let mut flagged = request("SELECT 1");
        flagged.context_change = true;
        assert!(refuse_session_state(&flagged).is_err());
    }
}
