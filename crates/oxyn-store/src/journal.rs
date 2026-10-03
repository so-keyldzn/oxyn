//! The `audit_journal` table: the audit trail, **append-only**.
//!
//! It is the user's history *and* the agents' journal — a single mechanism, as
//! ARCHITECTURE §8 requires. A **refused** command is recorded too: a journal
//! that only records what worked says nothing of what an agent attempted.
//!
//! # Tamper-proofing holds in two places, and two are needed
//!
//! * **The API.** [`Journal`] exposes [`append`](Journal::append) and reads.
//!   There is no `update`, `delete` or `purge`: what does not exist cannot be
//!   called by mistake, and no agent tool can reach it since the tools are
//!   exactly the `Command`s (ADR-0004).
//! * **The file.** Two SQLite triggers abort any `UPDATE` and any `DELETE` on
//!   the table — `audit_journal_forbid_update` and
//!   `audit_journal_forbid_delete`, set by the initial migration. That is what
//!   keeps the guarantee even when someone opens the local state with the
//!   command-line `sqlite3`.
//!
//! The API alone would only be a convention; the trigger alone would let
//! through a `DELETE` written inside the crate.
//!
//! What the trigger does **not** cover: a `DROP TABLE`, a
//! `PRAGMA writable_schema`, rewriting the file with a hex editor. The
//! protection targets mistakes and the agent that would want to erase its
//! trace through the product's ordinary means — not an attacker who already
//! has write access to the user's disk.
//!
//! # Reading back what is not understood
//!
//! Columns that have a **conservative** value fall back to it when
//! unreadable: `intent` becomes
//! [`Unknown`](oxyn_core::StatementIntent::Unknown), which counts as mutating,
//! and `policy_decision` becomes [`PolicyOutcome::Denied`]. Those that have
//! none — `risk` — make the read fail: inventing a risk would be worse than
//! sending the operator to the raw row, which stays readable with `sqlite3`
//! (I-11).
//!
//! # What never goes in here
//!
//! The statement's **text** is recorded: it is the object of the audit.
//! **Bound values** are not — [`Command::statement_text`] does not return
//! them — and no secret goes through this module (I-03).

use chrono::{DateTime, Utc};
use oxyn_core::{
    Actor, AgentId, AgentSessionId, Command, CommandId, ConnectionId, Decision, ErrorClass,
    MutationRisk, OxynError, QueryLanguage, StatementIntent,
};
use oxyn_query::redact_password_literals;
use rusqlite::{Row, params};
use std::time::Duration;

use crate::encoding::{
    count_from_i64, count_to_i64, duration_to_ms, error_class_from_text, intent_from_text,
    limit_to_i64, parse_id_opt, tag_from_json, tag_to_json,
};
use crate::error::Result;
use crate::store::Store;

/// Who issued the command, reduced to what fits in a column.
///
/// A **closed** enum, like [`Actor`] from which it derives: the human/agent
/// dichotomy carries the whole policy (ADR-0004), and a third actor would be
/// an ADR decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorKind {
    /// The user, through the interface.
    Human,
    /// An AI agent.
    Agent,
}

impl ActorKind {
    /// Stable name, the one written in the `actor_kind` column.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    /// Reads back the `actor_kind` column.
    ///
    /// An unknown value returns [`Agent`](Self::Agent): in an audit trail,
    /// the safe fallback is the one that **triggers** scrutiny, not the one
    /// that avoids it. Attributing to a human an action that cannot be
    /// attributed would be exactly the mistake not to make.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "human" => Self::Human,
            "agent" => Self::Agent,
            _ => {
                tracing::warn!(
                    column = "actor_kind",
                    "unknown actor kind in audit journal, falling back to `agent`"
                );
                Self::Agent
            }
        }
    }

    /// Is it an agent?
    #[must_use]
    pub const fn is_agent(&self) -> bool {
        matches!(self, Self::Agent)
    }
}

impl From<&Actor> for ActorKind {
    fn from(actor: &Actor) -> Self {
        if actor.is_agent() {
            Self::Agent
        } else {
            Self::Human
        }
    }
}

