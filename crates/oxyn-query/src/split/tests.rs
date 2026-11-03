use super::*;

fn texts(sql: &str, dialect: SqlDialect) -> Vec<String> {
    split(sql, dialect)
        .into_iter()
        .map(|f| f.text.to_owned())
        .collect()
}

fn executable(sql: &str) -> String {
    executable_sql(sql, SplitProfile::for_dialect(SqlDialect::MySql)).into_owned()
}

#[test]
fn the_executable_view_blanks_the_markers_and_the_version_gate_only() {
    assert_eq!(executable("/*!50000 USE x */"), "         USE x   ");
    assert_eq!(executable("/*M!100000 USE x */"), "           USE x   ");
    assert_eq!(executable("/*! USE x */"), "    USE x   ");
    assert_eq!(
        executable("SELECT 1 /*!50000 ; DROP t */"),
        "SELECT 1          ; DROP t   "
    );
    // Only the digits that follow the marker are the gate.
    assert_eq!(
        executable("/*!50000 SELECT 5000 */"),
        "         SELECT 5000   "
    );
}

#[test]
fn the_executable_view_keeps_every_offset_and_every_character() {
    let sql = "SELECT 'é' /*!50000 , 'ü' */ FROM `ñ` -- ça";
    let view = executable(sql);
    assert_eq!(view.len(), sql.len());
    for (index, (original, seen)) in sql.bytes().zip(view.bytes()).enumerate() {
        assert!(
            original == seen || (seen == b' ' && original.is_ascii()),
            "byte {index} changed from {original:#x} to {seen:#x}"
        );
    }
}

#[test]
fn the_executable_view_leaves_every_other_comment_alone() {
    for sql in [
        "SELECT 1 /* USE x */",
        "SELECT /*+ USE x */ 1",
        "SELECT '/*!50000 USE x */'",
        "SELECT `/*!50000 USE x */`",
        "-- /*!50000 USE x */\nSELECT 1",
        "# /*!50000 USE x */\nSELECT 1",
    ] {
        let view = executable_sql(sql, SplitProfile::for_dialect(SqlDialect::MySql));
        assert!(matches!(view, Cow::Borrowed(_)), "{sql}");
    }
}

#[test]
fn an_unclosed_executable_comment_is_read_to_the_end() {
    // The server fails the statement; reading the body as code costs a
    // confirmation at worst.
    assert_eq!(executable("/*!50000 USE x"), "         USE x");
}

#[test]
fn current_statement_syntax_errors_keep_document_offsets() {
    let sql = "SELECT 1; SELECT";
    let error = current_statement(sql, SqlDialect::Postgres, sql.len())
        .expect_err("incomplete second statement");
    let QueryError::Syntax { span, .. } = error else {
        panic!("syntax error");
    };
    assert_eq!(span.start, sql.rfind("SELECT").expect("second SELECT"));
    assert_eq!(span.end, sql.len());
}

#[test]
fn current_statement_keeps_the_sqlite_trigger_body() {
    let sql = "CREATE TRIGGER t AFTER INSERT ON x BEGIN INSERT INTO log VALUES(CASE WHEN NEW.id > 0 THEN 1 ELSE 0 END); UPDATE x SET seen = 1; END; SELECT 2";
    let cursor = sql.find("UPDATE x").expect("trigger body");
    let fragment = current_statement(sql, SqlDialect::Sqlite, cursor)
        .expect("valid trigger")
        .expect("trigger selection");
    assert!(fragment.text.starts_with("CREATE TRIGGER"));
    assert!(fragment.text.contains("UPDATE x SET seen"));
    assert!(!fragment.text.contains("SELECT 2"));
}

#[test]
fn current_statement_refuses_ambiguous_positions_and_texts() {
    let sql = "SELECT 'é';  SELECT 2";
    assert!(current_statement(sql, SqlDialect::Postgres, 9).is_err());
    assert_eq!(
        current_statement(sql, SqlDialect::Postgres, 13).expect("second space"),
        None
    );
    assert!(current_statement("SELECT 'unfinished", SqlDialect::Postgres, 4).is_err());
}

