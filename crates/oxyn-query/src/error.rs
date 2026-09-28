//! The failures this crate knows how to name.
//!
//! Intent analysis, for its part, **never** fails: what it does not understand
//! becomes [`Unknown`](oxyn_core::StatementIntent::Unknown), which counts as
//! mutating. An error raised here serves the editor — underlining a line,
//! refusing to reformat — not an authorization decision.

use std::ops::Range;

use oxyn_core::{OxynError, QueryLanguage};

/// What prevents reading a query text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// The cursor does not designate a whole executable statement.
    #[error("cannot select a statement at byte {cursor}: {message}")]
    Selection {
        /// Cursor position in the UTF-8 text.
        cursor: usize,
        /// Reason for the refusal.
        message: String,
    },

    /// The text does not parse in the requested dialect.
    ///
    /// The message comes from `sqlparser` and may repeat a fragment of the
    /// faulty SQL. That is acceptable: the text of a query is already kept as
    /// is by [`ExecRequest`](oxyn_core::ExecRequest) and is part of what the
    /// journal records. **Bound values**, on the other hand, never go through
    /// here.
    #[error("cannot parse SQL at byte {}: {message}", .span.start)]
    Syntax {
        /// Parser message.
        message: String,
        /// Byte bounds of the faulty statement in the original text.
        span: Range<usize>,
    },

    /// The language is not SQL: this crate cannot parse it.
    ///
    /// It is not a defect of the query. `oxyn-query` parses SQL; a graph or
    /// document language will be classified by its own parser (ADR-0003), not
    /// brought back to SQL.
    #[error("language not parsed by oxyn-query: {language}")]
    UnsupportedLanguage {
        /// The refused language.
        language: QueryLanguage,
    },
}

impl From<QueryError> for OxynError {
    fn from(error: QueryError) -> Self {
        Self::Query(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::SqlDialect;

    #[test]
    fn a_syntax_error_locates_the_culprit() {
        let failure = QueryError::Syntax {
            message: "Expected an expression".to_owned(),
            span: 42..60,
        };
        assert!(failure.to_string().contains("42"));
    }

    #[test]
    fn a_non_sql_language_is_named() {
        let failure = QueryError::UnsupportedLanguage {
            language: QueryLanguage::Cypher,
        };
        assert!(failure.to_string().contains("cypher"));
    }

    #[test]
    fn the_error_folds_into_the_domain_vocabulary() {
        let failure: OxynError = QueryError::UnsupportedLanguage {
            language: QueryLanguage::Sql(SqlDialect::Postgres),
        }
        .into();
        assert!(matches!(failure, OxynError::Query(_)));
        assert!(failure.is_user_error());
    }
}
