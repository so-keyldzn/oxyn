//! The vocabulary of the command bus (ADR-0004).
//!
//! Every possible action in Oxyn is a typed [`Command`] value. The interface
//! does nothing but produce `Command`s; **the tools exposed to agents are
//! exactly these same commands.** There is therefore no parallel "tools" API
//! to audit separately — and no second, poorly logged execution path.
//!
//! Each command carries its [`Actor`] and can answer three questions the
//! `PolicyGate` depends on entirely:
//!
//! * [`Command::intent`] — what does it do to the database?
//! * [`Command::mutation_risk`] — is its scope bounded?
//! * [`Command::target_connection`] — what does it act on?
//!
//! # Classify on the effect, never on the name
//!
//! `EXPLAIN ANALYZE` **runs** the analyzed query, `DELETE` included. A
//! materialized view refreshes. A function called in a `SELECT` can write.
//! That is why [`Command::Execute`] guesses nothing: it reads the intent
//! *declared* in the [`ExecRequest`], and the default intent is
//! [`StatementIntent::Unknown`], which counts as mutating.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::connection::ConnectionConfig;
use crate::ids::{
    AgentId, AgentSessionId, ConnectionId, DocumentId, ResultId, SessionId, StatementHandle,
    WorkspaceId,
};
use crate::preview::PreviewShape;
use crate::query::{ExecRequest, MutationRisk, StatementIntent};

/// Who is asking.
///
/// **Closed** enumeration: the human/agent dichotomy is ADR-0004's, and the
/// whole policy rests on it. A third actor — a plugin, an automation — would be
/// an ADR decision, not a variant added along the way.
///
/// There is no global "agent mode": the actor is carried by the command. A
/// global state gets out of sync, and then it is the audit log that lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Actor {
    /// The user, through the interface.
    Human,
    /// An AI agent, identified by its role and its conversation.
    Agent {
        /// Which agent (SQL, Schema, Performance…).
        id: AgentId,
        /// In which conversation.
        session: AgentSessionId,
    },
}

impl Actor {
    /// Is it an agent?
    #[must_use]
    pub const fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    /// Is it the user?
    #[must_use]
    pub const fn is_human(&self) -> bool {
        matches!(self, Self::Human)
    }

    /// Builds an agent actor.
    #[must_use]
    pub const fn agent(id: AgentId, session: AgentSessionId) -> Self {
        Self::Agent { id, session }
    }
}

impl std::fmt::Display for Actor {
    /// Renders only the role: the conversation identifier has no business in
    /// an interface message.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => f.write_str("human"),
            Self::Agent { .. } => f.write_str("agent"),
        }
    }
}

/// Export format of a result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExportFormat {
    /// CSV.
    Csv,
    /// TSV.
    Tsv,
    /// JSON, an array of objects.
    Json,
    /// JSON lines.
    JsonLines,
    /// Parquet.
    Parquet,
    /// Arrow IPC — the format results already live in (ADR-0002), hence the
    /// only export that converts nothing.
    ArrowIpc,
    /// Instructions `INSERT`.
    Sql,
    /// Markdown table.
    Markdown,
}

impl ExportFormat {
    /// Usual file extension.
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Json => "json",
            Self::JsonLines => "jsonl",
            Self::Parquet => "parquet",
            Self::ArrowIpc => "arrow",
            Self::Sql => "sql",
            Self::Markdown => "md",
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.extension())
    }
}