#[test]
fn current_statement_ties_the_separator_to_the_previous_statement() {
    let first = "SELECT 1";
    let trailing = "SELECT 1;";
    assert_eq!(
        current_statement(trailing, SqlDialect::Postgres, trailing.len())
            .expect("trailing separator")
            .expect("statement")
            .text,
        first
    );
    let adjacent = "SELECT 1;SELECT 2";
    assert_eq!(
        current_statement(adjacent, SqlDialect::Postgres, first.len())
            .expect("separator")
            .expect("previous")
            .text,
        first
    );
    assert_eq!(
        current_statement(adjacent, SqlDialect::Postgres, first.len() + 1)
            .expect("next statement")
            .expect("next")
            .text,
        "SELECT 2"
    );
    assert_eq!(
        current_statement("SELECT 1;  SELECT 2", SqlDialect::Postgres, 9)
            .expect("first space")
            .expect("previous")
            .text,
        first
    );
    assert!(
        current_statement("SELECT 1;  SELECT 2", SqlDialect::Postgres, 10)
            .expect("second space")
            .is_none()
    );
    let dollar = "SELECT $$;$$;";
    assert_eq!(
        current_statement(dollar, SqlDialect::Postgres, dollar.len())
            .expect("dollar quote")
            .expect("statement")
            .text,
        "SELECT $$;$$"
    );
}

#[test]
fn current_statement_does_not_jump_past_a_leading_comment() {
    for sql in [
        "SELECT 1; -- comment\nDELETE FROM t;",
        "SELECT 1;-- comment\nDELETE FROM t;",
    ] {
        let cursor = sql.find(';').expect("separator") + 1;
        assert_eq!(
            current_statement(sql, SqlDialect::Postgres, cursor)
                .expect("selection")
                .expect("previous statement")
                .text,
            "SELECT 1"
        );
    }
}

#[test]
fn current_statement_refuses_ambiguous_words_in_a_sqlite_trigger() {
    for sql in [
        "CREATE TRIGGER BEGIN AFTER INSERT ON x BEGIN UPDATE x SET id = 1; END;",
        "CREATE TRIGGER t AFTER INSERT ON x BEGIN UPDATE x SET CASE = 1; END;",
    ] {
        assert!(
            current_statement(sql, SqlDialect::Sqlite, 8).is_err(),
            "{sql}"
        );
    }
}

#[test]
fn current_statement_never_isolates_a_sqlite_trigger_body() {
    let trigger = "CREATE TEMP TRIGGER t AFTER INSERT ON source BEGIN SELECT end FROM source; UPDATE other SET id = CASE WHEN NEW.id > 0 THEN 999 ELSE 1 END; END;";
    for needle in ["SELECT end", "UPDATE other", "CASE WHEN", "END;"] {
        let cursor = trigger.find(needle).expect("body token");
        let result = current_statement(trigger, SqlDialect::Sqlite, cursor);
        assert!(
            result.is_err()
                || result
                    .expect("selection")
                    .is_some_and(|fragment| fragment.text.starts_with("CREATE TEMP TRIGGER")),
            "{needle}"
        );
    }
}

#[test]
fn create_table_with_reserved_identifiers_is_not_a_trigger() {
    let sql = "CREATE TABLE things (trigger text, begin integer, end integer); UPDATE things SET end = 1;";
    let cursor = sql.find("UPDATE").expect("update");
    assert_eq!(
        current_statement(sql, SqlDialect::Sqlite, cursor)
            .expect("table statements")
            .expect("update")
            .text,
        "UPDATE things SET end = 1"
    );
}

#[test]
fn a_simple_batch_is_split() {
    assert_eq!(
        texts("SELECT 1; SELECT 2", SqlDialect::Ansi),
        ["SELECT 1", "SELECT 2"]
    );
}

