//! The `query_history` table: what the user ran.
//!
//! # What sets it apart from the audit journal
//!
//! The history and the journal ([`crate::journal`]) record the same thing,
//! but do not answer the same question, and **only the history can be
//! erased**:
//!
//! | | `query_history` | `audit_journal` |
//! |---|---|---|
//! | Question | "what did I run yesterday?" | "what was allowed, and to whom?" |
//! | Content | executions | **all** commands, refusals included |
//! | Erasable | yes, [`purge_before`](History::purge_before) and [`clear`](History::clear) | **no** |
//!
//! Purging the history does **not touch** the journal. That is what makes it
//! possible to offer "clear my history" without opening a way to erase an
//! agent's trace — the convenient feature that, without this separation, would
//! end up being the hole in the audit trail.
//!
//! # The connection name is copied
//!
//! The `connection_id` column has no foreign key and the name is denormalized
//! on write: deleting a connection does not erase the history, and a row whose
//! connection no longer exists stays readable.

mod listing;
pub use listing::{HistoryConnectionPage, HistoryConnectionSummary, HistoryPage, HistorySummary};

use chrono::{DateTime, Utc};
use oxyn_core::{
    Actor, AgentId, Command, ConnectionId, ErrorClass, OxynError, QueryLanguage, StatementIntent,
};
use oxyn_query::redact_password_literals;
use rusqlite::{OptionalExtension, Row, params};
use std::time::Duration;

use crate::encoding::{
    count_from_i64, count_to_i64, duration_to_ms, error_class_from_text, escape_like,
    intent_from_text, limit_to_i64, parse_id_opt, tag_from_json, tag_to_json,
};
use crate::error::Result;
use crate::journal::ActorKind;
use crate::store::Store;

/// How an execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HistoryStatus {
    /// Submitted, not finished yet.
    Running,
    /// Finished without error.
    Succeeded,
    /// The server or the driver returned an error.
    Failed,
    /// Interrupted on request.
    Cancelled,
    /// Refused by the `PolicyGate`: it never reached the server.
    Denied,
}

impl HistoryStatus {
    /// Stable name, the one written in the `status` column.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Denied => "denied",
        }
    }

    /// Reads back the `status` column.
    ///
    /// An unknown value returns [`Failed`](Self::Failed): an execution that
    /// cannot be said to have succeeded did not succeed.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "denied" => Self::Denied,
            _ => {
                tracing::warn!(
                    column = "status",
                    "unknown history status in local state, falling back to `failed`"
                );
                Self::Failed
            }
        }
    }
}

impl std::fmt::Display for HistoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An execution, as recorded in the history.
#[derive(Debug, Clone)]
pub struct HistoryRecord {
    /// When the execution was submitted.
    pub ts: DateTime<Utc>,
    /// The targeted connection.
    pub connection: Option<ConnectionId>,
    /// The connection's **name**, copied to survive its deletion.
    pub connection_name: Option<String>,
    /// Human or agent.
    pub actor_kind: ActorKind,
    /// Which agent, if any.
    pub actor_id: Option<AgentId>,
    /// The query's language, dialect included.
    pub language: QueryLanguage,
    /// The text, as the user or the agent wrote it — without the bound values
    /// (I-03).
    pub statement: String,
    /// The retained intent.
    pub intent: StatementIntent,
    /// Execution duration, once finished.
    pub duration: Option<Duration>,
    /// Rows produced or affected, when known.
    pub rows: Option<u64>,
    /// How it ended.
    pub status: HistoryStatus,
    /// Error message or refusal reason.
    pub error: Option<String>,
    /// The error's class, when there is one.
    ///
    /// Kept apart from the message on purpose: **it** is what a caller reads
    /// to decide whether it may offer a rerun, never the text. A message
    /// changes — it gets reworded, translated — and a caller that parsed it
    /// then silently breaks ([`ErrorClass`], [I-13](../../../CLAUDE.md#i-13)).
    ///
    /// [`Ambiguous`](ErrorClass::Ambiguous) is the case that matters: the
    /// server may have applied the write, and replaying creates a duplicate in
    /// the user's data.
    ///
    /// **`None` has two meanings, which is why it is not read directly.** On a
    /// successful row, it says "no error"; on a row written by an Oxyn version
    /// older than the column, it says "unknown class" — and those rows carry
    /// precisely the expired writes that must not be replayed. Going through
    /// [`Self::is_retryable`] rather than this field closes the confusion.
    ///
    /// On a [`Denied`](HistoryStatus::Denied) row, the class does **not**
    /// describe a server verdict — nothing reached it. It only says the row
    /// cannot be replayed. A count of server failures therefore filters on
    /// `status`, not on this column alone.
    pub error_class: Option<ErrorClass>,
    /// A result identity from this application run; it may have expired.
    pub result: Option<oxyn_core::ResultId>,
    /// When the user confirmed having inspected the server state of an
    /// unresolved write. Never set by Oxyn on its own: it is a statement about
    /// the server that only the user can make.
    pub reconciled_at: Option<DateTime<Utc>>,
}