/// One lazily loaded metadata level, independent of the catalog crate.
///
/// Names are raw identifiers, never SQL. The executor validates their paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
#[non_exhaustive]
pub enum CatalogRefreshScope {
    /// Server identity and the first available hierarchy level.
    Root,
    /// Schemas in a catalog, or directly on a source without catalogs.
    Namespaces {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
    },
    /// Relation summaries only; no fields, indexes or foreign keys.
    Relations {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
    },
    /// Fields and metadata of one explicitly requested relation, plus its
    /// indexes and foreign keys when the session declares `INDEXES` and
    /// `FOREIGN_KEYS`.
    ///
    /// A source that declares neither leaves both unread rather than storing an
    /// empty list: the cache distinguishes "not read" from "none", and a view
    /// that confused them would claim a table has no index because nobody can
    /// tell. Constraints have their own explicit scope.
    Relation {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw relation name.
        relation: String,
    },
    /// Constraints of one relation, gated by the session's `CONSTRAINTS` capability.
    Constraints {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw relation name, never SQL.
        relation: String,
    },
    /// Foreign keys declared by relations that reference this target.
    IncomingForeignKeys {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw target relation name.
        relation: String,
    },
    /// Creation statements for one relation; this remains a metadata read.
    Definition {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw target relation name.
        relation: String,
    },
    /// The namespaces an unqualified name resolves in, in priority order.
    ///
    /// A metadata read of the server that names no object. A source that
    /// cannot say leaves the cache on "unknown" rather than on an empty path,
    /// which would assert that nothing resolves
    /// ([ADR-0057](../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md)).
    ResolutionPath,
}

/// Longest focus accepted by [`Command::DescribeCatalog`], in bytes.
///
/// A focus is a few search words. The bound keeps what an agent writes there
/// from growing the audit journal or the search without limit.
pub const MAX_CATALOG_FOCUS_BYTES: usize = 256;

