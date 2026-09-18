//! The tree of a conversation: exchanges, their versions, and the one shown.
//!
//! # What an exchange is
//!
//! One question and what became of it. A follow-up is a **child** of the
//! exchange it follows; regenerating or editing a question adds a **sibling** —
//! another version of that exchange — and the versions stay navigable. What the
//! panel shows is the chain of ancestors of one leaf, the
//! [`selected`](super::Conversation::selected) node.
//!
//! # What the file makes impossible, rather than checks
//!
//! * **A cycle.** The store assigns a node's identifier, one past the largest,
//!   and the file requires `parent < node`: a parent is always older than its
//!   child, so no chain of parents can come back on itself.
//! * **A parent from another conversation.** The foreign key is composite,
//!   `(conversation_id, parent)`: there is no row for it to point at elsewhere.
//! * **Moving a node.** A trigger refuses to update its conversation, identifier
//!   or parent.
//!
//! # An exchange that received a sample keeps its question only
//!
//! The user's decision. Such an exchange keeps its question, the **counts** of
//! the sample — rows and columns, never their names — and its outcome. Never
//! the answer, the reasoning, a tool call nor an error message: the answer may
//! quote the sample's values, and the workspace file is one of the six channels
//! of [I-03](../../../../CLAUDE.md#i-03).
//!
//! [`Conversations::withhold_sample`] erases what was already written for the
//! exchange, and [`Conversations::append`] refuses what comes after. The file
//! holds the same rule in triggers, so it binds `sqlite3` as well — and since
//! the store opens with `secure_delete`, the erased text does not linger in the
//! file's free pages either.
//!
//! Every method here can block and takes the store lock: never call them from
//! the interface thread ([I-05](../../../../CLAUDE.md#i-05)).

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
use oxyn_core::{ConversationId, PrivacyTier, ProviderId};
use rusqlite::{OptionalExtension, Row, params};

use super::{Conversations, Destination, DestinationKind, MAX_TURN_TEXT_BYTES};
use crate::encoding::privacy_tier_from_column;
use crate::error::{Result, StoreError};

/// Exchanges one conversation holds, versions included.
///
/// The bound the panel already has in memory (`MAX_NODES` in the desktop
/// backend). The file holds it too: `node` is checked to lie in `0..256`.
pub const MAX_EXCHANGES_PER_CONVERSATION: u32 = 256;

/// Longest destination label accepted, in bytes — the bound a provider or
/// external-agent declaration already has (`oxyn_core::MAX_PROVIDER_LABEL_BYTES`).
pub const MAX_DESTINATION_LABEL_BYTES: usize = 128;

/// Longest model name accepted, in bytes.
pub const MAX_DESTINATION_MODEL_BYTES: usize = 128;

/// Most sample rows an exchange counts — the bound the bus puts on a preview.
pub const MAX_SAMPLE_ROWS: u32 = 1_000;

/// Most sample columns an exchange counts.
pub const MAX_SAMPLE_COLUMNS: u32 = 256;

/// Most exchanges one [`Conversations::branch_page`] call returns.
///
/// # What one page costs at worst
///
/// Computed from the bounds in the file and the measured layout of
/// [`Exchange`] (`EXCHANGE_LAYOUT_BYTES`, checked by a test):
///
/// | Per exchange | Read from disk | Decoded |
/// |---|---|---|
/// | question | 1 048 576 | 1 048 576 |
/// | destination id, label, model | 64 + 128 + 128 | 320 |
/// | tier, kind, outcome words, counters | ≤ 90 | ≤ 90 |
/// | the `Exchange` value itself | — | 136 |
/// | **total** | **1 048 986** | **1 049 122** |
///
/// So a page of 16 reads at most **16.0 MiB** and holds at most **16.0 MiB**
/// once decoded, whatever the conversation contains. The ancestor chain it is
/// cut from is at most 256 small integers. An exchange's turns are read
/// separately, by [`Conversations::transcript_page`]'s bound.
pub const MAX_EXCHANGE_PAGE: u16 = 16;