impl std::fmt::Display for ActorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the `PolicyGate` answered, reduced to what fits in a column.
///
/// A **closed** enum, like [`Decision`]: ADR-0004's triad is the bus
/// contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyOutcome {
    /// `Allow`: the command could run.
    Allowed,
    /// `RequireApproval`: explicit consent was required.
    ApprovalRequired,
    /// `Deny`: the command did not happen.
    Denied,
}

impl PolicyOutcome {
    /// Stable name, the one written in the `policy_decision` column.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::ApprovalRequired => "approval_required",
            Self::Denied => "denied",
        }
    }

    /// Reads back the `policy_decision` column.
    ///
    /// An unknown value returns [`Denied`](Self::Denied). The question a
    /// journal reading serves is "what was allowed?": a row that cannot be
    /// classified must not inflate that count. The inconsistency stays
    /// visible — a `denied` row carrying a duration and affected rows stands
    /// out, and that is the point.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "allowed" => Self::Allowed,
            "approval_required" => Self::ApprovalRequired,
            "denied" => Self::Denied,
            _ => {
                tracing::warn!(
                    column = "policy_decision",
                    "unknown policy decision in audit journal, falling back to `denied`"
                );
                Self::Denied
            }
        }
    }

    /// Was the command allowed without further formality?
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

impl From<&Decision> for PolicyOutcome {
    fn from(decision: &Decision) -> Self {
        match decision {
            Decision::Allow => Self::Allowed,
            Decision::RequireApproval { .. } => Self::ApprovalRequired,
            Decision::Deny { .. } => Self::Denied,
        }
    }
}

impl std::fmt::Display for PolicyOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What gets appended to the journal.
///
/// The normal path is [`JournalRecord::new`], which derives everything it can
/// from the command and the decision rather than letting the caller copy it —
/// a field copied by hand is a field that will end up lying.
#[derive(Debug, Clone)]
pub struct JournalRecord {
    /// When the command was submitted.
    pub ts: DateTime<Utc>,
    /// Correlation key with the approval request and the history.
    pub command_id: Option<CommandId>,
    /// Human or agent.
    pub actor_kind: ActorKind,
    /// Which agent, if any.
    pub actor_id: Option<AgentId>,
    /// In which conversation, if any.
    pub agent_session: Option<AgentSessionId>,
    /// The targeted connection, when the command targets one.
    pub connection: Option<ConnectionId>,
    /// Stable name of the command ([`Command::name`]).
    pub command_kind: String,
    /// The statement's text, without the bound values.
    pub statement: Option<String>,
    /// The intent retained at decision time.
    pub intent: StatementIntent,
    /// The risk retained at decision time.
    pub risk: MutationRisk,
    /// What the `PolicyGate` answered.
    pub decision: PolicyOutcome,
    /// The reason returned with the decision, when there is one.
    pub decision_reason: Option<String>,
    /// Who approved, for a command that required consent.
    pub approved_by: Option<String>,
    /// Execution duration, when the command was run.
    pub duration: Option<Duration>,
    /// Affected rows, when the driver returns them.
    pub rows_affected: Option<u64>,
    /// Error message, if the execution failed.
    pub error: Option<String>,
    /// The error's class, when there is one.
    ///
    /// Kept apart from the message for the same reason as in
    /// [`HistoryRecord`](crate::HistoryRecord): a message gets reworded, and a
    /// reader that parsed it then silently gets it wrong. Here, though, the
    /// stakes are higher than elsewhere — it is the audit trail, the one read
    /// **after** the incident, and it is append-only: what was not written at
    /// the right moment is never added.
    ///
    /// [`Ambiguous`](ErrorClass::Ambiguous) is the case that justifies the
    /// column: it says it is not known whether the server applied the write,
    /// which no message says by itself ([I-13](../../../CLAUDE.md#i-13)).
    ///
    /// `None` on a command that did not fail — and on any row written by an
    /// Oxyn version older than the column, where it means "unknown class".
    /// The two meanings are told apart by [`Self::error`].
    pub error_class: Option<ErrorClass>,
}