/// An action submitted to the bus.
///
/// A command does **one** thing. A "convenient" command that wraps three
/// bypasses the policy: the `PolicyGate` only decides on what it sees, and it
/// would see a read where there is a hidden write.
///
/// The enumeration is **closed**, unlike the repository's convention. The
/// execution bus dispatches on an exhaustive `match`: adding a command must
/// break its compilation, otherwise there is a command nothing executes — or
/// worse, one a `_ =>` swallows silently after the `PolicyGate` authorized it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum Command {
    /// Open a session on a configured connection.
    Connect {
        /// The target connection.
        connection: ConnectionId,
    },

    /// Close a connection's sessions.
    Disconnect {
        /// The target connection.
        connection: ConnectionId,
    },

    /// Close one session without disconnecting sibling consoles or the catalog.
    CloseSession {
        /// Owning connection, checked before closing.
        connection: ConnectionId,
        /// Session to release.
        session: SessionId,
    },

    /// Declare where a session resolves unqualified names.
    ///
    /// The levels travel as strings rather than as a `CatalogPath`:
    /// `oxyn-core` does not depend on `oxyn-catalog`, and the executor rebuilds
    /// the path at the boundary, as for [`PreviewRelation`](Self::PreviewRelation).
    SetSessionContext {
        /// The connection whose policy applies.
        connection: ConnectionId,
        /// The target session. The context never leaves it.
        session: SessionId,
        /// Catalog level, when the engine has one.
        catalog: Option<String>,
        /// Namespace level — a schema, where there is one.
        namespace: Option<String>,
    },

    /// Execute a statement.
    Execute {
        /// The target connection.
        connection: ConnectionId,
        /// The session to execute on.
        session: SessionId,
        /// What to execute. Boxed so as not to make every other variant
        /// bigger.
        request: Box<ExecRequest>,
    },

    /// Read a bounded preview using identifiers quoted by the session driver.
    PreviewRelation {
        /// Connection whose policy applies.
        connection: ConnectionId,
        /// Open session belonging to that connection.
        session: SessionId,
        /// Catalog from the metadata path, when present.
        catalog: Option<String>,
        /// Namespace from the metadata path, when present.
        namespace: Option<String>,
        /// Exact relation name, never SQL text.
        relation: String,
        /// Maximum rows requested, in `1..=1000`.
        limit: u32,
        /// Requested order, filters and page.
        ///
        /// Empty for the automatic preview of a table just selected. The driver
        /// refuses what it cannot do rather than ignore it: a silently dropped
        /// filter would return rows the user believes excluded
        /// ([ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
        shape: PreviewShape,
    },

    /// Cancel an execution in progress.
    ///
    /// Always allowed: refusing a cancellation protects nothing and leaves a
    /// query running on the server side.
    Cancel {
        /// The target connection.
        connection: ConnectionId,
        /// The execution to interrupt.
        statement: StatementHandle,
    },

    /// Read a connection's catalog again.
    RefreshCatalog {
        /// The target connection.
        connection: ConnectionId,
    },

    /// Read the connection's **local** catalog cache, without contacting the
    /// server.
    ///
    /// What an agent asks for when it needs the structure of the database. The
    /// executor hands back the cache itself, never a rendering: what of it
    /// reaches a prompt is decided in `oxyn-ai`, under the connection's
    /// privacy tier ([I-04](../../../CLAUDE.md#i-04)). Its outcome carries no
    /// row value, because the catalog holds none.
    DescribeCatalog {
        /// Connection whose cache is read, and whose policy applies.
        connection: ConnectionId,
        /// Words to orient the selection of relations — a table name, a
        /// topic. Search terms, never query text: nothing composes a
        /// statement from them. Bounded by [`MAX_CATALOG_FOCUS_BYTES`].
        focus: Option<String>,
    },

    /// Refresh one metadata level through an existing session.
    RefreshCatalogScope {
        /// Connection whose policy and cache apply.
        connection: ConnectionId,
        /// Explicit lazy scope; never recursively loads descendants.
        scope: CatalogRefreshScope,
    },

    /// Inspect a bounded text page of one existing value, without executing SQL.
    InspectResultValue {
        /// Connection owning the result.
        connection: ConnectionId,
        /// Existing result.
        result: ResultId,
        /// Global zero-based row position.
        row: usize,
        /// Original Arrow column position, regardless of UI visibility.
        column: usize,
        /// UTF-8 byte offset in the formatted value.
        offset: usize,
    },

    /// Load one already-produced result page without executing a query.
    ReadResultPage {
        /// Connection that owns the result and supplies its policy.
        connection: ConnectionId,
        /// Existing result, including a result whose query has finished.
        result: ResultId,
        /// Zero-based batch position, never a row number or SQL offset.
        batch: usize,
    },

    /// Write a result set to a file.
    Export {
        /// The connection the result comes from, for the audit.
        connection: ConnectionId,
        /// The result to export.
        result: ResultId,
        /// The format.
        format: ExportFormat,
        /// The destination file.
        destination: PathBuf,
    },

    /// Read versioned display preferences without contacting a database server.
    ReadWorkspacePreferences {
        /// Owning workspace.
        workspace: WorkspaceId,
    },
    /// Save a newer display snapshot. Only the human may change their interface.
    WriteWorkspacePreferences {
        /// Owning workspace.
        workspace: WorkspaceId,
        /// Validated, monotonically revised snapshot.
        snapshot: Box<crate::PreferencesSnapshot>,
    },

    /// Save or remove one window's layout: its rectangle, object tab and
    /// consoles. Only the human arranges their windows (ADR-0043).
    WriteWindowLayout {
        /// Owning workspace.
        workspace: WorkspaceId,
        change: Box<crate::WindowLayoutChange>,
    },

    /// List bounded local document summaries without loading query bodies.
    ListQueryDocuments {
        workspace: WorkspaceId,
        filter: Box<crate::DocumentFilter>,
    },
    /// Persist a draft and optionally its named copy, without executing it.
    SaveQueryDocument {
        workspace: WorkspaceId,
        update: Box<crate::QueryDocumentUpdate>,
    },
    /// Close a working document, retaining a barrier against late writes.
    CloseQueryDocument {
        /// Optional optimistic concurrency check, applied atomically with closing.
        #[serde(default)]
        expected_revision: Option<u64>,
        workspace: WorkspaceId,
        document: DocumentId,
        revision: u64,
        discard: bool,
    },
    /// Delete a named query locally; never deletes database objects.
    DeleteQueryDocument {
        workspace: WorkspaceId,
        document: DocumentId,
        revision: u64,
    },
    /// Search the user's local history, not a server query log.
    ReadHistory { filter: Box<crate::HistoryFilter> },
    /// Load a full selected historical statement separately from its list row.
    ReadHistoryEntry { entry: i64 },
    /// Record that the user inspected the server state of a write whose outcome
    /// was unknown, so it stops warning at each launch. Local only: nothing is
    /// sent to the server and nothing is retried (I-13).
    ReconcileHistoryEntry { entry: i64 },
    /// List the connections history recorded, including ones since removed.
    ///
    /// Separate from [`Command::ReadHistory`] on purpose: that one answers
    /// "which executions match these filters", this one answers "which
    /// connections may be filtered on". Folding them together would make a
    /// single command whose two answers have no reason to be paged alike.
    ListHistoryConnections {
        /// Workspace against which existence is resolved, not a history scope.
        workspace: WorkspaceId,
        /// Page bounds; history may name many connections.
        filter: crate::HistoryConnectionFilter,
    },
    /// Reopen an existing result without creating a query or a session.
    OpenRetainedResult {
        connection: ConnectionId,
        result: ResultId,
    },

    /// Open a workspace document.
    OpenDocument {
        /// The owning workspace.
        workspace: WorkspaceId,
        /// The document.
        document: DocumentId,
    },

    /// Write a workspace document.
    ///
    /// Reaches no database: see [`Command::intent`] for what that implies —
    /// and does not imply.
    WriteDocument {
        /// The owning workspace.
        workspace: WorkspaceId,
        /// The document.
        document: DocumentId,
        /// The new content.
        text: String,
    },

    /// Register a new connection.
    CreateConnection {
        /// The configuration, without any secret in clear.
        config: Box<ConnectionConfig>,
    },

    /// Open a session on a configuration, then close it at once.
    ///
    /// Nothing is saved and no session outlives the command: it answers "would
    /// this connection open?" before the user commits to it (UX-SPEC,
    /// « First workspace navigation »). It reaches the server, so the
    /// `PolicyGate` refuses it to an [`Actor::Agent`]: an agent that could
    /// point it at the host of its choice would hold an exfiltration channel,
    /// the same one [`CreateConnection`](Self::CreateConnection) guards.
    TestConnection {
        /// The configuration to try, carrying a secret reference and never a
        /// secret in clear.
        config: Box<ConnectionConfig>,
    },

    /// Modify an existing connection.
    UpdateConnection {
        /// The full configuration after modification.
        config: Box<ConnectionConfig>,
    },

    /// Remove a connection from the workspace.
    DeleteConnection {
        /// The target connection.
        connection: ConnectionId,
    },

    /// List the AI providers declared on this machine, without contacting any
    /// of them.
    ///
    /// This is what decides whether the AI workspace exists at all: an empty
    /// list is the default installation, not a failure
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    /// It resolves no name and reports no reach — a stored classification would
    /// be yesterday's DNS answer applied to today's request.
    ListAiProviders,

    /// Declare an AI provider, or replace the declaration bearing its
    /// identifier.
    SaveAiProvider {
        /// The declaration, carrying a secret reference and never a key.
        /// Boxed like [`CreateConnection`](Self::CreateConnection): it would
        /// otherwise widen every other variant.
        config: Box<crate::ai::AiProviderConfig>,
    },

    /// List the declared external agents.
    ///
    /// Separate from [`ListAiProviders`](Self::ListAiProviders) because the two
    /// declarations have nothing in common: an external agent has no endpoint,
    /// no model, and above all **no secret reference**
    /// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    ListExternalAgents,

    /// Declare an external agent, or replace the declaration bearing its
    /// identifier.
    SaveExternalAgent {
        /// The declaration: a command to run, its arguments, its environment.
        /// **Never a secret** — an external agent carries its own
        /// authentication, which is the point of the mode.
        agent: Box<crate::ai::ExternalAgentConfig>,
    },

    /// Remove a declared external agent.
    ///
    /// Nothing else is revoked: unlike a provider, there is no keychain entry
    /// behind it.
    RemoveExternalAgent {
        /// The declaration to forget.
        id: crate::ai::ProviderId,
    },

    /// Remove a declared AI provider. Documents keep the provenance they were
    /// written with.
    RemoveAiProvider {
        /// The declaration to forget.
        id: crate::ai::ProviderId,
    },
}