/// The measured size of one [`Exchange`] value, on aarch64-apple-darwin
/// (2026-09-17). A test fails if the layout grows past it, which is the day
/// [`MAX_EXCHANGE_PAGE`]'s worst case must be computed again.
pub const EXCHANGE_LAYOUT_BYTES: usize = 136;

/// How an answered exchange ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnswerEnding {
    /// A whole answer.
    Answered,
    /// The model declined to go on. Not an answer, not a failure.
    Refused,
    /// The turn ceiling was reached.
    TurnLimit,
    /// An external agent reached its own request ceiling.
    AgentLimit,
    /// The provider paused the turn; continuing is the user's call.
    Paused,
    /// The answer is not whole.
    Truncated,
    /// An ending this build cannot name — a reading fallback, never written.
    ///
    /// Never read as [`Answered`](Self::Answered): an ending nobody can name is
    /// not a whole answer.
    Unknown,
}

impl AnswerEnding {
    /// The stable word written in `outcome_detail`, in English.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Refused => "refused",
            Self::TurnLimit => "turn_limit",
            Self::AgentLimit => "agent_limit",
            Self::Paused => "paused",
            Self::Truncated => "truncated",
            Self::Unknown => "unknown",
        }
    }

    /// Reads the stored word, falling back to [`Unknown`](Self::Unknown).
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "answered" => Self::Answered,
            "refused" => Self::Refused,
            "turn_limit" => Self::TurnLimit,
            "agent_limit" => Self::AgentLimit,
            "paused" => Self::Paused,
            "truncated" => Self::Truncated,
            _ => {
                tracing::warn!(
                    column = "outcome_detail",
                    "unknown answer ending in local state, falling back to `unknown`"
                );
                Self::Unknown
            }
        }
    }
}

/// Why an exchange failed, as a category — never as a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FailureKind {
    /// The tier or the declaration forbids it.
    Refused,
    /// The model endpoint failed.
    Provider,
    /// Oxyn could not prepare the conversation.
    Setup,
    /// The agent's program is not where its declaration says.
    AgentNotFound,
    /// The agent wants its user signed in.
    AgentSignIn,
    /// The agent speaks another protocol version.
    AgentIncompatible,
    /// The agent's process stopped.
    AgentExited,
    /// The agent answered with an error.
    Agent,
    /// A category this build cannot name — a reading fallback, never written.
    Unknown,
}

impl FailureKind {
    /// The stable word written in `outcome_detail`, in English.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::Provider => "provider",
            Self::Setup => "setup",
            Self::AgentNotFound => "agent_not_found",
            Self::AgentSignIn => "agent_sign_in",
            Self::AgentIncompatible => "agent_incompatible",
            Self::AgentExited => "agent_exited",
            Self::Agent => "agent",
            Self::Unknown => "unknown",
        }
    }

    /// Reads the stored word, falling back to [`Unknown`](Self::Unknown).
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "refused" => Self::Refused,
            "provider" => Self::Provider,
            "setup" => Self::Setup,
            "agent_not_found" => Self::AgentNotFound,
            "agent_sign_in" => Self::AgentSignIn,
            "agent_incompatible" => Self::AgentIncompatible,
            "agent_exited" => Self::AgentExited,
            "agent" => Self::Agent,
            _ => {
                tracing::warn!(
                    column = "outcome_detail",
                    "unknown failure category in local state, falling back to `unknown`"
                );
                Self::Unknown
            }
        }
    }
}

/// What became of an exchange. Distinct from `StopReason`, which says why one
/// provider turn stopped; an exchange may span several turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExchangeOutcome {
    /// The exchange produced an answer, ending as said.
    Answered(AnswerEnding),
    /// The exchange failed. There is no message: only a category, and whether
    /// asking again can change anything.
    Failed {
        /// Why.
        kind: FailureKind,
        /// Whether asking again can succeed. Read as `false` when unreadable:
        /// inviting a retry is an affirmation, never a fallback.
        retryable: bool,
    },
    /// The user stopped it.
    Cancelled,
    /// An outcome this build cannot name — a reading fallback, never written.
    ///
    /// Never read as answered, failed or cancelled.
    Unknown,
}

