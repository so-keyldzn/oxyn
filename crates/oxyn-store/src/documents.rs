//! The `documents` table: saved tabs and queries.
//!
//! A document is what the user sees as a tab: a title, a language, a text,
//! and — optionally — the connection it runs against. It is the most precious
//! content of the state file: the rest can be rebuilt, the user's work cannot.
//!
//! Deleting the associated connection does **not** delete the document: its
//! reference becomes `NULL` (`ON DELETE SET NULL`). Losing a hand-written
//! query because a connection list was cleaned up would be data loss
//! disguised as tidying.

use chrono::{DateTime, Utc};
use oxyn_core::{ConnectionId, DocumentId, Provenance, QueryLanguage, WorkspaceId};
use rusqlite::{OptionalExtension, Row, params};

use crate::encoding::{parse_id, parse_id_opt, tag_from_json, tag_to_json};
use crate::error::{Result, StoreError};
mod lifecycle;
pub use lifecycle::DocumentRevision;
mod listing;
use crate::store::Store;
pub use listing::{DocumentPage, DocumentSummary};

/// A workspace document.
#[derive(Clone, PartialEq, Eq)]
pub struct Document {
    /// Internal identifier.
    pub id: DocumentId,
    /// The owning workspace.
    pub workspace: WorkspaceId,
    /// Title displayed on the tab.
    pub title: String,
    /// Language of the content, dialect included.
    pub language: QueryLanguage,
    /// The text, as the user wrote it.
    pub content: String,
    /// The connection it runs against, when there is one.
    pub connection: Option<ConnectionId>,
    /// Creation date.
    pub created_at: DateTime<Utc>,
    /// Date of the last write.
    pub updated_at: DateTime<Utc>,
    /// Latest working-copy revision.
    pub revision: u64,
    /// Latest named-state decision, including a close/discard barrier.
    pub saved_revision: u64,
    /// A named copy exists independently of the draft.
    pub is_saved: bool,
    /// The document was left open locally; this never reconnects it.
    pub is_open: bool,
    /// Explicitly saved text, absent for an unnamed draft.
    pub saved_content: Option<String>,
    /// Explicitly saved title.
    pub saved_title: Option<String>,
    /// Where this text comes from. `None` means "written by the user", and it
    /// is the case of every document older than migration 7.
    ///
    /// A provenance **is not erased** when the content changes: it dates the
    /// origin of the text, not the last keystroke
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    pub provenance: Option<Provenance>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("id", &self.id)
            .field("workspace", &self.workspace)
            .field("revision", &self.revision)
            .field("text_bytes", &self.content.len())
            .field("is_saved", &self.is_saved)
            .field("is_open", &self.is_open)
            // Showable, and it has to be: none of its fields is a secret, and
            // it is what gets read the day a document carries an unexpected
            // origin.
            .field("provenance", &self.provenance)
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Builds a new document, not yet persisted.
    #[must_use]
    pub fn new(workspace: WorkspaceId, title: impl Into<String>, language: QueryLanguage) -> Self {
        let now = Utc::now();
        Self {
            id: DocumentId::new(),
            workspace,
            title: title.into(),
            language,
            content: String::new(),
            connection: None,
            created_at: now,
            updated_at: now,
            revision: 0,
            saved_revision: 0,
            is_saved: true,
            is_open: false,
            saved_content: None,
            saved_title: None,
            provenance: None,
        }
    }

    /// Declares that this text comes from an agent.
    ///
    /// Only call it on the document **the agent wrote**: the provenance is not
    /// a session label, it is the origin of a text.
    #[must_use]
    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    /// Sets the content.
    #[must_use]
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }

    /// Attaches a connection.
    #[must_use]
    pub fn on_connection(mut self, connection: ConnectionId) -> Self {
        self.connection = Some(connection);
        self
    }
}

/// Typed access to the `documents` table.
#[derive(Debug)]
pub struct Documents<'a> {
    store: &'a Store,
}

