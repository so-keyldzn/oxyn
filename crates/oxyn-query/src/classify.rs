//! Intent classification: what feeds the `PolicyGate`.
//!
//! It is the critical point of the crate. The `PolicyGate` of `oxyn-core`
//! trusts the intent it is given (`oxyn-core::query`); this is where it is
//! established. If this module misclassifies, security falls: a `DELETE`
//! classified `Read` runs in production without confirmation.
//!
//! # The three rules that govern this module
//!
//! **What does not parse is mutating.** A text `sqlparser` refuses, a
//! statement whose form is not recognized, an empty batch: all of these give
//! [`StatementIntent::Unknown`], which `is_mutating()` (I-02). There is no
//! path through which a parse failure produces `Read`.
//!
//! **The name of the statement is not enough.** `EXPLAIN ANALYZE DELETE FROM t`
//! starts with `EXPLAIN` and deletes every row of the table — it is the example
//! I-07 gives. `WITH x AS (DELETE FROM t RETURNING *) SELECT *` starts with
//! `WITH` and does the same. Classification therefore works on the AST, not
//! on the first word, and descends into `WITH` clauses and `EXPLAIN` bodies.
//!
//! **A safety net under the AST.** After classification, a statement judged
//! `Read` is reread: if a mutating keyword appears outside strings and
//! comments while the AST saw no mutation, the analysis missed something, and
//! the statement falls back to `Unknown`. This net can only **restrict**; it
//! never relaxes anything.
//!
//! # Aggregating a batch
//!
//! A batch takes the highest intent of its statements:
//! `Read < Write < {Ddl, Unknown} < Grant`. On a tie between `Ddl` and
//! `Unknown`, `Unknown` wins: both are as restrictive, and `Unknown` tells the
//! truth — something was not understood.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::ops::Range;

use oxyn_core::{ExecRequest, MutationRisk, QueryLanguage, SqlDialect, StatementIntent};
use serde::{Deserialize, Serialize};
use sqlparser::ast::{
    BinaryOperator, CopyIntoSnowflakeKind, CopySource, CopyTarget, Expr, ObjectType, Query, Set,
    SetExpr, Statement, UnaryOperator, UtilityOption, Value,
};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

use crate::dialect::parser_dialect;
use crate::error::QueryError;
use crate::split::{self, Fragment, SplitProfile, Word};

/// What the classification of a statement rests on.
///
/// Serves the journal and the interface: an approval asked because the
/// statement could not be read is not presented like an approval asked
/// because it drops a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Basis {
    /// The statement was read and its form recognized.
    Ast,
    /// The statement could not be read: classified `Unknown` by default.
    Unparsed,
    /// The AST said `Read`, a bare mutating keyword was found: downgraded.
    KeywordSweep,
}

impl Basis {
    /// Does the classification come from a successful, uncorrected reading?
    #[must_use]
    pub const fn is_certain(&self) -> bool {
        matches!(self, Self::Ast)
    }

    /// Stable name, for the audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Unparsed => "unparsed",
            Self::KeywordSweep => "keyword-sweep",
        }
    }
}

impl std::fmt::Display for Basis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A statement of a batch, classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementInfo {
    /// The text of the statement, without its semicolon.
    pub text: String,
    /// Its byte bounds in the original batch, for the editor.
    pub span: Range<usize>,
    /// What it does.
    pub intent: StatementIntent,
    /// The risk its form reveals.
    pub risk: MutationRisk,
    /// Where this classification comes from.
    pub basis: Basis,
    /// Parser message, when it refused to read.
    pub error: Option<String>,
    /// Whether it begins, ends or marks a transaction: see
    /// [`ExecRequest::transaction_control`].
    pub transaction_control: bool,
    /// Whether it changes the session's current database or schema: see
    /// [`ExecRequest::context_change`].
    #[serde(default)]
    pub context_change: bool,
}

/// The result of analyzing a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    /// The intent of the batch: the highest of its statements.
    pub intent: StatementIntent,
    /// The risk of the batch: the most serious of its statements.
    pub risk: MutationRisk,
    /// Whether any of its statements controls a transaction.
    pub transaction_control: bool,
    /// Whether any of its statements changes the session's context: see
    /// [`ExecRequest::context_change`].
    #[serde(default)]
    pub context_change: bool,
    /// The detail, statement by statement, in text order.
    pub statements: Vec<StatementInfo>,
}