/// The counts of a sample an exchange received. Never the column names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WithheldSample {
    /// Rows sent, at most [`MAX_SAMPLE_ROWS`].
    pub rows: u32,
    /// Columns sent, at most [`MAX_SAMPLE_COLUMNS`].
    pub columns: u32,
}

/// An exchange, as it is written.
///
/// The `Debug` counts the question, it never renders it.
#[derive(Clone, PartialEq)]
pub struct ExchangeRecord {
    /// The exchange this one follows; `None` for a first question. A
    /// regeneration or an edit passes the parent of the exchange it revises.
    pub parent: Option<u32>,
    /// When the question was asked.
    pub created_at: DateTime<Utc>,
    /// The tier in force on the connection when it was asked.
    pub tier: PrivacyTier,
    /// The question, as the user typed it.
    pub question: String,
    /// Who was asked. A thread can change recipient between exchanges.
    pub destination: Destination,
}

impl ExchangeRecord {
    /// A question to `destination` under `tier`, timestamped now.
    #[must_use]
    pub fn new(
        parent: Option<u32>,
        tier: PrivacyTier,
        question: impl Into<String>,
        destination: Destination,
    ) -> Self {
        Self {
            parent,
            created_at: Utc::now(),
            tier,
            question: question.into(),
            destination,
        }
    }
}

impl std::fmt::Debug for ExchangeRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangeRecord")
            .field("parent", &self.parent)
            .field("tier", &self.tier)
            .field("question_bytes", &self.question.len())
            .field("destination_kind", &self.destination.kind)
            .finish_non_exhaustive()
    }
}

/// An exchange, as it is re-read.
///
/// The `Debug` counts the question, it never renders it.
#[derive(Clone, PartialEq)]
pub struct Exchange {
    /// Stable identifier within the conversation.
    pub node: u32,
    /// The exchange this one follows.
    pub parent: Option<u32>,
    /// When the question was asked.
    pub created_at: DateTime<Utc>,
    /// The tier in force when it was asked.
    pub tier: PrivacyTier,
    /// The question.
    pub question: String,
    /// Who was asked.
    pub destination: Destination,
    /// Set when the exchange received a sample: then it holds no answer.
    pub sample: Option<WithheldSample>,
    /// What became of it; `None` while it has not finished.
    pub outcome: Option<ExchangeOutcome>,
}

impl std::fmt::Debug for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exchange")
            .field("node", &self.node)
            .field("parent", &self.parent)
            .field("tier", &self.tier)
            .field("question_bytes", &self.question.len())
            .field("sample", &self.sample)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// One version of an exchange, as a version switcher lists it — no question,
/// no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    /// The version's node.
    pub node: u32,
    /// When it was asked.
    pub created_at: DateTime<Utc>,
    /// Whether it received a sample.
    pub withheld: bool,
    /// What became of it.
    pub outcome: Option<ExchangeOutcome>,
}

/// One bounded page of a branch, root first.
#[derive(Debug, Clone)]
pub struct BranchPage {
    /// At most [`MAX_EXCHANGE_PAGE`] exchanges.
    pub exchanges: Vec<Exchange>,
    /// The `start` to pass for the next page; `None` once the leaf is reached.
    pub next: Option<u32>,
}

