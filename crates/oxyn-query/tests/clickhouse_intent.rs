//! What ClickHouse statements mean to the `PolicyGate`.
//!
//! The production guard ([I-02](../../../CLAUDE.md#i-02)) and the agents'
//! refusals ([I-07](../../../CLAUDE.md#i-07)) read the intent this crate gives
//! a text. A ClickHouse write read as a read goes through without the
//! confirmation that names the connection. Every statement below that changes
//! something must therefore come out mutating — `Unknown` included, which is
//! mutating by definition; what only reads must come out a read, or the
//! console asks for approvals nobody needs.
//!
//! The forms are ClickHouse's own (26.3): mutations are `ALTER TABLE …
//! UPDATE/DELETE`, `OPTIMIZE` merges parts, `SYSTEM` acts on the server,
//! `KILL` stops other statements, `EXCHANGE` swaps two tables, `BACKUP`
//! writes to a disk, `INSERT INTO FUNCTION` writes through a table function.

use oxyn_core::{MutationRisk, SqlDialect, StatementIntent};
use oxyn_query::{changes_context, classify, split};

const DIALECT: SqlDialect = SqlDialect::ClickHouse;

#[test]
fn what_only_reads_is_a_read() {
    for sql in [
        "SELECT 1",
        "SELECT count() FROM events.hits WHERE CounterID = 62",
        "SELECT * FROM hits FINAL LIMIT 10",
        "SELECT a FROM t SETTINGS max_threads = 8",
        "SELECT a FROM t LIMIT 5 SETTINGS max_threads = 8",
        "WITH x AS (SELECT 1 AS a) SELECT a FROM x",
        "SELECT 'DROP TABLE t', `delete` FROM t",
        "SHOW TABLES",
        "SHOW DATABASES",
        "EXPLAIN SELECT 1",
    ] {
        let outcome = classify(sql, DIALECT);
        assert_eq!(outcome.intent, StatementIntent::Read, "{sql}: {outcome:?}");
    }
}

#[test]
fn every_clickhouse_write_is_mutating() {
    for sql in [
        "INSERT INTO t VALUES (1, 'a')",
        "INSERT INTO t SELECT * FROM s",
        "INSERT INTO FUNCTION url('http://example.com/x', 'CSV', 'a UInt8') SELECT 1",
        "ALTER TABLE t UPDATE x = 1 WHERE id = 2",
        "ALTER TABLE t DELETE WHERE 1",
        "ALTER TABLE t DROP PARTITION 202401",
        "ALTER TABLE t ADD COLUMN c UInt8",
        "DELETE FROM t WHERE id = 1",
        "OPTIMIZE TABLE t FINAL",
        "SYSTEM STOP MERGES",
        "SYSTEM DROP DNS CACHE",
        "SYSTEM FLUSH LOGS",
        "KILL QUERY WHERE query_id = 'x'",
        "KILL MUTATION WHERE mutation_id = 'x'",
        "ATTACH TABLE t",
        "DETACH TABLE t",
        "RENAME TABLE a TO b",
        "EXCHANGE TABLES a AND b",
        "TRUNCATE TABLE t",
        "DROP TABLE t",
        "UNDROP TABLE t",
        "CREATE TABLE t ENGINE = Memory AS SELECT 1",
        "CREATE TABLE t2 AS t",
        "CREATE MATERIALIZED VIEW mv ENGINE = Memory AS SELECT 1",
        "CREATE DICTIONARY d (id UInt64) PRIMARY KEY id SOURCE(NULL()) LAYOUT(FLAT()) LIFETIME(0)",
        "BACKUP TABLE t TO Disk('backups', 'x.zip')",
        "RESTORE TABLE t FROM Disk('backups', 'x.zip')",
        "GRANT SELECT ON db.* TO u",
        "REVOKE SELECT ON db.* FROM u",
        "CREATE USER u IDENTIFIED BY 'p'",
        "CREATE ROLE r",
        "SET max_threads = 1",
        "USE system",
        "SELECT 1; DROP TABLE t",
    ] {
        let outcome = classify(sql, DIALECT);
        assert!(outcome.is_mutating(), "{sql}: {outcome:?}");
    }
}

/// sqlparser 0.62 reads the `SETTINGS` of a `SELECT` without `FROM` as a
/// column alias, then fails on what follows. The text is then `Unknown` —
/// mutating, so it asks for a confirmation it does not need, never the other
/// way round.
#[test]
fn a_settings_clause_the_parser_misreads_stays_mutating() {
    let outcome = classify("SELECT 1 SETTINGS max_threads = 8", DIALECT);
    assert_eq!(outcome.intent, StatementIntent::Unknown, "{outcome:?}");
    assert!(outcome.is_mutating());
}

#[test]
fn destructive_forms_carry_their_risk() {
    assert_eq!(
        classify("TRUNCATE TABLE t", DIALECT).risk,
        MutationRisk::Truncate
    );
    assert_eq!(
        classify("DROP TABLE events.hits", DIALECT).risk,
        MutationRisk::DropObject
    );
    assert_eq!(
        classify("DELETE FROM t WHERE 1 = 1", DIALECT).risk,
        MutationRisk::UnboundedDelete
    );
    assert_eq!(
        classify("GRANT SELECT ON db.* TO u", DIALECT).intent,
        StatementIntent::Grant
    );
}

#[test]
fn use_is_a_context_change_and_quotes_or_comments_hide_nothing() {
    assert!(changes_context("USE system", DIALECT));
    assert!(changes_context("/* x */ use `a\\`b`", DIALECT));
    assert!(!changes_context("SELECT 'use x'", DIALECT));
    assert!(!changes_context("SELECT 1 # USE x", DIALECT));
}

#[test]
fn the_splitter_reads_clickhouse_strings_identifiers_and_comments() {
    // A backslash escapes the quote: the `;` stays in the string.
    assert_eq!(split(r"SELECT 'a\';b'", DIALECT).len(), 1);
    // Backtick and double-quoted identifiers take backslash escapes too.
    assert_eq!(split(r"SELECT `a\`;b`, 1", DIALECT).len(), 1);
    assert_eq!(split(r#"SELECT "a\";b", 1"#, DIALECT).len(), 1);
    // Read MySQL's way, `` `a\`` `` would continue the identifier over the
    // `;` and hide the DROP the server runs as a second statement.
    let hidden = r"SELECT `a\``; DROP TABLE t; -- `";
    assert_eq!(split(hidden, DIALECT).len(), 2);
    assert!(classify(hidden, DIALECT).is_mutating());
    // `#` and `--` comment out to the line's end; block comments nest.
    assert_eq!(split("SELECT 1 # ; DROP TABLE t\n", DIALECT).len(), 1);
    assert_eq!(split("SELECT /* a /* ; */ b */ 1", DIALECT).len(), 1);
    // A heredoc is a string.
    assert_eq!(split("SELECT $t$a;b$t$, $$c;d$$", DIALECT).len(), 1);
    assert_eq!(split("SELECT 1; SELECT 2", DIALECT).len(), 2);
}
