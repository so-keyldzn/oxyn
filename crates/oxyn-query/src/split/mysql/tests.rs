use oxyn_core::SqlDialect;
use rstest::rstest;

use crate::split::{current_statement, split};

/// The fragments of a MySQL batch, after checking their spans lead back to
/// their text: a fragment that drops a delimiter must not drift its bounds.
fn texts(sql: &str) -> Vec<&str> {
    split(sql, SqlDialect::MySql)
        .into_iter()
        .map(|fragment| {
            assert_eq!(sql.get(fragment.span.clone()), Some(fragment.text));
            fragment.text
        })
        .collect()
}

/// A statement of the body must never come out alone: the whole program is
/// the first fragment, and what follows the program is the second.
fn assert_program_then_select(sql: &str) {
    let fragments = texts(sql);
    assert_eq!(fragments.len(), 2, "{fragments:#?}");
    let program = sql
        .rfind("; SELECT 2")
        .expect("the test batch ends with `; SELECT 2`");
    assert_eq!(
        fragments.first().copied(),
        sql.get(..program).map(str::trim)
    );
    assert_eq!(fragments.get(1).copied(), Some("SELECT 2"));
}

#[test]
fn a_procedure_body_is_one_fragment() {
    let sql = "CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END";
    assert_eq!(texts(sql), [sql]);
    assert_program_then_select(&format!("{sql}; SELECT 2"));
}

#[test]
fn a_plain_mysql_batch_still_splits() {
    assert_eq!(
        texts("SELECT 1; DELETE FROM t"),
        ["SELECT 1", "DELETE FROM t"]
    );
    // `BEGIN` alone opens a transaction, not a block.
    assert_eq!(
        texts("BEGIN; DELETE FROM t; COMMIT"),
        ["BEGIN", "DELETE FROM t", "COMMIT"]
    );
    // Block words that follow no program keyword count for nothing.
    assert_eq!(
        texts("CREATE TABLE t (begin INT, loop_count INT); DELETE FROM t"),
        [
            "CREATE TABLE t (begin INT, loop_count INT)",
            "DELETE FROM t"
        ]
    );
}

#[rstest]
#[case::nested_begin(
    "CREATE PROCEDURE p() BEGIN DECLARE EXIT HANDLER FOR SQLEXCEPTION BEGIN ROLLBACK; END; \
     BEGIN SELECT 1; END; DELETE FROM t; END; SELECT 2"
)]
#[case::case_statement(
    "CREATE PROCEDURE p(x INT) BEGIN CASE x WHEN 1 THEN DELETE FROM t; ELSE SELECT 1; END CASE; \
     UPDATE t SET a = 1; END; SELECT 2"
)]
#[case::case_expression(
    "CREATE PROCEDURE p() BEGIN SELECT CASE WHEN a > 0 THEN 1 ELSE 0 END AS s FROM t; \
     DELETE FROM t; END; SELECT 2"
)]
#[case::case_expression_holding_if_function(
    "CREATE PROCEDURE p() BEGIN SELECT CASE WHEN a THEN IF(b, 1, 2) ELSE 3 END FROM t; \
     DELETE FROM u; END; SELECT 2"
)]
#[case::case_expression_inside_case_statement(
    "CREATE PROCEDURE p(v INT) BEGIN CASE v WHEN 1 THEN SET v = CASE WHEN v > 0 THEN 1 END; \
     DELETE FROM t; END CASE; END; SELECT 2"
)]
#[case::if_statement(
    "CREATE PROCEDURE p(x INT) BEGIN IF x > 0 THEN DELETE FROM t; ELSEIF x < 0 THEN \
     UPDATE t SET a = IF(x, 1, 2); ELSE DROP TABLE IF EXISTS tmp; END IF; END; SELECT 2"
)]
#[case::if_with_parenthesized_condition(
    "CREATE PROCEDURE p(x INT) BEGIN IF (x > 0) THEN DELETE FROM t; END IF; END; SELECT 2"
)]
#[case::if_not_exists_condition(
    "CREATE PROCEDURE p() BEGIN IF NOT EXISTS (SELECT 1 FROM t) THEN DELETE FROM u; END IF; \
     END; SELECT 2"
)]
#[case::loops(
    "CREATE PROCEDURE p() BEGIN DECLARE i INT DEFAULT 0; \
     LOOP SET i = i + 1; DELETE FROM t; END LOOP; \
     WHILE i > 0 DO SET i = i - 1; END WHILE; \
     REPEAT SET i = i + 1; SELECT REPEAT('a', 3); UNTIL i > 3 END REPEAT; END; SELECT 2"
)]
#[case::labels(
    "CREATE PROCEDURE p() outer_block: BEGIN inner_loop: LOOP DELETE FROM t; \
     LEAVE inner_loop; END LOOP inner_loop; END outer_block; SELECT 2"
)]
#[case::column_named_end(
    "CREATE PROCEDURE p() BEGIN SELECT start, end FROM t; DELETE FROM u; END; SELECT 2"
)]
#[case::keywords_in_strings_and_comments(
    "CREATE PROCEDURE p() BEGIN SELECT 'END'; -- END;\n DELETE FROM t; /* END; */ END; SELECT 2"
)]
#[case::mariadb_for_loop(
    "CREATE PROCEDURE p() BEGIN FOR i IN 1..3 DO DELETE FROM t; END FOR; END; SELECT 2"
)]
#[case::mariadb_anonymous_block("BEGIN NOT ATOMIC SELECT 1; DELETE FROM t; END; SELECT 2")]
fn a_body_stays_in_its_program(#[case] sql: &str) {
    assert_program_then_select(sql);
}

