//! Tables `ai_conversations` and `ai_conversation_turns`: what the assistant
//! was asked, and what it answered.
//!
//! # Why this table exists at all
//!
//! Until now the panel's history lived in the window and died with the process.
//! What the user asked yesterday was gone this morning, with no message. The
//! module that held it said what was missing: *a store table with an open
//! schema, decided with the store*.
//!
//! # What a row is, and what it can never be
//!
//! A turn holds **what the user reads**: the text, the model's reasoning blocks,
//! a rendering of each tool call, the tokens a provider declared, and why the
//! turn stopped.
//!
//! It holds **no** tool arguments, **no** query result, **no** bound value and
//! **no** key — and there is no column to put them in. That is the same method
//! as `external_agents`, which has no secret column: the certain way not to
//! write a value is to have nowhere to put it ([I-03](../../../CLAUDE.md#i-03)).
//!
//! The SQL a user wrote **does** stay: it is the question, and dropping it
//! would make the transcript unreadable. A query's *result* never does.
//!
//! # The tier is recorded on the turn
//!
//! Not on the conversation. A user can change a connection's tier mid-thread,
//! and a tier written once in the header would then lie about every earlier
//! turn. Re-reading an audit trail asks under which regime **that turn** ran
//! ([I-04](../../../CLAUDE.md#i-04)).
//!
//! # Reading is more permissive than writing
//!
//! A row this build cannot fully understand still opens: an unknown role reads
//! as [`TurnRole::Unknown`], an unreadable tier falls back to the most
//! constraining one, unreadable reasoning is dropped with a `warn` and the turn
//! survives. A transcript nobody can open protects nobody.
//!
//! Every method here can block and takes the store lock: never call them from
//! the interface thread ([I-05](../../../CLAUDE.md#i-05)).

mod mentions;
mod retention;
#[cfg(test)]
mod tests;
mod tree;

pub use mentions::{
    ExchangeMention, MAX_EXCHANGE_MENTIONS, MAX_MENTION_NAME_BYTES, MAX_MENTIONS_BYTES,
};
pub use retention::{PruneReport, RetentionPolicy};
pub use tree::{
    AnswerEnding, BranchPage, EXCHANGE_LAYOUT_BYTES, Exchange, ExchangeOutcome, ExchangeRecord,
    FailureKind, MAX_DESTINATION_LABEL_BYTES, MAX_DESTINATION_MODEL_BYTES, MAX_EXCHANGE_PAGE,
    MAX_EXCHANGES_PER_CONVERSATION, MAX_SAMPLE_COLUMNS, MAX_SAMPLE_ROWS, Version, WithheldSample,
};

use chrono::{DateTime, Utc};
use oxyn_core::ai::{ReasoningBlock, Role, StopReason};
use oxyn_core::{
    AgentSessionId, ConnectionId, ConversationId, ErrorClass, PrivacyTier, ProviderId, WorkspaceId,
};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::encoding::{parse_id, parse_id_opt, privacy_tier_from_column};
use crate::error::{Result, StoreError};
use crate::store::Store;

/// Largest title accepted, in bytes. The `CHECK` in the file holds the same
/// bound, so `sqlite3` is held to it too.
pub const MAX_TITLE_BYTES: usize = 512;

/// Largest turn text accepted, in bytes.
///
/// The same budget as an editor document: past a mebibyte a single answer is a
/// runaway loop, not a message.
pub const MAX_TURN_TEXT_BYTES: usize = 1024 * 1024;

/// Largest encoded reasoning accepted for one turn, in bytes.
///
/// A provider fills this column, not Oxyn: real signatures weigh a few
/// kibibytes, and without a bound a hostile or looping endpoint would decide
/// how much the local state allocates when it is opened.
pub const MAX_TURN_REASONING_BYTES: usize = 256 * 1024;

/// Largest encoded tool-call rendering accepted for one turn, in bytes.
pub const MAX_TURN_TOOL_CALLS_BYTES: usize = 64 * 1024;

/// Turns one conversation accepts before it must be replaced by a new one.
///
/// Two per exchange — a question and an answer — for the 256 exchanges the
/// panel already holds in memory. It is not a round number: it is the bound the
/// interface has, expressed in the unit this table stores.
///
/// A regeneration loop is the case it stops: without it one thread grows until
/// the disk complains. The file holds it too — a trigger refuses an `ordinal`
/// past it — but it does **not** bound a read: [`Conversations::transcript_page`]
/// does, whatever a thread holds.
pub const MAX_TURNS_PER_CONVERSATION: u32 = 512;

/// Largest encoded stop reason accepted, in bytes.
///
/// Every named reason fits in a few dozen bytes; only
/// [`StopReason::Other`] can be longer, and it carries a word a *provider*
/// chose. Without this bound a hostile endpoint could put a whole stream frame
/// there — megabytes per turn, read back at every opening.
pub const MAX_STOP_REASON_BYTES: usize = 256;

/// Most turns one [`Conversations::transcript_page`] call returns.
///
/// # What one page costs at worst
///
/// Computed from the bounds in the file and the measured layouts
/// (`ReasoningBlock` 48 bytes, `ToolCallRecord` 104, `Turn` 176, on
/// aarch64-apple-darwin, 2026-09-16):
///
/// | Per turn | Read from disk | Decoded |
/// |---|---|---|
/// | text | 1 048 576 | 1 048 576 |
/// | reasoning — 8 738 empty blocks of 30 JSON bytes, `Vec` capacity 16 384 | 262 144 | 786 432 + 262 144 of strings |
/// | tool calls — 1 110 empty records of 59 JSON bytes, capacity 2 048 | 65 536 | 212 992 + 65 536 of strings |
/// | stop reason, struct | 256 | 256 + 176 |
/// | **total** | **1 376 512** | **2 376 112** |
///
/// So a page of 16 reads at most **21 MiB** and holds at most **36.3 MiB**
/// once decoded, plus the one row's raw JSON (320 KiB) held while it decodes.
/// That is an eighth of one result's 256 MiB budget, whatever the thread holds
/// — the bound no longer depends on how many turns a file contains. An ordinary
/// twelve-exchange thread is 64 KiB whole.
pub const MAX_TURN_PAGE: u16 = 16;