impl Classification {
    /// Can the batch modify anything?
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        self.intent.is_mutating() || self.risk.is_some()
    }

    /// Is the batch certainly read-only?
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.intent.is_read_only() && !self.risk.is_some()
    }

    /// Were all the statements read and recognized?
    ///
    /// `false` signals that at least one statement is classified by default:
    /// the decision stays safe, but the interface had better say so.
    #[must_use]
    pub fn is_fully_understood(&self) -> bool {
        !self.statements.is_empty() && self.statements.iter().all(|s| s.basis.is_certain())
    }

    /// The number of statements in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// Does the batch contain no executable statement?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

    /// The statements that carry a risk, in text order.
    pub fn risky(&self) -> impl Iterator<Item = &StatementInfo> {
        self.statements.iter().filter(|s| s.risk.is_some())
    }

    /// Rewrites the intent and risk of an execution request.
    ///
    /// It is what `oxyn-exec` does before submitting to the `PolicyGate`: the
    /// intent **declared** by the caller is not trustworthy, an agent cannot
    /// declare itself read-only (ARCHITECTURE §8). The same goes for the two
    /// flags the gate refuses an agent on, `transaction_control` and
    /// `context_change`: both are set from the text, in either direction, so
    /// that a caller can neither declare one away nor invent one.
    #[must_use]
    pub fn qualify(&self, request: ExecRequest) -> ExecRequest {
        let mut request = request.with_intent(self.intent).with_risk(self.risk);
        request.transaction_control = self.transaction_control;
        request.context_change = self.context_change;
        request
    }

    /// Raises this classification to the worst of itself and another reading
    /// of the same text: intent and risk by [`Facts::merge`], both flags by
    /// OR. The statement list stays this reading's.
    ///
    /// A reading that found no statement says nothing: its `Unknown` is the
    /// default of an empty batch, not a finding. A text made of an executable
    /// comment alone is empty as typed and holds a statement as the server runs
    /// it.
    fn at_least(&mut self, other: &Self) {
        if !other.statements.is_empty() {
            let facts =
                Facts::new(self.intent, self.risk).merge(Facts::new(other.intent, other.risk));
            self.intent = facts.intent;
            self.risk = facts.risk;
        }
        self.transaction_control |= other.transaction_control;
        self.context_change |= other.context_change;
    }

    /// The batch nothing is known about: one opaque statement, `Unknown`.
    ///
    /// Its language is not SQL, so no transaction verb and no `USE` of SQL can
    /// be looked for: `Unknown` already sends it to a human's approval, preview
    /// shown.
    fn opaque(text: &str, error: String) -> Self {
        let trimmed = text.trim();
        let start = text.len() - text.trim_start().len();
        Self {
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            transaction_control: false,
            context_change: false,
            statements: vec![StatementInfo {
                text: trimmed.to_owned(),
                span: start..start + trimmed.len(),
                intent: StatementIntent::Unknown,
                risk: MutationRisk::None,
                basis: Basis::Unparsed,
                error: Some(error),
                transaction_control: false,
                context_change: false,
            }],
        }
    }
}

/// Analyzes a SQL batch and deduces intent and risk from it.
///
/// Never returns an error: what does not parse is classified
/// [`Unknown`](StatementIntent::Unknown). To get the parser's message, see
/// [`crate::validate`].
///
/// For [`SqlDialect::MySql`] the body of an executable comment, `/*!50000 … */`
/// or MariaDB's `/*M!100000 … */`, is read as code, because the server runs it:
/// the statements, the intent and the two flags the gate refuses an agent on
/// all see what hides there. A body gated on a version the server does not
/// reach is a comment to it, and reading it as code only costs a confirmation.
///
/// ```
/// use oxyn_core::{MutationRisk, SqlDialect, StatementIntent};
/// use oxyn_query::classify;
///
/// // The trap: `EXPLAIN ANALYZE` really runs what it analyzes.
/// let outcome = classify("EXPLAIN ANALYZE DELETE FROM orders", SqlDialect::Postgres);
/// assert_eq!(outcome.intent, StatementIntent::Write);
/// assert_eq!(outcome.risk, MutationRisk::UnboundedDelete);
///
/// // Without `ANALYZE`, only the plan is computed.
/// let outcome = classify("EXPLAIN DELETE FROM orders", SqlDialect::Postgres);
/// assert_eq!(outcome.intent, StatementIntent::Read);
/// ```
#[must_use]
pub fn classify(sql: &str, dialect: SqlDialect) -> Classification {
    // The statements the server runs, which is more than the ones a reader of
    // the text sees: MySQL and MariaDB run the body of `/*! … */`.
    let runnable = runnable_text(sql, dialect);
    let mut outcome = classify_text(&runnable, dialect);
    if matches!(runnable, Cow::Owned(_)) {
        // The view and the user's text differ by blanks only, so the span leads
        // back to what was typed, markers included.
        for info in &mut outcome.statements {
            if let Some(typed) = sql.get(info.span.clone()) {
                typed.clone_into(&mut info.text);
            }
        }
        // The view only ever adds code, but it finds where an executable
        // comment ends with the scanner's rule, not the server's: a block
        // comment nested in the body can make it hide what the server runs.
        // Reading the text as typed is the floor (fail closed): the batch is
        // never judged milder than either reading says.
        outcome.at_least(&classify_text(sql, dialect));
    }
    outcome
}

/// The classification of one reading of the text, as the scanner splits it.
fn classify_text(text: &str, dialect: SqlDialect) -> Classification {
    let grammar = parser_dialect(dialect);
    let mut statements: Vec<StatementInfo> = Vec::new();
    let mut aggregate: Option<Facts> = None;

    for fragment in split::split(text, dialect) {
        let info = classify_fragment(&fragment, dialect, grammar);
        let facts = Facts::new(info.intent, info.risk);
        aggregate = Some(match aggregate {
            Some(previous) => previous.merge(facts),
            None => facts,
        });
        statements.push(info);
    }

    // A batch without statements — empty text, comments only, or a split that
    // dropped everything — counts as `Unknown`, not `Read`. A splitting bug
    // that lost statements must not open the door: "no statement" is no proof
    // of read-only.
    let facts = aggregate.unwrap_or(Facts::UNKNOWN);

    Classification {
        intent: facts.intent,
        risk: facts.risk,
        transaction_control: statements.iter().any(|s| s.transaction_control),
        // OR-ed over the batch, not read off the last statement: `USE shop;
        // SELECT 1` changes the context although it ends on a read.
        context_change: statements.iter().any(|s| s.context_change),
        statements,
    }
}