impl Command {
    /// What the command does **to the database**.
    ///
    /// A few choices deserve to be spelled out, because they read badly:
    ///
    /// * [`WriteDocument`](Self::WriteDocument) answers
    ///   [`Read`](StatementIntent::Read). Writing a saved query touches no
    ///   database; qualifying it as mutating would bring up a "database"
    ///   approval request where there is only a local file, and a confirmation
    ///   that matches nothing ends up clicked without being read.
    /// * the connection management commands answer
    ///   [`Ddl`](StatementIntent::Ddl). It is not DDL in the SQL sense, but the
    ///   effect is of the same order: an agent that could create a connection
    ///   to the host of its choice would have an exfiltration channel. The
    ///   `PolicyGate` **refuses** `CreateConnection` and `UpdateConnection` to
    ///   an agent outright — a connection's switches, like sqlite-vec's, are
    ///   the human's (ADR-0054) — and judges `DeleteConnection` as DDL.
    /// * [`Cancel`](Self::Cancel) answers `Read`: cancelling modifies nothing,
    ///   and a cancellation that must be approved is not a cancellation.
    /// * [`ReconcileHistoryEntry`](Self::ReconcileHistoryEntry) answers `Read`
    ///   for the same reason as `WriteDocument`: it only writes the local
    ///   state. What protects it is a **refusal** to any [`Actor::Agent`]: it
    ///   asserts that the user checked the server, and an agent that could
    ///   assert it would silence the warning of a write nobody looked at
    ///   (I-13).
    /// * the AI provider declaration commands answer `Read`, like
    ///   [`WriteDocument`](Self::WriteDocument) and for the same reason: they
    ///   reach no database, and a "database" confirmation that matches nothing
    ///   ends up clicked without being read. What protects them is not an
    ///   approval but a **refusal**: the `PolicyGate` forbids
    ///   [`SaveAiProvider`](Self::SaveAiProvider) and
    ///   [`RemoveAiProvider`](Self::RemoveAiProvider) to an [`Actor::Agent`] —
    ///   an agent that declared the endpoint it speaks through would have an
    ///   exfiltration channel, and it is the category of action it has no
    ///   legitimate use for.
    /// * [`SetSessionContext`](Self::SetSessionContext) answers `Read`,
    ///   whereas a typed `USE` inside an [`Execute`](Self::Execute) is
    ///   [`Unknown`](StatementIntent::Unknown): the selector goes through the
    ///   session, which records the new context, and the typed statement does
    ///   not. That is why the selector is the user's path, and why the
    ///   `PolicyGate` refuses the typed one to an agent.
    #[must_use]
    pub fn intent(&self) -> StatementIntent {
        match self {
            Self::Execute { request, .. } => request.intent,
            Self::Connect { .. }
            | Self::Disconnect { .. }
            | Self::CloseSession { .. }
            | Self::SetSessionContext { .. }
            | Self::Cancel { .. }
            | Self::RefreshCatalog { .. }
            | Self::DescribeCatalog { .. }
            | Self::RefreshCatalogScope { .. }
            | Self::PreviewRelation { .. }
            | Self::ReadResultPage { .. }
            | Self::InspectResultValue { .. }
            | Self::Export { .. }
            | Self::ReadWorkspacePreferences { .. }
            | Self::WriteWorkspacePreferences { .. }
            | Self::WriteWindowLayout { .. }
            | Self::ListQueryDocuments { .. }
            | Self::SaveQueryDocument { .. }
            | Self::CloseQueryDocument { .. }
            | Self::DeleteQueryDocument { .. }
            | Self::ReadHistory { .. }
            | Self::ReadHistoryEntry { .. }
            | Self::ReconcileHistoryEntry { .. }
            | Self::ListHistoryConnections { .. }
            | Self::OpenRetainedResult { .. }
            | Self::OpenDocument { .. }
            | Self::WriteDocument { .. }
            | Self::ListAiProviders
            | Self::SaveAiProvider { .. }
            | Self::RemoveAiProvider { .. }
            | Self::ListExternalAgents
            | Self::SaveExternalAgent { .. }
            | Self::RemoveExternalAgent { .. }
            // Opens and closes a session, writes nothing: the same effect as
            // `Connect`, on a configuration the workspace does not hold yet.
            | Self::TestConnection { .. } => StatementIntent::Read,
            Self::CreateConnection { .. }
            | Self::UpdateConnection { .. }
            | Self::DeleteConnection { .. } => StatementIntent::Ddl,
        }
    }

