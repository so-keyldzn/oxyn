//! What crosses the boundary for query documents and local history.
//!
//! Lists carry no complete SQL bodies: a page is bounded, and a body is read
//! only when one entry is opened ([ADR-0014](../../../../docs/adr/0014-documents-et-historique.md)).
//! Bound values never appear here — they are never stored.

use oxyn_core::HistoryStatusFilter;
use oxyn_store::ActorKind;
use oxyn_store::documents::{DocumentPage, DocumentSummary};
use oxyn_store::history::{HistoryConnectionPage, HistoryPage, HistorySummary};
use serde::{Deserialize, Serialize};

/// A working or named copy change, as a console submits it.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChange {
    pub document: String,
    /// Monotonic per document, chosen by the console: the backend refuses a
    /// revision that does not advance.
    pub revision: u64,
    pub title: String,
    pub text: String,
    /// The connection the text was written for; it does not connect anything.
    pub connection: Option<String>,
    /// Also write the named copy the library lists.
    #[serde(default)]
    pub named: bool,
}

// Not derived: the text is the user's SQL, which a log line has no reason to
// carry whole.
impl std::fmt::Debug for DocumentChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentChange")
            .field("revision", &self.revision)
            .field("text_bytes", &self.text.len())
            .field("named", &self.named)
            .finish_non_exhaustive()
    }
}

/// The answer to a save or a close.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DocumentWrite {
    /// The store acknowledged exactly what was sent.
    #[serde(rename_all = "camelCase")]
    Saved {
        revision: u64,
        saved_revision: u64,
        is_saved: bool,
    },
    Closed,
    /// The stored document moved under this console. Nothing local is lost:
    /// the console keeps its text and offers a new copy
    /// ([ADR-0016](../../../../docs/adr/0016-autosauvegarde-bornee.md)).
    Conflict {
        message: String,
    },
    /// A newer draft replaced this one before it was written, or the user
    /// cancelled: not a failure.
    Superseded,
}

/// A document opened in full.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    pub id: String,
    pub title: String,
    pub text: String,
    pub saved_title: Option<String>,
    pub saved_text: Option<String>,
    pub revision: u64,
    pub saved_revision: u64,
    pub is_saved: bool,
    pub is_open: bool,
    pub connection: Option<String>,
    pub from_agent: bool,
}

impl From<oxyn_store::Document> for DocumentView {
    fn from(document: oxyn_store::Document) -> Self {
        Self {
            id: document.id.to_string(),
            title: document.title,
            text: document.content,
            saved_title: document.saved_title,
            saved_text: document.saved_content,
            revision: document.revision,
            saved_revision: document.saved_revision,
            is_saved: document.is_saved,
            is_open: document.is_open,
            connection: document.connection.map(|id| id.to_string()),
            from_agent: document.provenance.is_some(),
        }
    }
}

/// Which documents a library page lists.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentQuery {
    #[serde(default)]
    pub saved_only: bool,
    #[serde(default)]
    pub open_only: bool,
    #[serde(default)]
    pub search: String,
    pub before: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEntry {
    pub id: String,
    pub title: String,
    pub connection: Option<String>,
    /// Absent when the connection was removed.
    pub connection_name: Option<String>,
    /// RFC 3339, UTC.
    pub updated_at: String,
    pub is_saved: bool,
    pub is_open: bool,
    pub has_changes: bool,
    pub from_agent: bool,
}

