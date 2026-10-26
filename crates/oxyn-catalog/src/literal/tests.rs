use super::*;

#[test]
fn an_identifier_with_a_control_character_is_refused() {
    assert_eq!(check_identifier(r#"users"; DROP TABLE audit; --"#), Ok(()));
    assert_eq!(check_identifier("a\0b"), Err(LiteralError::Nul));
    for (name, code) in [
        ("\u{1b}[201~x", 0x1b),
        ("a\nb", 0x0a),
        ("\t", 0x09),
        ("\u{9b}", 0x9b),
    ] {
        assert_eq!(
            check_identifier(name),
            Err(LiteralError::ControlCharacter { code })
        );
    }
}

#[test]
fn bigquery_writes_line_breaks_as_escapes() {
    assert_eq!(
        literal("a\r\nb\tc", SqlDialect::BigQuery).expect("line breaks are data"),
        r"'a\r\nb\tc'"
    );
}

/// The value of the task that motivated this module: a cell that closes
/// its literal and runs a statement if concatenated.
const HOSTILE: &str = "'); DROP TABLE x; --";

fn literal(value: &str, dialect: SqlDialect) -> Result<String, LiteralError> {
    let mut out = String::new();
    push_string_literal(&mut out, value, dialect).map(|()| out)
}

const EVERY_DIALECT: [SqlDialect; 11] = [
    SqlDialect::Ansi,
    SqlDialect::Postgres,
    SqlDialect::MySql,
    SqlDialect::Sqlite,
    SqlDialect::SqlServer,
    SqlDialect::Oracle,
    SqlDialect::ClickHouse,
    SqlDialect::DuckDb,
    SqlDialect::Snowflake,
    SqlDialect::BigQuery,
    SqlDialect::Redshift,
];

#[test]
fn a_hostile_value_stays_one_literal_in_every_dialect() {
    assert_eq!(
        literal(HOSTILE, SqlDialect::Postgres).expect("no control"),
        "'''); DROP TABLE x; --'"
    );
    assert_eq!(
        literal(HOSTILE, SqlDialect::MySql).expect("no control"),
        "'''); DROP TABLE x; --'"
    );
    assert_eq!(
        literal(HOSTILE, SqlDialect::SqlServer).expect("no control"),
        "N'''); DROP TABLE x; --'"
    );
    assert_eq!(
        literal(HOSTILE, SqlDialect::BigQuery).expect("no control"),
        "'\\'); DROP TABLE x; --'"
    );
    // Whatever the form, reading the literal back the dialect's way gives
    // the value and consumes the whole text.
    for dialect in EVERY_DIALECT {
        let text = literal(HOSTILE, dialect).expect("no control");
        assert_eq!(
            read_back(&text, dialect),
            Some(HOSTILE.to_owned()),
            "{dialect:?}"
        );
    }
}

#[test]
fn a_backslash_is_escaped_where_the_dialect_reads_it() {
    let value = r"C:\temp\'); DROP TABLE x; --";
    assert_eq!(
        literal(value, SqlDialect::MySql).expect("no control"),
        r"'C:\\temp\\''); DROP TABLE x; --'",
        "MySQL: doubled backslash, doubled quote — safe with or without NO_BACKSLASH_ESCAPES"
    );
    assert_eq!(
        literal(value, SqlDialect::Postgres).expect("no control"),
        r"E'C:\\temp\\''); DROP TABLE x; --'",
        "PostgreSQL: an escape string, whatever standard_conforming_strings says"
    );
    assert_eq!(
        literal(r"a\b", SqlDialect::Sqlite).expect("no control"),
        r"'a\b'",
        "SQLite reads a backslash as itself"
    );
    assert_eq!(
        literal("plain", SqlDialect::Postgres).expect("no control"),
        "'plain'",
        "no backslash, no escape string"
    );
    for dialect in EVERY_DIALECT {
        for value in [value, r"\", r"\\'", r"end\"] {
            let text = literal(value, dialect).expect("no control");
            assert_eq!(
                read_back(&text, dialect),
                Some(value.to_owned()),
                "{dialect:?} {value}"
            );
        }
    }
}

#[test]
fn nul_and_control_characters_are_refused_but_line_breaks_are_data() {
    for dialect in EVERY_DIALECT {
        assert_eq!(literal("a\0b", dialect), Err(LiteralError::Nul));
        assert_eq!(
            literal("\u{1b}[201~DROP TABLE x;", dialect),
            Err(LiteralError::ControlCharacter { code: 0x1b })
        );
        assert_eq!(
            literal("\u{7f}", dialect),
            Err(LiteralError::ControlCharacter { code: 0x7f })
        );
        assert_eq!(
            literal("\u{9b}", dialect),
            Err(LiteralError::ControlCharacter { code: 0x9b })
        );
        let multiline = "line one\r\nline\ttwo";
        let text = literal(multiline, dialect).expect("line breaks are data");
        assert_eq!(read_back(&text, dialect), Some(multiline.to_owned()));
    }
    assert!(
        !LiteralError::ControlCharacter { code: 0x1b }
            .to_string()
            .contains('\u{1b}'),
        "the message names the code point, never the character"
    );
}

#[test]
fn a_refused_value_leaves_the_buffer_untouched() {
    let mut out = String::from("VALUES (");
    assert!(push_string_literal(&mut out, "x\0", SqlDialect::Postgres).is_err());
    assert_eq!(out, "VALUES (");
    push_string_literal(&mut out, "it's", SqlDialect::Postgres).expect("no control");
    assert_eq!(out, "VALUES ('it''s'");
}

/// Reads one literal the way `dialect` does, and `None` unless it spans
/// the whole text: a literal that closes early leaves text behind.
fn read_back(text: &str, dialect: SqlDialect) -> Option<String> {
    let escaping = Escaping::of(dialect);
    let mut chars = text.chars().peekable();
    let backslash_escapes = match escaping {
        Escaping::Backslash | Escaping::BackslashQuote => true,
        Escaping::PostgresEscape => chars.next_if_eq(&'E').is_some(),
        Escaping::Standard => false,
        Escaping::National => {
            chars.next_if_eq(&'N')?;
            false
        }
    };
    if chars.next()? != '\'' {
        return None;
    }
    let mut value = String::new();
    loop {
        match chars.next()? {
            '\\' if backslash_escapes => value.push(match chars.next()? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            }),
            '\'' if chars.next_if_eq(&'\'').is_some() => value.push('\''),
            '\'' => break,
            c => value.push(c),
        }
    }
    chars.next().is_none().then_some(value)
}
