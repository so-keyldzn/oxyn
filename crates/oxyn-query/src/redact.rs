//! Redaction of credential literals before a statement leaves process memory.

use std::{borrow::Cow, ops::Range};

use oxyn_core::{QueryLanguage, SqlDialect};

use crate::split::{SplitProfile, Tok, scan};

/// The stable, readable marker kept in place of a password literal.
pub const REDACTED_LITERAL: &str = "'<redacted>'";

#[derive(Clone, Copy, Default)]
enum StatementKind {
    #[default]
    Other,
    CreateOrAlter,
    RoleOrUser,
    Set,
}

#[derive(Clone, Copy, Default)]
enum SecretClause {
    #[default]
    None,
    Identified,
    IdentifiedWith,
    AwaitingLiteral,
    AwaitingSetLiteral,
}

/// Replaces password string literals in SQL while preserving every other byte.
///
/// PostgreSQL `CREATE`/`ALTER ROLE` and `USER`, MySQL `CREATE`/`ALTER USER`,
/// `IDENTIFIED WITH … BY`, and `SET PASSWORD` are recognized, along with MariaDB
/// user modifiers and `GRANT` authentication. Text in ordinary comments,
/// quoted identifiers, and unrelated string literals is left unchanged. An
/// unterminated password literal is redacted through the end of the text.
/// ANSI (an unspecified dialect) conservatively combines PostgreSQL and MySQL
/// lexical interpretations. This is not a general secret detector: credentials
/// embedded in comments or dynamic SQL are outside the recognized grammar.
#[must_use]
pub fn redact_password_literals(statement: &str, language: QueryLanguage) -> String {
    let dialect = match language {
        QueryLanguage::Sql(dialect) => dialect,
        _ => return statement.to_owned(),
    };
    let mut replacements = Vec::new();
    if dialect == SqlDialect::Ansi {
        collect_literals(statement, SqlDialect::Postgres, &mut replacements);
        collect_literals(statement, SqlDialect::MySql, &mut replacements);
    } else {
        collect_literals(statement, dialect, &mut replacements);
    }
    replacements.sort_unstable_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for span in replacements {
        if let Some(previous) = merged.last_mut()
            && span.start <= previous.end
        {
            previous.end = previous.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    let mut redacted = String::with_capacity(statement.len());
    let mut copied = 0;
    for span in merged {
        if let Some(prefix) = statement.get(copied..span.start) {
            redacted.push_str(prefix);
        }
        redacted.push_str(REDACTED_LITERAL);
        copied = span.end;
    }
    if let Some(suffix) = statement.get(copied..) {
        redacted.push_str(suffix);
    }
    redacted
}

fn collect_literals(statement: &str, dialect: SqlDialect, replacements: &mut Vec<Range<usize>>) {
    let profile = SplitProfile::for_dialect(dialect);
    collect_with_profile(statement, dialect, profile, replacements);
    if profile.backslash_escapes || matches!(dialect, SqlDialect::Postgres | SqlDialect::Redshift) {
        // SQL modes are not carried by persisted records. Keep the union of
        // both interpretations rather than exposing a suffix under either.
        collect_with_profile(
            statement,
            dialect,
            SplitProfile {
                backslash_escapes: !profile.backslash_escapes,
                ..profile
            },
            replacements,
        );
    }
}

fn collect_with_profile(
    statement: &str,
    dialect: SqlDialect,
    profile: SplitProfile,
    replacements: &mut Vec<Range<usize>>,
) {
    let mut kind = StatementKind::Other;
    let mut clause = SecretClause::None;
    let mut password_clause = false;
    let visible = if dialect == SqlDialect::MySql {
        executable_sql(statement, profile)
    } else {
        Cow::Borrowed(statement)
    };

    scan(&visible, profile, &mut |token, span| match token {
        Tok::Semicolon => {
            kind = StatementKind::Other;
            clause = SecretClause::None;
            password_clause = false;
        }
        Tok::Comment | Tok::UnreadableComment => {}
        Tok::Word => {
            let Some(word) = statement.get(span.clone()) else {
                return;
            };
            kind = if word.eq_ignore_ascii_case("CREATE") || word.eq_ignore_ascii_case("ALTER") {
                StatementKind::CreateOrAlter
            } else if matches!(kind, StatementKind::CreateOrAlter) {
                if word.eq_ignore_ascii_case("OR") || word.eq_ignore_ascii_case("REPLACE") {
                    StatementKind::CreateOrAlter
                } else if word.eq_ignore_ascii_case("ROLE") || word.eq_ignore_ascii_case("USER") {
                    StatementKind::RoleOrUser
                } else {
                    StatementKind::Other
                }
            } else if word.eq_ignore_ascii_case("GRANT") {
                StatementKind::RoleOrUser
            } else if word.eq_ignore_ascii_case("SET") && matches!(kind, StatementKind::Other) {
                StatementKind::Set
            } else {
                kind
            };

            clause = match clause {
                SecretClause::Identified if word.eq_ignore_ascii_case("BY") => {
                    SecretClause::AwaitingLiteral
                }
                SecretClause::Identified
                    if word.eq_ignore_ascii_case("WITH") || word.eq_ignore_ascii_case("VIA") =>
                {
                    SecretClause::IdentifiedWith
                }
                SecretClause::IdentifiedWith
                    if word.eq_ignore_ascii_case("BY")
                        || word.eq_ignore_ascii_case("AS")
                        || word.eq_ignore_ascii_case("USING") =>
                {
                    SecretClause::AwaitingLiteral
                }
                _ if matches!(kind, StatementKind::RoleOrUser)
                    && word.eq_ignore_ascii_case("PASSWORD") =>
                {
                    password_clause = true;
                    SecretClause::AwaitingLiteral
                }
                _ if matches!(kind, StatementKind::RoleOrUser)
                    && word.eq_ignore_ascii_case("IDENTIFIED") =>
                {
                    password_clause = true;
                    SecretClause::Identified
                }
                _ if matches!(kind, StatementKind::Set)
                    && word.eq_ignore_ascii_case("PASSWORD")
                    && !matches!(clause, SecretClause::AwaitingLiteral) =>
                {
                    password_clause = true;
                    SecretClause::AwaitingSetLiteral
                }
                _ if password_clause
                    && ["REPLACE", "USING", "AS"]
                        .iter()
                        .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
                {
                    SecretClause::AwaitingLiteral
                }
                SecretClause::AwaitingLiteral
                    if !["E", "N", "U", "PASSWORD"]
                        .iter()
                        .any(|prefix| word.eq_ignore_ascii_case(prefix))
                        && !word.starts_with('_') =>
                {
                    SecretClause::None
                }
                current => current,
            };
        }
        Tok::SingleQuoted if matches!(clause, SecretClause::AwaitingLiteral) => {
            replacements.push(span);
        }
        Tok::Quoted
            if matches!(clause, SecretClause::AwaitingLiteral)
                && statement.get(span.clone()).is_some_and(|literal| {
                    literal.starts_with('$')
                        || (dialect == SqlDialect::MySql && literal.starts_with('"'))
                }) =>
        {
            replacements.push(span);
        }
        Tok::Symbol
            if matches!(clause, SecretClause::AwaitingSetLiteral)
                && statement.get(span.clone()) == Some("=") =>
        {
            clause = SecretClause::AwaitingLiteral;
        }
        Tok::Symbol
            if matches!(clause, SecretClause::AwaitingLiteral)
                && statement
                    .get(span.clone())
                    .is_some_and(|symbol| [",", ")", "@"].contains(&symbol)) =>
        {
            clause = SecretClause::None;
        }
        Tok::Symbol | Tok::SingleQuoted | Tok::Quoted => {}
    });
}

/// Keep original byte offsets while exposing executable comment bodies to the
/// shared lexer. Changing only ASCII delimiters preserves valid UTF-8.
fn executable_sql(statement: &str, profile: SplitProfile) -> Cow<'_, str> {
    let mut unmasked: Option<Vec<u8>> = None;
    scan(statement, profile, &mut |token, span| {
        let Some(comment) = statement.get(span.clone()) else {
            return;
        };
        if token != Tok::Comment {
            return;
        }
        let prefix_len = if comment.starts_with("/*!") {
            3
        } else if comment.starts_with("/*M!") {
            4
        } else {
            return;
        };
        let bytes = unmasked.get_or_insert_with(|| statement.as_bytes().to_vec());
        if let Some(prefix) = bytes.get_mut(span.start..span.start + prefix_len) {
            prefix.fill(b' ');
        }
        if comment.ends_with("*/")
            && let Some(suffix) = bytes.get_mut(span.end.saturating_sub(2)..span.end)
        {
            suffix.fill(b' ');
        }
    });
    match unmasked {
        Some(bytes) => Cow::Owned(String::from_utf8_lossy(&bytes).into_owned()),
        None => Cow::Borrowed(statement),
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::{QueryLanguage, SqlDialect};
    use rstest::rstest;

    use super::{REDACTED_LITERAL, redact_password_literals};

    #[rstest]
    #[case(SqlDialect::Postgres, "ALTER ROLE app PASSWORD 'witness-secret'")]
    #[case(SqlDialect::Postgres, "CREATE USER app PASSWORD E'witness-secret'")]
    #[case(SqlDialect::MySql, "CREATE USER u IDENTIFIED BY 'witness-secret'")]
    #[case(
        SqlDialect::MySql,
        "ALTER USER u IDENTIFIED WITH caching_sha2_password BY 'witness-secret'"
    )]
    #[case(SqlDialect::MySql, "SET PASSWORD FOR u = 'witness-secret'")]
    #[case(SqlDialect::MySql, "SET PASSWORD = PASSWORD('witness-secret')")]
    #[case(
        SqlDialect::MySql,
        "/*M! CREATE USER u IDENTIFIED BY 'witness-secret' */"
    )]
    #[case(
        SqlDialect::MySql,
        "CREATE OR REPLACE USER u IDENTIFIED BY 'witness-secret'"
    )]
    #[case(
        SqlDialect::MySql,
        "GRANT SELECT ON db.* TO u IDENTIFIED BY 'witness-secret'"
    )]
    #[case(
        SqlDialect::MySql,
        "ALTER USER u IDENTIFIED VIA ed25519 USING PASSWORD('witness-secret')"
    )]
    #[case(
        SqlDialect::MySql,
        "CREATE USER u IDENTIFIED WITH plugin AS 'witness-secret'"
    )]
    #[case(
        SqlDialect::MySql,
        "/*! CREATE USER u IDENTIFIED BY 'witness-secret' */"
    )]
    #[case(
        SqlDialect::MySql,
        "CREATE USER u /*! IDENTIFIED BY 'witness-secret' */"
    )]
    #[case(
        SqlDialect::MySql,
        "SET PASSWORD FOR 'u'@'localhost' = 'witness-secret'"
    )]
    #[case(SqlDialect::Postgres, "ALTER ROLE app PASSWORD $pw$witness-secret$pw$")]
    #[case(SqlDialect::MySql, "CREATE USER u IDENTIFIED BY \"witness-secret\"")]
    #[case(
        SqlDialect::Postgres,
        "ALTER ROLE app PASSWORD 'first'\n'witness-secret'"
    )]
    #[case(
        SqlDialect::MySql,
        "ALTER USER u IDENTIFIED BY 'new' REPLACE 'witness-secret'"
    )]
    #[case(
        SqlDialect::MySql,
        "CREATE USER u IDENTIFIED BY _utf8mb4'witness-secret'"
    )]
    #[case(
        SqlDialect::MySql,
        "CREATE USER u IDENTIFIED BY 'escaped\\'witness-secret'"
    )]
    #[case(
        SqlDialect::Ansi,
        "CREATE USER u IDENTIFIED BY 'escaped\\'witness-secret'"
    )]
    #[case(
        SqlDialect::Postgres,
        "/* rotation */ ALTER /* role */ ROLE app PASSWORD 'witness-secret'"
    )]
    #[case(SqlDialect::Postgres, "ALTER ROLE app PASSWORD 'witness-secret")]
    #[case(SqlDialect::Postgres, "ALTER ROLE app PASSWORD U&'witness-secret'")]
    #[case(SqlDialect::Ansi, "Executed: ALTER ROLE app PASSWORD 'witness-secret'")]
    fn redacts_password_grammar(#[case] dialect: SqlDialect, #[case] sql: &str) {
        let redacted = redact_password_literals(sql, QueryLanguage::Sql(dialect));
        assert!(!redacted.contains("witness-secret"), "{redacted}");
        assert!(redacted.contains(REDACTED_LITERAL), "{redacted}");
    }

    #[rstest]
    #[case("SELECT password, 'visible' FROM users")]
    #[case("SELECT 'ALTER ROLE app PASSWORD ''visible''' AS example")]
    #[case("-- ALTER ROLE app PASSWORD 'visible'\nSELECT 1")]
    #[case("CREATE TABLE password (value TEXT DEFAULT 'visible')")]
    fn leaves_unrelated_literals_unchanged(#[case] sql: &str) {
        assert_eq!(
            redact_password_literals(sql, QueryLanguage::Sql(SqlDialect::Postgres)),
            sql
        );
    }

    #[test]
    fn redacts_each_statement_without_reformatting_the_batch() {
        let sql = "ALTER ROLE app PASSWORD 'one';\nCREATE USER u IDENTIFIED BY 'two';";
        assert_eq!(
            redact_password_literals(sql, QueryLanguage::Sql(SqlDialect::Postgres)),
            "ALTER ROLE app PASSWORD '<redacted>';\nCREATE USER u IDENTIFIED BY '<redacted>';"
        );
    }
}