impl<'a> Documents<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Inserts or updates a document, and sets `updated_at` to now.
    ///
    /// `created_at` is never overwritten by an update.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the workspace or the connection does
    /// not exist — the foreign keys refuse it — or if the write fails;
    /// [`crate::StoreError::Json`] if the language is not serializable.
    pub fn save(&self, document: &Document) -> Result<()> {
        let language = tag_to_json(&document.language)?;
        let provenance = provenance_to_column(document.provenance.as_ref())?;
        let now = Utc::now();

        self.store.with_connection(|conn| {
            let changed = conn.execute(
                "INSERT INTO documents
                     (id, workspace_id, title, language, content, connection_id,
                      created_at, updated_at, saved_content, saved_title, provenance)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5, ?3, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                     workspace_id  = excluded.workspace_id,
                     title         = excluded.title,
                     language      = excluded.language,
                     content       = excluded.content,
                     connection_id = excluded.connection_id,
                     updated_at    = excluded.updated_at,
                     saved_content = excluded.saved_content,
                     saved_title = excluded.saved_title,
                     -- A provenance is not erased when the content changes:
                     -- it dates the origin of the text, not the last keystroke
                     -- (ADR-0023). A caller that carries none — because it
                     -- reads a document back and rewrites it — therefore
                     -- cannot erase the existing one.
                     provenance    = coalesce(excluded.provenance, documents.provenance)
                 WHERE documents.is_deleted=0 AND documents.revision=0 AND documents.workspace_id=excluded.workspace_id",
                params![
                    document.id.to_string(),
                    document.workspace.to_string(),
                    document.title,
                    language,
                    document.content,
                    document.connection.map(|id| id.to_string()),
                    document.created_at,
                    now,
                    provenance,
                ],
            )?;
            if changed != 1 {
                return Err(StoreError::Corrupted {
                    field: "documents",
                    detail: "document is versioned, deleted, or belongs to another workspace".into(),
                });
            }
            Ok(())
        })
    }

    /// Reads a document back.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn get(&self, id: DocumentId) -> Result<Option<Document>> {
        self.get_cancellable(id, &oxyn_core::CancelToken::new())
    }

    /// Opens a bounded working copy with cancellation on the local connection.
    pub fn get_cancellable(
        &self,
        id: DocumentId,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<Option<Document>> {
        self.store.with_connection_cancellable(cancel, |conn| {
            conn.query_row(
                &format!("{SELECT_COLUMNS} WHERE id = ?1 AND is_deleted=0"),
                params![id.to_string()],
                |row| Ok(from_row(row)),
            )
            .optional()?
            .transpose()
        })
    }

    /// Lists a workspace's documents, most recently modified first.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] or [`crate::StoreError::Corrupted`].
    pub fn list(&self, workspace: WorkspaceId) -> Result<Vec<Document>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(&format!(
                "{SELECT_COLUMNS} WHERE workspace_id = ?1 AND is_deleted=0 ORDER BY updated_at DESC, id"
            ))?;
            query
                .query_and_then(params![workspace.to_string()], from_row)?
                .collect()
        })
    }

    /// Deletes a document. Returns `true` if a row was deleted.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the deletion fails.
    pub fn delete(&self, id: DocumentId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let touched = conn.execute(
                "UPDATE documents SET is_deleted=1,is_open=0,is_saved=0,content='',
                 saved_content=NULL,saved_title=NULL,revision=revision+1,saved_revision=revision+1
                 WHERE id=?1 AND is_deleted=0 AND revision<9223372036854775807",
                params![id.to_string()],
            )?;
            Ok(touched > 0)
        })
    }
}

/// Returns the provenance as it is written in the column.
///
/// The 512-byte budget is checked **before** the write: SQLite's `CHECK` would
/// refuse it anyway, but its message would speak of a constraint rather than
/// of a model name that is too long.
fn provenance_to_column(provenance: Option<&Provenance>) -> Result<Option<String>> {
    provenance
        .map(Provenance::to_json)
        .transpose()
        .map_err(|error| StoreError::Corrupted {
            field: "documents.provenance",
            detail: error.to_string(),
        })
}

