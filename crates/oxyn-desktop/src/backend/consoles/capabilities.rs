//! What the text a user wrote needs from the session it is about to run on.
//!
//! [ADR-0003](../../../../../docs/adr/0003-driver-capabilities.md) says an absent
//! capability is announced, never emulated. Announcing it *before* submission
//! turns `near "ANALYZE": syntax error` into a sentence naming the capability.
//!
//! **Not an authorisation**: the `PolicyGate` decides what may run. A keyword
//! this does not recognise passes, and the server answers. Ported from the GPUI
//! workspace, which leaves with it (ADR-0029).

use oxyn_core::{Capabilities, SqlDialect};
use oxyn_query::split::{split, words};

/// How far into a statement `ANALYZE` may still belong to the `EXPLAIN` head:
/// `EXPLAIN (ANALYZE, VERBOSE, COSTS false) SELECT …`.
const EXPLAIN_HEAD_WORDS: usize = 8;

/// The first capability the text needs and the session lacks, as the sentence
/// the console shows.
#[must_use]
pub fn missing_for(
    text: &str,
    dialect: SqlDialect,
    capabilities: Capabilities,
) -> Option<&'static str> {
    let fragments = split(text, dialect);
    if fragments.len() > 1 && !capabilities.contains(Capabilities::MULTIPLE_STATEMENTS) {
        return Some(
            "This session takes one statement per submission. \
             Run them one at a time — a batch is never split silently.",
        );
    }
    fragments
        .iter()
        .find_map(|fragment| needed_by(fragment.text, dialect))
        .filter(|(capability, _)| !capabilities.contains(*capability))
        .map(|(_, message)| message)
}

fn needed_by(statement: &str, dialect: SqlDialect) -> Option<(Capabilities, &'static str)> {
    let mots = words(statement, dialect);
    let head = mots.first()?.text;
    if head.eq_ignore_ascii_case("explain") || head.eq_ignore_ascii_case("describe") {
        // `EXPLAIN ANALYZE` executes what it analyses, `DELETE` included (I-07).
        let analyze = mots.iter().take(EXPLAIN_HEAD_WORDS).any(|mot| {
            mot.text.eq_ignore_ascii_case("analyze") || mot.text.eq_ignore_ascii_case("analyse")
        });
        return Some(if analyze {
            (
                Capabilities::EXPLAIN_ANALYZE,
                "This session gives no measured plan: EXPLAIN ANALYZE is unsupported here, \
                 and Oxyn will not run the statement instead to guess one.",
            )
        } else {
            (
                Capabilities::EXPLAIN,
                "This session gives no query plan: EXPLAIN is unsupported here.",
            )
        });
    }
    if head.eq_ignore_ascii_case("savepoint") || head.eq_ignore_ascii_case("release") {
        return Some((
            Capabilities::SAVEPOINTS,
            "This session has no savepoint: a partial rollback is not available here.",
        ));
    }
    if ["rollback", "commit", "begin", "start"]
        .iter()
        .any(|word| head.eq_ignore_ascii_case(word))
    {
        return Some((
            Capabilities::TRANSACTIONS,
            "This session has no transaction: rollback is offered only when supported, \
             and Oxyn will not fake one with a sequence of statements.",
        ));
    }
    None
}

/// The one statement `Explain` submits, prefixed for the dialect.
///
/// Rejects a batch and an existing `EXPLAIN`: one click never turns into the
/// analysis of several statements, and `ANALYZE` is never added.
pub fn explain_sql(text: &str, dialect: SqlDialect) -> Result<String, &'static str> {
    let statements = split(text, dialect);
    let [statement] = statements.as_slice() else {
        return Err("Explain requires exactly one SQL statement.");
    };
    if words(statement.text, dialect)
        .first()
        .is_some_and(|word| word.text.eq_ignore_ascii_case("explain"))
    {
        return Err("This statement already starts with EXPLAIN.");
    }
    let prefix = match dialect {
        SqlDialect::Postgres | SqlDialect::Redshift => "EXPLAIN ",
        SqlDialect::Sqlite => "EXPLAIN QUERY PLAN ",
        _ => return Err("Explain is unavailable for this SQL dialect."),
    };
    Ok(format!("{prefix}{}", statement.text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSTGRES: Capabilities = Capabilities::EXPLAIN
        .union(Capabilities::EXPLAIN_ANALYZE)
        .union(Capabilities::AFFECTED_ROWS);
    const SQLITE: Capabilities = Capabilities::TRANSACTIONS
        .union(Capabilities::MULTIPLE_STATEMENTS)
        .union(Capabilities::EXPLAIN)
        .union(Capabilities::AFFECTED_ROWS);

    #[test]
    fn explain_analyze_is_announced_where_the_session_lacks_it() {
        assert!(missing_for("EXPLAIN ANALYZE SELECT 1", SqlDialect::Sqlite, SQLITE).is_some());
        assert!(missing_for("EXPLAIN ANALYZE SELECT 1", SqlDialect::Postgres, POSTGRES).is_none());
    }

    #[test]
    fn a_batch_is_never_split_silently() {
        assert!(missing_for("SELECT 1; SELECT 2", SqlDialect::Postgres, POSTGRES).is_some());
        assert!(missing_for("SELECT 1; SELECT 2", SqlDialect::Sqlite, SQLITE).is_none());
    }

    #[test]
    fn a_quoted_keyword_announces_nothing() {
        assert!(missing_for("SELECT 'ROLLBACK'", SqlDialect::Postgres, POSTGRES).is_none());
    }

    #[test]
    fn explain_takes_one_statement_and_never_adds_analyze() {
        assert_eq!(
            explain_sql("SELECT 1", SqlDialect::Postgres).as_deref(),
            Ok("EXPLAIN SELECT 1")
        );
        assert!(explain_sql("SELECT 1; DELETE FROM t", SqlDialect::Postgres).is_err());
        assert!(explain_sql("EXPLAIN SELECT 1", SqlDialect::Postgres).is_err());
    }
}