impl HistoryRecord {
    /// Whether replay controls must be withheld until the server state is reconciled.
    pub fn requires_reconciliation(&self) -> bool {
        self.reconciled_at.is_none()
            && requires_reconciliation(self.intent, self.status, self.error_class)
    }

    /// Builds a history entry for an execution that starts.
    #[must_use]
    pub fn new(actor: &Actor, language: QueryLanguage, statement: impl Into<String>) -> Self {
        Self {
            ts: Utc::now(),
            connection: None,
            connection_name: None,
            actor_kind: ActorKind::from(actor),
            actor_id: match actor {
                Actor::Human => None,
                Actor::Agent { id, .. } => Some(*id),
            },
            language,
            statement: statement.into(),
            intent: StatementIntent::Unknown,
            duration: None,
            rows: None,
            status: HistoryStatus::Running,
            error: None,
            error_class: None,
            result: None,
            reconciled_at: None,
        }
    }

    /// Builds an entry from an execution command.
    ///
    /// Returns `None` for any other command: a `Connect` or an `Export` are
    /// not queries, and listing them in the *query* history would make it
    /// unreadable. They stay in the audit journal, which records them all
    /// ([`crate::journal`]).
    #[must_use]
    pub fn from_command(actor: &Actor, command: &Command) -> Option<Self> {
        match command {
            Command::Execute {
                connection,
                request,
                ..
            } => {
                let mut record = Self::new(
                    actor,
                    request.language,
                    redact_password_literals(&request.text, request.language),
                );
                record.connection = Some(*connection);
                record.intent = request.intent;
                Some(record)
            }
            _ => None,
        }
    }

    /// Names the targeted connection.
    #[must_use]
    pub fn on_connection(mut self, id: ConnectionId, name: impl Into<String>) -> Self {
        self.connection = Some(id);
        self.connection_name = Some(name.into());
        self
    }