/// Does the text change the session's current database or schema?
///
/// True when any statement of the batch opens with `USE`. It is the lexical
/// rule [`classify`] raises [`Classification::context_change`] with, offered
/// to a driver so that the two cannot drift apart: a driver refuses a typed
/// `USE` before sending anything, and does not depend on the server's
/// one-statement prepare to notice the second statement of `SELECT 1; USE x`.
///
/// Read from the leading word, not from the AST, and it **over-reads on
/// purpose**, like the transaction verbs: over-reading only costs a refusal. A
/// statement the parser would reject still counts, and so does every form of
/// `USE` — `sqlparser` reads `USE ROLE` and `USE WAREHOUSE` as the same kind of
/// statement as `USE shop`. Quoted and commented text is not read: `SELECT
/// 'use'` and `-- USE x` do not change anything. The body of an executable
/// comment of MySQL and MariaDB is not commented text, though: the server runs
/// `/*!50000 USE x */`, so it counts, wherever it sits in the text.
///
/// A fragment that [`classify`] cannot read because of a carriage return in a
/// comment is not reported here: it raises the flag there, as the worst it
/// could be, whereas a driver only refuses what it can see is a `USE`.
#[must_use]
pub fn changes_context(text: &str, dialect: SqlDialect) -> bool {
    let leads_with_a_use = |text: &str| {
        split::split(text, dialect)
            .iter()
            .any(|fragment| leads_with_use(fragment.text, dialect))
    };
    // Both readings, as in [`classify`]: the executable-comment view is where
    // a `USE` hides, the text as typed is where a nested comment leaves one.
    leads_with_a_use(&runnable_text(text, dialect)) || leads_with_a_use(text)
}

/// The text as the server runs it, for every check that reads the text.
///
/// Only MySQL and MariaDB run the body of `/*! … */` and `/*M! … */`; to every
/// other dialect they are comments, and reading them as code there would
/// refuse what no server runs. Public for a driver's own lexical check, which
/// must read the same code as [`changes_context`]. Allocates only when the text
/// holds such a comment; byte offsets are the original text's.
#[must_use]
pub fn runnable_text(sql: &str, dialect: SqlDialect) -> Cow<'_, str> {
    if dialect == SqlDialect::MySql {
        split::executable_sql(sql, SplitProfile::for_dialect(dialect))
    } else {
        Cow::Borrowed(sql)
    }
}

/// Analyzes a text in its declared language.
///
/// A language that is not SQL is not analyzed: the result is `Unknown`, hence
/// mutating. Assuming that a Redis command or a Cypher query "looks like a
/// read" would be exactly the mistake I-02 forbids.
#[must_use]
pub fn classify_language(language: QueryLanguage, sql: &str) -> Classification {
    match language.sql_dialect() {
        Some(dialect) => classify(sql, dialect),
        None => Classification::opaque(
            sql,
            format!("language not parsed by oxyn-query: {language}"),
        ),
    }
}

/// Checks that the whole text parses in the requested dialect.
///
/// It is the strict counterpart of [`classify`]: where that one silently
/// falls back on `Unknown`, this one returns the parser's message and the
/// bounds of the faulty statement, so that the editor can underline it.
///
/// **Never use it to decide an authorization.** A text that fails here is not
/// "refused": it is `Unknown`, hence subject to approval like any mutation.
///
/// # Errors
///
/// [`QueryError::Syntax`] at the first statement the parser refuses.
pub fn validate(sql: &str, dialect: SqlDialect) -> Result<(), QueryError> {
    let grammar = parser_dialect(dialect);
    for fragment in split::split(sql, dialect) {
        if fragment.unreadable_comment {
            return Err(QueryError::Syntax {
                message: UNREADABLE_COMMENT.to_owned(),
                span: fragment.span,
            });
        }
        if let Err(err) = Parser::parse_sql(grammar, fragment.text) {
            return Err(QueryError::Syntax {
                message: err.to_string(),
                span: fragment.span,
            });
        }
    }
    Ok(())
}

/// Reclassifies an execution request from its text alone.
///
/// `oxyn-exec` calls this before each submission to the `PolicyGate`: the
/// intent carried by the `Command` comes from the caller, and an agent is a
/// caller (ARCHITECTURE §8, I-07).
#[must_use]
pub fn reclassify(request: &ExecRequest) -> Classification {
    classify_language(request.language, &request.text)
}

/// Does the request do nothing but begin, end or mark a transaction?
///
/// `oxyn-exec` leaves such a request out of the read-only bound of a
/// production read: a `COMMIT` that calls no function has nothing for the
/// bound to refuse, and a driver that cannot apply the bound inside an open
/// transaction — MySQL — would otherwise refuse the very statement that
/// settles it (issue #182). Stricter than
/// [`ExecRequest::transaction_control`], which over-reads on purpose: every
/// statement must parse to a bare transaction verb, and a `BEGIN … END` block,
/// which can hold anything, never does.
#[must_use]
pub fn only_controls_transactions(request: &ExecRequest) -> bool {
    let Some(dialect) = request.language.sql_dialect() else {
        return false;
    };
    let grammar = parser_dialect(dialect);
    let fragments = split::split(&request.text, dialect);
    !fragments.is_empty()
        && fragments.iter().all(|fragment| {
            !fragment.unreadable_comment
                && Parser::parse_sql(grammar, fragment.text).is_ok_and(|parsed| {
                    !parsed.is_empty() && parsed.iter().all(is_bare_transaction_verb)
                })
        })
}