#[test]
fn the_last_semicolon_creates_no_empty_fragment() {
    assert_eq!(texts("SELECT 1;", SqlDialect::Ansi), ["SELECT 1"]);
    assert_eq!(texts("SELECT 1;;;", SqlDialect::Ansi), ["SELECT 1"]);
    assert_eq!(texts("  ;  ", SqlDialect::Ansi), Vec::<String>::new());
}

#[test]
fn a_semicolon_inside_a_string_does_not_separate() {
    assert_eq!(texts("SELECT ';'", SqlDialect::Ansi), ["SELECT ';'"]);
    assert_eq!(
        texts("SELECT 'a;b', 'c;d' FROM t", SqlDialect::Ansi),
        ["SELECT 'a;b', 'c;d' FROM t"]
    );
}

#[test]
fn a_doubled_apostrophe_does_not_close_the_string() {
    assert_eq!(
        texts("SELECT 'it''s naïve ; rest'", SqlDialect::Postgres),
        ["SELECT 'it''s naïve ; rest'"]
    );
}

/// The trap the specification cites: a comment that contains a semicolon.
#[test]
fn a_semicolon_inside_a_comment_does_not_separate() {
    assert_eq!(
        texts("SELECT 1 -- keep ; here\n, 2", SqlDialect::Ansi),
        ["SELECT 1 -- keep ; here\n, 2"]
    );
    assert_eq!(
        texts("SELECT /* ; */ 1", SqlDialect::Ansi),
        ["SELECT /* ; */ 1"]
    );
}

#[test]
fn a_block_comment_nests_in_postgres() {
    assert_eq!(
        texts("SELECT /* a /* ; */ ; */ 1", SqlDialect::Postgres),
        ["SELECT /* a /* ; */ ; */ 1"]
    );
}