impl JournalRecord {
    /// Builds an entry from the command and the decision returned.
    ///
    /// The timestamp is taken now. What is only known after execution —
    /// duration, rows, error — is added by [`completed`](Self::completed) or
    /// [`failed`](Self::failed).
    #[must_use]
    pub fn new(actor: &Actor, command: &Command, decision: &Decision) -> Self {
        let (actor_id, agent_session) = match actor {
            Actor::Human => (None, None),
            Actor::Agent { id, session } => (Some(*id), Some(*session)),
        };
        let decision_reason = match decision {
            Decision::Allow => None,
            Decision::RequireApproval { reason, .. } | Decision::Deny { reason } => {
                Some(reason.clone())
            }
        };

        Self {
            ts: Utc::now(),
            command_id: None,
            actor_kind: ActorKind::from(actor),
            actor_id,
            agent_session,
            connection: command.target_connection(),
            command_kind: command.name().to_owned(),
            statement: command.statement_text().map(|statement| {
                let language = match command {
                    Command::Execute { request, .. } => request.language,
                    _ => QueryLanguage::SQL,
                };
                redact_password_literals(statement, language)
            }),
            intent: command.intent(),
            risk: command.mutation_risk(),
            decision: PolicyOutcome::from(decision),
            decision_reason,
            approved_by: None,
            duration: None,
            rows_affected: None,
            error: None,
            error_class: None,
        }
    }

    /// Attaches the command identifier, the correlation key with the history
    /// and the approval request.
    #[must_use]
    pub fn with_command_id(mut self, command_id: CommandId) -> Self {
        self.command_id = Some(command_id);
        self
    }

    /// Records who gave consent.
    #[must_use]
    pub fn approved_by(mut self, who: impl Into<String>) -> Self {
        self.approved_by = Some(who.into());
        self
    }

    /// Records a successful execution.
    #[must_use]
    pub fn completed(mut self, duration: Duration, rows_affected: Option<u64>) -> Self {
        self.duration = Some(duration);
        self.rows_affected = rows_affected;
        self.error = None;
        self.error_class = None;
        self
    }

    /// Records a failure, with its class.
    ///
    /// The message is the domain error's. `oxyn-core` guarantees it carries
    /// neither secret nor bound value (I-03); that is the responsibility of
    /// whoever builds the variant, not of this module.
    ///
    /// The class is kept **apart**, in [`error_class`](Self::error_class): it,
    /// and not the text, says whether the server-side effect is known. An
    /// audit trail read after an incident where this information only existed
    /// as a sentence would force its reader to interpret — exactly what
    /// [`ErrorClass`] exists to avoid ([I-13](../../../CLAUDE.md#i-13)).
    #[must_use]
    pub fn failed(mut self, error: &OxynError) -> Self {
        self.error = Some(error.to_string());
        self.error_class = Some(error.class());
        self
    }
}

/// A journal entry read back.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    /// Sequence number, increasing and never reused.
    pub id: i64,
    /// The entry's content.
    pub record: JournalRecord,
}

/// Typed access to the `audit_journal` table.
///
/// **There is deliberately no write method other than
/// [`append`](Self::append).** The absence is half of the guarantee; the other
/// half is in the schema's SQLite triggers.
#[derive(Debug)]
pub struct Journal<'a> {
    store: &'a Store,
}