    /// The target connection, when there is one.
    ///
    /// It is what the `PolicyGate` uses to find the environment marking and the
    /// read-only flag.
    #[must_use]
    pub fn target_connection(&self) -> Option<ConnectionId> {
        match self {
            Self::ReadHistory { filter } => filter.connection,
            Self::Connect { connection }
            | Self::Disconnect { connection }
            | Self::CloseSession { connection, .. }
            | Self::SetSessionContext { connection, .. }
            | Self::Execute { connection, .. }
            | Self::Cancel { connection, .. }
            | Self::RefreshCatalog { connection }
            | Self::DescribeCatalog { connection, .. }
            | Self::RefreshCatalogScope { connection, .. }
            | Self::PreviewRelation { connection, .. }
            | Self::ReadResultPage { connection, .. }
            | Self::OpenRetainedResult { connection, .. }
            | Self::InspectResultValue { connection, .. }
            | Self::Export { connection, .. }
            | Self::DeleteConnection { connection } => Some(*connection),
            Self::CreateConnection { config }
            | Self::UpdateConnection { config }
            | Self::TestConnection { config } => Some(config.id),
            Self::ListQueryDocuments { .. }
            | Self::SaveQueryDocument { .. }
            | Self::CloseQueryDocument { .. }
            | Self::DeleteQueryDocument { .. }
            | Self::ReadHistoryEntry { .. }
            | Self::ReconcileHistoryEntry { .. }
            | Self::ListHistoryConnections { .. }
            | Self::OpenDocument { .. }
            | Self::WriteDocument { .. }
            | Self::ReadWorkspacePreferences { .. }
            | Self::WriteWorkspacePreferences { .. }
            | Self::WriteWindowLayout { .. }
            // A provider is declared **per machine** (ADR-0023): it has no
            // target connection, and the privacy tier that governs its use
            // stays the one of the open connection.
            | Self::ListAiProviders
            | Self::SaveAiProvider { .. }
            | Self::RemoveAiProvider { .. }
            | Self::ListExternalAgents
            | Self::SaveExternalAgent { .. }
            | Self::RemoveExternalAgent { .. } => None,
        }
    }

