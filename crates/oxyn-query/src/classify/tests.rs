use super::*;
use oxyn_core::ExecLimits;
use rstest::rstest;

fn pg(sql: &str) -> Classification {
    classify(sql, SqlDialect::Postgres)
}

// ── The case table of the basic rules ────────────────────────────────────

#[rstest]
#[case("SELECT 1", StatementIntent::Read)]
#[case("SELECT * FROM clients WHERE id = 1", StatementIntent::Read)]
#[case("WITH r AS (SELECT 1) SELECT * FROM r", StatementIntent::Read)]
#[case("SELECT 1 UNION SELECT 2", StatementIntent::Read)]
#[case("VALUES (1), (2)", StatementIntent::Read)]
#[case("SHOW TABLES", StatementIntent::Read)]
#[case("EXPLAIN SELECT 1", StatementIntent::Read)]
#[case("EXPLAIN ANALYZE SELECT 1", StatementIntent::Read)]
#[case("COPY clients TO '/tmp/x.csv'", StatementIntent::Write)]
#[case("INSERT INTO t (a) VALUES (1)", StatementIntent::Write)]
#[case("UPDATE t SET a = 1 WHERE id = 2", StatementIntent::Write)]
#[case("DELETE FROM t WHERE id = 2", StatementIntent::Write)]
#[case(
    "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN UPDATE SET t.a = s.a",
    StatementIntent::Write
)]
#[case("COPY t FROM '/tmp/x.csv'", StatementIntent::Write)]
#[case("SELECT * INTO backup FROM clients", StatementIntent::Ddl)]
#[case("SELECT a INTO TEMP backup FROM clients", StatementIntent::Ddl)]
#[case(
    "WITH r AS (SELECT 1) SELECT * INTO backup FROM r",
    StatementIntent::Ddl
)]
#[case("CREATE TABLE t (a int)", StatementIntent::Ddl)]
#[case("ALTER TABLE t ADD COLUMN b int", StatementIntent::Ddl)]
#[case("DROP TABLE t", StatementIntent::Ddl)]
#[case("TRUNCATE TABLE t", StatementIntent::Ddl)]
#[case("CREATE INDEX i ON t (a)", StatementIntent::Ddl)]
#[case("GRANT SELECT ON t TO reader", StatementIntent::Grant)]
#[case("REVOKE SELECT ON t FROM reader", StatementIntent::Grant)]
#[case("CREATE ROLE analyst", StatementIntent::Grant)]
#[case("ALTER ROLE analyst WITH LOGIN", StatementIntent::Grant)]
#[case("SET ROLE analyst", StatementIntent::Grant)]
#[case("DROP ROLE analyst", StatementIntent::Grant)]
#[case("VACUUM FULL", StatementIntent::Unknown)]
#[case("this is not SQL", StatementIntent::Unknown)]
fn the_rule_table(#[case] sql: &str, #[case] expected: StatementIntent) {
    let outcome = pg(sql);
    assert_eq!(outcome.intent, expected, "{sql} → {outcome:?}");
}

#[test]
fn copy_server_side_effects_are_mutating_and_stdout_stays_read() {
    for (sql, reason) in [
        (
            "COPY (SELECT 1) TO PROGRAM 'x'",
            "runs a program on the database server",
        ),
        (
            "COPY t TO PROGRAM 'x' WITH (FORMAT csv)",
            "runs a program on the database server",
        ),
        (
            "COPY clients TO '/tmp/x.csv'",
            "writes a file on the database server",
        ),
        (
            "COPY (SELECT 1) TO '/tmp/x.csv' WITH (FORMAT csv)",
            "writes a file on the database server",
        ),
    ] {
        let outcome = pg(sql);
        assert!(outcome.is_mutating(), "{sql}: {outcome:?}");
        assert_eq!(outcome.intent, StatementIntent::Write, "{sql}");
        assert!(
            outcome
                .risk
                .reason()
                .is_some_and(|risk| risk.contains(reason)),
            "{sql}: {outcome:?}"
        );
    }
    for sql in [
        "COPY t TO STDOUT",
        "COPY (SELECT 1) TO STDOUT WITH (FORMAT csv)",
    ] {
        assert_eq!(pg(sql).intent, StatementIntent::Read, "{sql}");
    }
    for sql in [
        "COPY t TO PROGRAM $$x$$",
        "COPY t TO $$file$$",
        "COPY t TO PROGRAM",
        "COPY t TO '/tmp/x.csv' WITH (unsupported_option true)",
    ] {
        let outcome = pg(sql);
        assert!(outcome.is_mutating(), "{sql}: {outcome:?}");
        assert_eq!(outcome.intent, StatementIntent::Unknown, "{sql}");
        assert_eq!(outcome.statements[0].basis, Basis::Unparsed, "{sql}");
    }
}

#[test]
fn describe_and_show_are_reads() {
    assert_eq!(
        classify("DESCRIBE clients", SqlDialect::MySql).intent,
        StatementIntent::Read
    );
    assert_eq!(
        classify("SHOW CREATE TABLE clients", SqlDialect::MySql).intent,
        StatementIntent::Read
    );
}