fn is_bare_transaction_verb(statement: &Statement) -> bool {
    match statement {
        Statement::StartTransaction {
            statements,
            exception,
            has_end_keyword,
            ..
        } => statements.is_empty() && exception.is_none() && !has_end_keyword,
        Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. } => true,
        _ => false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Facts of a statement
// ─────────────────────────────────────────────────────────────────────────────

/// Intent and risk, carried together because they are deduced together and
/// aggregated together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Facts {
    intent: StatementIntent,
    risk: MutationRisk,
}

impl Facts {
    const READ: Self = Self::new(StatementIntent::Read, MutationRisk::None);
    const WRITE: Self = Self::new(StatementIntent::Write, MutationRisk::None);
    const DDL: Self = Self::new(StatementIntent::Ddl, MutationRisk::None);
    const GRANT: Self = Self::new(StatementIntent::Grant, MutationRisk::None);
    const UNKNOWN: Self = Self::new(StatementIntent::Unknown, MutationRisk::None);

    const fn new(intent: StatementIntent, risk: MutationRisk) -> Self {
        Self { intent, risk }
    }

    /// The worse of the two, field by field.
    fn merge(self, other: Self) -> Self {
        Self {
            intent: max_intent(self.intent, other.intent),
            risk: max_risk(self.risk, other.risk),
        }
    }
}

/// `Read < Write < {Ddl, Unknown} < Grant`.
///
/// `Unknown` is placed at the level of `Ddl`: it is what ARCHITECTURE §8 says.
const fn intent_rank(intent: StatementIntent) -> u8 {
    match intent {
        StatementIntent::Read => 0,
        StatementIntent::Write => 1,
        StatementIntent::Ddl | StatementIntent::Unknown => 2,
        StatementIntent::Grant => 3,
    }
}

fn max_intent(a: StatementIntent, b: StatementIntent) -> StatementIntent {
    match intent_rank(a).cmp(&intent_rank(b)) {
        Ordering::Greater => a,
        Ordering::Less => b,
        // At equal rank, `Unknown` wins: as restrictive, and more honest.
        Ordering::Equal => {
            if a == StatementIntent::Unknown || b == StatementIntent::Unknown {
                StatementIntent::Unknown
            } else {
                a
            }
        }
    }
}

/// Seriousness of a risk, from least to most serious.
///
/// The order only serves to choose **the displayed reason** when a batch
/// carries several: for the `PolicyGate`, any non-null risk means approval.
/// `DROP` comes before `TRUNCATE` because it takes the structure along with the
/// data; a variant added to the enumeration later (it is `#[non_exhaustive]`)
/// is treated as the most serious, for want of knowing.
const fn risk_rank(risk: MutationRisk) -> u8 {
    match risk {
        MutationRisk::None => 0,
        MutationRisk::UnboundedUpdate => 1,
        MutationRisk::UnboundedDelete => 2,
        MutationRisk::Truncate => 3,
        MutationRisk::DropObject => 4,
        MutationRisk::CopyToServerFile => 5,
        MutationRisk::CopyServerProgram => 6,
        _ => u8::MAX,
    }
}

fn max_risk(a: MutationRisk, b: MutationRisk) -> MutationRisk {
    if risk_rank(b) > risk_rank(a) { b } else { a }
}

// ─────────────────────────────────────────────────────────────────────────────
// Classifying a fragment
// ─────────────────────────────────────────────────────────────────────────────

/// Why a fragment holding an [`unreadable_comment`](Fragment::unreadable_comment)
/// is not read.
const UNREADABLE_COMMENT: &str = "a line comment contains a carriage return followed by text; \
     whether it ends there depends on the server, so the statements it holds cannot be known";

