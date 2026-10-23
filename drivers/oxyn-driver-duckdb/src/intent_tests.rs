//! What DuckDB's own statements count as, before anything runs.
//!
//! The production confirmation (I-02), the agent's refusal (I-07) and the
//! read-only bound all start from the intent `oxyn-query` gives a statement
//! with the DuckDB profile. DuckDB statements reach files, the network and
//! the engine's settings from SQL: none of them may read as a plain read. A
//! statement the classifier cannot prove harmless counts as mutating — the
//! answer is conservative, never the other way round.

use oxyn_core::{SqlDialect, StatementIntent};
use oxyn_query::classify;

use crate::error::Effect;
use crate::stream::effect_of;

/// The single statement of `sql`, classified with the DuckDB profile.
fn effect(sql: &str) -> (Effect, StatementIntent) {
    let statements = classify(sql, SqlDialect::DuckDb).statements;
    assert_eq!(statements.len(), 1, "{sql}: {statements:?}");
    let info = statements.first().expect("one statement");
    (effect_of(info), info.intent)
}

#[test]
fn statements_that_reach_files_the_network_or_settings_never_read() {
    let misread = [
        // A COPY that writes a file is "a SELECT" to a careless reader.
        "COPY t TO 'out.csv'",
        "COPY (SELECT * FROM t) TO 'out.parquet' (FORMAT parquet)",
        "COPY t FROM 'in.csv'",
        "EXPORT DATABASE 'dump'",
        "IMPORT DATABASE 'dump'",
        "ATTACH 'other.duckdb' AS other",
        "ATTACH 'other.duckdb' AS other (READ_ONLY)",
        "DETACH other",
        "INSTALL httpfs",
        "FORCE INSTALL httpfs",
        "LOAD httpfs",
        "SET threads = 4",
        "SET GLOBAL enable_external_access = true",
        "RESET threads",
        "PRAGMA enable_profiling",
        "CHECKPOINT",
        "VACUUM",
        "CALL pragma_version()",
        "CREATE SECRET s (TYPE s3, KEY_ID 'k', SECRET 'v')",
        "CREATE MACRO plus(a, b) AS a + b",
        "EXPLAIN ANALYZE DELETE FROM t",
    ]
    .into_iter()
    .filter_map(|sql| {
        // The intent is what the production confirmation reads; the effect
        // is what the read-only bound and a Stop read. Both must say "not a
        // read".
        let (effect, intent) = effect(sql);
        (effect != Effect::Mutating || intent == StatementIntent::Read)
            .then(|| format!("{sql}: {effect:?}, {intent}"))
    })
    .collect::<Vec<_>>();
    assert!(misread.is_empty(), "read as harmless: {misread:#?}");
}

#[test]
fn attaching_a_database_is_ddl_so_the_tree_refreshes() {
    // The desktop backend invalidates the catalog after a `Ddl` intent: an
    // attached database appears in the tree, a detached one leaves it.
    for sql in [
        "ATTACH 'other.duckdb' AS other",
        "DETACH other",
        "CREATE SCHEMA s",
        "CREATE VIEW v AS SELECT 1",
    ] {
        assert_eq!(effect(sql).1, StatementIntent::Ddl, "{sql}");
    }
}

#[test]
fn transaction_control_is_mutating_though_its_intent_reads() {
    // `oxyn-query` gives `BEGIN` and `COMMIT` the intent of a read and marks
    // them as transaction control; the driver's effect counts the mark, so a
    // read-only execution refuses them and a Stop on them is never a
    // cancellation.
    for sql in ["BEGIN TRANSACTION", "COMMIT", "ROLLBACK"] {
        assert_eq!(effect(sql).0, Effect::Mutating, "{sql}");
    }
}

#[test]
fn reads_stay_reads() {
    for sql in [
        "SELECT * FROM t",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        // Reading a file is a read; whether the file is reachable is the
        // engine's access policy, not the classifier's (`crate::open`).
        "SELECT * FROM read_csv('in.csv')",
        "SELECT * FROM 'in.parquet'",
        "EXPLAIN SELECT * FROM t",
    ] {
        assert_eq!(
            effect(sql),
            (Effect::ReadOnly, StatementIntent::Read),
            "{sql}"
        );
    }
}
