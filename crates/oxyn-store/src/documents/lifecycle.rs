//! Working and named copies have separate revisions so async saves cannot erase drafts.

use super::*;
use oxyn_core::QueryDocumentUpdate;

fn invalid(detail: &str) -> StoreError {
    StoreError::Corrupted {
        field: "documents",
        detail: detail.into(),
    }
}

/// An optional compare-and-swap check paired with the new revision.
#[derive(Debug, Clone, Copy)]
pub struct DocumentRevision {
    /// New monotonic revision.
    pub next: u64,
    /// Last revision observed by this writer.
    pub expected: Option<u64>,
}

impl Documents<'_> {
    /// Applies a versioned working update and optionally a named save atomically.
    pub fn update_query(
        &self,
        workspace: WorkspaceId,
        update: &QueryDocumentUpdate,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<Document> {
        update
            .validate()
            .map_err(|error| invalid(&error.to_string()))?;
        let revision =
            i64::try_from(update.revision).map_err(|_| invalid("revision out of range"))?;
        let language = tag_to_json(&update.language)?;
        self.store.with_connection_cancellable(cancel, |connection| {
            let transaction = connection.unchecked_transaction()?;
            if let Some(connection_id) = update.connection {
                let belongs: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND workspace_id=?2)", params![connection_id.to_string(), workspace.to_string()], |row| row.get(0))?;
                if !belongs { return Err(invalid("document connection does not belong to the workspace")); }
            }
            let prior: Option<(String, bool)> = transaction.query_row("SELECT workspace_id, is_deleted FROM documents WHERE id=?1", [update.document.to_string()], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
            if let Some((owner, deleted)) = prior {
                if owner != workspace.to_string() { return Err(invalid("document belongs to another workspace")); }
                if deleted { return Err(invalid("document was deleted; save a new copy")); }
                let current = transaction.query_row(&format!("{SELECT_COLONNES} WHERE id=?1"), [update.document.to_string()], |row| Ok(depuis_ligne(row)))??;
                if update.expected_revision.is_some_and(|expected| expected != current.revision) { return Err(invalid("document revision changed; preserve a new copy")); }
                if current.connection != update.connection || current.language != update.language { return Err(invalid("document connection or language changed; save a new copy")); }
                if current.revision == update.revision && (current.content != update.text || current.title != update.title || current.is_open != update.is_open) {
                    return Err(invalid("document revision conflict; reopen before saving"));
                }
                if update.save_named && current.is_saved && current.saved_revision == update.revision
                    && (current.saved_content.as_deref() != Some(&update.text) || current.saved_title.as_deref() != Some(&update.title)) {
                    return Err(invalid("saved revision conflict; reopen before saving"));
                }
                if update.revision > current.revision {
                    transaction.execute("UPDATE documents SET title=?2, content=?3, revision=?4, is_open=?5, updated_at=?6 WHERE id=?1", params![update.document.to_string(), update.title, update.text, revision, update.is_open, Utc::now()])?;
                }
                if update.save_named && update.revision > current.saved_revision {
                    transaction.execute("UPDATE documents SET saved_content=?2, saved_title=?3, saved_revision=?4, is_saved=1, updated_at=?5 WHERE id=?1", params![update.document.to_string(), update.text, update.title, revision, Utc::now()])?;
                }
            } else {
                if update.expected_revision.is_some_and(|expected| expected != 0) { return Err(invalid("document disappeared; preserve a new copy")); }
                transaction.execute("INSERT INTO documents (id,workspace_id,title,language,content,connection_id,created_at,updated_at,revision,saved_revision,is_saved,is_open,saved_content,saved_title)
                    VALUES (?1,?2,?3,?4,?5,?6,?7,?7,?8,?9,?10,?11,?12,?13)", params![update.document.to_string(), workspace.to_string(), update.title, language, update.text, update.connection.map(|id| id.to_string()), Utc::now(), revision, if update.save_named { revision } else { 0 }, update.save_named, update.is_open, update.save_named.then_some(&update.text), update.save_named.then_some(&update.title)])?;
            }
            let document = transaction.query_row(&format!("{SELECT_COLONNES} WHERE id=?1"), [update.document.to_string()], |row| Ok(depuis_ligne(row)))??;
            transaction.commit()?;
            Ok(document)
        })
    }

    /// Closes or deletes a document, retaining a barrier against late draft/save replies.
    /// Without discard, unsaved changes are refused. Deletion always clears both texts.
    pub fn close_query(
        &self,
        workspace: WorkspaceId,
        document: DocumentId,
        revision: u64,
        discard: bool,
        delete: bool,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<()> {
        self.close_query_checked(
            workspace,
            document,
            DocumentRevision {
                next: revision,
                expected: None,
            },
            discard,
            delete,
            cancel,
        )
    }

    /// Closes only the revision observed by the writer, without erasing a concurrent draft.
    pub fn close_query_checked(
        &self,
        workspace: WorkspaceId,
        document: DocumentId,
        revision: DocumentRevision,
        discard: bool,
        delete: bool,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<()> {
        let expected = revision.expected;
        let revision = revision.next;
        let revision = i64::try_from(revision).map_err(|_| invalid("revision out of range"))?;
        if revision <= 0 {
            return Err(invalid("revision must be positive"));
        }
        self.store.with_connection_cancellable(cancel, |connection| {
            let transaction = connection.unchecked_transaction()?;
            let current = transaction.query_row(&format!("{SELECT_COLONNES} WHERE id=?1 AND workspace_id=?2 AND is_deleted=0"), params![document.to_string(), workspace.to_string()], |row| Ok(depuis_ligne(row))).optional()?.transpose()?;
            let Some(current) = current else {
                let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM documents WHERE id=?1)", [document.to_string()], |row| row.get(0))?;
                if exists || expected.is_some_and(|revision| revision != 0) || (!discard && !delete) { return Err(invalid("document is not in the expected state")); }
                transaction.execute("INSERT INTO documents(id,workspace_id,title,language,content,created_at,updated_at,revision,saved_revision,is_saved,is_open,is_deleted) VALUES(?1,?2,'Closed query',?3,'',?4,?4,?5,?5,0,0,1)", params![document.to_string(), workspace.to_string(), tag_to_json(&QueryLanguage::SQL)?, Utc::now(), revision])?;
                transaction.commit()?;
                return Ok(());
            };
            if expected.is_some_and(|expected| expected != current.revision) { return Err(invalid("document revision changed while closing")); }
            if u64::try_from(revision).unwrap_or(0) <= current.revision { return Err(invalid("document changed while closing")); }
            let dirty = current.saved_content.as_deref().unwrap_or("") != current.content || current.saved_title.as_deref().unwrap_or(&current.title) != current.title;
            if dirty && !discard && !delete { return Err(invalid("save or discard the working changes before closing")); }
            let content = if delete { "" } else { current.saved_content.as_deref().unwrap_or("") };
            let title = current.saved_title.as_deref().unwrap_or(&current.title);
            transaction.execute("UPDATE documents SET content=?2,title=?3,is_open=0,is_deleted=?4,revision=?5,saved_revision=?5,
                saved_content=CASE WHEN ?4 THEN NULL ELSE saved_content END,
                saved_title=CASE WHEN ?4 THEN NULL ELSE saved_title END,
                is_saved=CASE WHEN ?4 THEN 0 ELSE is_saved END, updated_at=?6 WHERE id=?1",
                params![document.to_string(), content, title, delete, revision, Utc::now()])?;
            transaction.commit()?;
            Ok(())
        })
    }
}