impl<'a> Journal<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Appends an entry and returns its sequence number.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the write fails,
    /// [`crate::StoreError::Json`] if the risk is not serializable.
    pub fn append(&self, record: &JournalRecord) -> Result<i64> {
        let risk = tag_to_json(&record.risk)?;
        let statement = record
            .statement
            .as_deref()
            .map(|sql| redact_password_literals(sql, QueryLanguage::SQL));

        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO audit_journal
                     (ts, command_id, actor_kind, actor_id, agent_session_id, connection_id,
                      command_kind, statement, intent, risk, policy_decision, decision_reason,
                      approved_by, duration_ms, rows_affected, error, error_class)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17)",
                params![
                    record.ts,
                    record.command_id.map(|id| id.to_string()),
                    record.actor_kind.as_str(),
                    record.actor_id.map(|id| id.to_string()),
                    record.agent_session.map(|id| id.to_string()),
                    record.connection.map(|id| id.to_string()),
                    record.command_kind,
                    statement,
                    record.intent.as_str(),
                    risk,
                    record.decision.as_str(),
                    record.decision_reason,
                    record.approved_by,
                    record.duration.map(duration_to_ms),
                    record.rows_affected.map(count_to_i64),
                    record.error,
                    record.error_class.map(|class| class.as_str()),
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// The last `limit` entries, most recent first.
    ///
    /// The order is that of sequence numbers, not timestamps: in an audit
    /// trail, recording order is what is authoritative, and it does not
    /// depend on the machine's clock.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn recent(&self, limit: usize) -> Result<Vec<JournalEntry>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!("{SELECT_COLUMNS} ORDER BY id DESC LIMIT ?1"))?;
            query
                .query_and_then(params![limit_to_i64(limit)], from_row)?
                .collect()
        })
    }

    /// The last `limit` entries targeting a given connection.
    ///
    /// These entries survive the connection's deletion: the table carries no
    /// foreign key to `connections`.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn for_connection(
        &self,
        connection: ConnectionId,
        limit: usize,
    ) -> Result<Vec<JournalEntry>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!(
                "{SELECT_COLUMNS} WHERE connection_id = ?1 ORDER BY id DESC LIMIT ?2"
            ))?;
            query
                .query_and_then(
                    params![connection.to_string(), limit_to_i64(limit)],
                    from_row,
                )?
                .collect()
        })
    }

    /// The last `limit` entries attributed to an agent.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn for_agent(&self, agent: AgentId, limit: usize) -> Result<Vec<JournalEntry>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!(
                "{SELECT_COLUMNS} WHERE actor_id = ?1 ORDER BY id DESC LIMIT ?2"
            ))?;
            query
                .query_and_then(params![agent.to_string(), limit_to_i64(limit)], from_row)?
                .collect()
        })
    }

    /// Total number of entries. Never decreases.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the read fails.
    pub fn count(&self) -> Result<u64> {
        self.store.with_connection(|conn| {
            let total: i64 =
                conn.query_row("SELECT COUNT(*) FROM audit_journal", [], |row| row.get(0))?;
            Ok(count_from_i64(total))
        })
    }
}

/// The column list, shared by every read so that [`from_row`] has only
/// one row shape to know.
const SELECT_COLUMNS: &str = "SELECT id, ts, command_id, actor_kind, actor_id, agent_session_id, \
     connection_id, command_kind, statement, intent, risk, policy_decision, \
     decision_reason, approved_by, duration_ms, rows_affected, error, error_class \
     FROM audit_journal";