/// Where a conversation's answers come from.
///
/// Two kinds and not a flag, because what is knowable differs: a declared
/// provider has a model, an external agent does not — Oxyn cannot know it
/// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DestinationKind {
    /// A provider declared by the user.
    Provider,
    /// An external agent run as a subprocess.
    ExternalAgent,
    /// A kind this build cannot name.
    ///
    /// Never read as a provider: a thread whose destination is unknown must not
    /// be shown as one that had a measured reach.
    Unknown,
}

impl DestinationKind {
    /// The stable word written in `destination_kind`, in English.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::ExternalAgent => "agent",
            Self::Unknown => "unknown",
        }
    }

    /// Reads `destination_kind`, falling back to [`Unknown`](Self::Unknown).
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "provider" => Self::Provider,
            "agent" => Self::ExternalAgent,
            _ => {
                tracing::warn!(
                    column = "destination_kind",
                    "unknown conversation destination in local state, falling back to `unknown`"
                );
                Self::Unknown
            }
        }
    }
}

/// Who answered a conversation, as it was declared at the time.
///
/// The label is copied rather than joined: removing a provider does not erase
/// what the user asked it, and a thread whose header went blank would read as
/// corrupt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// Provider or external agent.
    pub kind: DestinationKind,
    /// The declaration this thread used, when it still can be named. No foreign
    /// key: the declaration may be gone.
    pub id: Option<ProviderId>,
    /// The name the user gave the declaration. This is what is shown.
    pub label: String,
    /// The model, when there is one to know. Always `None` for an external
    /// agent.
    pub model: Option<String>,
}

impl Destination {
    /// A declared provider answering with `model`.
    #[must_use]
    pub fn provider(id: ProviderId, label: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            kind: DestinationKind::Provider,
            id: Some(id),
            label: label.into(),
            model: Some(model.into()),
        }
    }

    /// An external agent. It carries no model, and inventing one would put a
    /// name on something Oxyn never observed.
    #[must_use]
    pub fn external_agent(id: ProviderId, label: impl Into<String>) -> Self {
        Self {
            kind: DestinationKind::ExternalAgent,
            id: Some(id),
            label: label.into(),
            model: None,
        }
    }
}

/// Who speaks in a turn.
///
/// Open where [`oxyn_core::ai::Role`] is closed, and the difference is the point:
/// the wire enum is closed so that a new role breaks every provider at compile
/// time, while a *stored* role comes from a file this build did not necessarily
/// write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TurnRole {
    /// Framing set by Oxyn, never by the user.
    System,
    /// The user's turn.
    User,
    /// The model's turn.
    Assistant,
    /// A tool's report, handed back to the model.
    Tool,
    /// A role this build cannot name.
    ///
    /// Shown as neither the user nor the assistant: attributing a model's words
    /// to the user, or the reverse, is the one reading mistake a transcript must
    /// not make.
    Unknown,
}

impl TurnRole {
    /// The stable word written in `role`, in English.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Unknown => "unknown",
        }
    }

    /// Reads `role`, falling back to [`Unknown`](Self::Unknown).
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "system" => Self::System,
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "unknown" => Self::Unknown,
            _ => {
                tracing::warn!(
                    column = "role",
                    "unknown turn role in local state, falling back to `unknown`"
                );
                Self::Unknown
            }
        }
    }
}

impl From<Role> for TurnRole {
    fn from(role: Role) -> Self {
        match role {
            Role::System => Self::System,
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
            Role::Tool => Self::Tool,
        }
    }
}

impl std::fmt::Display for TurnRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What became of a tool call, as the panel showed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToolCallStatus {
    /// It ran and reported.
    Completed,
    /// The policy held it back and the user had not decided yet.
    AwaitingApproval,
    /// The policy refused it. Nothing reached the server.
    Denied,
    /// It ran and failed.
    Failed,
    /// The conversation was stopped while it ran.
    Cancelled,
    /// An outcome this build cannot name.
    ///
    /// Never read as [`Completed`](Self::Completed): a call whose outcome is
    /// unreadable must not be shown as one that succeeded.
    Unknown,
}

impl ToolCallStatus {
    /// The stable word written in the `tool_calls` payload, in English.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    /// Reads the stored word, falling back to [`Unknown`](Self::Unknown).
    #[must_use]
    pub fn from_text(raw: &str) -> Self {
        match raw {
            "completed" => Self::Completed,
            "awaiting_approval" => Self::AwaitingApproval,
            "denied" => Self::Denied,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "unknown" => Self::Unknown,
            _ => {
                tracing::warn!(
                    column = "tool_calls.status",
                    "unknown tool call status in local state, falling back to `unknown`"
                );
                Self::Unknown
            }
        }
    }
}

