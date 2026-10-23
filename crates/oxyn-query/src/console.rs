//! What a console runs: the language it speaks to a session, and the request
//! under its cursor.
//!
//! SQL is one language among others (ADR-0003). A console on a session that
//! declares Redis commands sends Redis commands, not SQL that the driver would
//! refuse; and where a request ends is a property of its language, so a text
//! this crate cannot cut is refused, never cut as if it were SQL.
//!
//! # Adding a language
//!
//! [`current_request`] knows SQL only. A language whose request boundaries a
//! module of this crate can find gets **one** arm in its `match`, pointing at
//! that module; every other language keeps the refusal, which asks the user to
//! select what to run.

use oxyn_core::{Capabilities, DriverId, QueryLanguage};

use crate::dialect::dialect_for;
use crate::error::QueryError;
use crate::split::current_statement;

/// Every language that is not SQL, in the order of
/// [`Capabilities::LANGUAGES`].
///
/// SQL is left out: it carries a dialect, which only the driver can name.
const NOT_SQL: [QueryLanguage; 9] = [
    QueryLanguage::Cypher,
    QueryLanguage::Gremlin,
    QueryLanguage::MongoQuery,
    QueryLanguage::RedisCommand,
    QueryLanguage::SearchDsl,
    QueryLanguage::Cql,
    QueryLanguage::PartiQl,
    QueryLanguage::InfluxQl,
    QueryLanguage::Flux,
];

/// The language a console speaks to a session that declares `capabilities`.
///
/// SQL, in the driver's dialect ([`dialect_for`]), when the session declares
/// it; otherwise the one other language the session declares. `None` when it
/// declares none, or several besides SQL: which one the user writes cannot be
/// guessed, and a guess would send a text to the wrong parser.
#[must_use]
pub fn console_language(driver: &DriverId, capabilities: Capabilities) -> Option<QueryLanguage> {
    if capabilities.contains(Capabilities::SQL) {
        return Some(QueryLanguage::Sql(dialect_for(driver)));
    }
    let mut declared = NOT_SQL
        .into_iter()
        .filter(|language| capabilities.supports_language(*language));
    let language = declared.next()?;
    declared.next().is_none().then_some(language)
}

/// The text of the complete request under the cursor, or `None` between two.
///
/// `cursor_byte` is a UTF-8 byte index into `text`. For SQL, this is exactly
/// [`current_statement`].
///
/// # Errors
/// Those of [`current_statement`] for SQL, and
/// [`QueryError::UnsupportedLanguage`] for a language whose boundaries no
/// module of this crate knows: the caller asks for a selection instead.
pub fn current_request(
    text: &str,
    language: QueryLanguage,
    cursor_byte: usize,
) -> Result<Option<&str>, QueryError> {
    match language {
        QueryLanguage::Sql(dialect) => {
            Ok(current_statement(text, dialect, cursor_byte)?.map(|fragment| fragment.text))
        }
        QueryLanguage::MongoQuery => Ok(crate::mongo::command_at(text, cursor_byte)),
        language => Err(QueryError::UnsupportedLanguage { language }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::SqlDialect;

    #[test]
    fn a_sql_session_speaks_the_dialect_of_its_driver() {
        assert_eq!(
            console_language(&DriverId::postgres(), Capabilities::SQL),
            Some(QueryLanguage::Sql(SqlDialect::Postgres))
        );
    }

    #[test]
    fn sql_wins_over_another_declared_language() {
        assert_eq!(
            console_language(
                &DriverId::mysql(),
                Capabilities::SQL | Capabilities::REDIS_COMMAND
            ),
            Some(QueryLanguage::Sql(SqlDialect::MySql))
        );
    }

    #[test]
    fn a_session_without_sql_speaks_its_one_language() {
        let redis = DriverId::new("redis").expect("valid identifier");
        assert_eq!(
            console_language(
                &redis,
                Capabilities::REDIS_COMMAND | Capabilities::TRANSACTIONS
            ),
            Some(QueryLanguage::RedisCommand)
        );
    }

    #[test]
    fn two_languages_besides_sql_are_not_guessed_between() {
        let graph = DriverId::new("graph").expect("valid identifier");
        assert_eq!(
            console_language(&graph, Capabilities::CYPHER | Capabilities::GREMLIN),
            None
        );
    }

    #[test]
    fn a_session_declaring_no_language_has_no_console_language() {
        assert_eq!(
            console_language(&DriverId::sqlite(), Capabilities::TRANSACTIONS),
            None
        );
    }

    /// A language flag added to `oxyn-core` without its language here would
    /// make a session declaring it alone unreachable from a console.
    #[test]
    fn every_language_flag_has_its_language() {
        let covered = NOT_SQL
            .into_iter()
            .fold(Capabilities::SQL, |flags, language| {
                flags | Capabilities::for_language(language)
            });
        assert_eq!(covered, Capabilities::LANGUAGES);
    }

    #[test]
    fn a_sql_request_is_the_statement_under_the_cursor() {
        let text = "SELECT 1; DELETE FROM t";
        assert_eq!(
            current_request(text, QueryLanguage::Sql(SqlDialect::Postgres), 2),
            Ok(Some("SELECT 1"))
        );
    }

    #[test]
    fn a_mongodb_request_is_the_command_under_the_cursor() {
        let text = "{\"find\": \"users\"}\n{\"drop\": \"users\"}";
        assert_eq!(
            current_request(text, QueryLanguage::MongoQuery, 2),
            Ok(Some("{\"find\": \"users\"}"))
        );
    }

    #[test]
    fn a_language_without_known_boundaries_is_refused_by_name() {
        let refused = current_request("GET a\nDEL b", QueryLanguage::RedisCommand, 1);
        assert_eq!(
            refused,
            Err(QueryError::UnsupportedLanguage {
                language: QueryLanguage::RedisCommand
            })
        );
    }
}