    /// The declared risk, for the commands that carry one.
    #[must_use]
    pub fn mutation_risk(&self) -> MutationRisk {
        match self {
            Self::Execute { request, .. } => request.risk,
            _ => MutationRisk::None,
        }
    }

    /// Does the command begin, end or mark a transaction on its session?
    ///
    /// Only an [`Execute`](Self::Execute) can, and only as its reclassified
    /// request says: see [`ExecRequest::transaction_control`].
    #[must_use]
    pub fn controls_transaction(&self) -> bool {
        match self {
            Self::Execute { request, .. } => request.transaction_control,
            _ => false,
        }
    }

    /// Does the command change the session's current database or schema by
    /// typing it (`USE ...`)?
    ///
    /// Only an [`Execute`](Self::Execute) can, and only as its reclassified
    /// request says: see [`ExecRequest::context_change`]. The declared way to
    /// change the context is [`SetSessionContext`](Self::SetSessionContext),
    /// which the session records: a typed `USE` would move the server's state
    /// without the session knowing.
    #[must_use]
    pub fn changes_context(&self) -> bool {
        match self {
            Self::Execute { request, .. } => request.context_change,
            _ => false,
        }
    }

    /// Does the command run **against the server**?
    ///
    /// Tells what crosses the external boundary from what stays in the
    /// workspace. A connection's read-only flag can only protect the first
    /// category — refusing to remove a connection from the workspace because
    /// it is read-only would make no sense.
    #[must_use]
    pub const fn touches_database(&self) -> bool {
        matches!(
            self,
            Self::Connect { .. }
                | Self::Disconnect { .. }
                | Self::CloseSession { .. }
                | Self::SetSessionContext { .. }
                | Self::Execute { .. }
                | Self::Cancel { .. }
                | Self::RefreshCatalog { .. }
                | Self::RefreshCatalogScope { .. }
                | Self::PreviewRelation { .. }
                | Self::Export { .. }
                | Self::TestConnection { .. }
        )
    }