/// A tool call, **as it was rendered** — never as it was parameterised.
///
/// There is deliberately no field for the call's arguments nor for what it
/// returned. A model's arguments carry bound values, and a report carries rows;
/// both are governed by the connection's tier at the moment they existed, and
/// neither can be re-checked when the thread is re-read a month later. What is
/// kept is what the user saw ([I-03](../../../CLAUDE.md#i-03)).
///
/// The `Debug` is hand-written and counts instead of rendering, like every
/// other type of this module: a statement written by an agent copies the values
/// it read (`UPDATE … SET email = 'alice@…'`), sometimes a credential
/// (`ALTER ROLE app PASSWORD '…'`), and the summary quotes the statement.
#[derive(Clone, PartialEq, Eq)]
pub struct ToolCallRecord {
    /// The provider's call identifier, which ties a call to its report. Opaque:
    /// it is never rendered.
    pub call_id: String,
    /// The tool the model asked for.
    pub tool: String,
    /// The one-line rendering the panel showed. Never the arguments.
    pub summary: String,
    /// The statement the command carried, when it carried one. Bound values are
    /// not part of a statement — `ExecRequest` keeps them apart, and so does
    /// this.
    pub statement: Option<String>,
    /// How it ended.
    pub status: ToolCallStatus,
    /// The driver's error family, recorded rather than deduced from a message
    /// ([I-13](../../../CLAUDE.md#i-13)).
    pub error_class: Option<ErrorClass>,
}

impl std::fmt::Debug for ToolCallRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCallRecord")
            .field("tool", &self.tool)
            .field("status", &self.status)
            .field("error_class", &self.error_class)
            .field("summary_bytes", &self.summary.len())
            .field("statement_bytes", &self.statement.as_ref().map(String::len))
            .finish_non_exhaustive()
    }
}

impl ToolCallRecord {
    /// Records a call by its rendering.
    #[must_use]
    pub fn new(
        call_id: impl Into<String>,
        tool: impl Into<String>,
        summary: impl Into<String>,
        status: ToolCallStatus,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            tool: tool.into(),
            summary: summary.into(),
            statement: None,
            status,
            error_class: None,
        }
    }

    /// Adds the statement the command carried.
    #[must_use]
    pub fn with_statement(mut self, statement: impl Into<String>) -> Self {
        self.statement = Some(statement.into());
        self
    }

    /// Adds the driver's error family.
    #[must_use]
    pub fn with_error_class(mut self, class: ErrorClass) -> Self {
        self.error_class = Some(class);
        self
    }
}

/// The `tool_calls` column, as plain words rather than a serde enum encoding.
///
/// A separate mirror and not `#[derive(Serialize)]` on [`ToolCallRecord`], for
/// two reasons that both come back to the file being read without Oxyn (I-11):
/// the payload stays `{"status":"denied"}` instead of an enum encoding, and an
/// unknown word can fall back on reading instead of failing the whole row.
#[derive(Serialize, Deserialize)]
struct ToolCallPayload {
    call_id: String,
    tool: String,
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    statement: Option<String>,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_class: Option<String>,
}

/// What a provider declared it spent on one turn, counted in tokens.
///
/// Every field is `Option` and none defaults to `0`: « not declared » and
/// « zero » are different facts, and showing « 0 read from cache » where the
/// provider said nothing would look like a cache that does not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnUsage {
    /// Input tokens billed, cache excluded.
    pub prompt: Option<u32>,
    /// Tokens produced.
    pub completion: Option<u32>,
    /// Tokens **written** to the prefix cache.
    pub cache_write: Option<u32>,
    /// Tokens **read** from the prefix cache. They are not in `prompt`: the
    /// input total is the sum of the three.
    pub cache_read: Option<u32>,
    /// Tokens spent reasoning, when the provider isolates them.
    pub reasoning: Option<u32>,
}

impl TurnUsage {
    /// Has the provider declared anything at all?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.prompt.is_none()
            && self.completion.is_none()
            && self.cache_write.is_none()
            && self.cache_read.is_none()
            && self.reasoning.is_none()
    }
}

/// One turn, as it is written.
///
/// The `Debug` is hand-written: it counts the text, it never renders it. A turn
/// quotes what the user typed, and a user pastes what they like into a question
/// ([I-03](../../../CLAUDE.md#i-03)).
#[derive(Clone, PartialEq)]
pub struct TurnRecord {
    /// When the turn happened.
    pub ts: DateTime<Utc>,
    /// Who spoke.
    pub role: TurnRole,
    /// The tier in force on the connection **at that moment**.
    pub tier: PrivacyTier,
    /// The agent session this turn belonged to, when it had one.
    ///
    /// This is the join to `audit_journal`: without it, a thread is an island
    /// and nothing ties a tool call in the transcript to the command that was
    /// authorised or refused.
    pub agent_session: Option<AgentSessionId>,
    /// The exchange this turn belongs to, when the thread is a tree.
    ///
    /// `None` on every turn written before the migration 13, and they stay
    /// readable. A turn cannot be attached to an exchange that received a
    /// sample: [`Conversations::append`] refuses it, and so does the file.
    pub node: Option<u32>,
    /// The text. Empty is legitimate for an assistant turn that only calls
    /// tools.
    pub text: String,
    /// The model's reasoning blocks, kept **verbatim**.
    pub reasoning: Vec<ReasoningBlock>,
    /// The tool calls of this turn, by their rendering.
    pub tool_calls: Vec<ToolCallRecord>,
    /// What the provider declared it spent.
    pub usage: TurnUsage,
    /// Why the turn stopped, when it is known.
    pub stop: Option<StopReason>,
}

impl TurnRecord {
    /// A turn spoken by `role` under `tier`, timestamped now.
    #[must_use]
    pub fn new(role: TurnRole, tier: PrivacyTier, text: impl Into<String>) -> Self {
        Self {
            ts: Utc::now(),
            role,
            tier,
            agent_session: None,
            node: None,
            text: text.into(),
            reasoning: Vec::new(),
            tool_calls: Vec::new(),
            usage: TurnUsage::default(),
            stop: None,
        }
    }

    /// Attaches this turn to an exchange of the tree.
    #[must_use]
    pub fn in_exchange(mut self, node: u32) -> Self {
        self.node = Some(node);
        self
    }

    /// Ties this turn to the agent session that ran it.
    #[must_use]
    pub fn in_agent_session(mut self, session: AgentSessionId) -> Self {
        self.agent_session = Some(session);
        self
    }