#[rstest]
#[case::definer_quoted(
    "CREATE DEFINER = `root`@`localhost` PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END; SELECT 2"
)]
#[case::function_with_definer(
    "CREATE DEFINER=root@localhost FUNCTION f() RETURNS INT DETERMINISTIC \
     BEGIN DELETE FROM t; RETURN 1; END; SELECT 2"
)]
#[case::trigger_with_definer(
    "CREATE DEFINER = 'admin'@'%' TRIGGER tr BEFORE INSERT ON t FOR EACH ROW \
     BEGIN DELETE FROM audit; SET NEW.a = 1; END; SELECT 2"
)]
#[case::event_with_definer(
    "CREATE DEFINER = CURRENT_USER EVENT e ON SCHEDULE EVERY 1 DAY DO \
     BEGIN DELETE FROM t; END; SELECT 2"
)]
#[case::event_without_definer(
    "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO BEGIN DELETE FROM t; SELECT 1; END; SELECT 2"
)]
#[case::or_replace("CREATE OR REPLACE PROCEDURE p() BEGIN DELETE FROM t; END; SELECT 2")]
#[case::if_not_exists("CREATE PROCEDURE IF NOT EXISTS p() BEGIN DELETE FROM t; END; SELECT 2")]
#[case::alter_event("ALTER EVENT e DO BEGIN DELETE FROM t; SELECT 1; END; SELECT 2")]
#[case::trigger_body_without_begin(
    "CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW IF NEW.a > 0 THEN SET NEW.b = 1; \
     END IF; SELECT 2"
)]
fn every_kind_of_program_keeps_its_body(#[case] sql: &str) {
    assert_program_then_select(sql);
}

#[test]
fn a_program_without_a_block_splits_at_its_semicolon() {
    assert_eq!(
        texts("CREATE FUNCTION f() RETURNS INT RETURN 1; SELECT 2"),
        ["CREATE FUNCTION f() RETURNS INT RETURN 1", "SELECT 2"]
    );
    assert_eq!(
        texts("CREATE FUNCTION f(x INT) RETURNS INT RETURN CASE WHEN x > 0 THEN 1 END; SELECT 2"),
        [
            "CREATE FUNCTION f(x INT) RETURNS INT RETURN CASE WHEN x > 0 THEN 1 END",
            "SELECT 2"
        ]
    );
    assert_eq!(
        texts("CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO DELETE FROM t; SELECT 2"),
        [
            "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO DELETE FROM t",
            "SELECT 2"
        ]
    );
}

/// A closing word that matches nothing, and a block never closed: both merge
/// to the end, rather than guess where the body stopped.
#[rstest]
#[case::end_closes_an_if("CREATE PROCEDURE p() BEGIN IF x THEN DELETE FROM t; END; SELECT 2")]
#[case::end_if_without_if("CREATE PROCEDURE p() BEGIN DELETE FROM t; END IF; END; SELECT 2")]
#[case::unclosed("CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t")]
fn a_misread_body_merges_the_rest(#[case] sql: &str) {
    assert_eq!(texts(sql), [sql]);
}

// ── DELIMITER ────────────────────────────────────────────────────────────────

#[rstest]
#[case::dollars("$$")]
#[case::slashes("//")]
fn a_delimiter_block_ends_statements_with_its_delimiter(#[case] delimiter: &str) {
    let sql = format!(
        "DELIMITER {delimiter}\n\
         CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  DELETE FROM t;\nEND{delimiter}\n\
         CREATE FUNCTION f() RETURNS INT RETURN 1 {delimiter}\n\
         SELECT 1; SELECT 2{delimiter}\n\
         delimiter ;\n\
         SELECT 3;\nSELECT 4"
    );
    let fragments = texts(&sql);
    assert_eq!(
        fragments,
        [
            "CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  DELETE FROM t;\nEND",
            "CREATE FUNCTION f() RETURNS INT RETURN 1",
            // Under a custom delimiter, `;` separates nothing: the server
            // refuses the pair, as it would from the `mysql` client.
            "SELECT 1; SELECT 2",
            "SELECT 3",
            "SELECT 4",
        ]
    );
    let all = split(&sql, SqlDialect::MySql);
    assert!(all.iter().take(3).all(|fragment| fragment.terminated));
}