impl Conversations<'_> {
    /// Adds an exchange and returns its node. `None` if the conversation is
    /// gone.
    ///
    /// The node is assigned here, one past the largest: that is what makes a
    /// cycle impossible rather than merely checked. The thread's `updated_at`
    /// and last destination move in the same transaction.
    ///
    /// # Errors
    /// [`StoreError::Corrupted`] if `parent` is not a node of this
    /// conversation; [`StoreError::TooLarge`] past
    /// [`MAX_EXCHANGES_PER_CONVERSATION`], or for a question, label or model
    /// past its bound; [`StoreError::Sqlite`] if the write fails.
    pub fn append_exchange(
        &self,
        id: ConversationId,
        exchange: &ExchangeRecord,
    ) -> Result<Option<u32>> {
        within(
            "ai_conversation_nodes.question",
            exchange.question.len(),
            MAX_TURN_TEXT_BYTES,
        )?;
        validate_destination(&exchange.destination)?;
        let destination = &exchange.destination;

        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let touched = transaction.execute(
                "UPDATE ai_conversations
                    SET updated_at = ?2, destination_kind = ?3, destination_id = ?4,
                        destination_label = ?5, model = ?6
                  WHERE id = ?1",
                params![
                    id.to_string(),
                    exchange.created_at,
                    destination.kind.as_str(),
                    destination.id.as_ref().map(ProviderId::as_str),
                    destination.label,
                    destination.model,
                ],
            )?;
            if touched == 0 {
                return Ok(None);
            }
            if let Some(parent) = exchange.parent
                && !node_exists(&transaction, id, parent)?
            {
                return Err(StoreError::Corrupted {
                    field: "ai_conversation_nodes.parent",
                    detail: "the parent is not a node of this conversation".into(),
                });
            }
            let next: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(node), -1) + 1 FROM ai_conversation_nodes
                 WHERE conversation_id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )?;
            let node = u32::try_from(next).unwrap_or(u32::MAX);
            if node >= MAX_EXCHANGES_PER_CONVERSATION {
                return Err(StoreError::TooLarge {
                    field: "ai_conversation_nodes.node",
                    limit: u64::from(MAX_EXCHANGES_PER_CONVERSATION),
                });
            }
            transaction.execute(
                "INSERT INTO ai_conversation_nodes
                     (conversation_id, node, parent, created_at, privacy_tier, question,
                      destination_kind, destination_id, destination_label, model)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id.to_string(),
                    i64::from(node),
                    exchange.parent.map(i64::from),
                    exchange.created_at,
                    exchange.tier.as_str(),
                    exchange.question,
                    destination.kind.as_str(),
                    destination.id.as_ref().map(ProviderId::as_str),
                    destination.label,
                    destination.model,
                ],
            )?;
            transaction.commit()?;
            Ok(Some(node))
        })
    }

    /// Marks an exchange as having received a sample, and **erases** what was
    /// written for it. Returns `false` if the exchange does not exist.
    ///
    /// Irreversible, by the file's rule. Called again, it updates the counts —
    /// pass the totals of every sample the exchange received.
    ///
    /// After the erasure the write-ahead log is checkpointed and truncated, so
    /// the erased text is neither in the database's free pages
    /// (`secure_delete`) nor in older log frames. If another process holds the
    /// file, the checkpoint cannot complete; the rows are gone all the same,
    /// and the next checkpoint overwrites the frames — a `warn` says so.
    ///
    /// # Errors
    /// [`StoreError::TooLarge`] past [`MAX_SAMPLE_ROWS`] or
    /// [`MAX_SAMPLE_COLUMNS`]; [`StoreError::Sqlite`] if the write fails.
    pub fn withhold_sample(
        &self,
        id: ConversationId,
        node: u32,
        sample: WithheldSample,
    ) -> Result<bool> {
        if sample.rows > MAX_SAMPLE_ROWS {
            return Err(StoreError::TooLarge {
                field: "ai_conversation_nodes.sample_rows",
                limit: u64::from(MAX_SAMPLE_ROWS),
            });
        }
        if sample.columns > MAX_SAMPLE_COLUMNS {
            return Err(StoreError::TooLarge {
                field: "ai_conversation_nodes.sample_columns",
                limit: u64::from(MAX_SAMPLE_COLUMNS),
            });
        }
        self.store.with_connection(|connection| {
            let touched = connection.execute(
                "UPDATE ai_conversation_nodes
                    SET sample_withheld = 1, sample_rows = ?3, sample_columns = ?4
                  WHERE conversation_id = ?1 AND node = ?2",
                params![
                    id.to_string(),
                    i64::from(node),
                    i64::from(sample.rows),
                    i64::from(sample.columns),
                ],
            )?;
            if touched == 0 {
                return Ok(false);
            }
            // `PRAGMA wal_checkpoint` answers one row: busy, log frames,
            // checkpointed frames. On an in-memory store it answers too.
            let busy: i64 =
                connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
            if busy != 0 {
                tracing::warn!(
                    "write-ahead log not truncated after withholding a sample: \
                     another connection holds the file"
                );
            }
            Ok(true)
        })
    }

    /// Records what became of an exchange. Returns `false` if it does not
    /// exist.
    ///
    /// # Errors
    /// [`StoreError::Corrupted`] for an `Unknown` outcome, ending or category:
    /// those are what a reader falls back to, not facts to write.
    /// [`StoreError::Sqlite`] if the write fails.
    pub fn finish_exchange(
        &self,
        id: ConversationId,
        node: u32,
        outcome: ExchangeOutcome,
    ) -> Result<bool> {
        let (word, detail, retryable) = encode_outcome(outcome)?;
        self.store.with_connection(|connection| {
            let touched = connection.execute(
                "UPDATE ai_conversation_nodes
                    SET outcome = ?3, outcome_detail = ?4, retryable = ?5
                  WHERE conversation_id = ?1 AND node = ?2",
                params![id.to_string(), i64::from(node), word, detail, retryable],
            )?;
            Ok(touched > 0)
        })
    }

    /// Selects the leaf whose branch the conversation shows; `None` clears it.
    /// Returns `false` if the conversation does not exist.
    ///
    /// # Errors
    /// [`StoreError::Corrupted`] if `node` is not a node of this conversation;
    /// [`StoreError::Sqlite`] if the write fails.
    pub fn select(&self, id: ConversationId, node: Option<u32>) -> Result<bool> {
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            if let Some(node) = node
                && !node_exists(&transaction, id, node)?
            {
                return Err(StoreError::Corrupted {
                    field: "ai_conversations.selected_node",
                    detail: "the node is not a node of this conversation".into(),
                });
            }
            let touched = transaction.execute(
                "UPDATE ai_conversations SET selected_node = ?2 WHERE id = ?1",
                params![id.to_string(), node.map(i64::from)],
            )?;
            transaction.commit()?;
            Ok(touched > 0)
        })
    }

    /// One page of the branch that ends at `leaf`, root first.
    ///
    /// `start` is the position in the branch to read from, `0` for the root;
    /// the answer's `next` is the `start` of the following page. A branch never
    /// changes once written — a node's parent is final — so a position stays
    /// valid between two calls. `limit` is clamped to `1..=`[`MAX_EXCHANGE_PAGE`].
    /// An unknown leaf yields an empty page.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], or [`StoreError::Corrupted`] if a stored
    /// identifier is unreadable. An unreadable tier, destination or outcome
    /// falls back and warns.
    pub fn branch_page(
        &self,
        id: ConversationId,
        leaf: u32,
        start: u32,
        limit: u16,
    ) -> Result<BranchPage> {
        let limit = u32::from(limit.clamp(1, MAX_EXCHANGE_PAGE));
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let chain: Vec<u32> = {
                // `depth < 256` never binds on a file Oxyn wrote — `parent <
                // node` already ends every chain — but it keeps a file edited
                // past its triggers from looping.
                let mut query = transaction.prepare(
                    "WITH RECURSIVE chain(node, parent, depth) AS (
                         SELECT node, parent, 0 FROM ai_conversation_nodes
                          WHERE conversation_id = ?1 AND node = ?2
                         UNION ALL
                         SELECT n.node, n.parent, c.depth + 1
                           FROM ai_conversation_nodes n JOIN chain c
                             ON n.conversation_id = ?1 AND n.node = c.parent
                          WHERE c.depth < 256
                     )
                     SELECT node FROM chain ORDER BY depth DESC",
                )?;
                query
                    .query_map(params![id.to_string(), i64::from(leaf)], |row| {
                        row.get::<_, i64>(0)
                    })?
                    .map(|node| Ok(u32::try_from(node?).unwrap_or(u32::MAX)))
                    .collect::<Result<_>>()?
            };
            let start_index = usize::try_from(start).unwrap_or(usize::MAX);
            let end_index = start_index.saturating_add(usize::try_from(limit).unwrap_or(1));
            let mut exchanges = Vec::new();
            for node in chain.iter().skip(start_index).take(end_index - start_index) {
                let exchange = transaction
                    .query_row(
                        &format!("{EXCHANGE_COLUMNS} WHERE conversation_id = ?1 AND node = ?2"),
                        params![id.to_string(), i64::from(*node)],
                        |row| Ok(exchange_from_row(row)),
                    )
                    .optional()?
                    .transpose()?;
                exchanges.extend(exchange);
            }
            let next =
                (end_index < chain.len()).then(|| u32::try_from(end_index).unwrap_or(u32::MAX));
            Ok(BranchPage { exchanges, next })
        })
    }

    /// The versions of the exchange `node` — itself and its siblings — oldest
    /// first. Empty if `node` does not exist.
    ///
    /// Bounded without paging: at most [`MAX_EXCHANGES_PER_CONVERSATION`]
    /// values of a few dozen bytes each, no question and no answer.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the read fails.
    pub fn versions(&self, id: ConversationId, node: u32) -> Result<Vec<Version>> {
        self.store.with_connection(|connection| {
            let mut query = connection.prepare(
                "SELECT v.node, v.created_at, v.sample_withheld, v.outcome, v.outcome_detail,
                        v.retryable
                   FROM ai_conversation_nodes v
                   JOIN ai_conversation_nodes me
                     ON me.conversation_id = v.conversation_id AND me.node = ?2
                  WHERE v.conversation_id = ?1 AND v.parent IS me.parent
                  ORDER BY v.node",
            )?;
            query
                .query_and_then(params![id.to_string(), i64::from(node)], |row| {
                    let node: i64 = row.get("node")?;
                    Ok(Version {
                        node: u32::try_from(node).unwrap_or(u32::MAX),
                        created_at: row.get("created_at")?,
                        withheld: row.get("sample_withheld")?,
                        outcome: decode_outcome(row)?,
                    })
                })?
                .collect()
        })
    }
}