/// Creating a program runs none of its body: the `DELETE` inside is not
/// the batch's. `sqlparser` reads MySQL triggers, not procedures, functions
/// or events: those are `Unknown`, which the gate treats as `Ddl`.
#[rstest]
#[case::procedure("CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END")]
#[case::delimited(
    "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END$$\nDELIMITER ;"
)]
#[case::function(
    "CREATE DEFINER = `root`@`localhost` FUNCTION f() RETURNS INT BEGIN DELETE FROM t; RETURN 1; END"
)]
#[case::event("CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO BEGIN DELETE FROM t; END")]
#[case::trigger("CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW BEGIN DELETE FROM audit; END")]
fn a_mysql_program_is_not_the_delete_of_its_body(#[case] sql: &str) {
    let outcome = classify(sql, SqlDialect::MySql);
    assert_eq!(outcome.statements.len(), 1, "{outcome:?}");
    assert!(
        matches!(
            outcome.intent,
            StatementIntent::Ddl | StatementIntent::Unknown
        ),
        "{outcome:?}"
    );
    assert_eq!(outcome.risk, MutationRisk::None, "{outcome:?}");
    assert!(outcome.is_mutating());
}

#[test]
fn a_mysql_trigger_is_ddl() {
    let outcome = classify(
        "CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW BEGIN DELETE FROM audit; END",
        SqlDialect::MySql,
    );
    assert_eq!(outcome.intent, StatementIntent::Ddl, "{outcome:?}");
}

/// What the program's body does is what a `CALL` does: the call stays
/// `Unknown`, whatever the batch around it.
#[test]
fn a_mysql_call_is_unknown() {
    let outcome = classify(
        "CREATE PROCEDURE p() BEGIN DELETE FROM t; END; CALL p()",
        SqlDialect::MySql,
    );
    assert_eq!(outcome.statements.len(), 2, "{outcome:?}");
    assert_eq!(outcome.statements[1].intent, StatementIntent::Unknown);
}

#[test]
fn a_mysql_batch_is_still_split() {
    let outcome = classify("SELECT 1; DELETE FROM t", SqlDialect::MySql);
    assert_eq!(outcome.statements.len(), 2, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
}

#[test]
fn rename_is_ddl() {
    assert_eq!(
        classify("RENAME TABLE a TO b", SqlDialect::MySql).intent,
        StatementIntent::Ddl
    );
}

// ── The traps ────────────────────────────────────────────────────────────

/// I-07, word for word: `EXPLAIN ANALYZE` runs the query it analyzes.
#[test]
fn explain_analyze_of_a_delete_is_not_a_read() {
    let outcome = pg("EXPLAIN ANALYZE DELETE FROM orders");
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
    assert!(outcome.is_mutating());
    assert!(!outcome.is_read_only());
}

/// The PostgreSQL form goes through the options and not the bare keyword.
#[test]
fn explain_with_analyze_options_is_not_a_read() {
    for sql in [
        "EXPLAIN (ANALYZE) DELETE FROM orders",
        "EXPLAIN (ANALYZE, VERBOSE) DELETE FROM orders",
        "EXPLAIN (VERBOSE, ANALYZE true) UPDATE t SET a = 1",
    ] {
        let outcome = pg(sql);
        assert_eq!(
            outcome.intent,
            StatementIntent::Write,
            "{sql} → {outcome:?}"
        );
    }
}

/// PostgreSQL also spells it `ANALYSE`, which `sqlparser` reads as an
/// unknown option: that spelling ran the `DELETE` as a read (#246).
#[rstest]
#[case("EXPLAIN (ANALYSE) DELETE FROM orders", MutationRisk::UnboundedDelete)]
#[case("EXPLAIN (analyse) DELETE FROM orders", MutationRisk::UnboundedDelete)]
#[case(
    "EXPLAIN (AnAlYsE true) DELETE FROM orders",
    MutationRisk::UnboundedDelete
)]
#[case(
    "EXPLAIN (VERBOSE, analyse true) UPDATE t SET a = 1",
    MutationRisk::UnboundedUpdate
)]
#[case(
    "EXPLAIN (FORMAT JSON, ANALYSE on) DELETE FROM orders",
    MutationRisk::UnboundedDelete
)]
#[case(
    "EXPLAIN (\"analyze\") DELETE FROM orders",
    MutationRisk::UnboundedDelete
)]
#[case(
    "EXPLAIN (analyze 1) DELETE FROM orders",
    MutationRisk::UnboundedDelete
)]
// The argument is not read: a disabled `ANALYSE` costs a confirmation.
#[case(
    "EXPLAIN (analyse off) DELETE FROM orders",
    MutationRisk::UnboundedDelete
)]
#[case(
    "EXPLAIN (ANALYZE false) DELETE FROM orders",
    MutationRisk::UnboundedDelete
)]
#[case("EXPLAIN (ANALYSE) DELETE FROM t WHERE id = 1", MutationRisk::None)]
fn explain_with_the_british_spelling_is_not_a_read(#[case] sql: &str, #[case] risk: MutationRisk) {
    let outcome = pg(sql);
    assert_eq!(
        outcome.intent,
        StatementIntent::Write,
        "{sql} → {outcome:?}"
    );
    assert_eq!(outcome.risk, risk, "{sql}");
    assert_eq!(outcome.statements[0].basis, Basis::Ast, "{sql}");
    assert!(outcome.is_mutating());
}

/// The other grammars that take PostgreSQL's option list run it the same.
#[rstest]
#[case(SqlDialect::DuckDb)]
#[case(SqlDialect::Ansi)]
#[case(SqlDialect::Redshift)]
fn explain_analyse_is_not_a_read_in_other_grammars(#[case] dialect: SqlDialect) {
    let outcome = classify("EXPLAIN (ANALYSE) DELETE FROM orders", dialect);
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
}

