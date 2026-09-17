//! Bounded library pages that do not load document bodies.

use super::*;
use oxyn_core::DocumentFilter;

/// Metadata used by the library; opening a body is a separate request.
#[derive(Debug, Clone)]
pub struct DocumentSummary {
    /// Stable identity.
    pub id: DocumentId,
    /// Named or working title, according to the list scope.
    pub title: String,
    /// Optional original connection.
    pub connection: Option<ConnectionId>,
    /// Display name, absent if the connection was removed.
    pub connection_name: Option<String>,
    /// Last local modification.
    pub updated_at: DateTime<Utc>,
    /// Whether a named copy exists.
    pub is_saved: bool,
    /// Whether the document was left open.
    pub is_open: bool,
    /// Whether the working copy differs from the named copy.
    pub has_changes: bool,
    /// Whether an agent wrote this text.
    ///
    /// Le **fait**, pas la provenance entière : la liste n'a besoin que de la
    /// marque, et charger un JSON par ligne pour afficher un préfixe irait
    /// contre la raison d'être de ce listage — des pages bornées qui ne
    /// matérialisent pas les corps. Le détail, lui, ouvre le document et lit la
    /// provenance complète.
    pub from_agent: bool,
}

/// One page, with a stable creation-identity cursor.
#[derive(Debug, Clone)]
pub struct DocumentPage {
    /// At most the requested number of summaries.
    pub entries: Vec<DocumentSummary>,
    /// Pass this cursor to request the next older page.
    pub next: Option<DocumentId>,
}

impl Documents<'_> {
    /// Searches saved/open documents without materializing their SQL bodies.
    pub fn page(
        &self,
        workspace: WorkspaceId,
        filter: &DocumentFilter,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<DocumentPage> {
        filter.validate().map_err(|error| StoreError::Corrupted {
            field: "document filter",
            detail: error.to_string(),
        })?;
        let search = format!(
            "%{}%",
            filter
                .search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        self.store.with_connection_cancellable(cancel, |connection| {
            let mut query = connection.prepare("SELECT d.id, substr(CASE WHEN ?2 THEN coalesce(d.saved_title,d.title) ELSE d.title END,1,256), d.connection_id, substr(c.name,1,256), d.updated_at, d.is_saved, d.is_open,
                (d.content != coalesce(d.saved_content,'') OR d.title != coalesce(d.saved_title,d.title)),
                d.provenance IS NOT NULL
                FROM documents d LEFT JOIN connections c ON c.id=d.connection_id
                WHERE d.workspace_id=?1 AND d.is_deleted=0 AND (?2=0 OR d.is_saved=1) AND (?3=0 OR d.is_open=1)
                AND (CASE WHEN ?2 THEN coalesce(d.saved_title,d.title) ELSE d.title END LIKE ?4 ESCAPE '\\' OR CASE WHEN ?2 THEN d.saved_content ELSE d.content END LIKE ?4 ESCAPE '\\')
                AND (?5 IS NULL OR d.id < ?5) ORDER BY d.id DESC LIMIT ?6")?;
            let mut entries = query.query_and_then(params![workspace.to_string(), filter.saved_only, filter.open_only, search, filter.before.map(|id| id.to_string()), i64::from(filter.limit)+1], |row| -> Result<DocumentSummary> {
                Ok(DocumentSummary { id: parse_id(&row.get::<_, String>(0)?, "documents.id")?, title: row.get(1)?, connection: parse_id_opt(row.get(2)?, "documents.connection_id")?, connection_name: row.get(3)?, updated_at: row.get(4)?, is_saved: row.get(5)?, is_open: row.get(6)?, has_changes: row.get(7)?, from_agent: row.get(8)? })
            })?.collect::<Result<Vec<_>>>()?;
            let more = entries.len() > usize::from(filter.limit);
            if more { entries.pop(); }
            let next = if more { entries.last().map(|entry| entry.id) } else { None };
            Ok(DocumentPage { entries, next })
        })
    }
}
