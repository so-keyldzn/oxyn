//! Reformatting of a SQL text.
//!
//! Reformatting goes through the `Display` of the `sqlparser` AST: the text is
//! parsed, then rewritten from the tree. That is what gives a consistent layout
//! without writing a rendering engine.
//!
//! # Two deliberate refusals
//!
//! **What does not parse is not touched.** A statement the parser refuses is
//! rendered as is. A reformatting never returns damaged SQL: the user would
//! lose their work without noticing, and would run it.
//!
//! **A text that carries comments is not reformatted at all.** The AST's
//! `Display` does not keep them; dropping them silently would be the same
//! loss, only quieter. The report says so
//! ([`declined_for_comments`](FormatReport::declined_for_comments)) so that the
//! interface can explain it rather than look inert.
//!
// TODO(phase 2): a reformatting that puts comments back requires tracking
// source positions (`sqlparser` exposes them through `Span`). It is a real
// component, not a touch-up.

use oxyn_core::SqlDialect;
use sqlparser::parser::Parser;

use crate::dialect::parser_dialect;
use crate::split;

/// What a reformatting actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatReport {
    /// The text to display. Always valid SQL in the sense that at worst it is
    /// the original text.
    pub text: String,
    /// Number of statements rewritten from their tree.
    pub reformatted: usize,
    /// Number of statements rendered as is, for want of being able to parse
    /// them.
    pub kept_verbatim: usize,
    /// Does the text differ from the input?
    pub changed: bool,
    /// The reformatting was abandoned as a whole: the text carries comments,
    /// which rendering the AST would lose.
    pub declined_for_comments: bool,
}

/// Reformats a SQL text.
///
/// Never returns an error and never loses content: at worst, the original
/// text.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::format;
///
/// assert_eq!(format("select   1", SqlDialect::Postgres), "SELECT 1");
///
/// // Unparsable: rendered as is rather than damaged.
/// assert_eq!(format("SELEKT 1", SqlDialect::Postgres), "SELEKT 1");
///
/// // A comment suspends reformatting: losing it would be a silent loss of
/// // work.
/// assert_eq!(
///     format("select   1 -- keep", SqlDialect::Postgres),
///     "select   1 -- keep"
/// );
/// ```
#[must_use]
pub fn format(sql: &str, dialect: SqlDialect) -> String {
    format_report(sql, dialect).text
}

/// Reformats, and says what was done.
#[must_use]
pub fn format_report(sql: &str, dialect: SqlDialect) -> FormatReport {
    let fragments = split::split(sql, dialect);

    if split::contains_comment(sql, dialect) {
        return FormatReport {
            text: sql.to_owned(),
            reformatted: 0,
            kept_verbatim: fragments.len(),
            changed: false,
            declined_for_comments: true,
        };
    }

    if fragments.is_empty() {
        return FormatReport {
            text: sql.to_owned(),
            reformatted: 0,
            kept_verbatim: 0,
            changed: false,
            declined_for_comments: false,
        };
    }

    let grammar = parser_dialect(dialect);
    // In a batch, every statement is terminated: without a semicolon, the
    // rendered text would no longer run. A single statement, on the other
    // hand, keeps the punctuation the user wrote.
    let terminate_all = fragments.len() > 1;

    let mut text = String::with_capacity(sql.len());
    let mut reformatted = 0usize;
    let mut kept_verbatim = 0usize;

    for (index, fragment) in fragments.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }

        match render(grammar, fragment.text) {
            Some(rendered) => {
                reformatted += 1;
                text.push_str(&rendered);
            }
            None => {
                kept_verbatim += 1;
                text.push_str(fragment.text);
            }
        }

        if terminate_all || fragment.terminated {
            text.push(';');
        }
    }

    let changed = text != sql;
    FormatReport {
        text,
        reformatted,
        kept_verbatim,
        changed,
        declined_for_comments: false,
    }
}

