//! Table `ai_egress`: which data left for an AI provider, from where, and to
//! whom.
//!
//! # Why a table of its own
//!
//! `audit_journal` answers « what command was authorised, and to whom ». It
//! cannot answer « which data from `clients` went to which provider »: its
//! columns — `command_kind`, the policy triad, `statement`, `rows_affected` —
//! mean something else, and bending them would make both questions unreadable.
//! A read of a sample is journalled there as the `PreviewRelation` it is; the
//! fact that its rows then **left the machine** is journalled here, and
//! `command_id` ties the two together.
//!
//! # What an entry holds, and what it can never hold
//!
//! The source relation, the **names** of the columns sent, how many rows, and
//! the recipient — provider, model, reach. **Never a value.** There is no
//! column to put one in, and the file refuses what could smuggle one in the
//! column list: an element that is not a string, a name longer than an
//! identifier, a list longer than a relation. Nor any token: the recipient is a
//! declaration identifier, never its key ([I-03](../../../CLAUDE.md#i-03)).
//!
//! # Retention: the audit journal's, which is none
//!
//! Like `audit_journal`, this table is **append-only** and is never pruned.
//! [`Egress`] exposes [`append`](Egress::append) and reads — no `update`, no
//! `delete` — and two triggers abort any `UPDATE` or `DELETE`, `sqlite3`
//! included. An egress record that could be erased would answer the one
//! question it exists for with « nothing left », which is the worst possible
//! wrong answer. No foreign key either: the record survives the deletion of the
//! connection, the provider and the conversation it names.
//!
//! What that costs is bounded per entry — see [`MAX_COLUMNS`] and friends —
//! and not in total, exactly as for `audit_journal`.
//!
//! Every method here can block and takes the store lock: never call them from
//! the interface thread ([I-05](../../../CLAUDE.md#i-05)).

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
use oxyn_core::{CommandId, ConnectionId, ConversationId, ProviderId};
use rusqlite::{Row, params};

use crate::encoding::{parse_id, parse_id_opt};
use crate::error::{Result, StoreError};
use crate::store::Store;

/// Most column names one entry records.
///
/// A sample is approved column by column; a list past this is not an approval
/// anyone read.
pub const MAX_COLUMNS: usize = 256;

/// Longest column name recorded, in bytes.
///
/// Longer than any identifier the supported engines accept. A « name » past it
/// is not a name — it is where a value would be smuggled.
pub const MAX_COLUMN_NAME_BYTES: usize = 256;

/// Longest source path recorded, in bytes — three identifiers and separators.
pub const MAX_SOURCE_BYTES: usize = 1024;

/// Most rows one entry records, the bound the bus puts on a preview.
pub const MAX_ROWS: u32 = 1_000;

/// Longest model name recorded, in bytes. The same bound as a provider
/// declaration's model (`oxyn_core::MAX_PROVIDER_MODEL_BYTES`).
pub const MAX_MODEL_BYTES: usize = 128;

/// Most entries one [`Egress::for_connection`] call returns.
///
/// One entry is at most about 70 KiB of column names, source and recipient, so
/// a page holds at most about 7 MiB, whatever the table contains.
pub const MAX_EGRESS_PAGE: u16 = 100;

/// Where the recipient runs, as measured when the data was sent.
///
/// Persisted here although `oxyn-llm` refuses to persist a reach, and the two
/// are consistent: a stored reach would be *yesterday's DNS answer applied to
/// today's send*, while this one is the answer **that governed this send**. It
/// is a historical fact, not a cached classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EgressReach {
    /// A loopback address: the data did not leave the machine.
    Local,
    /// The data left the machine.
    Remote,
    /// Unknown — an unresolvable host, or an external agent, whose reach cannot
    /// be known. Counts as remote.
    Unresolved,
}

impl EgressReach {
    /// The stable word written in `reach`, in English.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Unresolved => "unresolved",
        }
    }

    /// Reads `reach`, falling back to [`Unresolved`](Self::Unresolved).
    ///
    /// Never to `Local`: an unreadable reach must not turn a send that may
    /// have left the machine into one that did not.
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "local" => Self::Local,
            "remote" => Self::Remote,
            "unresolved" => Self::Unresolved,
            _ => {
                tracing::warn!(
                    column = "reach",
                    "unknown reach in local state, falling back to `unresolved`"
                );
                Self::Unresolved
            }
        }
    }

    /// Did the data leave the machine, as far as anyone knows?
    #[must_use]
    pub const fn leaves_machine(&self) -> bool {
        !matches!(self, Self::Local)
    }
}