/// The legacy form `EXPLAIN ANALYSE statement` does not parse: it is
/// `Unknown`, hence mutating, never a read.
#[rstest]
#[case("EXPLAIN ANALYSE DELETE FROM orders", SqlDialect::Postgres)]
#[case("explain analyse verbose DELETE FROM orders", SqlDialect::Postgres)]
#[case("EXPLAIN ANALYSE SELECT 1", SqlDialect::Postgres)]
#[case("EXPLAIN ANALYSE DELETE FROM orders", SqlDialect::MySql)]
fn the_legacy_analyse_form_is_unknown(#[case] sql: &str, #[case] dialect: SqlDialect) {
    let outcome = classify(sql, dialect);
    assert_eq!(
        outcome.intent,
        StatementIntent::Unknown,
        "{sql} → {outcome:?}"
    );
    assert_eq!(outcome.statements[0].basis, Basis::Unparsed, "{sql}");
    assert!(outcome.is_mutating());
}

/// An option Oxyn does not know may run the statement: it inherits the
/// statement's intent rather than passing for a read.
#[test]
fn an_unknown_explain_option_inherits_the_statement() {
    let outcome = pg("EXPLAIN (EXECUTE_IT) DELETE FROM orders");
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
}

#[rstest]
#[case("EXPLAIN DELETE FROM orders")]
#[case("EXPLAIN (VERBOSE) DELETE FROM orders")]
#[case("EXPLAIN (verbose true, costs off) DELETE FROM orders")]
#[case("EXPLAIN (SETTINGS, BUFFERS, SUMMARY, MEMORY) UPDATE t SET a = 1")]
#[case("EXPLAIN (GENERIC_PLAN) DELETE FROM orders WHERE id = $1")]
#[case("EXPLAIN (FORMAT JSON) DELETE FROM orders")]
#[case("EXPLAIN (Format yaml, Wal, Timing, Serialize text) DELETE FROM orders")]
#[case("EXPLAIN VERBOSE DELETE FROM orders")]
fn explain_without_analyze_stays_a_read(#[case] sql: &str) {
    let outcome = pg(sql);
    assert_eq!(outcome.intent, StatementIntent::Read, "{sql} → {outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::None, "{sql}");
    assert_eq!(outcome.statements[0].basis, Basis::Ast, "{sql}");
}

/// The same nesting `statement_facts` does not anticipate must meet the
/// net whether or not `EXPLAIN ANALYZE` precedes it. PostgreSQL refuses a
/// `DELETE` in `FROM`; it stands here for any form the AST walk misses.
#[test]
fn the_net_sweeps_an_explain_that_runs_its_statement() {
    let bare = pg("SELECT * FROM (DELETE FROM orders RETURNING *) AS d");
    assert_eq!(bare.statements[0].basis, Basis::KeywordSweep, "{bare:?}");
    for sql in [
        "EXPLAIN ANALYZE SELECT * FROM (DELETE FROM orders RETURNING *) AS d",
        "EXPLAIN (ANALYSE) SELECT * FROM (DELETE FROM orders RETURNING *) AS d",
    ] {
        let outcome = pg(sql);
        assert_eq!(
            outcome.intent,
            StatementIntent::Unknown,
            "{sql} → {outcome:?}"
        );
        assert_eq!(outcome.statements[0].basis, Basis::KeywordSweep, "{sql}");
    }
}

/// `COPY (DELETE … RETURNING *) TO STDOUT` deletes: PostgreSQL copies the
/// output of a data-modifying statement. The net used to be the only thing
/// standing between it and a read; the AST now says what it does.
#[rstest]
#[case(
    "COPY (DELETE FROM orders RETURNING *) TO STDOUT",
    MutationRisk::UnboundedDelete
)]
#[case(
    "COPY (UPDATE t SET a = 1 RETURNING *) TO STDOUT",
    MutationRisk::UnboundedUpdate
)]
#[case(
    "COPY (WITH d AS (DELETE FROM orders RETURNING *) SELECT * FROM d) TO STDOUT",
    MutationRisk::UnboundedDelete
)]
#[case(
    "EXPLAIN ANALYZE COPY (DELETE FROM orders RETURNING *) TO STDOUT",
    MutationRisk::UnboundedDelete
)]
fn a_copy_of_a_mutation_is_not_a_read(#[case] sql: &str, #[case] risk: MutationRisk) {
    let outcome = pg(sql);
    assert_eq!(
        outcome.intent,
        StatementIntent::Write,
        "{sql} → {outcome:?}"
    );
    assert_eq!(outcome.risk, risk, "{sql}");
    assert_eq!(outcome.statements[0].basis, Basis::Ast, "{sql}");
}

/// The server-side destination keeps its own, more serious reason.
#[test]
fn a_copy_of_a_mutation_to_a_program_keeps_the_program_risk() {
    let outcome = pg("COPY (DELETE FROM orders RETURNING *) TO PROGRAM 'x'");
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::CopyServerProgram);
}

/// The statement starts with `WITH` and deletes every row.
#[test]
fn a_with_that_deletes_is_not_a_read() {
    let outcome =
        pg("WITH removed AS (DELETE FROM orders RETURNING *) SELECT count(*) FROM removed");
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
}

#[test]
fn a_with_followed_by_a_delete_is_not_a_read() {
    let outcome =
        pg("WITH targets AS (SELECT id FROM t) DELETE FROM u WHERE id IN (SELECT id FROM targets)");
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::None, "the DELETE is bounded");
}

#[test]
fn a_with_that_updates_is_flagged() {
    let outcome = pg("WITH m AS (UPDATE t SET a = 1 RETURNING *) SELECT * FROM m");
    assert_eq!(outcome.intent, StatementIntent::Write);
    assert_eq!(outcome.risk, MutationRisk::UnboundedUpdate);
}