#[test]
fn a_quoted_identifier_does_not_separate() {
    assert_eq!(
        texts(r#"SELECT "a;b" FROM t"#, SqlDialect::Postgres),
        [r#"SELECT "a;b" FROM t"#]
    );
    assert_eq!(
        texts("SELECT `a;b` FROM t", SqlDialect::MySql),
        ["SELECT `a;b` FROM t"]
    );
    assert_eq!(
        texts("SELECT [a;b] FROM t", SqlDialect::SqlServer),
        ["SELECT [a;b] FROM t"]
    );
}

/// A `$$ … $$` body almost always contains semicolons: it is the case where
/// a naive split breaks most visibly.
#[test]
fn a_dollar_body_does_not_separate() {
    let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN DELETE FROM t; RETURN 1; END $$ LANGUAGE plpgsql; SELECT 2";
    let fragments = texts(sql, SqlDialect::Postgres);
    assert_eq!(fragments.len(), 2, "{fragments:?}");
    assert!(fragments[0].starts_with("CREATE FUNCTION"));
    assert_eq!(fragments[1], "SELECT 2");
}

#[test]
fn a_named_dollar_tag_is_recognized() {
    let sql = "DO $corps$ BEGIN ; END $corps$; SELECT 1";
    assert_eq!(
        texts(sql, SqlDialect::Postgres),
        ["DO $corps$ BEGIN ; END $corps$", "SELECT 1"]
    );
}

/// `$1` is a parameter placeholder, not the opening of a body: confusing
/// them would swallow the whole rest of the batch.
#[test]
fn a_parameter_placeholder_does_not_open_a_body() {
    assert_eq!(
        texts("SELECT $1; DELETE FROM t", SqlDialect::Postgres),
        ["SELECT $1", "DELETE FROM t"]
    );
}

#[test]
fn mysql_escapes_with_backslash() {
    // `'a\';'` is a single string in MySQL: the backslash protects the
    // apostrophe.
    assert_eq!(
        texts(r"SELECT 'a\';' , 1", SqlDialect::MySql),
        [r"SELECT 'a\';' , 1"]
    );
}

#[test]
fn postgres_does_not_escape_with_backslash_without_prefix() {
    // Without an `E` prefix, `'a\'` is a complete string: the `;` separates.
    assert_eq!(
        texts(r"SELECT 'a\'; SELECT 2", SqlDialect::Postgres),
        [r"SELECT 'a\'", "SELECT 2"]
    );
}

#[test]
fn postgres_escapes_with_the_e_prefix() {
    assert_eq!(
        texts(r"SELECT E'a\';' , 1", SqlDialect::Postgres),
        [r"SELECT E'a\';' , 1"]
    );
    // `table_e` must not be taken for a prefix: the `e` is glued to a
    // word.
    assert_eq!(
        texts(r"SELECT line'a\'; SELECT 2", SqlDialect::Postgres),
        [r"SELECT line'a\'", "SELECT 2"]
    );
}

#[test]
fn mysql_comments_with_the_hash() {
    assert_eq!(
        texts("SELECT 1 # ; nothing\n, 2", SqlDialect::MySql),
        ["SELECT 1 # ; nothing\n, 2"]
    );
}

#[test]
fn an_unclosed_string_merges_rather_than_cuts() {
    // Direction of the error: a single unreadable fragment, hence `Unknown`.
    let fragments = split("SELECT 'oops ; DELETE FROM t", SqlDialect::Ansi);
    assert_eq!(fragments.len(), 1);
}

#[test]
fn the_bounds_designate_the_exact_text() {
    let sql = "  SELECT 1 ;\n  DELETE FROM t  ";
    for fragment in split(sql, SqlDialect::Ansi) {
        assert_eq!(
            sql.get(fragment.span.clone()),
            Some(fragment.text),
            "wrong bounds for {fragment:?}"
        );
    }
}

#[test]
fn the_comment_flag_follows_the_fragment() {
    let fragments = split("SELECT 1; -- note\nSELECT 2", SqlDialect::Ansi);
    assert_eq!(fragments.len(), 2);
    assert!(!fragments[0].has_comment);
    assert!(fragments[1].has_comment);
}

#[test]
fn termination_is_reported() {
    let fragments = split("SELECT 1; SELECT 2", SqlDialect::Ansi);
    assert!(fragments[0].terminated);
    assert!(!fragments[1].terminated);
}

#[test]
fn a_batch_of_comments_only_gives_no_statement() {
    assert!(split("-- nothing\n/* nothing either */", SqlDialect::Ansi).is_empty());
    assert!(contains_comment(
        "-- nothing\n/* nothing either */",
        SqlDialect::Ansi
    ));
    assert!(!contains_comment(
        "SELECT '-- not a comment'",
        SqlDialect::Ansi
    ));
}

#[test]
fn bare_words_ignore_strings_and_comments() {
    let tokens = words(
        "SELECT x /* DROP */ FROM t WHERE note = 'DELETE FROM u' -- TRUNCATE",
        SqlDialect::Ansi,
    );
    let texts: Vec<&str> = tokens.iter().map(|m| m.text).collect();
    assert_eq!(texts, ["SELECT", "x", "FROM", "t", "WHERE", "note"]);
}

#[test]
fn a_function_call_is_flagged() {
    let tokens = words(
        "SELECT TRUNCATE(1.234, 2), truncate  (x)",
        SqlDialect::MySql,
    );
    let seen_tokens: Vec<(&str, bool)> = tokens.iter().map(|m| (m.text, m.call)).collect();
    assert_eq!(
        seen_tokens,
        [
            ("SELECT", false),
            ("TRUNCATE", true),
            ("truncate", true),
            ("x", false),
        ],
        "{tokens:?}"
    );
}

#[test]
fn word_bounds_designate_the_text() {
    let sql = "SELECT café FROM t";
    for word in words(sql, SqlDialect::Ansi) {
        assert_eq!(sql.get(word.span.clone()), Some(word.text));
    }
}