    /// Attaches the reasoning blocks, in the order the provider sent them.
    ///
    /// They are stored as received. Rebuilding, reordering or dropping one is
    /// what makes a provider refuse the next turn ([`oxyn_core::ai::ReasoningBlock`]).
    #[must_use]
    pub fn with_reasoning(mut self, blocks: Vec<ReasoningBlock>) -> Self {
        self.reasoning = blocks;
        self
    }

    /// Attaches the tool-call renderings.
    #[must_use]
    pub fn with_tool_calls(mut self, calls: Vec<ToolCallRecord>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Attaches what the provider declared it spent.
    #[must_use]
    pub fn with_usage(mut self, usage: TurnUsage) -> Self {
        self.usage = usage;
        self
    }

    /// Records why the turn stopped.
    #[must_use]
    pub fn stopped(mut self, stop: StopReason) -> Self {
        self.stop = Some(stop);
        self
    }
}

impl std::fmt::Debug for TurnRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnRecord")
            .field("role", &self.role)
            .field("tier", &self.tier)
            .field("text_bytes", &self.text.len())
            .field("reasoning", &self.reasoning.len())
            .field("tool_calls", &self.tool_calls.len())
            .field("stop", &self.stop)
            .finish_non_exhaustive()
    }
}

/// A turn as it is re-read, with its place in the thread.
#[derive(Debug, Clone)]
pub struct Turn {
    /// Position in the thread, from `0`, never reused.
    pub ordinal: u32,
    /// What the turn holds.
    pub record: TurnRecord,
}

/// One bounded page of a transcript.
#[derive(Debug, Clone)]
pub struct TurnPage {
    /// The turns, oldest first. At most [`MAX_TURN_PAGE`].
    pub turns: Vec<Turn>,
    /// The `after` to pass for the next page; `None` once the thread is read.
    pub next: Option<u32>,
}

/// A conversation with the assistant.
///
/// The `Debug` counts the title rather than showing it: a title is often the
/// user's first question, shortened.
#[derive(Clone, PartialEq)]
pub struct Conversation {
    /// Identity, stable across restarts.
    pub id: ConversationId,
    /// The owning workspace. Deleting it deletes the thread.
    pub workspace: WorkspaceId,
    /// The connection the thread is about. No foreign key: deleting a
    /// connection does not erase what was asked about it.
    pub connection: Option<ConnectionId>,
    /// The connection's **name**, copied so the thread stays readable after the
    /// connection is gone.
    pub connection_name: Option<String>,
    /// Who answered.
    pub destination: Destination,
    /// The title shown in the history panel.
    pub title: String,
    /// When the thread was opened.
    pub created_at: DateTime<Utc>,
    /// When it last had a turn. This is what the retention policy orders on.
    pub updated_at: DateTime<Utc>,
    /// The leaf whose branch is shown, when one is selected.
    ///
    /// Written only by [`Conversations::select`]; [`Conversations::save`]
    /// leaves it as it is.
    pub selected: Option<u32>,
}

impl Conversation {
    /// Opens a thread, not yet persisted.
    #[must_use]
    pub fn new(workspace: WorkspaceId, destination: Destination, title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: ConversationId::new(),
            workspace,
            connection: None,
            connection_name: None,
            destination,
            title: title.into(),
            created_at: now,
            updated_at: now,
            selected: None,
        }
    }

    /// Names the connection the thread is about.
    #[must_use]
    pub fn on_connection(mut self, id: ConnectionId, name: impl Into<String>) -> Self {
        self.connection = Some(id);
        self.connection_name = Some(name.into());
        self
    }
}

impl std::fmt::Debug for Conversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conversation")
            .field("id", &self.id)
            .field("workspace", &self.workspace)
            .field("connection", &self.connection)
            .field("destination_kind", &self.destination.kind)
            .field("title_bytes", &self.title.len())
            .field("updated_at", &self.updated_at)
            .finish_non_exhaustive()
    }
}

/// A conversation in the history list, without its transcript.
///
/// The `Debug` counts the title like [`Conversation`]'s does, and for the same
/// reason: a title is usually the user's first question, shortened. Showing it
/// here and hiding it there would make the hidden one look like a precaution
/// nobody took seriously.
#[derive(Clone)]
pub struct ConversationSummary {
    /// Identity, to open it.
    pub id: ConversationId,
    /// The title shown.
    pub title: String,
    /// Who answered.
    pub destination: Destination,
    /// When it was opened.
    pub created_at: DateTime<Utc>,
    /// When it last had a turn.
    pub updated_at: DateTime<Utc>,
    /// How many turns it holds.
    pub turns: u32,
}

impl std::fmt::Debug for ConversationSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationSummary")
            .field("id", &self.id)
            .field("destination_kind", &self.destination.kind)
            .field("title_bytes", &self.title.len())
            .field("updated_at", &self.updated_at)
            .field("turns", &self.turns)
            .finish_non_exhaustive()
    }
}

/// Typed access to `ai_conversations` and `ai_conversation_turns`.
#[derive(Debug)]
pub struct Conversations<'a> {
    store: &'a Store,
}