#[test]
fn invalid_sql_is_unknown_hence_mutating() {
    let outcome = pg("SELEKT * FORM t");
    assert_eq!(outcome.intent, StatementIntent::Unknown);
    assert!(outcome.is_mutating(), "Unknown must count as mutating");
    assert_eq!(outcome.statements.len(), 1);
    assert_eq!(outcome.statements[0].basis, Basis::Unparsed);
    assert!(outcome.statements[0].error.is_some());
    assert!(!outcome.is_fully_understood());
}

/// The semicolon of a comment must not fabricate a second statement — and
/// above all not one that would read as a read.
#[test]
fn a_comment_containing_a_semicolon_does_not_cut() {
    let outcome = pg("DELETE FROM t -- keep ; this\n");
    assert_eq!(outcome.statements.len(), 1, "{outcome:?}");
    assert_eq!(outcome.intent, StatementIntent::Write);
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
}

#[test]
fn a_semicolon_inside_a_string_does_not_cut() {
    let outcome = pg("SELECT ';' FROM t");
    assert_eq!(outcome.statements.len(), 1);
    assert_eq!(outcome.intent, StatementIntent::Read);
}

// ── Scope detection ──────────────────────────────────────────────────────

#[rstest]
#[case("DELETE FROM t", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE 1=1", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE 1 = 1", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE true", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE TRUE", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE (1=1)", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE 'x' = 'x'", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE id = id", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE id = 1 OR 1=1", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE 1=1 AND true", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE NOT false", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE 1 <> 2", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE 2 > 1", MutationRisk::UnboundedDelete)]
#[case("DELETE FROM t WHERE id = 1", MutationRisk::None)]
#[case("DELETE FROM t WHERE 1=1 AND id = 2", MutationRisk::None)]
#[case("DELETE FROM t WHERE active", MutationRisk::None)]
#[case("DELETE FROM t WHERE 1 = 2", MutationRisk::None)]
fn the_scope_of_a_delete(#[case] sql: &str, #[case] expected: MutationRisk) {
    assert_eq!(pg(sql).risk, expected, "{sql}");
}

#[rstest]
#[case("UPDATE t SET a = 1", MutationRisk::UnboundedUpdate)]
#[case("UPDATE t SET a = 1 WHERE 1=1", MutationRisk::UnboundedUpdate)]
#[case("UPDATE t SET a = 1 WHERE true", MutationRisk::UnboundedUpdate)]
#[case("UPDATE t SET a = 1 WHERE id = 2", MutationRisk::None)]
fn the_scope_of_an_update(#[case] sql: &str, #[case] expected: MutationRisk) {
    assert_eq!(pg(sql).risk, expected, "{sql}");
}

#[test]
fn truncate_and_drop_carry_their_risk() {
    assert_eq!(pg("TRUNCATE TABLE t").risk, MutationRisk::Truncate);
    assert_eq!(pg("DROP TABLE t").risk, MutationRisk::DropObject);
    assert_eq!(pg("DROP VIEW v").risk, MutationRisk::DropObject);
    assert_eq!(pg("DROP SCHEMA s CASCADE").risk, MutationRisk::DropObject);
}

#[test]
fn a_homonymous_function_call_triggers_nothing() {
    // `TRUNCATE(x, 2)` is a numeric function, not a table truncation.
    let outcome = classify("SELECT TRUNCATE(1.234, 2)", SqlDialect::MySql);
    assert_eq!(outcome.intent, StatementIntent::Read, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::None);
}

/// `SELECT … FOR UPDATE` locks rows without modifying them: the keyword
/// net must not downgrade it.
#[test]
fn a_row_lock_stays_a_read() {
    let outcome = pg("SELECT * FROM t WHERE id = 1 FOR UPDATE");
    assert_eq!(outcome.intent, StatementIntent::Read, "{outcome:?}");
    assert_eq!(outcome.statements[0].basis, Basis::Ast);
}

/// The net does not apply to an `EXPLAIN`, whose form is already settled:
/// without that, `EXPLAIN DELETE …` would ask for an approval for a plan
/// computation.
#[test]
fn the_net_spares_an_explain() {
    let outcome = pg("EXPLAIN DELETE FROM orders");
    assert_eq!(outcome.intent, StatementIntent::Read, "{outcome:?}");
    assert_eq!(outcome.statements[0].basis, Basis::Ast);
}

#[test]
fn a_mutating_keyword_inside_a_string_triggers_nothing() {
    let outcome = pg("SELECT * FROM audit WHERE action = 'DELETE FROM clients'");
    assert_eq!(outcome.intent, StatementIntent::Read, "{outcome:?}");
}

// ── Aggregating a batch ──────────────────────────────────────────────────

#[test]
fn a_batch_takes_the_highest_intent() {
    let outcome = pg("SELECT 1; UPDATE t SET a = 1 WHERE id = 2; SELECT 2");
    assert_eq!(outcome.intent, StatementIntent::Write);
    assert_eq!(outcome.statements.len(), 3);
    assert_eq!(outcome.statements[0].intent, StatementIntent::Read);
    assert_eq!(outcome.statements[1].intent, StatementIntent::Write);
}

#[test]
fn a_batch_takes_the_most_serious_risk() {
    let outcome = pg("UPDATE t SET a = 1; DROP TABLE u");
    assert_eq!(outcome.intent, StatementIntent::Ddl);
    assert_eq!(outcome.risk, MutationRisk::DropObject);
    assert_eq!(outcome.risky().count(), 2);
}

#[test]
fn an_unparsable_statement_contaminates_the_batch() {
    let outcome = pg("SELECT 1; this is not SQL");
    assert_eq!(outcome.intent, StatementIntent::Unknown);
    assert!(outcome.is_mutating());
}

