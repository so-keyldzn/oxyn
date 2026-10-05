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

use std::cmp::Ordering;
use std::ops::Range;

use oxyn_core::{ExecRequest, MutationRisk, QueryLanguage, SqlDialect, StatementIntent};
use serde::{Deserialize, Serialize};
use sqlparser::ast::{
    BinaryOperator, CopyIntoSnowflakeKind, CopyTarget, Expr, ObjectType, Query, Set, SetExpr,
    Statement, UnaryOperator, UtilityOption, Value,
};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

use crate::dialect::parser_dialect;
use crate::error::QueryError;
use crate::split::{self, Fragment, Word};

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
    /// declare itself read-only (ARCHITECTURE §8).
    #[must_use]
    pub fn qualify(&self, request: ExecRequest) -> ExecRequest {
        let mut request = request.with_intent(self.intent).with_risk(self.risk);
        request.transaction_control = self.transaction_control;
        request
    }

    /// The batch nothing is known about: one opaque statement, `Unknown`.
    ///
    /// Its language is not SQL, so no transaction verb of SQL can be looked
    /// for: `Unknown` already sends it to a human's approval, preview shown.
    fn opaque(text: &str, error: String) -> Self {
        let trimmed = text.trim();
        let start = text.len() - text.trim_start().len();
        Self {
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            transaction_control: false,
            statements: vec![StatementInfo {
                text: trimmed.to_owned(),
                span: start..start + trimmed.len(),
                intent: StatementIntent::Unknown,
                risk: MutationRisk::None,
                basis: Basis::Unparsed,
                error: Some(error),
                transaction_control: false,
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
    let grammar = parser_dialect(dialect);
    let mut statements: Vec<StatementInfo> = Vec::new();
    let mut aggregate: Option<Facts> = None;

    for fragment in split::split(sql, dialect) {
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
        statements,
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
            // Whether a `COMMIT` hides behind the comment depends on the
            // server: what cannot be read counts as the worst it could be.
            transaction_control: true,
        };
    }
    let (facts, basis, error) = match Parser::parse_sql(grammar, fragment.text) {
        Ok(parsed) if !parsed.is_empty() => {
            let facts = parsed
                .iter()
                .map(statement_facts)
                .reduce(Facts::merge)
                .unwrap_or(Facts::UNKNOWN);

            // An `EXPLAIN` of a mutation necessarily contains the mutating
            // keyword, and its form was already settled by the AST — with or
            // without `ANALYZE`. Letting the net act would turn
            // `EXPLAIN DELETE …`, which only computes a plan, into a statement
            // to approve.
            let sweepable = parsed.iter().all(|statement| {
                !matches!(
                    statement,
                    Statement::Explain { .. } | Statement::ExplainTable { .. }
                )
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
            if *analyze || analyze_requested(options.as_deref()) {
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

        // Transaction control and context change: these statements neither
        // read nor write by themselves. Counting them as mutating would ask for
        // an approval for `BEGIN; SELECT 1; COMMIT;`; the batch takes the
        // intent of what it contains anyway. What the transaction verbs do to
        // the session's open transaction is carried apart, by
        // `controls_a_transaction`.
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. }
        | Statement::Use(_) => Facts::READ,

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
        Statement::Copy { to, target, .. } => match (to, target) {
            (true, CopyTarget::Stdout) => Facts::READ,
            (true, CopyTarget::File { .. }) => {
                Facts::new(StatementIntent::Write, MutationRisk::CopyToServerFile)
            }
            (_, CopyTarget::Program { .. }) => {
                Facts::new(StatementIntent::Write, MutationRisk::CopyServerProgram)
            }
            _ => Facts::WRITE,
        },
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

/// PostgreSQL's `EXPLAIN (ANALYZE, VERBOSE) …` goes through the options, not
/// through the AST's `analyze` flag.
///
/// The option's argument is not looked at: `EXPLAIN (ANALYZE false) DELETE` is
/// treated as if it analyzed. Over-classifying costs a confirmation;
/// under-classifying runs the `DELETE`.
fn analyze_requested(options: Option<&[UtilityOption]>) -> bool {
    options.is_some_and(|options| {
        options
            .iter()
            .any(|option| option.name.value.eq_ignore_ascii_case("analyze"))
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
mod tests {
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
    #[case::trigger(
        "CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW BEGIN DELETE FROM audit; END"
    )]
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

    #[test]
    fn explain_without_analyze_stays_a_read() {
        assert_eq!(
            pg("EXPLAIN DELETE FROM orders").intent,
            StatementIntent::Read
        );
        assert_eq!(
            pg("EXPLAIN (VERBOSE) DELETE FROM orders").intent,
            StatementIntent::Read
        );
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
        let outcome = pg(
            "WITH targets AS (SELECT id FROM t) DELETE FROM u WHERE id IN (SELECT id FROM targets)",
        );
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
}
