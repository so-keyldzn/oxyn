//! Toolkit-independent requests for local query documents and history.

use crate::{ConnectionId, DocumentId, OxynError, QueryLanguage, Result};
use serde::{Deserialize, Serialize};

/// Maximum SQL bytes accepted for a local editable document.
pub const MAX_QUERY_DOCUMENT_BYTES: usize = 1024 * 1024;

/// One working-copy update. A named save can complete after a newer draft update.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryDocumentUpdate {
    /// Stable document identity.
    pub document: DocumentId,
    /// Monotonic working-copy revision.
    pub revision: u64,
    /// Expected stored revision; zero means absent or a legacy revision-zero document.
    #[serde(default)]
    pub expected_revision: Option<u64>,
    /// Working title, at most 256 UTF-8 bytes.
    pub title: String,
    /// Source language, including its dialect.
    pub language: QueryLanguage,
    /// Unexecuted SQL or query text. Bound values are never included.
    pub text: String,
    /// Optional original connection; this does not connect it.
    pub connection: Option<ConnectionId>,
    /// Save the named copy as well as the working draft.
    pub save_named: bool,
    /// Whether the document should be offered for local restoration.
    pub is_open: bool,
}

impl std::fmt::Debug for QueryDocumentUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryDocumentUpdate")
            .field("document", &self.document)
            .field("revision", &self.revision)
            .field("text_bytes", &self.text.len())
            .field("save_named", &self.save_named)
            .field("is_open", &self.is_open)
            .finish_non_exhaustive()
    }
}

impl QueryDocumentUpdate {
    /// Refuses invalid input without echoing document text.
    pub fn validate(&self) -> Result<()> {
        if self.revision == 0
            || i64::try_from(self.revision).is_err()
            || self.expected_revision.is_some_and(|expected| {
                i64::try_from(expected).is_err() || expected >= self.revision
            })
        {
            return Err(OxynError::Config(
                "document revision must be a positive storage integer".into(),
            ));
        }
        if (self.save_named && self.title.trim().is_empty()) || self.title.len() > 256 {
            return Err(OxynError::Config(
                "document title must fit 256 UTF-8 bytes; a named save requires a nonempty title"
                    .into(),
            ));
        }
        if self.text.len() > MAX_QUERY_DOCUMENT_BYTES {
            return Err(OxynError::Config(
                "query text exceeds the 1 MiB editor limit".into(),
            ));
        }
        Ok(())
    }
}

/// Stable pagination and filters for saved or open query documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFilter {
    /// Show named saves only.
    pub saved_only: bool,
    /// Show documents left open only.
    pub open_only: bool,
    /// Literal search in title and working/saved text.
    pub search: String,
    /// Fetch identifiers older than this cursor.
    pub before: Option<DocumentId>,
    /// Page size, from 1 to 200.
    pub limit: u16,
}
impl Default for DocumentFilter {
    fn default() -> Self {
        Self {
            saved_only: true,
            open_only: false,
            search: String::new(),
            before: None,
            limit: 100,
        }
    }
}
impl DocumentFilter {
    /// Validates bounds before a local query is submitted.
    pub fn validate(&self) -> Result<()> {
        validate_page(self.limit, &self.search)
    }
}

/// Explicit history outcome filter; ambiguity is not inferred from error text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryStatusFilter {
    /// Any outcome.
    #[default]
    All,
    /// Still running or interrupted before a final record was written.
    Running,
    /// Successful completion.
    Succeeded,
    /// Execution failure.
    Failed,
    /// Explicit cancellation.
    Cancelled,
    /// Policy refusal.
    Denied,
    /// A possibly applied operation.
    Ambiguous,
}
impl HistoryStatusFilter {
    /// Stable local query tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Denied => "denied",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// Paginated local history, scoped explicitly to all or one connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryFilter {
    /// Only entries associated with a result; the buffer may have expired.
    #[serde(default)]
    pub results_only: bool,
    /// None includes all local connections, including removed ones.
    pub connection: Option<ConnectionId>,
    /// Literal text search, not a LIKE pattern supplied by the caller.
    pub search: String,
    /// Relative date range, or all dates when absent.
    pub days: Option<u16>,
    /// Outcome filter.
    pub status: HistoryStatusFilter,
    /// Entries older than this monotonically assigned identifier.
    pub before: Option<i64>,
    /// Page size, from 1 to 200.
    pub limit: u16,
}
impl Default for HistoryFilter {
    fn default() -> Self {
        Self {
            connection: None,
            search: String::new(),
            days: Some(7),
            results_only: false,
            status: HistoryStatusFilter::All,
            before: None,
            limit: 100,
        }
    }
}
impl HistoryFilter {
    /// Validates query size and date bounds.
    pub fn validate(&self) -> Result<()> {
        validate_page(self.limit, &self.search)?;
        if self.days.is_some_and(|days| days == 0 || days > 3650)
            || self.before.is_some_and(|id| id <= 0)
        {
            return Err(OxynError::Config(
                "invalid history date range or cursor".into(),
            ));
        }
        Ok(())
    }
}
fn validate_page(limit: u16, search: &str) -> Result<()> {
    if !(1..=200).contains(&limit) || search.len() > 1024 {
        return Err(OxynError::Config(
            "local list limit must be 1..=200 and search at most 1024 bytes".into(),
        ));
    }
    Ok(())
}