#[test]
fn unknown_beats_ddl_at_equal_rank() {
    let outcome = pg("DROP TABLE t; this is not SQL");
    assert_eq!(outcome.intent, StatementIntent::Unknown, "{outcome:?}");
    // The `DROP`'s risk stays visible all the same.
    assert_eq!(outcome.risk, MutationRisk::DropObject);
}

#[test]
fn grant_beats_everything() {
    let outcome = pg("SELECT 1; DROP TABLE t; GRANT SELECT ON u TO r");
    assert_eq!(outcome.intent, StatementIntent::Grant);
}

#[test]
fn a_transaction_does_not_inflate_a_read() {
    let outcome = pg("BEGIN; SELECT 1; COMMIT");
    assert_eq!(outcome.intent, StatementIntent::Read, "{outcome:?}");
    assert!(outcome.is_read_only());
}

#[test]
fn a_transaction_does_not_hide_a_write() {
    let outcome = pg("BEGIN; DELETE FROM t; COMMIT");
    assert_eq!(outcome.intent, StatementIntent::Write);
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
}

/// Read, yet settling what the session holds: the gate refuses these to
/// an agent (issue #19). Parsed or not — `END` and `ABORT` are
/// PostgreSQL's own spellings of `COMMIT` and `ROLLBACK`.
#[rstest]
#[case::begin("BEGIN")]
#[case::start("START TRANSACTION READ WRITE")]
#[case::commit("COMMIT")]
#[case::commented_commit("/* end */ commit")]
#[case::end("END")]
#[case::rollback("ROLLBACK")]
#[case::abort("ABORT")]
#[case::savepoint("SAVEPOINT s1")]
#[case::rollback_to("ROLLBACK TO SAVEPOINT s1")]
#[case::release("RELEASE SAVEPOINT s1")]
#[case::prepare_transaction("PREPARE TRANSACTION 'x'")]
#[case::commit_prepared("COMMIT PREPARED 'x'")]
#[case::in_a_batch("SELECT 1; ROLLBACK")]
fn transaction_control_is_flagged(#[case] sql: &str) {
    let outcome = pg(sql);
    assert!(outcome.transaction_control, "{outcome:?}");
    let request = outcome.qualify(ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        sql,
    ));
    assert!(request.transaction_control, "qualify carries it");
}

#[rstest]
#[case::read("SELECT 1")]
#[case::write("DELETE FROM t WHERE id = 1")]
#[case::prepared_statement("PREPARE plan AS SELECT 1")]
#[case::column_name("SELECT commit, rollback FROM journal")]
#[case::string("SELECT 'COMMIT'")]
fn the_rest_is_not(#[case] sql: &str) {
    assert!(!pg(sql).transaction_control, "{sql}");
}

fn mysql_request(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::MySql), sql)
}

/// What settles or opens a transaction, and nothing else: left out of
/// the read-only bound of a production read (issue #182).
#[rstest]
#[case::start("START TRANSACTION")]
#[case::begin("BEGIN")]
#[case::commit("COMMIT")]
#[case::commit_and_chain("COMMIT AND CHAIN")]
#[case::rollback("ROLLBACK")]
#[case::rollback_to("ROLLBACK TO SAVEPOINT s1")]
#[case::savepoint("SAVEPOINT s1")]
#[case::release("RELEASE SAVEPOINT s1")]
#[case::commented("/* settle */ commit;")]
#[case::two_verbs("COMMIT; START TRANSACTION")]
fn a_bare_transaction_verb_only_controls_a_transaction(#[case] sql: &str) {
    assert!(only_controls_transactions(&mysql_request(sql)), "{sql}");
}

/// Anything else keeps the bound: a read that may call a writing
/// function, a verb hiding a statement after it, a `BEGIN … END` block, a
/// text that does not parse or is not SQL.
#[rstest]
#[case::read("SELECT audit_touch()")]
#[case::then_a_read("COMMIT; SELECT audit_touch()")]
#[case::before_a_read("SELECT audit_touch(); ROLLBACK")]
#[case::empty("")]
#[case::comment_only("-- COMMIT")]
#[case::unparsed("ROLLBACK WHATEVER")]
fn the_rest_does_not(#[case] sql: &str) {
    assert!(!only_controls_transactions(&mysql_request(sql)), "{sql}");
}

#[test]
fn a_begin_block_does_not_only_control_a_transaction() {
    let request = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::BigQuery),
        "BEGIN SELECT audit_touch(); END",
    );
    assert!(!only_controls_transactions(&request));
    let opaque = ExecRequest::new(QueryLanguage::RedisCommand, "MULTI");
    assert!(!only_controls_transactions(&opaque));
}

#[test]
fn qualify_replaces_what_the_caller_declares() {
    let mut declare = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "COMMIT");
    declare.transaction_control = false;
    assert!(pg("COMMIT").qualify(declare).transaction_control);

    let mut declare = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "SELECT 1");
    declare.transaction_control = true;
    assert!(!pg("SELECT 1").qualify(declare).transaction_control);
}

// ── A typed `USE` (issue #223) ───────────────────────────────────────────

fn mysql(sql: &str) -> Classification {
    classify(sql, SqlDialect::MySql)
}