/// Rewrites a statement from its tree, or `None` if it does not parse.
///
/// A fragment must give exactly one statement. Anything else means the
/// splitter and the parser disagree — in which case nothing is rewritten.
fn render(grammar: &dyn sqlparser::dialect::Dialect, text: &str) -> Option<String> {
    let statements = Parser::parse_sql(grammar, text).ok()?;
    match statements.as_slice() {
        [statement] => Some(statement.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_is_normalized() {
        assert_eq!(format("select   1", SqlDialect::Postgres), "SELECT 1");
        assert_eq!(
            format("select a,b from t where x=1", SqlDialect::Postgres),
            "SELECT a, b FROM t WHERE x = 1"
        );
    }

    #[test]
    fn a_batch_is_repunctuated() {
        let output = format("select 1; select 2", SqlDialect::Postgres);
        assert_eq!(output, "SELECT 1;\nSELECT 2;");
    }

    #[test]
    fn a_single_statement_keeps_its_punctuation() {
        assert_eq!(format("select 1", SqlDialect::Postgres), "SELECT 1");
        assert_eq!(format("select 1;", SqlDialect::Postgres), "SELECT 1;");
    }

    /// The central guarantee: never damaged SQL.
    #[test]
    fn an_unparsable_text_is_rendered_as_is() {
        let source = "SELEKT * FORM t";
        let report = format_report(source, SqlDialect::Postgres);
        assert_eq!(report.text, source);
        assert_eq!(report.reformatted, 0);
        assert_eq!(report.kept_verbatim, 1);
        assert!(!report.changed);
    }

    #[test]
    fn an_unparsable_statement_does_not_block_the_others() {
        let report = format_report("select 1; SELEKT 2", SqlDialect::Postgres);
        assert_eq!(report.reformatted, 1);
        assert_eq!(report.kept_verbatim, 1);
        assert_eq!(report.text, "SELECT 1;\nSELEKT 2;");
    }

    #[test]
    fn a_comment_suspends_reformatting() {
        for source in [
            "select   1 -- note",
            "select   1 /* note */",
            "-- nothing but this",
        ] {
            let report = format_report(source, SqlDialect::Postgres);
            assert_eq!(report.text, source, "{source}");
            assert!(report.declined_for_comments, "{source}");
            assert!(!report.changed);
            assert_eq!(report.reformatted, 0);
        }
    }

    #[test]
    fn a_fake_comment_inside_a_string_suspends_nothing() {
        let report = format_report("select   '-- not a comment'", SqlDialect::Postgres);
        assert!(!report.declined_for_comments);
        assert_eq!(report.text, "SELECT '-- not a comment'");
    }

    #[test]
    fn an_empty_text_stays_empty() {
        for source in ["", "   ", ";"] {
            let report = format_report(source, SqlDialect::Postgres);
            assert_eq!(report.text, source, "{source:?}");
            assert!(!report.changed);
        }
    }

    /// Reformatting has no right to change the meaning: what comes out must
    /// classify like what went in.
    #[test]
    fn reformatting_preserves_the_intent() {
        for source in [
            "delete from t",
            "update t set a=1 where id=2",
            "drop table t",
            "explain analyze delete from t",
            "grant select on t to r",
        ] {
            let before = crate::classify(source, SqlDialect::Postgres);
            let after =
                crate::classify(&format(source, SqlDialect::Postgres), SqlDialect::Postgres);
            assert_eq!(before.intent, after.intent, "{source}");
            assert_eq!(before.risk, after.risk, "{source}");
        }
    }

    #[test]
    fn the_dialect_quoting_is_kept() {
        // MySQL backticks must come out as is: a reformatting that replaced
        // them with double quotes would produce SQL MySQL no longer reads.
        assert_eq!(
            format("select `a` from `t`", SqlDialect::MySql),
            "SELECT `a` FROM `t`"
        );
    }
}