    /// Declares the retained intent.
    #[must_use]
    pub fn with_intent(mut self, intent: StatementIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Marks the execution as succeeded.
    #[must_use]
    pub fn succeeded(mut self, duration: Duration, rows: Option<u64>) -> Self {
        self.status = HistoryStatus::Succeeded;
        self.duration = Some(duration);
        self.rows = rows;
        self.error = None;
        self.error_class = None;
        self
    }

    /// Marks the execution as failed.
    ///
    /// A cancellation ([`OxynError::Cancelled`]) is classified
    /// [`Cancelled`](HistoryStatus::Cancelled), not `Failed`: it is not a
    /// failure, it is a user decision, and conflating them skews any reading
    /// of the failure rate.
    ///
    /// A policy refusal ([`OxynError::PolicyDenied`]) is classified
    /// [`Denied`](HistoryStatus::Denied), not `Failed`: nothing was submitted
    /// to the server, and presenting it as a failure would send the user
    /// looking for an incident that did not happen. The last barrier before
    /// the driver returns this refusal as an error; it must read like the
    /// refusals returned earlier by the `PolicyGate`.
    ///
    /// The error's class is kept **apart** in
    /// [`error_class`](Self::error_class), never merged into the message
    /// (I-13).
    #[must_use]
    pub fn failed(mut self, error: &OxynError) -> Self {
        self.status = match error {
            _ if error.is_cancelled() => HistoryStatus::Cancelled,
            OxynError::PolicyDenied { .. } => HistoryStatus::Denied,
            _ => HistoryStatus::Failed,
        };
        self.error = Some(error.to_string());
        self.error_class = Some(error.class());
        self
    }

    /// Marks the execution as refused by the policy.
    ///
    /// The retained class is [`Permanent`](ErrorClass::Permanent): a refusal
    /// is not replayed, it is corrected. Any row whose class is not
    /// [`Transient`](ErrorClass::Transient) is out of reach of a "rerun"
    /// button.
    #[must_use]
    pub fn denied(mut self, reason: impl Into<String>) -> Self {
        self.status = HistoryStatus::Denied;
        self.error = Some(reason.into());
        self.error_class = Some(ErrorClass::Permanent);
        self
    }

    /// Can this execution be resubmitted as is?
    ///
    /// **Only** the [`Transient`](ErrorClass::Transient) class answers `true`.
    /// Everything else answers `false`, including the absence of a class: a
    /// row written before the column existed may be an expired write, and
    /// replaying it would create a silent duplicate in the user's data
    /// ([I-13](../../../CLAUDE.md#i-13)).
    ///
    /// It is the only question a caller needs to ask: reading
    /// [`error_class`](Self::error_class) to answer it oneself reintroduces
    /// the interpretation of `None` this method exists to avoid.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        // The rule lives in `ErrorClass`, not here: copying it would make two
        // definitions of "replayable" diverge the day a class is added.
        self.error_class.is_some_and(|class| class.is_retryable())
    }
}

/// What [`History::reconcile`] did with an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reconciliation {
    /// The acknowledgement is recorded, now or earlier.
    Recorded,
    /// Gone, or never in doubt: acknowledging it would vouch for nothing.
    NotNeeded,
    /// Still running in this launch: its outcome is not known yet.
    StillRunning,
}

/// A history entry read back.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Sequence number.
    pub id: i64,
    /// The entry's content.
    pub record: HistoryRecord,
}

/// Typed access to the `query_history` table.
#[derive(Debug)]
pub struct History<'a> {
    store: &'a Store,
}