/// The gate refuses these to an agent whatever the environment, and the
/// MySQL driver refuses them to everyone: both read this one flag.
#[rstest]
#[case::bare("USE shop")]
#[case::lower_case("use shop")]
#[case::quoted_name("use `shop`")]
#[case::commented("/* c */ USE shop")]
#[case::after_a_read("SELECT 1; USE shop")]
#[case::before_a_read("USE shop; SELECT 1")]
#[case::hostile_name("USE `a;b`; SELECT 1")]
#[case::unparsable("USE")]
// The server runs the body of an executable comment: MySQL's `/*!`, and
// MariaDB's `/*M!`, with or without the version gate.
#[case::executable("/*!50000 USE shop */")]
#[case::executable_without_a_gate("/*! USE shop */")]
#[case::mariadb_executable("/*M!100000 USE shop */")]
#[case::executable_after_a_read("SELECT 1; /*!50000 USE shop */")]
#[case::executable_before_a_read("/*!50000 USE shop */; SELECT 1")]
#[case::executable_ending_the_statement("SELECT 1 /*!50000 ; USE shop */")]
#[case::executable_quoted_name("/*!50000 USE `a;b` */")]
fn a_typed_use_changes_the_context(#[case] sql: &str) {
    let outcome = mysql(sql);
    assert!(outcome.context_change, "{sql} → {outcome:?}");
    assert!(changes_context(sql, SqlDialect::MySql), "{sql}");
    let request = outcome.qualify(mysql_request(sql));
    assert!(request.context_change, "qualify carries it: {sql}");
}

#[rstest]
#[case::read("SELECT 1")]
#[case::string("SELECT 'use'")]
#[case::line_comment("-- USE x\nSELECT 1")]
#[case::block_comment("/* USE x */ SELECT 1")]
#[case::quoted_identifier("SELECT `use` FROM t")]
#[case::index_hint("SELECT * FROM t USE INDEX (i)")]
#[case::column_name("SELECT used, user FROM t")]
#[case::transaction("BEGIN")]
#[case::write("DELETE FROM t WHERE id = 1")]
#[case::second_word("SET @use = 1")]
// Dumps carry executable comments all the time; only a leading `USE` in
// one changes the context.
#[case::executable_set("/*!40101 SET NAMES utf8mb4 */")]
#[case::executable_read("/*!50000 SELECT 1 */")]
#[case::executable_hint("SELECT /*!50000 DISTINCT */ a FROM t")]
#[case::executable_index_hint("SELECT * FROM t /*!50000 USE INDEX (i) */")]
#[case::optimizer_hint("SELECT /*+ USE x */ 1")]
#[case::executable_in_a_line_comment("-- /*! USE x */\nSELECT 1")]
#[case::executable_in_a_string("SELECT '/*!50000 USE x */'")]
fn the_rest_does_not_change_the_context(#[case] sql: &str) {
    let outcome = mysql(sql);
    assert!(!outcome.context_change, "{sql} → {outcome:?}");
    assert!(!changes_context(sql, SqlDialect::MySql), "{sql}");
}

/// Checkbox 1 of #223: the classifier no longer takes a context change for
/// a read, which is what let an agent's `USE other` skip the refusal of
/// ADR-0019 and bypass the production confirmation.
#[test]
fn a_typed_use_is_not_a_read() {
    let outcome = mysql("USE shop");
    assert_eq!(outcome.intent, StatementIntent::Unknown, "{outcome:?}");
    assert!(outcome.is_mutating());
    assert!(!outcome.is_read_only());
    assert!(outcome.context_change);
    assert!(!outcome.transaction_control, "a different flag");
    // The part of the family the parser reads as a `USE` is settled by
    // the AST; the others fall back on the same `Unknown`.
    assert_eq!(outcome.statements[0].basis, Basis::Ast, "{outcome:?}");
}

/// The executable-comment view ends a comment where the scanner does, at the
/// first `*/`, and a block comment nested in the body ends it early. The
/// text as typed is the floor: whichever reading shows the statement, the
/// batch is judged by it (fail closed).
#[rstest]
#[case::use_after_a_nested_comment("/*!50000 /* c */ USE other */")]
#[case::mariadb_use_after_a_nested_comment("/*M!100000 /* c */ USE other */")]
#[case::use_before_a_nested_comment("/*!50000 USE /* c */ other */")]
fn a_nested_comment_does_not_hide_a_use(#[case] sql: &str) {
    let outcome = mysql(sql);
    assert!(outcome.context_change, "{sql} → {outcome:?}");
    assert!(changes_context(sql, SqlDialect::MySql), "{sql}");
    assert!(outcome.is_mutating(), "{sql} → {outcome:?}");
}

/// The floor holds for the other facts too: a `DELETE` behind a nested
/// comment is never judged a read, and a `COMMIT` still controls a
/// transaction.
#[test]
fn a_nested_comment_does_not_make_a_batch_milder() {
    for sql in [
        "/*!50000 /* c */ DELETE FROM t */",
        "/*!50000 /* c */ COMMIT */",
    ] {
        let outcome = mysql(sql);
        assert!(outcome.is_mutating(), "{sql} → {outcome:?}");
    }
    assert!(mysql("/*!50000 /* c */ COMMIT */").transaction_control);
}

/// Every dialect, not only MySQL: the classifier is dialect-wide, and a
/// text that leads with `USE` changes a context wherever it is accepted.
#[rstest]
#[case::snowflake_role(SqlDialect::Snowflake, "USE ROLE analyst")]
#[case::snowflake_warehouse(SqlDialect::Snowflake, "USE WAREHOUSE w")]
#[case::snowflake_secondary(SqlDialect::Snowflake, "USE SECONDARY ROLES ALL")]
#[case::duckdb(SqlDialect::DuckDb, "USE memory")]
#[case::sql_server(SqlDialect::SqlServer, "USE master")]
#[case::postgres(SqlDialect::Postgres, "USE shop")]
fn every_form_of_use_is_unknown_and_flagged(#[case] dialect: SqlDialect, #[case] sql: &str) {
    let outcome = classify(sql, dialect);
    assert_eq!(
        outcome.intent,
        StatementIntent::Unknown,
        "{sql} → {outcome:?}"
    );
    assert!(outcome.is_mutating(), "{sql}");
    assert!(outcome.context_change, "{sql} → {outcome:?}");
}