impl From<DocumentSummary> for DocumentEntry {
    fn from(summary: DocumentSummary) -> Self {
        Self {
            id: summary.id.to_string(),
            title: summary.title,
            connection: summary.connection.map(|id| id.to_string()),
            connection_name: summary.connection_name,
            updated_at: summary.updated_at.to_rfc3339(),
            is_saved: summary.is_saved,
            is_open: summary.is_open,
            has_changes: summary.has_changes,
            from_agent: summary.from_agent,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentList {
    pub entries: Vec<DocumentEntry>,
    pub next: Option<String>,
}

impl From<DocumentPage> for DocumentList {
    fn from(page: DocumentPage) -> Self {
        Self {
            entries: page.entries.into_iter().map(Into::into).collect(),
            next: page.next.map(|id| id.to_string()),
        }
    }
}

/// Which executions a history page lists.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    pub connection: Option<String>,
    #[serde(default)]
    pub search: String,
    /// `None` means all dates.
    pub days: Option<u16>,
    #[serde(default)]
    pub status: HistoryStatusChoice,
    pub before: Option<i64>,
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryStatusChoice {
    #[default]
    All,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Denied,
    Ambiguous,
}

impl From<HistoryStatusChoice> for HistoryStatusFilter {
    fn from(choice: HistoryStatusChoice) -> Self {
        match choice {
            HistoryStatusChoice::All => Self::All,
            HistoryStatusChoice::Running => Self::Running,
            HistoryStatusChoice::Succeeded => Self::Succeeded,
            HistoryStatusChoice::Failed => Self::Failed,
            HistoryStatusChoice::Cancelled => Self::Cancelled,
            HistoryStatusChoice::Denied => Self::Denied,
            HistoryStatusChoice::Ambiguous => Self::Ambiguous,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub id: i64,
    /// RFC 3339, UTC.
    pub at: String,
    pub connection_name: Option<String>,
    /// At most 256 characters; never executed.
    pub preview: String,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub rows: Option<u64>,
    /// An ambiguous or unresolved write: inspect the server, never replay
    /// ([I-13](../../../../CLAUDE.md#i-13)).
    pub needs_inspection: bool,
    /// The user declared this write inspected on the server; it no longer warns.
    pub reconciled: bool,
    /// The connection it ran on, to address a retained result. Never rendered.
    pub connection: Option<String>,
    /// A result that may still be retained; its buffer can have expired.
    pub result: Option<String>,
    /// Submitted by an agent: a copy keeps saying so
    /// ([ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    pub from_agent: bool,
}

impl From<HistorySummary> for HistoryRow {
    fn from(summary: HistorySummary) -> Self {
        Self {
            id: summary.id,
            at: summary.ts.to_rfc3339(),
            connection_name: summary.connection_name,
            preview: summary.statement_preview,
            status: summary.status.as_str().to_owned(),
            duration_ms: summary.duration.map(crate::ipc::millis),
            rows: summary.rows,
            needs_inspection: summary.requires_reconciliation,
            reconciled: summary.reconciled_at.is_some(),
            connection: summary.connection.map(|id| id.to_string()),
            result: summary.result.map(|id| id.to_string()),
            from_agent: summary.actor_kind == ActorKind::Agent,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryList {
    pub entries: Vec<HistoryRow>,
    pub next: Option<i64>,
}

impl From<HistoryPage> for HistoryList {
    fn from(page: HistoryPage) -> Self {
        Self {
            entries: page.entries.into_iter().map(Into::into).collect(),
            next: page.next,
        }
    }
}

/// One execution, read in full to be copied into a console — never replayed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDetail {
    pub id: i64,
    pub statement: String,
    pub connection_name: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub needs_inspection: bool,
    /// Submitted by an agent: the console it is copied into says so.
    pub from_agent: bool,
}

impl From<oxyn_store::HistoryEntry> for HistoryDetail {
    fn from(entry: oxyn_store::HistoryEntry) -> Self {
        let needs_inspection = entry.record.requires_reconciliation();
        Self {
            id: entry.id,
            statement: entry.record.statement,
            connection_name: entry.record.connection_name,
            status: entry.record.status.as_str().to_owned(),
            error: entry.record.error,
            needs_inspection,
            from_agent: entry.record.actor_kind == ActorKind::Agent,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryConnection {
    pub connection: String,
    pub name: Option<String>,
    /// False once removed: its entries stay listed and searchable.
    pub in_workspace: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryConnectionList {
    pub entries: Vec<HistoryConnection>,
    pub next: Option<i64>,
}

impl From<HistoryConnectionPage> for HistoryConnectionList {
    fn from(page: HistoryConnectionPage) -> Self {
        Self {
            entries: page
                .entries
                .into_iter()
                .map(|entry| HistoryConnection {
                    connection: entry.connection.to_string(),
                    name: entry.name,
                    in_workspace: entry.in_workspace,
                })
                .collect(),
            next: page.next,
        }
    }
}

/// A retained result reopened without running anything.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RetainedResult {
    #[serde(rename_all = "camelCase")]
    Open {
        result: String,
        columns: Vec<crate::ipc::ResultColumn>,
        rows: usize,
        /// Only an exhausted stream describes a whole result.
        complete: bool,
        /// The buffer stopped retaining rows at its bound.
        truncated: bool,
    },
    /// The buffer was released: nothing is rerun to bring it back.
    Expired,
}
