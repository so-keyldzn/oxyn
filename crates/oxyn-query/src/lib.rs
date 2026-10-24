//! Query analysis: dialects, splitting, intent classification,
//! reformatting.
//!
//! `oxyn-query` answers a single question, and it is the one the product's
//! security depends on: **what does this text do?** The `PolicyGate` of
//! `oxyn-core` decides from a [`StatementIntent`](oxyn_core::StatementIntent)
//! and a [`MutationRisk`](oxyn_core::MutationRisk) it is given; this is where
//! they are established. A classification error here fails nowhere: it simply
//! lets a write through.
//!
//! # What it contains
//!
//! | Module | Subject |
//! |---|---|
//! | [`dialect`](mod@dialect) | `SqlDialect` ↔ `sqlparser` grammar mapping, dialect of a driver |
//! | [`console`](mod@console) | the language a console speaks to a session, the request under its cursor |
//! | [`split`](mod@split) | splitting a batch into statements, reading bare words |
//! | [`classify`](mod@classify) | intent and risk of a text — the critical point |
//! | [`format`](mod@format) | reformatting by rendering the AST |
//! | [`error`](mod@error) | the failures the crate knows how to name |
//!
//! # The three rules
//!
//! **Analysis never reports a failure to the `PolicyGate`.** What does not
//! parse becomes [`Unknown`](oxyn_core::StatementIntent::Unknown), which counts
//! as mutating (I-02). [`classify()`] does not return a `Result`: there is no
//! path through which a failure would produce a read.
//!
//! **Classification works on the tree, not on the first word.** `EXPLAIN
//! ANALYZE DELETE` really runs the `DELETE` (I-07), and so does `WITH x AS
//! (DELETE …) SELECT`. The analysis descends into `WITH` clauses and `EXPLAIN`
//! bodies, and a keyword safety net rereads what it classified as a read.
//!
//! **A splitting error costs a confirmation, never a write.** When the scanner
//! is in doubt — unclosed string, unclosed comment — it merges instead of
//! cutting: the result no longer parses, so it is `Unknown`.
//!
//! # Example
//!
//! ```
//! use oxyn_core::{DriverId, MutationRisk, StatementIntent};
//! use oxyn_query::{classify, dialect_for, format};
//!
//! let sql_dialect = dialect_for(&DriverId::postgres());
//!
//! // The trap SECURITY asks to test explicitly.
//! let parsed = classify("EXPLAIN ANALYZE DELETE FROM orders", sql_dialect);
//! assert_eq!(parsed.intent, StatementIntent::Write);
//! assert_eq!(parsed.risk, MutationRisk::UnboundedDelete);
//!
//! // A trivially true `WHERE` bounds nothing.
//! let parsed = classify("UPDATE clients SET active = false WHERE 1=1", sql_dialect);
//! assert_eq!(parsed.risk, MutationRisk::UnboundedUpdate);
//!
//! // What does not parse is mutating, not "probably harmless".
//! let parsed = classify("SELEKT * FORM t", sql_dialect);
//! assert_eq!(parsed.intent, StatementIntent::Unknown);
//! assert!(parsed.is_mutating());
//!
//! assert_eq!(format("select   1", sql_dialect), "SELECT 1");
//! ```

pub mod classify;
pub mod console;
pub mod dialect;
pub mod error;
pub mod format;
pub mod redact;
mod search_dsl;
pub mod split;

pub use classify::{
    Basis, Classification, StatementInfo, changes_context, classify, classify_language,
    only_controls_transactions, reclassify, runnable_text, validate,
};
pub use console::{console_language, current_request};
pub use dialect::{dialect_for, dialect_for_language, parser_dialect};
pub use error::QueryError;
pub use format::{FormatReport, format, format_report};
pub use redact::{REDACTED_LITERAL, redact_password_literals};
pub use split::{
    Fragment, LineCommentEnd, SplitProfile, Token, TokenKind, Word, contains_comment,
    current_statement, split, tokens, words,
};

#[cfg(test)]
mod tests {
    use oxyn_core::prelude::*;

    use crate::{classify, reclassify};