/// The flag must never come from the last statement only: `USE shop;
/// SELECT 1` ends on a read.
#[test]
fn the_context_change_is_raised_by_any_statement_of_the_batch() {
    let outcome = mysql("USE shop; SELECT 1; SELECT 2");
    assert!(outcome.context_change);
    assert_eq!(
        outcome
            .statements
            .iter()
            .map(|statement| statement.context_change)
            .collect::<Vec<_>>(),
        [true, false, false]
    );
}

/// `qualify` replaces what the caller declares, in both directions: an
/// agent cannot declare a `USE` away, and a read cannot be refused as a
/// context change because its caller said so.
#[test]
fn qualify_replaces_the_declared_context_change() {
    let mut declare = mysql_request("USE shop");
    declare.context_change = false;
    assert!(mysql("USE shop").qualify(declare).context_change);

    let mut declare = mysql_request("SELECT 1");
    declare.context_change = true;
    assert!(!mysql("SELECT 1").qualify(declare).context_change);
}

/// `reclassify` reads the request's own text and language, which is what
/// the executor calls: nothing the agent declares reaches the gate.
#[test]
fn an_agent_cannot_declare_a_use_away() {
    let mut declared = mysql_request("USE other").with_intent(StatementIntent::Read);
    declared.context_change = false;
    declared.transaction_control = false;

    let requalified = reclassify(&declared).qualify(declared);

    assert!(requalified.context_change);
    assert_eq!(requalified.intent, StatementIntent::Unknown);
    assert!(requalified.is_mutating());
}

/// A `USE` behind a comment whose end depends on the server's lexer cannot
/// be ruled out, so it counts, as the transaction verbs do: otherwise an
/// agent hides one behind a carriage return.
#[test]
fn an_unreadable_comment_counts_as_a_context_change() {
    let sql = "SELECT 1 -- x\rUSE other";
    let outcome = mysql(sql);
    assert!(
        outcome.statements[0].basis == Basis::Unparsed,
        "{outcome:?}"
    );
    assert!(outcome.context_change, "{outcome:?}");
    assert!(outcome.transaction_control, "the same worst case");
    assert_eq!(outcome.intent, StatementIntent::Unknown);

    // The lexical rule the driver reads sees no leading `USE` there: it
    // refuses what it can see, and the gate refuses the agent.
    assert!(!changes_context(sql, SqlDialect::MySql));
}

#[test]
fn changes_context_reads_every_fragment() {
    assert!(changes_context("SELECT 1; USE x", SqlDialect::MySql));
    assert!(changes_context(
        "SELECT 1;\nUSE x;\nSELECT 2",
        SqlDialect::MySql
    ));
    assert!(!changes_context("SELECT 1; SELECT 2", SqlDialect::MySql));
    assert!(!changes_context("", SqlDialect::MySql));
    assert!(!changes_context("-- USE x", SqlDialect::MySql));
}

/// A procedure body is one statement for the server, `USE` inside it
/// included, and does not lead the fragment: it is the `CREATE`'s business.
#[test]
fn a_use_inside_a_stored_program_is_not_the_leading_verb() {
    let sql = "CREATE PROCEDURE p() BEGIN USE other; SELECT 1; END";
    assert!(!changes_context(sql, SqlDialect::MySql));
    assert!(!mysql(sql).context_change);
}

/// Executable comments are MySQL's and MariaDB's: to any other dialect
/// they are comments, and a refusal there would refuse nothing a server
/// runs.
#[test]
fn an_executable_comment_is_only_code_for_mysql() {
    for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite, SqlDialect::Ansi] {
        for sql in ["/*!50000 USE shop */", "/*M!100000 USE shop */"] {
            let outcome = classify(sql, dialect);
            assert!(!outcome.context_change, "{dialect:?} {sql} → {outcome:?}");
            assert!(!changes_context(sql, dialect), "{dialect:?} {sql}");
        }
        let outcome = classify("/*!50000 COMMIT */", dialect);
        assert!(!outcome.transaction_control, "{dialect:?} → {outcome:?}");
    }
}

/// A `USE` the server runs must not hide behind a comment from the agent
/// refusal of ADR-0019, whatever the caller declares: the batch used to be
/// empty, hence `Unknown` with no flag, and the gate only asked an agent's
/// approval.
#[test]
fn an_agent_cannot_hide_a_use_in_an_executable_comment() {
    for sql in [
        "/*!50000 USE other */",
        "/*M!100000 USE other */",
        "SELECT 1 /*!50000 ; USE other */",
    ] {
        let mut declared = mysql_request(sql).with_intent(StatementIntent::Read);
        declared.context_change = false;

        let requalified = reclassify(&declared).qualify(declared);

        assert!(requalified.context_change, "{sql}");
        assert!(requalified.is_mutating(), "{sql}");
    }
}

#[rstest]
#[case::mysql("/*!50000 COMMIT */")]
#[case::mariadb("/*M!100000 ROLLBACK */")]
#[case::after_a_read("SELECT 1 /*!50000 ; COMMIT */")]
fn a_transaction_verb_in_an_executable_comment_is_flagged(#[case] sql: &str) {
    let outcome = mysql(sql);
    assert!(outcome.transaction_control, "{sql} → {outcome:?}");
    assert!(!outcome.context_change, "a different flag: {sql}");
}