impl<'a> History<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Records an execution and returns its sequence number.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Json`].
    pub fn record(&self, record: &HistoryRecord) -> Result<i64> {
        let language = tag_to_json(&record.language)?;
        let statement = redact_password_literals(&record.statement, record.language);

        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO query_history
                     (ts, connection_id, connection_name, actor_kind, actor_id, language,
                      statement, intent, duration_ms, row_count, status, error, error_class, result_id,
                      reconciled_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    record.ts,
                    record.connection.map(|id| id.to_string()),
                    record.connection_name,
                    record.actor_kind.as_str(),
                    record.actor_id.map(|id| id.to_string()),
                    language,
                    statement,
                    record.intent.as_str(),
                    record.duration.map(duration_to_ms),
                    record.rows.map(count_to_i64),
                    record.status.as_str(),
                    record.error,
                    record.error_class.map(|class| class.as_str()),
                    record.result.map(|result| result.to_string()),
                    record.reconciled_at,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Completes an entry recorded during execution: outcome, duration, rows,
    /// error. Returns `true` if the row existed.
    ///
    /// The entry is first recorded as [`Running`](HistoryStatus::Running) at
    /// submission time — the user sees their query in the history while it
    /// runs —, then completed at the end.
    ///
    /// The text, the connection and the timestamp are **not** rewritten: what
    /// was submitted does not change retroactively. It is also the difference
    /// with the audit journal, where this method does not exist and where the
    /// file itself would refuse it ([`crate::journal`]).
    ///
    /// A reconciliation declared while the execution was running is erased: it
    /// was about a server state the outcome has just changed, and a new
    /// ambiguous outcome must alert again.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the write fails.
    pub fn finish(&self, id: i64, record: &HistoryRecord) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touched = conn.execute(
                "UPDATE query_history
                    SET status = ?2, duration_ms = ?3, row_count = ?4, error = ?5,
                        error_class = ?6, result_id = ?7, reconciled_at = NULL
                  WHERE id = ?1",
                params![
                    id,
                    record.status.as_str(),
                    record.duration.map(duration_to_ms),
                    record.rows.map(count_to_i64),
                    record.error,
                    record.error_class.map(|class| class.as_str()),
                    record.result.map(|result| result.to_string()),
                ],
            )?;
            Ok(touched > 0)
        })
    }

    /// Records that the user inspected the server state of an unresolved write.
    ///
    /// A row still `running` since `live_since` — the start of the launch that
    /// asks — is refused: its outcome is not known yet, and an acknowledgement
    /// taken now would survive a crash before `finish`, silencing the very
    /// warning recovery exists for. A `running` row older than that was left by
    /// an earlier launch that never finished it.
    ///
    /// An entry already reconciled keeps its first date. Only this column
    /// changes: what was submitted and how it ended stay as recorded, and
    /// nothing is retried ([I-13](../../../CLAUDE.md#i-13)).
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the read or the write fails.
    pub fn reconcile(
        &self,
        id: i64,
        live_since: DateTime<Utc>,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<Reconciliation> {
        self.store.with_connection_cancellable(cancel, |connection| {
            let transaction = connection.unchecked_transaction()?;
            let row = transaction
                .query_row(
                    "SELECT ts, intent, status, error_class, reconciled_at FROM query_history WHERE id=?1",
                    [id],
                    |row| {
                        Ok((
                            row.get::<_, DateTime<Utc>>("ts")?,
                            row.get::<_, String>("intent")?,
                            row.get::<_, String>("status")?,
                            row.get::<_, Option<String>>("error_class")?,
                            row.get::<_, Option<String>>("reconciled_at")?,
                        ))
                    },
                )
                .optional()?;
            let Some((ts, intent, status, error_class, reconciled_at)) = row else {
                return Ok(Reconciliation::NotNeeded);
            };
            if reconciled_at.is_some() {
                return Ok(Reconciliation::Recorded);
            }
            let status = HistoryStatus::from_text(&status);
            if !requires_reconciliation(
                intent_from_text(&intent),
                status,
                error_class.as_deref().map(error_class_from_text),
            ) {
                return Ok(Reconciliation::NotNeeded);
            }
            if status == HistoryStatus::Running && ts >= live_since {
                return Ok(Reconciliation::StillRunning);
            }
            transaction.execute(
                "UPDATE query_history SET reconciled_at=?2 WHERE id=?1",
                params![id, Utc::now()],
            )?;
            transaction.commit()?;
            Ok(Reconciliation::Recorded)
        })
    }

    /// The `limit` most recent executions.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!(
                "{SELECT_COLUMNS} ORDER BY ts DESC, id DESC LIMIT ?1"
            ))?;
            query
                .query_and_then(params![limit_to_i64(limit)], from_row)?
                .collect()
        })
    }

    /// The `limit` most recent executions on a connection.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn for_connection(
        &self,
        connection: ConnectionId,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!(
                "{SELECT_COLUMNS} WHERE connection_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2"
            ))?;
            query
                .query_and_then(
                    params![connection.to_string(), limit_to_i64(limit)],
                    from_row,
                )?
                .collect()
        })
    }

    /// Searches `needle` in the query text, case-insensitively.
    ///
    /// `needle`'s `LIKE` metacharacters are escaped: searching `100%` finds
    /// `100%`, not every row. The pattern is **bound**, never concatenated
    /// (I-10).
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn search(&self, needle: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let pattern = format!("%{}%", escape_like(needle));
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!(
                "{SELECT_COLUMNS} WHERE statement LIKE ?1 ESCAPE '\\' \
                 ORDER BY ts DESC, id DESC LIMIT ?2"
            ))?;
            query
                .query_and_then(params![pattern, limit_to_i64(limit)], from_row)?
                .collect()
        })
    }

    /// Number of entries.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the read fails.
    pub fn count(&self) -> Result<u64> {
        self.store.with_connection(|conn| {
            let total: i64 =
                conn.query_row("SELECT COUNT(*) FROM query_history", [], |row| row.get(0))?;
            Ok(count_from_i64(total))
        })
    }

    /// Erases the entries older than `cutoff` and returns their number.
    ///
    /// **Does not touch the audit journal.**
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the deletion fails.
    pub fn purge_before(&self, cutoff: DateTime<Utc>) -> Result<usize> {
        self.store.with_connection(|conn| {
            Ok(conn.execute("DELETE FROM query_history WHERE ts < ?1", params![cutoff])?)
        })
    }

    /// Erases the whole history and returns the number of deleted entries.
    ///
    /// **Does not touch the audit journal**: that is precisely the guarantee
    /// that makes it possible to offer this feature.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the deletion fails.
    pub fn clear(&self) -> Result<usize> {
        self.store
            .with_connection(|conn| Ok(conn.execute("DELETE FROM query_history", [])?))
    }
}