    /// Can the command modify something?
    ///
    /// True as soon as the intent is mutating **or** a risk is reported. An
    /// inconsistent declaration is settled on the cautious side.
    #[must_use]
    pub fn is_mutating(&self) -> bool {
        self.intent().is_mutating() || self.mutation_risk().is_some()
    }

    /// Stable name of the command, for the audit log.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Connect { .. } => "Connect",
            Self::Disconnect { .. } => "Disconnect",
            Self::CloseSession { .. } => "CloseSession",
            Self::SetSessionContext { .. } => "SetSessionContext",
            Self::Execute { .. } => "Execute",
            Self::PreviewRelation { .. } => "PreviewRelation",
            Self::ReadResultPage { .. } => "ReadResultPage",
            Self::InspectResultValue { .. } => "InspectResultValue",
            Self::Cancel { .. } => "Cancel",
            Self::RefreshCatalog { .. } => "RefreshCatalog",
            Self::DescribeCatalog { .. } => "DescribeCatalog",
            Self::RefreshCatalogScope { .. } => "RefreshCatalogScope",
            Self::Export { .. } => "Export",
            Self::OpenDocument { .. } => "OpenDocument",
            Self::ListQueryDocuments { .. } => "ListQueryDocuments",
            Self::SaveQueryDocument { .. } => "SaveQueryDocument",
            Self::CloseQueryDocument { .. } => "CloseQueryDocument",
            Self::DeleteQueryDocument { .. } => "DeleteQueryDocument",
            Self::ReadHistory { .. } => "ReadHistory",
            Self::ReadHistoryEntry { .. } => "ReadHistoryEntry",
            Self::ReconcileHistoryEntry { .. } => "ReconcileHistoryEntry",
            Self::ListHistoryConnections { .. } => "ListHistoryConnections",
            Self::OpenRetainedResult { .. } => "OpenRetainedResult",
            Self::ReadWorkspacePreferences { .. } => "ReadWorkspacePreferences",
            Self::WriteWorkspacePreferences { .. } => "WriteWorkspacePreferences",
            Self::WriteWindowLayout { .. } => "WriteWindowLayout",
            Self::WriteDocument { .. } => "WriteDocument",
            Self::CreateConnection { .. } => "CreateConnection",
            Self::TestConnection { .. } => "TestConnection",
            Self::UpdateConnection { .. } => "UpdateConnection",
            Self::DeleteConnection { .. } => "DeleteConnection",
            Self::ListAiProviders => "ListAiProviders",
            Self::SaveAiProvider { .. } => "SaveAiProvider",
            Self::RemoveAiProvider { .. } => "RemoveAiProvider",
            Self::ListExternalAgents => "ListExternalAgents",
            Self::SaveExternalAgent { .. } => "SaveExternalAgent",
            Self::RemoveExternalAgent { .. } => "RemoveExternalAgent",
        }
    }

    /// The statement text, for an approval preview.
    ///
    /// Only exposes what the user wrote themselves: never bound values (I-03).
    #[must_use]
    pub fn statement_text(&self) -> Option<&str> {
        match self {
            Self::Execute { request, .. } => Some(&request.text),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