/// The body is classified as the statement it holds, not as a batch with
/// no statement: `DELETE` in a comment is a delete to the server.
#[test]
fn the_body_of_an_executable_comment_is_classified_as_code() {
    let outcome = mysql("/*!50000 DELETE FROM t */");
    assert_eq!(outcome.intent, StatementIntent::Write, "{outcome:?}");
    assert_eq!(outcome.risk, MutationRisk::UnboundedDelete, "{outcome:?}");
    assert_eq!(outcome.len(), 1);

    // The `;` inside the comment ends the statement before it.
    let outcome = mysql("SELECT 1 /*!50000 ; DROP TABLE b */");
    assert_eq!(outcome.len(), 2, "{outcome:?}");
    assert_eq!(outcome.statements[0].intent, StatementIntent::Read);
    assert_eq!(outcome.statements[1].intent, StatementIntent::Ddl);
    assert!(outcome.is_mutating());

    // A harmless dump line stays what it is: no more than a confirmation.
    let outcome = mysql("/*!40101 SET NAMES utf8mb4 */");
    assert_eq!(outcome.intent, StatementIntent::Unknown, "{outcome:?}");
}

/// Reading the body as code must not change what the caller sees: the
/// statement keeps the text the user typed, and its span still leads back to
/// the spot in the buffer.
#[test]
fn a_statement_with_an_executable_comment_keeps_the_typed_text() {
    let sql = "SELECT /*!50000 DISTINCT */ a FROM t; /*!50000 USE shop */";
    let outcome = mysql(sql);
    let [select, using] = outcome.statements.as_slice() else {
        panic!("two statements expected: {outcome:?}");
    };
    assert_eq!(select.text, "SELECT /*!50000 DISTINCT */ a FROM t");
    assert_eq!(sql.get(select.span.clone()), Some(select.text.as_str()));
    assert_eq!(sql.get(using.span.clone()), Some(using.text.as_str()));
    assert!(using.context_change && !select.context_change);
}

/// An empty batch does not read as a read: a split that lost statements
/// must not open the door.
#[test]
fn an_empty_batch_is_unknown() {
    for sql in ["", "   ", ";;", "-- just a note"] {
        let outcome = pg(sql);
        assert_eq!(
            outcome.intent,
            StatementIntent::Unknown,
            "{sql:?} → {outcome:?}"
        );
        assert!(outcome.is_empty());
        assert!(outcome.is_mutating());
        assert!(!outcome.is_fully_understood());
    }
}

// ── Non-SQL languages ────────────────────────────────────────────────────

#[test]
fn a_non_sql_language_is_not_guessed() {
    let outcome = classify_language(QueryLanguage::Cypher, "MATCH (n) RETURN n");
    assert_eq!(outcome.intent, StatementIntent::Unknown);
    assert!(outcome.is_mutating());
    assert_eq!(outcome.statements.len(), 1);
    assert_eq!(outcome.statements[0].basis, Basis::Unparsed);
}

#[test]
fn declared_sql_goes_through_its_dialect() {
    let outcome = classify_language(QueryLanguage::Sql(SqlDialect::MySql), "SELECT 1");
    assert_eq!(outcome.intent, StatementIntent::Read);
}

// ── Requalification ──────────────────────────────────────────────────────

/// The scenario of ARCHITECTURE §8: an agent declares a read, the text
/// says otherwise. The text wins.
#[test]
fn a_wrongly_declared_intent_is_corrected() {
    let requested = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "DELETE FROM clients",
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default());

    let outcome = reclassify(&requested);
    let requalified = outcome.qualify(requested);

    assert_eq!(requalified.intent, StatementIntent::Write);
    assert_eq!(requalified.risk, MutationRisk::UnboundedDelete);
    assert!(requalified.is_mutating());
}

// ── The safety net under the AST ─────────────────────────────────────────

#[test]
fn the_net_recognizes_a_bare_mutating_keyword() {
    assert!(hides_a_mutation(
        "SELECT * FROM (DELETE FROM t)",
        SqlDialect::Postgres
    ));
    assert!(hides_a_mutation("SELECT 1; GRANT", SqlDialect::Postgres));
    assert!(hides_a_mutation(
        "COPY t TO PROGRAM 'x'",
        SqlDialect::Postgres
    ));
}

#[test]
fn the_net_does_not_fire_wrongly() {
    for (sql, target_dialect) in [
        (
            "SELECT * FROM audit WHERE action = 'DELETE FROM t'",
            SqlDialect::Postgres,
        ),
        ("SELECT x /* DROP TABLE t */ FROM t", SqlDialect::Postgres),
        ("SELECT TRUNCATE(1.234, 2)", SqlDialect::MySql),
        ("SELECT * FROM t FOR UPDATE", SqlDialect::Postgres),
        ("SHOW CREATE TABLE t", SqlDialect::MySql),
        ("SELECT deleted_at, drop_date FROM t", SqlDialect::Postgres),
        (r#"SELECT "delete" FROM t"#, SqlDialect::Postgres),
    ] {
        assert!(
            !hides_a_mutation(sql, target_dialect),
            "false positive: {sql}"
        );
    }
}

// ── Strict check ─────────────────────────────────────────────────────────

#[test]
fn the_strict_check_locates_the_culprit() {
    assert!(validate("SELECT 1; SELECT 2", SqlDialect::Postgres).is_ok());

    let sql = "SELECT 1; SELEKT 2";
    let Err(QueryError::Syntax { span, .. }) = validate(sql, SqlDialect::Postgres) else {
        panic!("the faulty statement should have been reported");
    };
    assert_eq!(sql.get(span), Some("SELEKT 2"));
}

#[test]
fn the_bounds_lead_back_to_the_statement() {
    let sql = "SELECT 1;\n  DELETE FROM t";
    let outcome = pg(sql);
    for statement in &outcome.statements {
        assert_eq!(
            sql.get(statement.span.clone()),
            Some(statement.text.as_str())
        );
    }
}