/// One rule for a row read whole and for the startup scan, which reads only
/// these three columns: two copies would drift, and the recovery warning would
/// then stay silent over a write the history still flags.
fn requires_reconciliation(
    intent: StatementIntent,
    status: HistoryStatus,
    error_class: Option<ErrorClass>,
) -> bool {
    error_class == Some(ErrorClass::Ambiguous)
        || (intent.is_mutating()
            && (matches!(status, HistoryStatus::Running | HistoryStatus::Cancelled)
                || status == HistoryStatus::Failed && error_class.is_none()))
}

/// The column list, shared by every read.
const SELECT_COLUMNS: &str = "SELECT id, ts, connection_id, connection_name, actor_kind, \
     actor_id, language, statement, intent, duration_ms, row_count, status, error, error_class, result_id, \
     reconciled_at FROM query_history";

/// Rebuilds a [`HistoryEntry`] from a row.
fn from_row(row: &Row<'_>) -> Result<HistoryEntry> {
    let actor_kind: String = row.get("actor_kind")?;
    let language: String = row.get("language")?;
    let intent: String = row.get("intent")?;
    let status: String = row.get("status")?;
    let duration_ms: Option<i64> = row.get("duration_ms")?;
    let rows: Option<i64> = row.get("row_count")?;
    let error_class: Option<String> = row.get("error_class")?;

    Ok(HistoryEntry {
        id: row.get("id")?,
        record: HistoryRecord {
            ts: row.get("ts")?,
            connection: parse_id_opt(row.get("connection_id")?, "query_history.connection_id")?,
            connection_name: row.get("connection_name")?,
            actor_kind: ActorKind::from_text(&actor_kind),
            actor_id: parse_id_opt(row.get("actor_id")?, "query_history.actor_id")?,
            language: tag_from_json(&language, "query_history.language")?,
            statement: row.get("statement")?,
            intent: intent_from_text(&intent),
            duration: duration_ms.map(|ms| Duration::from_millis(count_from_i64(ms))),
            rows: rows.map(count_from_i64),
            status: HistoryStatus::from_text(&status),
            error: row.get("error")?,
            error_class: error_class.as_deref().map(error_class_from_text),
            result: parse_id_opt(row.get("result_id")?, "query_history.result_id")?,
            reconciled_at: row.get("reconciled_at")?,
        },
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod library_tests;