/// Is `node` a node of conversation `id`, and has its sample been withheld?
/// `None` when the node does not exist.
pub(super) fn node_state(
    connection: &rusqlite::Connection,
    id: ConversationId,
    node: u32,
) -> Result<Option<bool>> {
    Ok(connection
        .query_row(
            "SELECT sample_withheld FROM ai_conversation_nodes
              WHERE conversation_id = ?1 AND node = ?2",
            params![id.to_string(), i64::from(node)],
            |row| row.get::<_, bool>(0),
        )
        .optional()?)
}

/// Is `node` a node of conversation `id`?
fn node_exists(connection: &rusqlite::Connection, id: ConversationId, node: u32) -> Result<bool> {
    Ok(node_state(connection, id, node)?.is_some())
}

/// The exchange columns, shared by every exchange read.
const EXCHANGE_COLUMNS: &str = "SELECT node, parent, created_at, privacy_tier, question, \
     destination_kind, destination_id, destination_label, model, sample_withheld, sample_rows, \
     sample_columns, outcome, outcome_detail, retryable FROM ai_conversation_nodes";

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

/// Checks a destination against the bounds the file holds.
fn validate_destination(destination: &Destination) -> Result<()> {
    if destination.kind == DestinationKind::Unknown {
        return Err(StoreError::Corrupted {
            field: "ai_conversation_nodes.destination_kind",
            detail: "an unknown destination is a reading fallback, not a value to write".into(),
        });
    }
    within(
        "ai_conversation_nodes.destination_label",
        destination.label.len(),
        MAX_DESTINATION_LABEL_BYTES,
    )?;
    if let Some(model) = &destination.model {
        within(
            "ai_conversation_nodes.model",
            model.len(),
            MAX_DESTINATION_MODEL_BYTES,
        )?;
    }
    Ok(())
}