fn classify_fragment(
    fragment: &Fragment<'_>,
    dialect: SqlDialect,
    grammar: &dyn Dialect,
) -> StatementInfo {
    // The parser's own reading of the comment is no better than the splitter's:
    // for this dialect, nobody checked which one the server shares.
    if fragment.unreadable_comment {
        return StatementInfo {
            text: fragment.text.to_owned(),
            span: fragment.span.clone(),
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            basis: Basis::Unparsed,
            error: Some(UNREADABLE_COMMENT.to_owned()),
            // Whether a `COMMIT` or a `USE` hides behind the comment depends
            // on the server: what cannot be read counts as the worst it could
            // be.
            transaction_control: true,
            context_change: true,
        };
    }
    let (facts, basis, error) = match Parser::parse_sql(grammar, fragment.text) {
        Ok(parsed) if !parsed.is_empty() => {
            let facts = parsed
                .iter()
                .map(statement_facts)
                .reduce(Facts::merge)
                .unwrap_or(Facts::UNKNOWN);

            // An `EXPLAIN` that only computes a plan necessarily contains the
            // mutating keyword of what it explains, and the AST already
            // settled that it runs none of it: letting the net act would turn
            // `EXPLAIN DELETE …` into a statement to approve. An `EXPLAIN` that
            // runs its statement is swept like that statement would be on its
            // own — otherwise prefixing `EXPLAIN ANALYZE` would switch the net
            // off for exactly the forms it exists to catch.
            let sweepable = parsed.iter().all(|statement| match statement {
                Statement::Explain {
                    analyze, options, ..
                } => explain_executes(*analyze, options.as_deref()),
                Statement::ExplainTable { .. } => false,
                _ => true,
            });

            if facts == Facts::READ && sweepable && hides_a_mutation(fragment.text, dialect) {
                tracing::debug!(
                    span = ?fragment.span,
                    "mutating keyword outside the AST: statement downgraded to Unknown"
                );
                (Facts::UNKNOWN, Basis::KeywordSweep, None)
            } else {
                (facts, Basis::Ast, None)
            }
        }
        // `split` only returns fragments that carry code: getting no statement
        // is already an anomaly.
        Ok(_) => (
            Facts::UNKNOWN,
            Basis::Unparsed,
            Some("no statement was read".to_owned()),
        ),
        Err(err) => {
            // Not the parser's message: it quotes the token it stopped at,
            // which can be a password literal (I-03). The span locates it.
            tracing::debug!(
                span = ?fragment.span,
                "statement could not be parsed: classified as Unknown"
            );
            (Facts::UNKNOWN, Basis::Unparsed, Some(err.to_string()))
        }
    };

    StatementInfo {
        text: fragment.text.to_owned(),
        span: fragment.span.clone(),
        intent: facts.intent,
        risk: facts.risk,
        basis,
        error,
        transaction_control: controls_a_transaction(fragment.text, dialect),
        context_change: leads_with_use(fragment.text, dialect),
    }
}