impl<'a> Conversations<'a> {
    /// Binds the accessor to its store.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Writes a thread's header, or updates it in place.
    ///
    /// `created_at` is never overwritten; `updated_at` is the caller's, so
    /// re-saving a header does not move a thread to the top of the list.
    ///
    /// # Errors
    /// [`StoreError::TooLarge`] if the title exceeds [`MAX_TITLE_BYTES`];
    /// [`StoreError::Sqlite`] if the workspace does not exist — the foreign key
    /// refuses it — or if the write fails.
    pub fn save(&self, conversation: &Conversation) -> Result<()> {
        check_size(
            "ai_conversations.title",
            conversation.title.len(),
            MAX_TITLE_BYTES,
        )?;
        self.store.with_connection(|connection| {
            connection.execute(
                "INSERT INTO ai_conversations
                     (id, workspace_id, connection_id, connection_name, destination_kind,
                      destination_id, destination_label, model, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                     connection_id     = excluded.connection_id,
                     connection_name   = excluded.connection_name,
                     destination_kind  = excluded.destination_kind,
                     destination_id    = excluded.destination_id,
                     destination_label = excluded.destination_label,
                     model             = excluded.model,
                     title             = excluded.title,
                     updated_at        = excluded.updated_at",
                params![
                    conversation.id.to_string(),
                    conversation.workspace.to_string(),
                    conversation.connection.map(|id| id.to_string()),
                    conversation.connection_name,
                    conversation.destination.kind.as_str(),
                    conversation.destination.id.as_ref().map(ProviderId::as_str),
                    conversation.destination.label,
                    conversation.destination.model,
                    conversation.title,
                    conversation.created_at,
                    conversation.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Renames a thread. Returns `false` if it no longer exists.
    ///
    /// Renaming does not touch `updated_at`: a rename is not activity, and
    /// moving a thread to the top of the list for it would reorder the history
    /// under the user's cursor.
    ///
    /// # Errors
    /// [`StoreError::TooLarge`] past [`MAX_TITLE_BYTES`]; [`StoreError::Sqlite`]
    /// if the write fails.
    pub fn rename(&self, id: ConversationId, title: &str) -> Result<bool> {
        check_size("ai_conversations.title", title.len(), MAX_TITLE_BYTES)?;
        self.store.with_connection(|connection| {
            let touched = connection.execute(
                "UPDATE ai_conversations SET title = ?2 WHERE id = ?1",
                params![id.to_string(), title],
            )?;
            Ok(touched > 0)
        })
    }

    /// Appends a turn and returns its ordinal. `None` if the thread is gone.
    ///
    /// `None` is not an error: a thread can be pruned or deleted while a turn is
    /// in flight, and the caller has to tell that apart from a broken disk.
    ///
    /// The turn and the thread's `updated_at` move in **one transaction**: a
    /// thread whose header says it has been idle for a month while its last turn
    /// is from today is a transcript that lies about itself.
    ///
    /// # Errors
    /// [`StoreError::TooLarge`] if the text, the encoded reasoning or the
    /// encoded tool calls exceed their bound, or if the thread has reached
    /// [`MAX_TURNS_PER_CONVERSATION`]; [`StoreError::Json`] if a reasoning block
    /// cannot be encoded; [`StoreError::Sqlite`] if the write fails.
    pub fn append(&self, id: ConversationId, turn: &TurnRecord) -> Result<Option<u32>> {
        check_size(
            "ai_conversation_turns.text",
            turn.text.len(),
            MAX_TURN_TEXT_BYTES,
        )?;
        let reasoning = encode_reasoning(&turn.reasoning)?;
        let tool_calls = encode_tool_calls(&turn.tool_calls)?;
        let stop = turn.stop.as_ref().map(encode_stop_reason).transpose()?;

        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let touched = transaction.execute(
                "UPDATE ai_conversations SET updated_at = ?2 WHERE id = ?1",
                params![id.to_string(), turn.ts],
            )?;
            if touched == 0 {
                return Ok(None);
            }
            // Checked here for a named refusal; the file refuses the same
            // insert in a trigger, for whoever does not come through this API.
            if let Some(node) = turn.node {
                match tree::node_state(&transaction, id, node)? {
                    None => {
                        return Err(StoreError::Corrupted {
                            field: "ai_conversation_turns.node",
                            detail: "the node is not a node of this conversation".into(),
                        });
                    }
                    Some(true) => return Err(StoreError::SampleWithheld { node }),
                    Some(false) => {}
                }
            }

            let next: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(ordinal), -1) + 1 FROM ai_conversation_turns
                 WHERE conversation_id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )?;
            let ordinal = u32::try_from(next).unwrap_or(u32::MAX);
            if ordinal >= MAX_TURNS_PER_CONVERSATION {
                return Err(StoreError::TooLarge {
                    field: "ai_conversation_turns.ordinal",
                    limit: u64::from(MAX_TURNS_PER_CONVERSATION),
                });
            }

            transaction.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, agent_session_id, text,
                      reasoning, tool_calls, stop_reason, prompt_tokens, completion_tokens,
                      cache_write_tokens, cache_read_tokens, reasoning_tokens, node)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    id.to_string(),
                    next,
                    turn.ts,
                    turn.role.as_str(),
                    turn.tier.as_str(),
                    turn.agent_session.map(|session| session.to_string()),
                    turn.text,
                    reasoning,
                    tool_calls,
                    stop,
                    turn.usage.prompt.map(i64::from),
                    turn.usage.completion.map(i64::from),
                    turn.usage.cache_write.map(i64::from),
                    turn.usage.cache_read.map(i64::from),
                    turn.usage.reasoning.map(i64::from),
                    turn.node.map(i64::from),
                ],
            )?;
            transaction.commit()?;
            Ok(Some(ordinal))
        })
    }

    /// Reads a thread's header.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], or [`StoreError::Corrupted`] if an identifier in
    /// the row is unreadable.
    pub fn get(&self, id: ConversationId) -> Result<Option<Conversation>> {
        self.store.with_connection(|connection| {
            connection
                .query_row(
                    &format!("{HEADER_COLUMNS} WHERE id = ?1"),
                    params![id.to_string()],
                    |row| Ok(conversation_from_row(row)),
                )
                .optional()?
                .transpose()
        })
    }

    /// Reads one page of a transcript, oldest turn first.
    ///
    /// `after` is the last ordinal already read, `None` for the first page;
    /// the answer's `next` is what to pass for the following one, `None` when
    /// the thread is exhausted. `limit` is clamped to `1..=`[`MAX_TURN_PAGE`].
    ///
    /// **There is no whole-transcript read, on purpose.** A thread read in one
    /// piece and handed to the interface is the out-of-memory
    /// [I-06](../../../CLAUDE.md#i-06) describes. One call costs at most what
    /// [`MAX_TURN_PAGE`] computes — 21 MiB read, 36.3 MiB decoded — whatever
    /// the file holds, including rows a third party wrote past the turn bound.
    /// A caller that wants the whole thread loops, and decides what it keeps.
    ///
    /// # Before handing a transcript back to a provider
    ///
    /// Compare each turn's [`tier`](TurnRecord::tier) with the one in force on
    /// the connection **now**. A thread whose turns ran under
    /// [`Sampled`](PrivacyTier::Sampled) must not be replayed into a prompt
    /// under [`Metadata`](PrivacyTier::Metadata): what left under one regime
    /// would leave again under another, which is the failure
    /// [I-04](../../../CLAUDE.md#i-04) exists to prevent. That comparison is
    /// the whole reason the tier is on the turn — this crate records it, it
    /// cannot enforce it.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], or [`StoreError::Corrupted`] if an identifier in
    /// a row is unreadable. An unreadable *payload* — reasoning, tool calls,
    /// role, tier — does **not** fail the read: it falls back and warns.
    pub fn transcript_page(
        &self,
        id: ConversationId,
        after: Option<u32>,
        limit: u16,
    ) -> Result<TurnPage> {
        let limit = limit.clamp(1, MAX_TURN_PAGE);
        let after = after.map_or(-1, i64::from);
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let turns = {
                let mut query = transaction.prepare(&format!(
                    "{TURN_COLUMNS} WHERE conversation_id = ?1 AND ordinal > ?2 \
                     ORDER BY ordinal LIMIT ?3"
                ))?;
                query
                    .query_and_then(
                        params![id.to_string(), after, i64::from(limit)],
                        turn_from_row,
                    )?
                    .collect::<Result<Vec<_>>>()?
            };
            // An existence probe, not `LIMIT limit + 1`: fetching one row more
            // would decode one more turn, and a turn is what the page budget
            // counts.
            let next = match turns.last() {
                Some(last) if turns.len() == usize::from(limit) => {
                    let more: bool = transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM ai_conversation_turns
                                        WHERE conversation_id = ?1 AND ordinal > ?2)",
                        params![id.to_string(), i64::from(last.ordinal)],
                        |row| row.get(0),
                    )?;
                    more.then_some(last.ordinal)
                }
                _ => None,
            };
            Ok(TurnPage { turns, next })
        })
    }

    /// The threads of one connection, most recently active first.
    ///
    /// Ordered by `updated_at` and broken by `id`: a history that reorders
    /// itself between two openings reads as a defect.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`], or [`StoreError::Corrupted`] if a stored
    /// identifier is unreadable.
    pub fn list(&self, connection: ConnectionId, limit: usize) -> Result<Vec<ConversationSummary>> {
        self.store.with_connection(|handle| {
            let mut query = handle.prepare(
                "SELECT c.id, c.destination_kind, c.destination_id, c.destination_label, c.model,
                        c.title, c.created_at, c.updated_at,
                        (SELECT COUNT(*) FROM ai_conversation_turns t
                          WHERE t.conversation_id = c.id) AS turns
                   FROM ai_conversations c
                  WHERE c.connection_id = ?1
                  ORDER BY c.updated_at DESC, c.id DESC
                  LIMIT ?2",
            )?;
            query
                .query_and_then(
                    params![connection.to_string(), crate::encoding::limit_to_i64(limit)],
                    summary_from_row,
                )?
                .collect()
        })
    }

    /// Deletes one thread and its turns. Returns `false` if it was already gone.
    ///
    /// The turns go with it through `ON DELETE CASCADE`, in the same
    /// transaction: there is no state where half a transcript survives.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the delete fails.
    pub fn delete(&self, id: ConversationId) -> Result<bool> {
        self.store.with_connection(|connection| {
            let removed = connection.execute(
                "DELETE FROM ai_conversations WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(removed > 0)
        })
    }
}

