//! Where a `--` comment ends, as the **server** reads it.
//!
//! The splitter decides which text is a statement, and the classifier decides
//! from that whether a statement writes. If the splitter reads a comment where
//! the server reads a statement, the `PolicyGate` judges a text the server does
//! not run. Every expectation below was checked against the engine itself, not
//! derived from this crate: PostgreSQL 17.11 through the extended protocol, and
//! SQLite 3.50.2, the version `rusqlite` bundles (docs/RESEARCH-NOTES.md,
//! « Fin d'un commentaire `--` »).

use oxyn_core::{MutationRisk, SqlDialect, StatementIntent};
use oxyn_query::classify;

/// What the gate receives for a text: statement count, intent, risk.
fn read(sql: &str, dialect: SqlDialect) -> (usize, StatementIntent, MutationRisk) {
    let classification = classify(sql, dialect);
    (
        classification.statements.len(),
        classification.intent,
        classification.risk,
    )
}

/// PostgreSQL ends `--` at `\r` (`scan.l`: `newline [\n\r]`). A statement
/// hidden behind a lone `\r` runs, so it must be classified.
///
/// Before the fix the splitter read the whole tail as a comment: the text had
/// **no statement**, hence `Unknown` with no risk — the server dropped the
/// table while the destructive-change approval never fired.
#[test]
fn postgres_classifies_the_statement_after_a_carriage_return() {
    for (sql, intent, risk) in [
        (
            "-- x\rDROP TABLE audit",
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
        (
            "; -- x\rDROP TABLE audit",
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
        (
            "-- x\rUPDATE accounts SET balance = 0",
            StatementIntent::Write,
            MutationRisk::UnboundedUpdate,
        ),
    ] {
        assert_eq!(
            read(sql, SqlDialect::Postgres),
            (1, intent, risk),
            "{sql:?}"
        );
    }
}

/// The gate bypass itself: a read followed by a hidden write was classified
/// `Read`, which an agent may run on production. The simple protocol runs both
/// commands; only the extended protocol of today's driver refuses the string.
#[test]
fn postgres_never_reads_a_hidden_write_as_a_read() {
    for (sql, intent, risk) in [
        (
            "SELECT 1; -- x\rDROP TABLE audit",
            StatementIntent::Ddl,
            MutationRisk::DropObject,
        ),
        (
            "SELECT 1; -- x\rUPDATE accounts SET balance = 0",
            StatementIntent::Write,
            MutationRisk::UnboundedUpdate,
        ),
    ] {
        let classification = classify(sql, SqlDialect::Postgres);
        assert!(classification.is_mutating(), "{sql:?}: {classification:?}");
        assert_eq!(
            (
                classification.statements.len(),
                classification.intent,
                classification.risk
            ),
            (2, intent, risk),
            "{sql:?}"
        );
    }
}

/// Where the splitter and the server already agreed, they still do.
#[test]
fn postgres_line_endings_that_were_never_ambiguous() {
    for (sql, expected) in [
        // PostgreSQL reads `SELECT 1 DELETE FROM accounts`: `delete` is a
        // column alias, and the server answers `SELECT 2`. The keyword sweep
        // now sees the bare `DELETE` the server sees, exactly as with a `\n`:
        // one confirmation too many, never a write too few.
        (
            "SELECT 1 -- x\rDELETE FROM accounts",
            (1, StatementIntent::Unknown, MutationRisk::None),
        ),
        (
            "-- x\r\nDROP TABLE audit",
            (1, StatementIntent::Ddl, MutationRisk::DropObject),
        ),
        (
            "-- x\n\rDROP TABLE audit",
            (1, StatementIntent::Ddl, MutationRisk::DropObject),
        ),
        (
            "SELECT 1 -- x\r",
            (1, StatementIntent::Read, MutationRisk::None),
        ),
        // The parser already saw both commands; the splitter now agrees.
        (
            "SELECT 1 -- x\r; DROP TABLE audit",
            (2, StatementIntent::Ddl, MutationRisk::DropObject),
        ),
        (
            "EXPLAIN -- x\rANALYZE DELETE FROM accounts",
            (1, StatementIntent::Write, MutationRisk::UnboundedDelete),
        ),
    ] {
        assert_eq!(read(sql, SqlDialect::Postgres), expected, "{sql:?}");
    }
}

/// SQLite ends `--` at `\n` only (`tokenize.c`), so the text after a lone `\r`
/// is a comment and nothing runs. Ending the comment at `\r` here would be the
/// dangerous direction: the second case would lose its `DROP` inside what the
/// splitter would then take for a block comment.
#[test]
fn sqlite_keeps_its_own_line_comment_end() {
    for (sql, expected) in [
        (
            "SELECT 1; -- x\rDROP TABLE audit",
            (1, StatementIntent::Read, MutationRisk::None),
        ),
        (
            "SELECT 1; -- x\r/*\nDROP TABLE audit -- */",
            (2, StatementIntent::Ddl, MutationRisk::DropObject),
        ),
        (
            "-- x\r\nDROP TABLE audit",
            (1, StatementIntent::Ddl, MutationRisk::DropObject),
        ),
    ] {
        assert_eq!(read(sql, SqlDialect::Sqlite), expected, "{sql:?}");
    }
}

/// A dialect whose lexer nobody checked cannot be read either way: a comment
/// holding a lone `\r` hides a statement from one reading or the other. The
/// text must count as mutating under both.
#[test]
fn an_unverified_dialect_treats_a_lone_carriage_return_as_unreadable() {
    for dialect in [
        SqlDialect::Ansi,
        SqlDialect::MySql,
        SqlDialect::SqlServer,
        SqlDialect::DuckDb,
        SqlDialect::Redshift,
    ] {
        for sql in [
            "SELECT 1; -- x\rDROP TABLE audit",
            "SELECT 1 -- x\r; DROP TABLE audit",
            "SELECT 1; -- x\r/*\nDROP TABLE audit -- */",
            "-- x\rDROP TABLE audit",
        ] {
            let classification = classify(sql, dialect);
            assert!(
                classification.is_mutating(),
                "{dialect} {sql:?}: {classification:?}"
            );
            // The editor's strict check refuses it too, so « run statement »
            // does not offer it as a readable statement.
            assert!(
                oxyn_query::validate(sql, dialect).is_err(),
                "{dialect} {sql:?}"
            );
        }
        // A Windows line ending is not ambiguous and must stay a plain read.
        assert!(
            classify("SELECT 1 -- note\r\n", dialect).is_read_only(),
            "{dialect}"
        );
    }
}