    /// The full path of phase 0, from text to decision: an agent announces a
    /// read, `oxyn-query` requalifies it, the `PolicyGate` refuses.
    ///
    /// It is the sequence ARCHITECTURE §8 describes — the intent carried by a
    /// command is not trustworthy — checked end to end with the real types.
    #[test]
    fn an_agent_cannot_declare_itself_read_only() {
        let policy = DefaultPolicy::new();
        let conn_config = ConnectionConfig::new("customer db", DriverId::postgres())
            .with_environment(Environment::Production);
        policy.register(&conn_config);

        // The agent declares a read. The text says otherwise.
        let requested = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            "WITH removed AS (DELETE FROM orders RETURNING *) SELECT count(*) FROM removed",
        )
        .with_intent(StatementIntent::Read);

        let requalified = reclassify(&requested).qualify(requested);
        assert_eq!(requalified.intent, StatementIntent::Write);
        assert_eq!(requalified.risk, MutationRisk::UnboundedDelete);

        let order = Command::Execute {
            connection: conn_config.id,
            session: SessionId::new(),
            request: Box::new(requalified),
        };
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let decision = policy.authorize(&agent, &order, Environment::Production);

        assert!(decision.is_denied(), "{decision:?}");
    }

    /// Issue #223, end to end. A typed `USE other` was classified a read, so an
    /// agent's `Execute` of it skipped the refusal of ADR-0019 (which only knew
    /// `SetSessionContext`) and, on production, the confirmation too. The agent
    /// declares a read with no context change; the text says otherwise, in
    /// every environment — development included, where an `Unknown` statement
    /// would otherwise only ask for an approval.
    #[test]
    fn an_agent_cannot_declare_a_typed_use_away() {
        // The last two are executable comments: MySQL and MariaDB run what is in
        // them, so they are a `USE` to the server, and the batch once read as
        // empty — `Unknown` with no flag, which on development only asks for an
        // approval.
        for sql in [
            "USE other",
            "/*!50000 USE other */",
            "/*M!100000 USE other */",
        ] {
            for env in [
                Environment::Production,
                Environment::Staging,
                Environment::Development,
                Environment::Local,
            ] {
                let policy = DefaultPolicy::new();
                let conn_config =
                    ConnectionConfig::new("shop", DriverId::mysql()).with_environment(env);
                policy.register(&conn_config);

                let mut requested = ExecRequest::new(QueryLanguage::Sql(SqlDialect::MySql), sql)
                    .with_intent(StatementIntent::Read)
                    .with_limits(ExecLimits::default().writable());
                requested.context_change = false;

                let requalified = reclassify(&requested).qualify(requested);
                assert!(requalified.context_change, "{sql} on {env}");
                assert_eq!(
                    requalified.intent,
                    StatementIntent::Unknown,
                    "{sql} on {env}"
                );

                let order = Command::Execute {
                    connection: conn_config.id,
                    session: SessionId::new(),
                    request: Box::new(requalified),
                };
                assert!(order.changes_context(), "{sql} on {env}");

                let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
                let decision = policy.authorize(&agent, &order, env);
                let Decision::Deny { reason } = &decision else {
                    panic!("an agent's `{sql}` must be refused on {env}: {decision:?}");
                };
                assert!(
                    reason.contains("context selector"),
                    "{sql} on {env}: the reason must point to the selector: {reason}"
                );

                // The refusal is the agent's: the user's own `USE` is not
                // denied, it is an unreadable-effect write like any other.
                let human = policy.authorize(&Actor::Human, &order, env);
                assert!(!human.is_denied(), "{sql} on {env}: {human:?}");
            }
        }
    }

    /// The same path for a real read: nothing must be asked.
    #[test]
    fn a_read_stays_a_read_up_to_the_gate() {
        let policy = DefaultPolicy::new();
        let conn_config = ConnectionConfig::new("workshop", DriverId::sqlite())
            .with_environment(Environment::Local);
        policy.register(&conn_config);

        let requested = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            "SELECT name FROM clients WHERE id = 1",
        );
        let requalified = reclassify(&requested).qualify(requested);
        assert_eq!(requalified.intent, StatementIntent::Read);

        let order = Command::Execute {
            connection: conn_config.id,
            session: SessionId::new(),
            request: Box::new(requalified),
        };
        let decision = policy.authorize(&Actor::Human, &order, Environment::Local);
        assert_eq!(decision, Decision::Allow);
    }

    /// An empty text does not pass for a read: it is the door the choice
    /// "empty batch = `Unknown`" closes.
    #[test]
    fn an_empty_batch_does_not_pass_for_a_read() {
        let parsed = classify("", SqlDialect::Postgres);
        assert!(parsed.is_mutating());
        assert!(!parsed.is_read_only());
    }
}