/// What a statement does, according to its form.
///
/// Any form not listed falls back on [`Facts::UNKNOWN`]. It is deliberate:
/// `sqlparser` knows more than a hundred statement forms, the list will move
/// with every version bump, and a new form must ask for an approval, not pass
/// for a read.
fn statement_facts(statement: &Statement) -> Facts {
    match statement {
        // ── Read ────────────────────────────────────────────────────────────
        Statement::Query(query) => query_facts(query),

        Statement::Explain {
            analyze,
            options,
            statement: inner,
            ..
        } => {
            if explain_executes(*analyze, options.as_deref()) {
                // `EXPLAIN ANALYZE` really runs what it analyzes (I-07): the
                // statement inherits everything, intent as well as risk.
                statement_facts(inner)
            } else {
                Facts::READ
            }
        }

        Statement::ExplainTable { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowProcessList { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowObjects(_)
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCollation { .. } => Facts::READ,

        // Transaction control: these statements neither read nor write by
        // themselves. Counting them as mutating would ask for an approval for
        // `BEGIN; SELECT 1; COMMIT;`; the batch takes the intent of what it
        // contains anyway. What the transaction verbs do to the session's open
        // transaction is carried apart, by `controls_a_transaction`.
        //
        // `USE` is not among them: it reads nothing, but it decides what every
        // statement after it means, so it is not a read (see below).
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. } => Facts::READ,

        // ── Write ───────────────────────────────────────────────────────────
        Statement::Insert(insert) => match &insert.source {
            // `INSERT INTO t WITH d AS (DELETE …) SELECT …`: the source can
            // carry its own mutation.
            Some(source) => Facts::WRITE.merge(query_facts(source)),
            None => Facts::WRITE,
        },

        Statement::Update(update) => Facts::new(
            StatementIntent::Write,
            if where_is_unbounded(update.selection.as_ref()) {
                MutationRisk::UnboundedUpdate
            } else {
                MutationRisk::None
            },
        ),

        Statement::Delete(delete) => Facts::new(
            StatementIntent::Write,
            if where_is_unbounded(delete.selection.as_ref()) {
                MutationRisk::UnboundedDelete
            } else {
                MutationRisk::None
            },
        ),

        // `MERGE` is bounded by its `ON` condition: no scope risk.
        Statement::Merge(_) => Facts::WRITE,

        // Server-side files and programs escape PostgreSQL's READ ONLY
        // transaction: only STDOUT exports through the client connection.
        Statement::Copy {
            source, to, target, ..
        } => {
            let target = match (to, target) {
                (true, CopyTarget::Stdout) => Facts::READ,
                (true, CopyTarget::File { .. }) => {
                    Facts::new(StatementIntent::Write, MutationRisk::CopyToServerFile)
                }
                (_, CopyTarget::Program { .. }) => {
                    Facts::new(StatementIntent::Write, MutationRisk::CopyServerProgram)
                }
                _ => Facts::WRITE,
            };
            // PostgreSQL copies the output of `(DELETE FROM t RETURNING *)` as
            // readily as that of a `SELECT`: the statement runs, whatever the
            // target.
            match source {
                CopySource::Query(query) => target.merge(query_facts(query)),
                CopySource::Table { .. } => target,
            }
        }
        Statement::CopyIntoSnowflake { kind, .. } => match kind {
            CopyIntoSnowflakeKind::Table => Facts::WRITE,
            CopyIntoSnowflakeKind::Location => Facts::READ,
        },

        // ── Structure ───────────────────────────────────────────────────────
        Statement::Truncate(_) => Facts::new(StatementIntent::Ddl, MutationRisk::Truncate),

        Statement::Drop { object_type, .. } => match object_type {
            // `DROP ROLE` / `DROP USER` touch rights, not the schema.
            ObjectType::Role | ObjectType::User => Facts::GRANT,
            _ => Facts::new(StatementIntent::Ddl, MutationRisk::DropObject),
        },

        Statement::DropFunction(_)
        | Statement::DropDomain(_)
        | Statement::DropProcedure { .. }
        | Statement::DropSecret { .. }
        | Statement::DropPolicy(_)
        | Statement::DropConnector { .. }
        | Statement::DropExtension(_)
        | Statement::DropOperator(_)
        | Statement::DropOperatorFamily(_)
        | Statement::DropOperatorClass(_)
        | Statement::DropTrigger(_) => Facts::new(StatementIntent::Ddl, MutationRisk::DropObject),

        Statement::CreateView(_)
        | Statement::CreateTable(_)
        | Statement::CreateVirtualTable { .. }
        | Statement::CreateIndex(_)
        | Statement::CreateSecret { .. }
        | Statement::CreateServer(_)
        | Statement::CreatePolicy(_)
        | Statement::CreateConnector(_)
        | Statement::CreateOperator(_)
        | Statement::CreateOperatorFamily(_)
        | Statement::CreateOperatorClass(_)
        | Statement::CreateExtension(_)
        | Statement::CreateCollation(_)
        | Statement::CreateSchema { .. }
        | Statement::CreateDatabase { .. }
        | Statement::CreateFunction(_)
        | Statement::CreateTrigger(_)
        | Statement::CreateProcedure { .. }
        | Statement::CreateMacro { .. }
        | Statement::CreateStage { .. }
        | Statement::CreateSequence { .. }
        | Statement::CreateDomain(_)
        | Statement::CreateType { .. }
        | Statement::AlterTable(_)
        | Statement::AlterSchema(_)
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterFunction(_)
        | Statement::AlterType(_)
        | Statement::AlterCollation(_)
        | Statement::AlterOperator(_)
        | Statement::AlterOperatorFamily(_)
        | Statement::AlterOperatorClass(_)
        | Statement::AlterPolicy(_)
        | Statement::AlterConnector { .. }
        | Statement::RenameTable(_)
        | Statement::Comment { .. }
        | Statement::AttachDatabase { .. }
        | Statement::AttachDuckDBDatabase { .. }
        | Statement::DetachDuckDBDatabase { .. } => Facts::DDL,

        // ── Rights ──────────────────────────────────────────────────────────
        Statement::Grant(_)
        | Statement::Revoke(_)
        | Statement::Deny(_)
        | Statement::CreateRole(_)
        | Statement::AlterRole { .. }
        | Statement::CreateUser(_)
        | Statement::AlterUser(_) => Facts::GRANT,

        // `SET ROLE` and `SET SESSION AUTHORIZATION` change the identity under
        // which everything else runs.
        Statement::Set(Set::SetRole { .. } | Set::SetSessionAuthorization(_)) => Facts::GRANT,
        // The other `SET`s are not harmless either: `SET TRANSACTION READ
        // WRITE` undoes a read-only session. For want of telling them apart one
        // by one, they stay `Unknown`.
        Statement::Set(_) => Facts::UNKNOWN,
        // `USE shop` writes nothing itself, yet the `DELETE FROM users` typed
        // next would hit another database from the one the user believes they
        // target, and the driver's declared context would no longer match the
        // server's (ADR-0019). The whole family is matched — `USE ROLE` and
        // `USE SECONDARY ROLES` change the session as well — and it stays
        // `Unknown`, like `SET`, rather than a guess at how far each one
        // reaches. `context_change` carries the part the gate refuses an agent.
        Statement::Use(_) => Facts::UNKNOWN,

        // ── The rest ────────────────────────────────────────────────────────
        // `ANALYZE`, `VACUUM`, `PRAGMA`, cursors, `EXECUTE` of a prepared
        // statement, procedure calls: so many forms that can write without
        // saying so.
        // TODO(phase 1): refine case by case when the real drivers show which
        // ones matter, with one test per added form.
        _ => Facts::UNKNOWN,
    }
}

/// What a query does, `WITH` clauses included.
///
/// PostgreSQL allows `WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x`.
/// The statement starts with `WITH`, `sqlparser` renders it as a `Query`, and
/// it deletes rows.
fn query_facts(query: &Query) -> Facts {
    let mut facts = Facts::READ;
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            facts = facts.merge(query_facts(&cte.query));
        }
    }
    facts.merge(set_expr_facts(&query.body))
}

fn set_expr_facts(body: &SetExpr) -> Facts {
    match body {
        // A subquery of a `SELECT` cannot mutate: only the top-level `WITH`
        // clause can, and it is handled elsewhere. Except `SELECT … INTO t`:
        // PostgreSQL and SQL Server create table `t` there and fill it. It is a
        // `CREATE TABLE AS` that starts with `SELECT`; read as a read, it went
        // through without the confirmation that names the connection (I-02).
        SetExpr::Select(select) if select.into.is_some() => Facts::DDL,
        SetExpr::Select(_) | SetExpr::Values(_) | SetExpr::Table(_) => Facts::READ,
        SetExpr::Query(inner) => query_facts(inner),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_facts(left).merge(set_expr_facts(right))
        }
        SetExpr::Insert(statement)
        | SetExpr::Update(statement)
        | SetExpr::Delete(statement)
        | SetExpr::Merge(statement) => statement_facts(statement),
    }
}