/// The column list, shared by every read.
const SELECT_COLUMNS: &str = "SELECT id, workspace_id,
     CASE WHEN length(CAST(title AS BLOB))<=4096 THEN title ELSE NULL END AS title, language,
     CASE WHEN length(CAST(content AS BLOB))<=1048576 THEN content ELSE NULL END AS content,
     connection_id, created_at, updated_at, revision, saved_revision, is_saved, is_open,
     CASE WHEN length(CAST(saved_content AS BLOB))<=1048576 THEN saved_content ELSE NULL END AS saved_content,
     CASE WHEN length(CAST(saved_title AS BLOB))<=4096 THEN saved_title ELSE NULL END AS saved_title,
     provenance FROM documents";

/// Rebuilds a [`Document`] from a row.
fn from_row(row: &Row<'_>) -> Result<Document> {
    let id: String = row.get("id")?;
    let workspace: String = row.get("workspace_id")?;
    let language: String = row.get("language")?;

    let is_saved: bool = row.get("is_saved")?;
    let saved_content: Option<String> = row.get("saved_content")?;
    let saved_title: Option<String> = row.get("saved_title")?;
    if is_saved && (saved_content.is_none() || saved_title.is_none()) {
        return Err(StoreError::Corrupted {
            field: "documents.saved_content",
            detail: "saved copy is missing or exceeds the supported text or title limit".into(),
        });
    }
    Ok(Document {
        id: parse_id(&id, "documents.id")?,
        workspace: parse_id(&workspace, "documents.workspace_id")?,
        title: row
            .get::<_, Option<String>>("title")?
            .ok_or_else(|| StoreError::Corrupted {
                field: "documents.title",
                detail: "legacy title exceeds the 4 KiB read limit".into(),
            })?,
        language: tag_from_json(&language, "documents.language")?,
        content: row
            .get::<_, Option<String>>("content")?
            .ok_or_else(|| StoreError::Corrupted {
                field: "documents.content",
                detail: "query exceeds the 1 MiB editor limit".into(),
            })?,
        connection: parse_id_opt(row.get("connection_id")?, "documents.connection_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        revision: u64::try_from(row.get::<_, i64>("revision")?).map_err(|_| {
            StoreError::Corrupted {
                field: "documents.revision",
                detail: "negative revision".into(),
            }
        })?,
        saved_revision: u64::try_from(row.get::<_, i64>("saved_revision")?).map_err(|_| {
            StoreError::Corrupted {
                field: "documents.saved_revision",
                detail: "negative revision".into(),
            }
        })?,
        is_saved,
        is_open: row.get("is_open")?,
        saved_content,
        saved_title,
        // A `NULL` column is the nominal case: the document was written by the
        // user.
        //
        // An **unreadable** value is dropped loudly, and the document opens
        // anyway. It is the opposite of what happens to the neighboring
        // columns, and the difference is deliberate: an unreadable `title` or
        // `content` would make the document wrong, whereas a lost provenance
        // only makes it less informed. And adding a provider family requires
        // **no migration**: a newer Oxyn can write a provenance this one cannot
        // read, on a schema it accepts, and refusing here would make the
        // document unopenable for metadata that ADR-0023 itself calls a trace,
        // not a seal. It is the trap `deny_unknown_fields` had already set for
        // preferences ([ADR-0013](../../../docs/adr/0013-preferences-workspace.md)).
        //
        // The price is accepted: the mark disappears from a document rewritten
        // by this binary. The log says so; the document stays openable.
        provenance: row.get::<_, Option<String>>("provenance")?.and_then(|raw| {
            match Provenance::from_json(&raw) {
                Ok(provenance) => Some(provenance),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "unreadable provenance dropped; the document opens without its mark"
                    );
                    None
                }
            }
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        AgentId, AgentSessionId, AiProviderKind, ConnectionConfig, DriverId, SqlDialect,
    };

    fn store_with_workspace() -> (Store, WorkspaceId) {
        let store = Store::open_in_memory().expect("open");
        let workspace = store.workspaces().create("workshop").expect("workspace");
        (store, workspace.id)
    }

    #[test]
    fn a_document_round_trips() {
        let (store, workspace) = store_with_workspace();
        let document = Document::new(
            workspace,
            "chiffre d'affaires",
            QueryLanguage::Sql(SqlDialect::DuckDb),
        )
        .with_content("SELECT sum(amount) FROM sales");

        store.documents().save(&document).expect("write");
        let read_back = store
            .documents()
            .get(document.id)
            .expect("read")
            .expect("present");

        assert_eq!(read_back.id, document.id);
        assert_eq!(read_back.workspace, workspace);
        assert_eq!(read_back.title, "chiffre d'affaires");
        assert_eq!(read_back.language, QueryLanguage::Sql(SqlDialect::DuckDb));
        assert_eq!(read_back.content, "SELECT sum(amount) FROM sales");
        assert!(read_back.connection.is_none());
    }

    #[test]
    fn rewriting_keeps_the_creation_date() {
        let (store, workspace) = store_with_workspace();
        let mut document = Document::new(workspace, "brouillon", QueryLanguage::SQL);
        store.documents().save(&document).expect("write");
        let creation = store
            .documents()
            .get(document.id)
            .expect("read")
            .expect("present")
            .created_at;

        document.content = "SELECT 1".to_owned();
        document.created_at = Utc::now(); // even if the caller gets it wrong
        store.documents().save(&document).expect("rewrite");

        let read_back = store
            .documents()
            .get(document.id)
            .expect("read")
            .expect("present");
        assert_eq!(read_back.content, "SELECT 1");
        assert_eq!(
            read_back.created_at.timestamp_millis(),
            creation.timestamp_millis()
        );
        assert!(read_back.updated_at >= read_back.created_at);
    }

    #[test]
    fn deleting_the_connection_does_not_delete_the_document() {
        // Losing a hand-written query because a connection list was cleaned up
        // would be data loss disguised as tidying.
        let (store, workspace) = store_with_workspace();
        let config = ConnectionConfig::new("customer db", DriverId::postgres());
        store
            .connections()
            .save(workspace, &config)
            .expect("connection");

        let document = Document::new(workspace, "monday query", QueryLanguage::SQL)
            .with_content("SELECT * FROM customers")
            .on_connection(config.id);
        store.documents().save(&document).expect("write");

        assert!(store.connections().delete(config.id).expect("deletion"));

        let read_back = store
            .documents()
            .get(document.id)
            .expect("read")
            .expect("the document must survive");
        assert_eq!(read_back.content, "SELECT * FROM customers");
        assert!(read_back.connection.is_none(), "the reference becomes NULL");
    }

    #[test]
    fn deleting_the_workspace_takes_its_documents() {
        let (store, workspace) = store_with_workspace();
        let document = Document::new(workspace, "ephemeral", QueryLanguage::SQL);
        store.documents().save(&document).expect("write");

        assert!(store.workspaces().delete(workspace).expect("deletion"));
        assert!(store.documents().get(document.id).expect("read").is_none());
    }

    #[test]
    fn a_document_without_workspace_is_refused() {
        let store = Store::open_in_memory().expect("open");
        let orphan = Document::new(WorkspaceId::new(), "orphelin", QueryLanguage::SQL);
        assert!(
            store.documents().save(&orphan).is_err(),
            "the foreign key must refuse a nonexistent workspace"
        );
    }

    /// ADR-0023: the provenance dates the origin of the text, not the last
    /// keystroke. The test turns red if a rewrite erases it.
    #[test]
    fn a_provenance_survives_rewriting_the_content() {
        let (store, workspace) = store_with_workspace();
        let origin = Provenance::new(
            AgentId::new(),
            AgentSessionId::new(),
            AiProviderKind::OpenAiCompatible,
            "llama3.2",
        );
        let mut document = Document::new(workspace, "proposed by the agent", QueryLanguage::SQL)
            .with_content("SELECT 1")
            .with_provenance(origin.clone());
        store.documents().save(&document).expect("write");

        // The user takes the text over, then saves through the library's
        // versioned path.
        document.content = "SELECT 2 -- rewritten by hand".to_owned();
        store.documents().save(&document).expect("rewrite");
        store
            .documents()
            .update_query(
                workspace,
                &oxyn_core::QueryDocumentUpdate {
                    document: document.id,
                    title: "proposed by the agent".to_owned(),
                    text: "SELECT 3".to_owned(),
                    language: QueryLanguage::SQL,
                    connection: None,
                    revision: 1,
                    expected_revision: None,
                    save_named: true,
                    is_open: true,
                    // The versioned path carries none: that is precisely what
                    // this test exercises.
                    provenance: None,
                },
                &oxyn_core::CancelToken::new(),
            )
            .expect("versioned save");

        let read_back = store
            .documents()
            .get(document.id)
            .expect("read")
            .expect("present");
        assert_eq!(read_back.content, "SELECT 3");
        assert_eq!(
            read_back.provenance.as_ref(),
            Some(&origin),
            "the provenance dates the origin of the text, not the last keystroke"
        );

        // And a caller that reads back then rewrites without carrying the
        // provenance does not erase it either: the case of a path that
        // ignores it.
        let mut without = read_back;
        without.provenance = None;
        without.revision = 0;
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE documents SET revision=0, saved_revision=0 WHERE id=?1",
                    params![without.id.to_string()],
                )?;
                Ok(())
            })
            .expect("back to an unversioned copy");
        store.documents().save(&without).expect("blind rewrite");
        assert_eq!(
            store
                .documents()
                .get(without.id)
                .expect("read")
                .expect("present")
                .provenance,
            Some(origin)
        );
    }

    #[test]
    fn a_document_written_by_the_user_has_no_provenance() {
        // `NULL` means "written by the user", and it is true.
        let (store, workspace) = store_with_workspace();
        let document = Document::new(workspace, "by hand", QueryLanguage::SQL)
            .with_content("SELECT * FROM sales");
        store.documents().save(&document).expect("write");

        let read_back = store
            .documents()
            .get(document.id)
            .expect("read")
            .expect("present");
        assert_eq!(read_back.provenance, None);
    }

    #[test]
    fn an_unreadable_provenance_neither_prevents_opening_nor_gets_erased() {
        // This test used to say the opposite: an unreadable provenance made
        // the read fail. The security review showed the price of that choice —
        // adding a provider family requires **no migration**, so a newer Oxyn
        // can write, on a schema this one accepts, a provenance it cannot
        // read. The document became unopenable for metadata that ADR-0023
        // calls a trace, not a seal — the trap `deny_unknown_fields` had
        // already set for preferences (ADR-0013).
        //
        // What matters is held another way: **nothing is destroyed**. The
        // value stays in the database, protected by the write's `coalesce`,
        // and a binary that can read it will find it again.
        let (store, workspace) = store_with_workspace();
        let document = Document::new(workspace, "written by a newer Oxyn", QueryLanguage::SQL);
        store.documents().save(&document).expect("write");
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE documents SET provenance='{\"agent\":\"not-a-uuid\"}' WHERE id=?1",
                    params![document.id.to_string()],
                )?;
                Ok(())
            })
            .expect("column written by a version this binary does not know");

        let read_back = store
            .documents()
            .get(document.id)
            .expect("the document opens despite an unreadable mark")
            .expect("present");
        assert!(
            read_back.provenance.is_none(),
            "this binary cannot read it, so it claims nothing"
        );

        // Nor does it overwrite it when rewriting the document.
        store.documents().save(&read_back).expect("rewrite");
        let raw: Option<String> = store
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT provenance FROM documents WHERE id=?1",
                    params![document.id.to_string()],
                    |row| row.get(0),
                )?)
            })
            .expect("raw read back");
        assert_eq!(
            raw.as_deref(),
            Some("{\"agent\":\"not-a-uuid\"}"),
            "the value stays in the database: a version that can read it will find it again"
        );
    }

    #[test]
    fn the_list_is_scoped_to_the_workspace() {
        let store = Store::open_in_memory().expect("open");
        let a = store.workspaces().create("a").expect("workspace");
        let b = store.workspaces().create("b").expect("workspace");

        store
            .documents()
            .save(&Document::new(a.id, "in a", QueryLanguage::SQL))
            .expect("write");
        store
            .documents()
            .save(&Document::new(b.id, "in b", QueryLanguage::SQL))
            .expect("write");

        let in_a = store.documents().list(a.id).expect("list");
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].title, "in a");
    }
}

#[cfg(test)]
mod library_tests;