/// One send of data to an AI recipient, as it is written.
///
/// The `Debug` is hand-written: it shows the source and the column **names** —
/// which is what an audit reads — and the recipient, and nothing else.
#[derive(Clone, PartialEq, Eq)]
pub struct EgressRecord {
    /// When the data was handed to the recipient.
    pub ts: DateTime<Utc>,
    /// The connection the data came from.
    pub connection: ConnectionId,
    /// The command that read the data, journalled in `audit_journal`.
    pub command: Option<CommandId>,
    /// The source, as `CatalogPath` renders it (`catalog.schema.table`).
    pub source: String,
    /// The names of the columns sent, in the order sent. Never their values.
    pub columns: Vec<String>,
    /// How many rows were sent.
    pub rows: u32,
    /// The provider or external-agent declaration that received the data.
    pub recipient: ProviderId,
    /// The model, when there is one to know. `None` for an external agent.
    pub model: Option<String>,
    /// Where the recipient ran, as measured for this send.
    pub reach: EgressReach,
    /// The conversation the send belonged to, once it is persisted.
    pub conversation: Option<ConversationId>,
    /// The exchange within that conversation.
    pub node: Option<u32>,
}

impl EgressRecord {
    /// A send of `rows` rows of `columns` from `source`, timestamped now.
    #[must_use]
    pub fn new(
        connection: ConnectionId,
        source: impl Into<String>,
        columns: Vec<String>,
        rows: u32,
        recipient: ProviderId,
        reach: EgressReach,
    ) -> Self {
        Self {
            ts: Utc::now(),
            connection,
            command: None,
            source: source.into(),
            columns,
            rows,
            recipient,
            model: None,
            reach,
            conversation: None,
            node: None,
        }
    }

    /// Ties the send to the command that read the data.
    #[must_use]
    pub fn read_by(mut self, command: CommandId) -> Self {
        self.command = Some(command);
        self
    }

    /// Names the model that received it.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Ties the send to a conversation exchange.
    #[must_use]
    pub fn in_conversation(mut self, conversation: ConversationId, node: Option<u32>) -> Self {
        self.conversation = Some(conversation);
        self.node = node;
        self
    }

    /// Checks the bounds the file also enforces, so the refusal names the
    /// field rather than surfacing as an SQLite constraint error.
    fn validate(&self) -> Result<()> {
        if self.source.is_empty() {
            return Err(invalid("ai_egress.source", "empty source"));
        }
        within("ai_egress.source", self.source.len(), MAX_SOURCE_BYTES)?;
        if self.columns.is_empty() {
            return Err(invalid("ai_egress.columns", "no column recorded"));
        }
        within("ai_egress.columns", self.columns.len(), MAX_COLUMNS)?;
        for name in &self.columns {
            if name.is_empty() {
                return Err(invalid("ai_egress.columns", "empty column name"));
            }
            within("ai_egress.columns", name.len(), MAX_COLUMN_NAME_BYTES)?;
        }
        // An identifier never holds a control character; a pasted row does.
        if self
            .columns
            .iter()
            .chain(std::iter::once(&self.source))
            .any(|text| text.chars().any(char::is_control))
        {
            return Err(invalid(
                "ai_egress.source or ai_egress.columns",
                "a name holds a control character",
            ));
        }
        if self.rows > MAX_ROWS {
            return Err(StoreError::TooLarge {
                field: "ai_egress.row_count",
                limit: u64::from(MAX_ROWS),
            });
        }
        if let Some(model) = &self.model {
            within("ai_egress.model", model.len(), MAX_MODEL_BYTES)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for EgressRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EgressRecord")
            .field("source", &self.source)
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .field("reach", &self.reach)
            .finish_non_exhaustive()
    }
}

/// An entry as it is re-read, with its place in the journal.
#[derive(Debug, Clone)]
pub struct EgressEntry {
    /// Recording order. This, not the timestamp, is what orders the journal.
    pub id: i64,
    /// What the entry holds.
    pub record: EgressRecord,
}

/// One bounded page of a connection's egress journal, newest first.
#[derive(Debug, Clone)]
pub struct EgressPage {
    /// At most [`MAX_EGRESS_PAGE`] entries.
    pub entries: Vec<EgressEntry>,
    /// The `before` to pass for the next, older page; `None` once exhausted.
    pub next: Option<i64>,
}

/// Typed access to `ai_egress`, in **append only**.
#[derive(Debug)]
pub struct Egress<'a> {
    store: &'a Store,
}