/// The options of a parenthesized `EXPLAIN` that only shape the plan shown.
///
/// PostgreSQL 18's synopsis lists twelve options; `ANALYZE` is the one that
/// runs the statement, and the others are these (sources in RESEARCH-NOTES,
/// under "PostgreSQL EXPLAIN options").
const PLAN_ONLY_EXPLAIN_OPTIONS: [&str; 11] = [
    "verbose",
    "costs",
    "settings",
    "generic_plan",
    "buffers",
    "serialize",
    "wal",
    "timing",
    "summary",
    "memory",
    "format",
];

/// Does an `EXPLAIN` run the statement it explains?
///
/// The bare `ANALYZE` keyword sets the AST's flag. PostgreSQL's parenthesized
/// form goes through the options, and its grammar takes `ANALYSE` there as well
/// as `ANALYZE`: `sqlparser` knows no `ANALYSE` keyword and keeps it as a plain
/// identifier, so a test on the name `analyze` let `EXPLAIN (ANALYSE) DELETE`
/// pass for a read (issue #246). The options are therefore read the other way
/// round: only names known to shape the plan keep the statement a plan; any
/// other — either spelling, a quoted `"analyze"`, an option a later version
/// adds — counts as running it.
///
/// The arguments are not looked at: `EXPLAIN (ANALYZE false) DELETE` is
/// treated as if it analyzed. Over-classifying costs a confirmation;
/// under-classifying runs the `DELETE`.
fn explain_executes(analyze: bool, options: Option<&[UtilityOption]>) -> bool {
    analyze
        || options.is_some_and(|options| {
            options.iter().any(|option| {
                !PLAN_ONLY_EXPLAIN_OPTIONS
                    .iter()
                    .any(|known| option.name.value.eq_ignore_ascii_case(known))
            })
        })
}

// ─────────────────────────────────────────────────────────────────────────────
// Scope of a mutation
// ─────────────────────────────────────────────────────────────────────────────

/// Does the `WHERE` clause let every row through?
///
/// No `WHERE` and a trivially true `WHERE` count the same: `DELETE FROM t
/// WHERE 1=1` deletes exactly what `DELETE FROM t` deletes, and it is the form
/// one writes when assembling a `WHERE` piece by piece.
fn where_is_unbounded(selection: Option<&Expr>) -> bool {
    match selection {
        None => true,
        Some(expr) => is_trivially_true(expr),
    }
}

/// Is the expression true without looking at a single row?
///
/// Deliberately incomplete: it is not an evaluator. Any expression this
/// function cannot settle is declared non-trivial, which gives
/// [`MutationRisk::None`] — the write stays a write, and the `PolicyGate`
/// treats it as such; only the "every row" reason is missing.
fn is_trivially_true(expr: &Expr) -> bool {
    match expr {
        Expr::Nested(inner) => is_trivially_true(inner),
        Expr::Value(value) => literal_truth(&value.value) == Some(true),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr: inner,
        } => is_trivially_false(inner),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::Or => is_trivially_true(left) || is_trivially_true(right),
            BinaryOperator::And => is_trivially_true(left) && is_trivially_true(right),
            // `1=1`, `'x'='x'`, and also `id = id`: on a non-null column, the
            // latter bounds nothing at all.
            BinaryOperator::Eq => {
                same_pure_expr(left, right)
                    || compare_numbers(left, right).is_some_and(Ordering::is_eq)
            }
            BinaryOperator::NotEq => compare_numbers(left, right).is_some_and(Ordering::is_ne),
            BinaryOperator::Gt => compare_numbers(left, right).is_some_and(Ordering::is_gt),
            BinaryOperator::Lt => compare_numbers(left, right).is_some_and(Ordering::is_lt),
            BinaryOperator::GtEq => compare_numbers(left, right).is_some_and(Ordering::is_ge),
            BinaryOperator::LtEq => compare_numbers(left, right).is_some_and(Ordering::is_le),
            _ => false,
        },
        _ => false,
    }
}

fn is_trivially_false(expr: &Expr) -> bool {
    match expr {
        Expr::Nested(inner) => is_trivially_false(inner),
        Expr::Value(value) => literal_truth(&value.value) == Some(false),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr: inner,
        } => is_trivially_true(inner),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::Or => is_trivially_false(left) && is_trivially_false(right),
            BinaryOperator::And => is_trivially_false(left) || is_trivially_false(right),
            BinaryOperator::Eq => compare_numbers(left, right).is_some_and(Ordering::is_ne),
            BinaryOperator::NotEq => {
                same_pure_expr(left, right)
                    || compare_numbers(left, right).is_some_and(Ordering::is_eq)
            }
            _ => false,
        },
        _ => false,
    }
}

/// Truth of a literal. `NULL` is neither true nor false: `WHERE NULL` selects
/// nothing, but it is not this function's job to say so.
fn literal_truth(value: &Value) -> Option<bool> {
    match value {
        Value::Boolean(state) => Some(*state),
        // MySQL's `WHERE 1`. `0.0` counts as false.
        Value::Number(text, _) => text.parse::<f64>().ok().map(|number| number != 0.0),
        _ => None,
    }
}

/// Are both sides the same side-effect-free expression?
///
/// The comparison is done on the rendering, to ignore the source positions the
/// AST carries. Restricted to identifiers and literals: an identical function
/// call on both sides (`random() = random()`) is not true.
fn same_pure_expr(left: &Expr, right: &Expr) -> bool {
    is_pure(left) && is_pure(right) && left.to_string() == right.to_string()
}