/// Rebuilds a [`JournalEntry`] from a row.
fn from_row(row: &Row<'_>) -> Result<JournalEntry> {
    let actor_kind: String = row.get("actor_kind")?;
    let intent: String = row.get("intent")?;
    let risk: String = row.get("risk")?;
    let decision: String = row.get("policy_decision")?;
    let duration_ms: Option<i64> = row.get("duration_ms")?;
    let rows_affected: Option<i64> = row.get("rows_affected")?;
    let error_class: Option<String> = row.get("error_class")?;

    Ok(JournalEntry {
        id: row.get("id")?,
        record: JournalRecord {
            ts: row.get("ts")?,
            command_id: parse_id_opt(row.get("command_id")?, "audit_journal.command_id")?,
            actor_kind: ActorKind::from_text(&actor_kind),
            actor_id: parse_id_opt(row.get("actor_id")?, "audit_journal.actor_id")?,
            agent_session: parse_id_opt(
                row.get("agent_session_id")?,
                "audit_journal.agent_session_id",
            )?,
            connection: parse_id_opt(row.get("connection_id")?, "audit_journal.connection_id")?,
            command_kind: row.get("command_kind")?,
            statement: row.get("statement")?,
            intent: intent_from_text(&intent),
            risk: tag_from_json(&risk, "audit_journal.risk")?,
            decision: PolicyOutcome::from_text(&decision),
            decision_reason: row.get("decision_reason")?,
            approved_by: row.get("approved_by")?,
            duration: duration_ms.map(|ms| Duration::from_millis(count_from_i64(ms))),
            rows_affected: rows_affected.map(count_from_i64),
            error: row.get("error")?,
            error_class: error_class.as_deref().map(error_class_from_text),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        ConnectionConfig, DriverId, Environment, ExecRequest, Preview, QueryLanguage, ScalarValue,
        SessionId,
    };

    fn execution(connection: ConnectionId, text: &str, intent: StatementIntent) -> Command {
        Command::Execute {
            connection,
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, text)
                    .with_intent(intent)
                    .with_params(vec![ScalarValue::Text("hunter2".to_owned())]),
            ),
        }
    }

    #[test]
    fn the_journal_refuses_updates() {
        let store = Store::open_in_memory().expect("open");
        let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        let id = store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::Allow,
            ))
            .expect("insert");

        let refusal = store.with_connection(|conn| {
            Ok(conn.execute(
                "UPDATE audit_journal SET statement = 'SELECT 2' WHERE id = ?1",
                params![id],
            )?)
        });
        let error = refusal.expect_err("an UPDATE must fail");
        assert!(
            error.to_string().contains("append-only"),
            "the trigger must be the cause: {error}"
        );

        let read_back = store.journal().recent(1).expect("read back");
        assert_eq!(
            read_back[0].record.statement.as_deref(),
            Some("SELECT 1"),
            "the row must not have moved"
        );
    }

    #[test]
    fn the_journal_refuses_deletions() {
        let store = Store::open_in_memory().expect("open");
        let command = execution(
            ConnectionId::new(),
            "DROP TABLE customers",
            StatementIntent::Ddl,
        );
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::deny("read-only connection"),
            ))
            .expect("insert");

        for sql in [
            "DELETE FROM audit_journal",
            "DELETE FROM audit_journal WHERE id = 1",
        ] {
            let refusal = store.with_connection(|conn| Ok(conn.execute(sql, [])?));
            let error = refusal.expect_err("a DELETE must fail");
            assert!(error.to_string().contains("append-only"), "{sql}: {error}");
        }

        assert_eq!(store.journal().count().expect("count"), 1);
    }

    #[test]
    fn a_refused_command_is_logged_with_its_reason() {
        // A journal that only records what worked says nothing of what an
        // agent attempted.
        let store = Store::open_in_memory().expect("open");
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let command = execution(
            ConnectionId::new(),
            "GRANT ALL ON customers TO PUBLIC",
            StatementIntent::Grant,
        );

        store
            .journal()
            .append(&JournalRecord::new(
                &agent,
                &command,
                &Decision::deny("an agent does not change privileges"),
            ))
            .expect("insert");

        let entry = store.journal().recent(10).expect("read back").remove(0);
        assert_eq!(entry.record.decision, PolicyOutcome::Denied);
        assert!(entry.record.actor_kind.is_agent());
        assert_eq!(
            entry.record.decision_reason.as_deref(),
            Some("an agent does not change privileges")
        );
        assert_eq!(entry.record.intent, StatementIntent::Grant);
        assert!(entry.record.duration.is_none(), "it was not executed");
    }

    #[test]
    fn bound_values_do_not_enter_the_journal() {
        // I-03: the query's text is audit material, its bound values are not.
        let store = Store::open_in_memory().expect("open");
        let command = execution(
            ConnectionId::new(),
            "SELECT * FROM accounts WHERE password = $1",
            StatementIntent::Read,
        );
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::Allow,
            ))
            .expect("insert");

        let all: String = store
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT group_concat(COALESCE(statement, '') || COALESCE(error, '')) \
                     FROM audit_journal",
                    [],
                    |row| row.get(0),
                )?)
            })
            .expect("raw read");
        assert!(!all.contains("hunter2"), "a bound value leaked: {all}");
    }

    #[test]
    fn an_entry_round_trips_completely() {
        let store = Store::open_in_memory().expect("open");
        let connection = ConnectionId::new();
        let agent_id = AgentId::new();
        let session = AgentSessionId::new();
        let command_id = CommandId::new();
        let actor = Actor::agent(agent_id, session);
        let command = execution(connection, "DELETE FROM orders", StatementIntent::Write);

        let record = JournalRecord::new(
            &actor,
            &command,
            &Decision::approval(
                "write requested by an agent",
                Some(Preview::new("DELETE FROM orders", "customer db")),
            ),
        )
        .with_command_id(command_id)
        .approved_by("nicolas")
        .completed(Duration::from_millis(1_234), Some(42));

        let id = store.journal().append(&record).expect("insert");
        assert!(id > 0);

        let read_back = store
            .journal()
            .for_connection(connection, 10)
            .expect("read back")
            .remove(0);

        assert_eq!(read_back.id, id);
        assert_eq!(read_back.record.command_id, Some(command_id));
        assert_eq!(read_back.record.actor_id, Some(agent_id));
        assert_eq!(read_back.record.agent_session, Some(session));
        assert_eq!(read_back.record.connection, Some(connection));
        assert_eq!(read_back.record.command_kind, "Execute");
        assert_eq!(
            read_back.record.statement.as_deref(),
            Some("DELETE FROM orders")
        );
        assert_eq!(read_back.record.intent, StatementIntent::Write);
        assert_eq!(read_back.record.decision, PolicyOutcome::ApprovalRequired);
        assert_eq!(read_back.record.approved_by.as_deref(), Some("nicolas"));
        assert_eq!(
            read_back.record.duration,
            Some(Duration::from_millis(1_234))
        );
        assert_eq!(read_back.record.rows_affected, Some(42));
        assert!(read_back.record.error.is_none());
    }

    #[test]
    fn the_declared_risk_is_preserved() {
        let store = Store::open_in_memory().expect("open");
        let command = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "TRUNCATE TABLE customers")
                    .with_intent(StatementIntent::Ddl)
                    .with_risk(MutationRisk::Truncate),
            ),
        };
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::approval("TRUNCATE", None),
            ))
            .expect("insert");

        let read_back = store.journal().recent(1).expect("read back").remove(0);
        assert_eq!(read_back.record.risk, MutationRisk::Truncate);
    }

    #[test]
    fn a_failure_is_logged_without_hiding_the_decision() {
        let store = Store::open_in_memory().expect("open");
        let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        let record = JournalRecord::new(&Actor::Human, &command, &Decision::Allow)
            .completed(Duration::from_millis(5), None)
            .failed(&OxynError::Query("missing relation".into()));

        store.journal().append(&record).expect("insert");
        let read_back = store.journal().recent(1).expect("read back").remove(0);

        assert_eq!(read_back.record.decision, PolicyOutcome::Allowed);
        assert!(
            read_back
                .record
                .error
                .as_deref()
                .is_some_and(|e| e.contains("missing relation"))
        );
        assert_eq!(read_back.record.error_class, Some(ErrorClass::Permanent));
    }

    /// An agent write whose effect is unknown stays **expressible** in the
    /// audit trail.
    ///
    /// It is the question put to the journal after an incident: did this
    /// agent modify the database? "Timed out after 30 s" does not answer;
    /// `ambiguous` answers "we do not know", and it is the only honest
    /// answer. The journal being append-only, a class not written at the
    /// moment of the incident will never be added
    /// ([I-13](../../../CLAUDE.md#i-13)).
    #[test]
    fn an_error_class_survives_in_the_audit_trail() {
        let store = Store::open_in_memory().expect("open");
        let command = execution(
            ConnectionId::new(),
            "INSERT INTO orders (customer) VALUES (1)",
            StatementIntent::Write,
        );
        let record = JournalRecord::new(
            &Actor::agent(AgentId::new(), AgentSessionId::new()),
            &command,
            &Decision::Allow,
        )
        .failed(&OxynError::Timeout {
            after: Duration::from_secs(30),
        });

        store.journal().append(&record).expect("insert");
        let read_back = store.journal().recent(1).expect("read back").remove(0);

        assert_eq!(read_back.record.error_class, Some(ErrorClass::Ambiguous));
        assert!(
            !read_back
                .record
                .error_class
                .expect("a family")
                .is_retryable(),
            "an expired `INSERT` is not replayed: the server may have applied it"
        );
        // The bound value did not follow here either (I-03).
        assert!(!format!("{:?}", read_back.record).contains("hunter2"));
    }

    /// A command that succeeds carries no class: there is no error to
    /// classify, and `completed` erases what an earlier `failed` would have
    /// set.
    #[test]
    fn a_successful_command_carries_no_class() {
        let store = Store::open_in_memory().expect("open");
        let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        let record = JournalRecord::new(&Actor::Human, &command, &Decision::Allow)
            .failed(&OxynError::Query("rejected".into()))
            .completed(Duration::from_millis(3), Some(1));

        store.journal().append(&record).expect("insert");
        let read_back = store.journal().recent(1).expect("read back").remove(0);

        assert_eq!(read_back.record.error_class, None);
        assert_eq!(read_back.record.error, None);
    }

    #[test]
    fn the_journal_survives_the_connection_deletion() {
        // That is the property that makes the audit trail useful: erasing the
        // connection does not erase what was done with it.
        let store = Store::open_in_memory().expect("open");
        let workspace = store.workspaces().create("workshop").expect("workspace");
        let config = ConnectionConfig::new("customer db", DriverId::postgres())
            .with_environment(Environment::Production);
        store
            .connections()
            .save(workspace.id, &config)
            .expect("write");

        let command = execution(config.id, "DELETE FROM customers", StatementIntent::Write);
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::approval("production", None),
            ))
            .expect("insert");

        assert!(store.connections().delete(config.id).expect("deletion"));
        assert!(store.workspaces().delete(workspace.id).expect("deletion"));

        assert_eq!(store.journal().count().expect("count"), 1);
        let remaining = store
            .journal()
            .for_connection(config.id, 10)
            .expect("read back");
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].record.statement.as_deref(),
            Some("DELETE FROM customers")
        );
    }

    #[test]
    fn an_unreadable_decision_does_not_count_as_allowed() {
        let store = Store::open_in_memory().expect("open");
        let command = execution(ConnectionId::new(), "SELECT 1", StatementIntent::Read);
        store
            .journal()
            .append(&JournalRecord::new(
                &Actor::Human,
                &command,
                &Decision::Allow,
            ))
            .expect("insert");

        // An UPDATE is impossible; a row written by a future version is
        // therefore simulated by directly inserting unknown tags.
        store
            .with_connection(|conn| {
                Ok(conn.execute(
                    "INSERT INTO audit_journal
                         (ts, actor_kind, command_kind, intent, risk, policy_decision)
                     VALUES (?1, 'quantum', 'Execute', 'levitate', '\"none\"', 'maybe')",
                    params![Utc::now()],
                )?)
            })
            .expect("insert");

        let read_back = store.journal().recent(1).expect("read back").remove(0);
        assert_eq!(read_back.record.decision, PolicyOutcome::Denied);
        assert!(read_back.record.actor_kind.is_agent());
        assert_eq!(read_back.record.intent, StatementIntent::Unknown);
        assert!(read_back.record.intent.is_mutating());
    }

    #[test]
    fn entries_come_out_in_reverse_recording_order() {
        let store = Store::open_in_memory().expect("open");
        let connection = ConnectionId::new();
        for n in 0..5 {
            let command = execution(connection, &format!("SELECT {n}"), StatementIntent::Read);
            store
                .journal()
                .append(&JournalRecord::new(
                    &Actor::Human,
                    &command,
                    &Decision::Allow,
                ))
                .expect("insert");
        }

        let recent = store.journal().recent(3).expect("read back");
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].record.statement.as_deref(), Some("SELECT 4"));
        assert_eq!(recent[2].record.statement.as_deref(), Some("SELECT 2"));
    }
}