/// Encodes an outcome into its three columns. The `Unknown` variants are
/// refused: writing ignorance would turn a fallback into a recorded fact.
fn encode_outcome(
    outcome: ExchangeOutcome,
) -> Result<(&'static str, Option<&'static str>, Option<i64>)> {
    let unknown = || StoreError::Corrupted {
        field: "ai_conversation_nodes.outcome",
        detail: "an unknown outcome is a reading fallback, not a value to write".into(),
    };
    match outcome {
        ExchangeOutcome::Answered(AnswerEnding::Unknown)
        | ExchangeOutcome::Failed {
            kind: FailureKind::Unknown,
            ..
        }
        | ExchangeOutcome::Unknown => Err(unknown()),
        ExchangeOutcome::Answered(ending) => Ok(("answered", Some(ending.as_str()), None)),
        ExchangeOutcome::Failed { kind, retryable } => {
            Ok(("failed", Some(kind.as_str()), Some(i64::from(retryable))))
        }
        ExchangeOutcome::Cancelled => Ok(("cancelled", None, None)),
    }
}

/// Reads an outcome, tolerantly — toward ignorance, never toward an affirmation.
fn decode_outcome(row: &Row<'_>) -> Result<Option<ExchangeOutcome>> {
    let outcome: Option<String> = row.get("outcome")?;
    let detail: Option<String> = row.get("outcome_detail")?;
    let retryable: Option<i64> = row.get("retryable")?;
    let Some(outcome) = outcome else {
        return Ok(None);
    };
    let detail = detail.unwrap_or_default();
    Ok(Some(match outcome.as_str() {
        "answered" => ExchangeOutcome::Answered(AnswerEnding::from_text(&detail)),
        "failed" => ExchangeOutcome::Failed {
            kind: FailureKind::from_text(&detail),
            // Only an explicit 1 invites a retry.
            retryable: retryable == Some(1),
        },
        "cancelled" => ExchangeOutcome::Cancelled,
        _ => {
            tracing::warn!(
                column = "outcome",
                "unknown exchange outcome in local state, falling back to `unknown`"
            );
            ExchangeOutcome::Unknown
        }
    }))
}