fn is_pure(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Value(_) => true,
        Expr::Nested(inner) => is_pure(inner),
        _ => false,
    }
}

fn compare_numbers(left: &Expr, right: &Expr) -> Option<Ordering> {
    let left = numeric_literal(left)?;
    let right = numeric_literal(right)?;
    left.partial_cmp(&right)
}

fn numeric_literal(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Nested(inner) => numeric_literal(inner),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr: inner,
        } => numeric_literal(inner).map(|number| -number),
        Expr::UnaryOp {
            op: UnaryOperator::Plus,
            expr: inner,
        } => numeric_literal(inner),
        Expr::Value(value) => match &value.value {
            Value::Number(text, _) => text.parse::<f64>().ok(),
            _ => None,
        },
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The safety net under the AST
// ─────────────────────────────────────────────────────────────────────────────

/// Words whose bare presence betrays a mutation the AST did not report.
///
/// `CREATE` and `ALTER` are left out on purpose: `SHOW CREATE TABLE t` is a
/// read, and a `CREATE` form nested in a query does not exist.
const MUTATING_WORDS: [&str; 8] = [
    "delete", "update", "insert", "merge", "truncate", "drop", "grant", "revoke",
];

/// Does a statement the AST classified `Read` hide a mutation?
///
/// Called **only** on statements classified `Read` without risk, and can only
/// downgrade them to `Unknown`. It is a defense in depth against a form of
/// nesting not anticipated in [`statement_facts`]: the cost of a false positive
/// is a confirmation, that of a false negative is a silent `DELETE`.
fn hides_a_mutation(text: &str, dialect: SqlDialect) -> bool {
    let words = split::words(text, dialect);
    // A COPY program target is a server-side effect even if a future AST
    // form misses it. Bare identifiers named `program` are still reads.
    if words
        .iter()
        .any(|word| word.text.eq_ignore_ascii_case("copy"))
        && words.windows(2).any(|pair| {
            matches!(pair, [to, program]
                if to.text.eq_ignore_ascii_case("to")
                    && program.text.eq_ignore_ascii_case("program"))
        })
    {
        return true;
    }
    for (index, word) in words.iter().enumerate() {
        // `TRUNCATE(x, 2)` and `INSERT('abc', 1, 1, 'z')` are functions.
        if word.call {
            continue;
        }
        if !MUTATING_WORDS
            .iter()
            .any(|keyword| word.text.eq_ignore_ascii_case(keyword))
        {
            continue;
        }
        // `SELECT … FOR UPDATE` locks rows, it does not modify them.
        if word.text.eq_ignore_ascii_case("update") && locked_for_update(&words, index) {
            continue;
        }
        return true;
    }
    false
}

/// Does the statement begin, end or mark a transaction?
///
/// Read from its leading words rather than from the AST: every such statement
/// opens with its verb, and the verbs the parser does not know — PostgreSQL's
/// `END` and `ABORT`, `PREPARE TRANSACTION`, `XA` — count as much as those it
/// does. Over-reading only costs an agent a refusal; a `BEGIN` opening a
/// procedural block is read as a transaction.
fn controls_a_transaction(text: &str, dialect: SqlDialect) -> bool {
    let words = split::words(text, dialect);
    let mut leading = words.iter().map(|word| word.text);
    let Some(verb) = leading.next() else {
        return false;
    };
    let is = |keyword: &str| verb.eq_ignore_ascii_case(keyword);
    if TRANSACTION_VERBS.iter().any(|keyword| is(keyword)) {
        return true;
    }
    // `START TRANSACTION`, `PREPARE TRANSACTION 'x'`, `SAVE TRAN` — but not
    // `PREPARE plan AS …`, a prepared statement.
    (is("start") || is("prepare") || is("save"))
        && leading.next().is_some_and(|second| {
            ["transaction", "tran"]
                .iter()
                .any(|keyword| second.eq_ignore_ascii_case(keyword))
        })
}

/// Does the statement open with `USE`?
///
/// The rule behind [`StatementInfo::context_change`] and [`changes_context`], in
/// one place. The same lexical reading as [`controls_a_transaction`], for the
/// same reasons: a text the parser rejects, or a form it does not know in this
/// dialect, still leads with its verb. The first bare word is the verb —
/// quoted and commented text is not a word — so `SELECT 'use'` and ``SELECT
/// `use` FROM t`` do not count, and `/* c */ USE shop` does.
fn leads_with_use(text: &str, dialect: SqlDialect) -> bool {
    split::words(text, dialect)
        .first()
        .is_some_and(|verb| verb.text.eq_ignore_ascii_case("use"))
}

/// Verbs that, leading a statement, always control a transaction.
const TRANSACTION_VERBS: [&str; 8] = [
    "begin",
    "commit",
    "end",
    "rollback",
    "abort",
    "savepoint",
    "release",
    "xa",
];

/// PostgreSQL writes `FOR UPDATE` but also `FOR NO KEY UPDATE`, hence a
/// three-word window backwards.
fn locked_for_update(words: &[Word<'_>], index: usize) -> bool {
    let from = index.saturating_sub(3);
    words.get(from..index).is_some_and(|window| {
        window
            .iter()
            .any(|word| word.text.eq_ignore_ascii_case("for"))
    })
}

#[cfg(test)]
mod tests;