impl<'a> Egress<'a> {
    /// Binds the accessor to its store.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Records a send and returns its recording order.
    ///
    /// **Call it before the data is handed to the recipient, and do not send
    /// if it fails.** A send that happened without its record is precisely
    /// the gap this table exists to close; a record without its send only
    /// overstates what left, which is the safe direction.
    ///
    /// # Errors
    /// [`StoreError::Corrupted`] for an empty source, an empty or
    /// control-character name, or no column; [`StoreError::TooLarge`] past
    /// [`MAX_SOURCE_BYTES`], [`MAX_COLUMNS`], [`MAX_COLUMN_NAME_BYTES`],
    /// [`MAX_ROWS`] or [`MAX_MODEL_BYTES`]; [`StoreError::Json`] if the list
    /// cannot be encoded; [`StoreError::Sqlite`] if the write fails. No message
    /// quotes a name or the source.
    pub fn append(&self, record: &EgressRecord) -> Result<i64> {
        record.validate()?;
        let columns = serde_json::to_string(&record.columns)?;
        self.store.with_connection(|connection| {
            connection.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, command_id, source, columns, row_count,
                      recipient_id, model, reach, conversation_id, node)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    record.ts,
                    record.connection.to_string(),
                    record.command.map(|id| id.to_string()),
                    record.source,
                    columns,
                    i64::from(record.rows),
                    record.recipient.as_str(),
                    record.model,
                    record.reach.as_str(),
                    record.conversation.map(|id| id.to_string()),
                    record.node.map(i64::from),
                ],
            )?;
            Ok(connection.last_insert_rowid())
        })
    }

    /// One page of what left from `connection`, newest first.
    ///
    /// `before` is the recording order to read below, `None` for the newest
    /// page; the answer's `next` is what to pass for the following one.
    /// `limit` is clamped to `1..=`[`MAX_EGRESS_PAGE`]. Entries survive the
    /// deletion of the connection.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], or [`StoreError::Corrupted`] if a stored
    /// identifier is unreadable — reading an audit trail does not guess an
    /// identity. An unreadable `reach` falls back to `unresolved`, an
    /// unreadable column list reads as empty: both warn.
    pub fn for_connection(
        &self,
        connection: ConnectionId,
        before: Option<i64>,
        limit: u16,
    ) -> Result<EgressPage> {
        let limit = limit.clamp(1, MAX_EGRESS_PAGE);
        self.store.with_connection(|handle| {
            let mut query = handle.prepare(
                "SELECT id, ts, connection_id, command_id, source, columns, row_count,
                        recipient_id, model, reach, conversation_id, node
                   FROM ai_egress
                  WHERE connection_id = ?1 AND (?2 IS NULL OR id < ?2)
                  ORDER BY id DESC
                  LIMIT ?3",
            )?;
            // One more row than the page, to know whether another exists: an
            // entry is small, unlike a transcript turn.
            let mut entries = query
                .query_and_then(
                    params![connection.to_string(), before, i64::from(limit) + 1],
                    entry_from_row,
                )?
                .collect::<Result<Vec<_>>>()?;
            let more = entries.len() > usize::from(limit);
            if more {
                entries.pop();
            }
            let next = if more {
                entries.last().map(|entry| entry.id)
            } else {
                None
            };
            Ok(EgressPage { entries, next })
        })
    }
}

/// A validation refusal naming the field, never the value.
fn invalid(field: &'static str, detail: &str) -> StoreError {
    StoreError::Corrupted {
        field,
        detail: detail.to_owned(),
    }
}

/// Refuses a length past its bound.
fn within(field: &'static str, len: usize, limit: usize) -> Result<()> {
    if len > limit {
        return Err(StoreError::TooLarge {
            field,
            limit: u64::try_from(limit).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

/// Rebuilds an entry from a row.
fn entry_from_row(row: &Row<'_>) -> Result<EgressEntry> {
    let connection: String = row.get("connection_id")?;
    let recipient: String = row.get("recipient_id")?;
    let columns: String = row.get("columns")?;
    let reach: String = row.get("reach")?;
    let rows: i64 = row.get("row_count")?;
    let node: Option<i64> = row.get("node")?;
    Ok(EgressEntry {
        id: row.get("id")?,
        record: EgressRecord {
            ts: row.get("ts")?,
            connection: parse_id(&connection, "ai_egress.connection_id")?,
            command: parse_id_opt(row.get("command_id")?, "ai_egress.command_id")?,
            source: row.get("source")?,
            columns: serde_json::from_str(&columns).unwrap_or_else(|_| {
                tracing::warn!(
                    column = "columns",
                    "unreadable column list in local state, read as empty"
                );
                Vec::new()
            }),
            rows: u32::try_from(rows).unwrap_or(u32::MAX),
            recipient: ProviderId::new(recipient).map_err(|err| StoreError::Corrupted {
                field: "ai_egress.recipient_id",
                detail: err.detail().to_owned(),
            })?,
            model: row.get("model")?,
            reach: EgressReach::from_text(&reach),
            conversation: parse_id_opt(row.get("conversation_id")?, "ai_egress.conversation_id")?,
            node: node.and_then(|node| u32::try_from(node).ok()),
        },
    })
}