/// Rebuilds an exchange from a row.
fn exchange_from_row(row: &Row<'_>) -> Result<Exchange> {
    let node: i64 = row.get("node")?;
    let parent: Option<i64> = row.get("parent")?;
    let tier: String = row.get("privacy_tier")?;
    let withheld: bool = row.get("sample_withheld")?;
    let rows: Option<i64> = row.get("sample_rows")?;
    let columns: Option<i64> = row.get("sample_columns")?;
    let kind: String = row.get("destination_kind")?;
    let raw_id: Option<String> = row.get("destination_id")?;
    Ok(Exchange {
        node: u32::try_from(node).unwrap_or(u32::MAX),
        parent: parent.and_then(|parent| u32::try_from(parent).ok()),
        created_at: row.get("created_at")?,
        tier: privacy_tier_from_column(Some(&tier)),
        question: row.get("question")?,
        destination: Destination {
            kind: DestinationKind::from_text(&kind),
            id: raw_id.and_then(|raw| ProviderId::new(raw).ok()),
            label: row.get("destination_label")?,
            model: row.get("model")?,
        },
        // A withheld exchange whose counts are unreadable still reads as
        // withheld: the marker is the fact that matters, the counts are detail.
        sample: withheld.then(|| WithheldSample {
            rows: rows.and_then(|v| u32::try_from(v).ok()).unwrap_or(0),
            columns: columns.and_then(|v| u32::try_from(v).ok()).unwrap_or(0),
        }),
        outcome: decode_outcome(row)?,
    })
}