#[test]
fn a_directive_accepts_leading_blanks_and_any_case() {
    assert_eq!(
        texts("  \tDeLiMiTeR ||\nSELECT 1; SELECT 2||\nDELIMITER ;\nSELECT 3"),
        ["SELECT 1; SELECT 2", "SELECT 3"]
    );
}

/// The `mysql` client sends what it holds before it obeys the line.
#[test]
fn a_directive_ends_the_statement_before_it() {
    let fragments = split("SELECT 1\nDELIMITER $$\nSELECT 2$$", SqlDialect::MySql);
    let texts: Vec<&str> = fragments.iter().map(|fragment| fragment.text).collect();
    assert_eq!(texts, ["SELECT 1", "SELECT 2"]);
    assert!(fragments.first().is_some_and(|first| !first.terminated));
}

#[rstest]
#[case::inside_a_string("SELECT '\nDELIMITER $$\n'; SELECT 2", 2)]
#[case::inside_a_block_comment("/*\nDELIMITER $$\n*/ SELECT 1; SELECT 2", 2)]
#[case::inside_a_line_comment("-- DELIMITER $$\nSELECT 1; SELECT 2", 2)]
#[case::not_first_on_its_line("SELECT 1; DELIMITER $$\nSELECT 2$$", 2)]
#[case::without_a_delimiter("DELIMITER\nSELECT 1; SELECT 2", 2)]
#[case::glued_to_its_delimiter("DELIMITER$$\nSELECT 1; SELECT 2", 2)]
fn not_every_delimiter_word_is_a_directive(#[case] sql: &str, #[case] expected: usize) {
    let fragments = texts(sql);
    assert_eq!(fragments.len(), expected, "{fragments:#?}");
    // Not a directive, so not stripped: the word stays in the text it is part of.
    assert!(
        fragments.iter().any(|text| text.contains("DELIMITER")),
        "{fragments:#?}"
    );
}

#[test]
fn a_directive_is_only_a_mysql_thing() {
    // `//`, not `$$`: PostgreSQL reads `$$ … $$` as a dollar body.
    let sql = "DELIMITER //\nSELECT 1; SELECT 2//";
    assert_eq!(split(sql, SqlDialect::Postgres).len(), 2);
    let body = "CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END";
    assert_eq!(split(body, SqlDialect::Postgres).len(), 3);
    assert_eq!(split(body, SqlDialect::Ansi).len(), 3);
}

// ── current_statement ────────────────────────────────────────────────────────

#[rstest]
#[case::plain("CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END; SELECT 2")]
#[case::delimited(
    "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END$$\nDELIMITER ;\nSELECT 2"
)]
fn the_current_statement_inside_a_body_is_the_whole_program(#[case] sql: &str) {
    for needle in ["SELECT 1", "DELETE", "END", "BEGIN"] {
        let cursor = sql.find(needle).expect("the needle is in the body");
        let fragment = current_statement(sql, SqlDialect::MySql, cursor)
            .expect("a closed program is selectable")
            .expect("the cursor is in the program");
        assert_eq!(
            fragment.text, "CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END",
            "{needle}"
        );
    }
    let last = current_statement(sql, SqlDialect::MySql, sql.len())
        .expect("a plain select")
        .expect("the last statement");
    assert_eq!(last.text, "SELECT 2");
}

#[test]
fn the_current_statement_is_a_program_the_parser_cannot_read() {
    let sql = "CREATE FUNCTION f() RETURNS INT RETURN 1; SELECT 2";
    let fragment = current_statement(sql, SqlDialect::MySql, 3)
        .expect("a program without a block is complete")
        .expect("the function");
    assert_eq!(fragment.text, "CREATE FUNCTION f() RETURNS INT RETURN 1");
}

#[test]
fn an_open_body_is_not_a_current_statement() {
    let sql = "CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t";
    let cursor = sql.find("DELETE").expect("the body");
    assert!(current_statement(sql, SqlDialect::MySql, cursor).is_err());
}

#[test]
fn a_directive_line_is_no_statement() {
    let sql = "DELIMITER $$\nSELECT 1$$";
    assert_eq!(
        current_statement(sql, SqlDialect::MySql, 3).expect("a valid cursor"),
        None
    );
}