/// The header columns, shared by every header read.
const HEADER_COLUMNS: &str = "SELECT id, workspace_id, connection_id, connection_name, \
     destination_kind, destination_id, destination_label, model, title, created_at, updated_at, \
     selected_node FROM ai_conversations";

/// The turn columns, shared by every transcript read.
///
/// `stop_reason` is only fetched when it fits [`MAX_STOP_REASON_BYTES`]: a row
/// written before the file enforced that bound still opens, and its oversized
/// value never reaches memory. `stop_reason_oversized` says it was there.
const TURN_COLUMNS: &str = "SELECT ordinal, ts, role, privacy_tier, agent_session_id, text, \
     reasoning, tool_calls, \
     CASE WHEN length(CAST(stop_reason AS BLOB)) <= 256 THEN stop_reason END AS stop_reason, \
     COALESCE(length(CAST(stop_reason AS BLOB)) > 256, 0) AS stop_reason_oversized, \
     prompt_tokens, completion_tokens, cache_write_tokens, cache_read_tokens, reasoning_tokens, \
     node FROM ai_conversation_turns";

/// Refuses a value that would not fit, naming the column and the bound.
///
/// Refusing rather than truncating: a turn cut in the middle is a transcript
/// that lies, and the caller is the only one who can tell the user that a
/// thread has to be replaced by a new one.
fn check_size(field: &'static str, len: usize, limit: usize) -> Result<()> {
    if len > limit {
        return Err(StoreError::TooLarge {
            field,
            limit: u64::try_from(limit).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

/// Encodes the reasoning blocks, `None` when there are none.
///
/// `serde` and not a hand-written form: a block is handed back to the provider
/// verbatim at the next turn, and a re-encoding that dropped a field of a
/// variant added later would make that turn be refused, months from here.
fn encode_reasoning(blocks: &[ReasoningBlock]) -> Result<Option<String>> {
    if blocks.is_empty() {
        return Ok(None);
    }
    let encoded = serde_json::to_string(blocks)?;
    check_size(
        "ai_conversation_turns.reasoning",
        encoded.len(),
        MAX_TURN_REASONING_BYTES,
    )?;
    Ok(Some(encoded))
}

/// Encodes the tool-call renderings, `None` when there are none.
fn encode_tool_calls(calls: &[ToolCallRecord]) -> Result<Option<String>> {
    if calls.is_empty() {
        return Ok(None);
    }
    let payload: Vec<ToolCallPayload> = calls
        .iter()
        .map(|call| ToolCallPayload {
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            summary: call.summary.clone(),
            statement: call.statement.clone(),
            status: call.status.as_str().to_owned(),
            error_class: call.error_class.map(|class| class.as_str().to_owned()),
        })
        .collect();
    let encoded = serde_json::to_string(&payload)?;
    check_size(
        "ai_conversation_turns.tool_calls",
        encoded.len(),
        MAX_TURN_TOOL_CALLS_BYTES,
    )?;
    Ok(Some(encoded))
}

/// Writes a stop reason as JSON, so `Other` keeps the provider's own word.
///
/// An `Other` word too long for [`MAX_STOP_REASON_BYTES`] is **shortened**,
/// with a trailing `…`, rather than refused. This is the one column of the
/// module that clips instead of refusing, and the difference is deliberate:
/// refusing would drop the whole turn — the user's question or the model's
/// answer — from the history because a provider chose a long word for why it
/// stopped. The word is a label, not a transcript; cutting it lies about
/// nothing the user wrote or read.
fn encode_stop_reason(stop: &StopReason) -> Result<String> {
    let encoded = serde_json::to_string(stop)?;
    if encoded.len() <= MAX_STOP_REASON_BYTES {
        return Ok(encoded);
    }
    let StopReason::Other(word) = stop else {
        // No named reason comes near the bound; if one ever does, the bound is
        // wrong and must be raised, not the reason silently rewritten.
        return Err(StoreError::TooLarge {
            field: "ai_conversation_turns.stop_reason",
            limit: u64::try_from(MAX_STOP_REASON_BYTES).unwrap_or(u64::MAX),
        });
    };
    // Cut at a character boundary first — the raw word is never longer than
    // its encoding — then trim until escaping fits too: at most a few hundred
    // tiny encodings, never one of the provider's megabytes.
    let mut cut = MAX_STOP_REASON_BYTES.min(word.len());
    while !word.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut short = word.get(..cut).unwrap_or_default().to_owned();
    loop {
        let candidate = serde_json::to_string(&StopReason::Other(format!("{short}…")))?;
        if candidate.len() <= MAX_STOP_REASON_BYTES {
            // The length only: the word is text a provider chose.
            tracing::warn!(
                column = "stop_reason",
                bytes = word.len(),
                "provider stop reason longer than its bound, stored shortened"
            );
            return Ok(candidate);
        }
        if short.pop().is_none() {
            return Err(StoreError::TooLarge {
                field: "ai_conversation_turns.stop_reason",
                limit: u64::try_from(MAX_STOP_REASON_BYTES).unwrap_or(u64::MAX),
            });
        }
    }
}

/// The words earlier builds wrote into `StopReason::Other` for an abnormal
/// ending, and the variant each one actually meant.
///
/// Before `Interrupted` and `ProviderError` existed, a stream cut mid-turn or
/// failed by the provider was stored as `{"Other":"…"}` and would re-read as an
/// ordinary ending — a truncated answer claiming to be complete. Reading is
/// more permissive than writing, so those forms are recognised here; nothing
/// writes them any more.
///
/// The two targets are not interchangeable. `Interrupted` is **ambiguous**: the
/// transport broke and nobody knows what the provider did. `ProviderError` is
/// truncated but **not** ambiguous: the provider announced its own failure, so
/// offering to retry is sound. Mapping one onto the other would either block a
/// legitimate retry or offer a doubtful one.
///
/// Source: `scratchpad/rapport-anthropic-llm-legacy.md` (2026-09-16), which
/// traces each word to the decoder line that produced it — three in the
/// committed `openai_compatible/decode.rs`, three in an uncommitted English
/// translation of it. The French words are **data found on disk**, not
/// messages: like the French codes `error_class_from_text` still reads, they
/// must match byte for byte.
///
/// Only an exact match counts. Any other `Other` word stays the provider's own
/// — `quota_exhausted` is not an interruption. Removing an entry would silently
/// turn old failures back into complete answers; each has its own test.
const LEGACY_ABNORMAL_ENDINGS: &[(&str, StopReason)] = &[
    ("flux interrompu", StopReason::Interrupted),
    ("flux illisible", StopReason::Interrupted),
    ("interrupted stream", StopReason::Interrupted),
    ("unreadable stream", StopReason::Interrupted),
    ("erreur du fournisseur", StopReason::ProviderError),
    ("provider error", StopReason::ProviderError),
];

/// The variant an old abnormal-ending word meant, if it is one of them.
fn legacy_abnormal_ending(word: &str) -> Option<StopReason> {
    LEGACY_ABNORMAL_ENDINGS
        .iter()
        .find(|(legacy, _)| *legacy == word)
        .map(|(_, stop)| stop.clone())
}

/// Reads a stop reason.
///
/// What cannot be parsed is kept as [`StopReason::Other`] rather than dropped:
/// the provider's word is the whole value of that column, and losing it would
/// turn an unfinished answer into an answer with no recorded ending.
fn decode_stop_reason(raw: Option<String>) -> Option<StopReason> {
    let raw = raw?;
    match serde_json::from_str(&raw) {
        Ok(StopReason::Other(word)) => match legacy_abnormal_ending(&word) {
            Some(stop) => {
                tracing::warn!(
                    column = "stop_reason",
                    "legacy abnormal-ending marker in local state, read as its named variant"
                );
                Some(stop)
            }
            None => Some(StopReason::Other(word)),
        },
        Ok(stop) => Some(stop),
        Err(_) => {
            tracing::warn!(
                column = "stop_reason",
                "unreadable stop reason in local state, kept verbatim"
            );
            Some(StopReason::Other(raw))
        }
    }
}

/// Reads the reasoning blocks. Unreadable ones are dropped with a `warn`.
///
/// Dropping rather than failing is the module's reading rule, and the cost is
/// named: a turn whose reasoning could not be read stays readable but can no
/// longer be handed back to a provider.
fn decode_reasoning(raw: Option<String>) -> Vec<ReasoningBlock> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| {
        tracing::warn!(
            column = "reasoning",
            "unreadable reasoning in local state, turn kept without it"
        );
        Vec::new()
    })
}

/// Reads the tool-call renderings. An unreadable payload yields none.
fn decode_tool_calls(raw: Option<String>) -> Vec<ToolCallRecord> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(payload) = serde_json::from_str::<Vec<ToolCallPayload>>(&raw) else {
        tracing::warn!(
            column = "tool_calls",
            "unreadable tool calls in local state, turn kept without them"
        );
        return Vec::new();
    };
    payload
        .into_iter()
        .map(|call| ToolCallRecord {
            call_id: call.call_id,
            tool: call.tool,
            summary: call.summary,
            statement: call.statement,
            status: ToolCallStatus::from_text(&call.status),
            error_class: call
                .error_class
                .as_deref()
                .map(crate::encoding::error_class_from_text),
        })
        .collect()
}

/// Reads a token count.
///
/// A value SQLite could hold but `u32` cannot reads as « not declared » rather
/// than as a wrong number.
fn decode_token_count(raw: Option<i64>) -> Option<u32> {
    let raw = raw?;
    match u32::try_from(raw) {
        Ok(count) => Some(count),
        Err(_) => {
            tracing::warn!(
                column = "tokens",
                "out-of-range token count in local state, read as undeclared"
            );
            None
        }
    }
}

/// Rebuilds a header from a row.
fn conversation_from_row(row: &Row<'_>) -> Result<Conversation> {
    let id: String = row.get("id")?;
    let workspace: String = row.get("workspace_id")?;
    Ok(Conversation {
        id: parse_id(&id, "ai_conversations.id")?,
        workspace: parse_id(&workspace, "ai_conversations.workspace_id")?,
        connection: parse_id_opt(row.get("connection_id")?, "ai_conversations.connection_id")?,
        connection_name: row.get("connection_name")?,
        destination: destination_from_row(row)?,
        title: row.get("title")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        selected: row
            .get::<_, Option<i64>>("selected_node")?
            .and_then(|node| u32::try_from(node).ok()),
    })
}

/// Rebuilds a destination from a row.
///
/// An identifier this build cannot parse leaves `id` at `None` and keeps the
/// label: the thread stays listed and readable, it simply can no longer point
/// at a declaration.
fn destination_from_row(row: &Row<'_>) -> Result<Destination> {
    let kind: String = row.get("destination_kind")?;
    let raw: Option<String> = row.get("destination_id")?;
    let id = raw.and_then(|raw| {
        ProviderId::new(raw).ok().or_else(|| {
            tracing::warn!(
                column = "destination_id",
                "unreadable destination identifier in local state, conversation kept without it"
            );
            None
        })
    });
    Ok(Destination {
        kind: DestinationKind::from_text(&kind),
        id,
        label: row.get("destination_label")?,
        model: row.get("model")?,
    })
}

/// Rebuilds a list row.
fn summary_from_row(row: &Row<'_>) -> Result<ConversationSummary> {
    let id: String = row.get("id")?;
    let turns: i64 = row.get("turns")?;
    Ok(ConversationSummary {
        id: parse_id(&id, "ai_conversations.id")?,
        title: row.get("title")?,
        destination: destination_from_row(row)?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        turns: u32::try_from(turns).unwrap_or(u32::MAX),
    })
}

/// Rebuilds a turn from a row.
fn turn_from_row(row: &Row<'_>) -> Result<Turn> {
    let ordinal: i64 = row.get("ordinal")?;
    let role: String = row.get("role")?;
    let tier: String = row.get("privacy_tier")?;
    Ok(Turn {
        ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
        record: TurnRecord {
            ts: row.get("ts")?,
            role: TurnRole::from_text(&role),
            tier: privacy_tier_from_column(Some(&tier)),
            agent_session: parse_id_opt(
                row.get("agent_session_id")?,
                "ai_conversation_turns.agent_session_id",
            )?,
            node: row
                .get::<_, Option<i64>>("node")?
                .and_then(|node| u32::try_from(node).ok()),
            text: row.get("text")?,
            reasoning: decode_reasoning(row.get("reasoning")?),
            tool_calls: decode_tool_calls(row.get("tool_calls")?),
            usage: TurnUsage {
                prompt: decode_token_count(row.get("prompt_tokens")?),
                completion: decode_token_count(row.get("completion_tokens")?),
                cache_write: decode_token_count(row.get("cache_write_tokens")?),
                cache_read: decode_token_count(row.get("cache_read_tokens")?),
                reasoning: decode_token_count(row.get("reasoning_tokens")?),
            },
            stop: if row.get::<_, bool>("stop_reason_oversized")? {
                tracing::warn!(
                    column = "stop_reason",
                    "oversized stop reason in local state, read as `Unspecified`"
                );
                Some(StopReason::Unspecified)
            } else {
                decode_stop_reason(row.get("stop_reason")?)
            },
        },
    })
}
