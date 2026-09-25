//! One bounded, backend-owned write queue per query document
//! ([ADR-0016](../../../../docs/adr/0016-autosauvegarde-bornee.md)).
//!
//! Drafts coalesce: of the drafts waiting, only the newest is written. A named
//! save and a close are never dropped. Every write carries the revision this
//! queue last saw acknowledged; once the store answers that the document moved,
//! the queue stops and refuses what follows, so a console can never overwrite
//! a copy another console or window wrote. The console keeps its text and
//! offers a new copy.
//!
//! The queue belongs to the backend, not to a view: a write submitted by a
//! console that closes, or a webview that reloads, still reaches the store,
//! and the shutdown waits for it ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).

use std::collections::HashMap;
use std::sync::Arc;

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, DocumentId, OxynError, QueryDocumentUpdate,
    QueryLanguage,
};
use oxyn_exec::Outcome;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use super::recovery::LocalWriteGuard;
use super::{Backend, Inner};
use crate::ipc::IpcError;
use crate::ipc::library::{DocumentChange, DocumentView, DocumentWrite};

const CONFLICT: &str =
    "The stored query changed elsewhere. Save a new query to preserve both versions.";

enum Operation {
    Save(Box<QueryDocumentUpdate>),
    Close { revision: u64, discard: bool },
}

impl Operation {
    fn revision(&self) -> u64 {
        match self {
            Self::Save(update) => update.revision,
            Self::Close { revision, .. } => *revision,
        }
    }
}

type Reply = oneshot::Sender<Result<Outcome, OxynError>>;

struct Pending {
    operation: Operation,
    cancel: CancelToken,
    reply: Reply,
}

struct Queue {
    known_revision: u64,
    last_submitted: u64,
    running: bool,
    closing: bool,
    closed: bool,
    blocked: bool,
    draft: Option<Pending>,
    named: Option<Pending>,
    close: Option<Pending>,
    suspended_draft: Option<Pending>,
}

/// Clones refer to the same queue.
#[derive(Clone)]
pub(crate) struct DocumentWriter {
    document: DocumentId,
    queue: Arc<Mutex<Queue>>,
}

/// The writers of the documents consoles hold, by document.
#[derive(Default)]
pub(crate) struct Writers {
    by_document: Mutex<HashMap<DocumentId, DocumentWriter>>,
}

impl DocumentWriter {
    fn new(document: DocumentId, known_revision: u64) -> Self {
        Self {
            document,
            queue: Arc::new(Mutex::new(Queue {
                known_revision,
                last_submitted: known_revision,
                running: false,
                closing: false,
                closed: false,
                blocked: false,
                draft: None,
                named: None,
                close: None,
                suspended_draft: None,
            })),
        }
    }

    fn enqueue(
        &self,
        inner: &Arc<Inner>,
        operation: Operation,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<Outcome, OxynError>> {
        let (reply, receiver) = oneshot::channel();
        let mut queue = self.queue.lock();
        if queue.blocked || queue.closed || queue.closing {
            let _ = reply.send(Err(OxynError::Serialization(
                "document writer is closed or conflicted; preserve a new copy".into(),
            )));
            return receiver;
        }
        if operation.revision() <= queue.last_submitted {
            let _ = reply.send(Err(OxynError::Config(
                "document revision did not advance".into(),
            )));
            return receiver;
        }
        queue.last_submitted = operation.revision();
        let pending = Pending {
            operation,
            cancel,
            reply,
        };
        match &pending.operation {
            Operation::Save(update) if update.document != self.document => {
                let _ = pending.reply.send(Err(OxynError::Config(
                    "document does not belong to this writer".into(),
                )));
                return receiver;
            }
            Operation::Save(update) if !update.save_named => {
                if let Some(old) = queue.draft.replace(pending) {
                    let _ = old.reply.send(Err(OxynError::Cancelled));
                }
            }
            Operation::Save(_) => {
                if queue.named.is_some() {
                    let _ = pending.reply.send(Err(OxynError::Config(
                        "a named save is already queued".into(),
                    )));
                    return receiver;
                }
                queue.named = Some(pending);
            }
            Operation::Close { .. } => {
                queue.closing = true;
                queue.suspended_draft = queue.draft.take();
                queue.close = Some(pending);
            }
        }
        if !queue.running {
            queue.running = true;
            let guard = inner.workbench.local.guard();
            let inner = Arc::clone(inner);
            let state = Arc::clone(&self.queue);
            let document = self.document;
            tauri::async_runtime::spawn(async move {
                drain(inner, state, document, guard).await;
            });
        }
        receiver
    }
}

/// Writes queued operations in revision order until none is left.
async fn drain(
    inner: Arc<Inner>,
    state: Arc<Mutex<Queue>>,
    document: DocumentId,
    _guard: LocalWriteGuard,
) {
    loop {
        let next = {
            let mut queue = state.lock();
            let kind = [
                queue.draft.as_ref(),
                queue.named.as_ref(),
                queue.close.as_ref(),
            ]
            .iter()
            .enumerate()
            .filter_map(|(index, pending)| pending.map(|p| (index, p.operation.revision())))
            .min_by_key(|(_, revision)| *revision)
            .map(|(kind, _)| kind);
            let next = match kind {
                Some(0) => queue.draft.take(),
                Some(1) => queue.named.take(),
                Some(2) => queue.close.take(),
                _ => None,
            };
            match next {
                Some(next) => Some((next, queue.known_revision)),
                None => {
                    queue.running = false;
                    None
                }
            }
        };
        let Some((pending, expected)) = next else {
            break;
        };
        let revision = pending.operation.revision();
        let is_close = matches!(pending.operation, Operation::Close { .. });
        let workspace = inner.executor.workspace();
        let command = match pending.operation {
            Operation::Save(mut update) => {
                update.expected_revision = Some(expected);
                Command::SaveQueryDocument { workspace, update }
            }
            Operation::Close { revision, discard } => Command::CloseQueryDocument {
                workspace,
                document,
                revision,
                expected_revision: Some(expected),
                discard,
            },
        };
        let result = if pending.cancel.is_cancelled() {
            Err(OxynError::Cancelled)
        } else {
            inner
                .executor
                .dispatch_as(CommandId::new(), Actor::Human, command, &pending.cancel)
                .await
        };
        let result = match result {
            Ok(Outcome::NeedsApproval { command, .. }) => {
                inner.executor.reject(command);
                Err(OxynError::PolicyDenied {
                    reason: "document persistence was not allowed by the policy".into(),
                })
            }
            Ok(Outcome::Denied { reason, .. }) => Err(OxynError::PolicyDenied { reason }),
            Ok(Outcome::DocumentOpened { document: saved })
                if !is_close && saved.id == document && saved.revision == revision =>
            {
                Ok(Outcome::DocumentOpened { document: saved })
            }
            Ok(Outcome::DocumentClosed { document: closed }) if is_close && closed == document => {
                Ok(Outcome::DocumentClosed { document: closed })
            }
            Ok(_) => Err(OxynError::Serialization(
                "document changed while being persisted".into(),
            )),
            error => error,
        };
        {
            let mut queue = state.lock();
            if result.is_ok() {
                queue.known_revision = revision;
            }
            if matches!(&result, Err(OxynError::Serialization(_))) {
                queue.blocked = true;
            }
            if is_close {
                queue.closing = false;
                queue.closed = result.is_ok();
                if !queue.closed && !queue.blocked {
                    queue.draft = queue.suspended_draft.take();
                }
            }
            if queue.closed || queue.blocked {
                let stopped = [
                    queue.draft.take(),
                    queue.named.take(),
                    queue.close.take(),
                    queue.suspended_draft.take(),
                ];
                for pending in stopped.into_iter().flatten() {
                    let error = if queue.blocked {
                        OxynError::Serialization(
                            "document changed elsewhere; pending writes were stopped".into(),
                        )
                    } else {
                        OxynError::Cancelled
                    };
                    let _ = pending.reply.send(Err(error));
                }
            }
        }
        let _ = pending.reply.send(result);
    }
}

/// The store refuses a revision that moved as unreadable local state, which
/// reaches the bus as a serialization error; the queue reports the same.
fn is_conflict(error: &OxynError) -> bool {
    matches!(error, OxynError::Serialization(_))
}

impl Backend {
    fn writer(&self, document: DocumentId, known_revision: u64) -> DocumentWriter {
        self.inner
            .workbench
            .writers
            .by_document
            .lock()
            .entry(document)
            .or_insert_with(|| DocumentWriter::new(document, known_revision))
            .clone()
    }

    /// Reads a document in full. A document no console writes yet gets a
    /// writer based on the revision read here.
    pub async fn open_query_document(
        &self,
        document: DocumentId,
    ) -> Result<DocumentView, IpcError> {
        let outcome = self
            .inner
            .executor
            .dispatch_as(
                CommandId::new(),
                Actor::Human,
                Command::OpenDocument {
                    workspace: self.inner.executor.workspace(),
                    document,
                },
                &CancelToken::new(),
            )
            .await?;
        let Outcome::DocumentOpened { document: loaded } = outcome else {
            return Err(IpcError::invalid("Unexpected response to opening a query"));
        };
        // A writer a console still holds keeps its base: adopting the stored
        // revision here would accept whatever another window wrote meanwhile.
        self.writer(document, loaded.revision.max(loaded.saved_revision));
        Ok(DocumentView::from(*loaded))
    }

    /// Submits a draft or a named save. Returns once the store answered.
    ///
    /// `id` reaches the write through [`Backend::cancel`]: a save cancelled
    /// before the store committed it answers `Superseded` and writes nothing.
    pub async fn save_query_document(
        &self,
        id: CommandId,
        document: DocumentId,
        connection: Option<oxyn_core::ConnectionId>,
        change: DocumentChange,
    ) -> Result<DocumentWrite, IpcError> {
        let tracked = self.track(id)?;
        self.save_under(document, connection, change, CancelToken::clone(&tracked))
            .await
    }

    async fn save_under(
        &self,
        document: DocumentId,
        connection: Option<oxyn_core::ConnectionId>,
        change: DocumentChange,
        cancel: CancelToken,
    ) -> Result<DocumentWrite, IpcError> {
        let language = match connection {
            Some(connection) => QueryLanguage::Sql(oxyn_query::dialect_for(
                &self.read_config(connection).await?.driver,
            )),
            None => QueryLanguage::SQL,
        };
        let update = QueryDocumentUpdate {
            document,
            revision: change.revision,
            expected_revision: None,
            title: change.title,
            language,
            text: change.text,
            connection,
            save_named: change.named,
            is_open: true,
            provenance: None,
        };
        // Refused here without echoing the text: the store would refuse too,
        // after a queue slot and a round trip.
        update.validate()?;
        let named = update
            .save_named
            .then(|| (update.text.clone(), update.title.clone()));
        let writer = self.writer(document, 0);
        let receiver = writer.enqueue(&self.inner, Operation::Save(Box::new(update)), cancel);
        let result = receiver
            .await
            .unwrap_or_else(|_| Err(OxynError::Internal("the document writer stopped".into())));
        match result {
            Ok(Outcome::DocumentOpened { document: saved }) => {
                // A named save is acknowledged only by exactly what was sent:
                // anything else means another writer's copy is the saved one.
                if let Some((text, title)) = named
                    && (saved.saved_revision != change.revision
                        || saved.saved_content.as_deref() != Some(text.as_str())
                        || saved.saved_title.as_deref() != Some(title.as_str()))
                {
                    return Ok(DocumentWrite::Conflict {
                        message: CONFLICT.into(),
                    });
                }
                Ok(DocumentWrite::Saved {
                    revision: saved.revision,
                    saved_revision: saved.saved_revision,
                    is_saved: saved.is_saved,
                })
            }
            Ok(_) => Err(IpcError::invalid("Unexpected response to a query save")),
            Err(OxynError::Cancelled) => Ok(DocumentWrite::Superseded),
            Err(error) if is_conflict(&error) => Ok(DocumentWrite::Conflict {
                message: CONFLICT.into(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    /// Closes a working copy. `discard` drops it; otherwise the named copy stays
    /// in the library and the working copy is no longer offered for recovery.
    ///
    /// `id` reaches the close through [`Backend::cancel`]: cancelled before the
    /// store committed it, the close answers `Superseded`, the document stays
    /// writable and the draft it held back is written again.
    pub async fn close_query_document(
        &self,
        id: CommandId,
        document: DocumentId,
        revision: u64,
        discard: bool,
    ) -> Result<DocumentWrite, IpcError> {
        let tracked = self.track(id)?;
        self.close_under(document, revision, discard, CancelToken::clone(&tracked))
            .await
    }

    async fn close_under(
        &self,
        document: DocumentId,
        revision: u64,
        discard: bool,
        cancel: CancelToken,
    ) -> Result<DocumentWrite, IpcError> {
        let writer = self.writer(document, 0);
        let receiver = writer.enqueue(&self.inner, Operation::Close { revision, discard }, cancel);
        let result = receiver
            .await
            .unwrap_or_else(|_| Err(OxynError::Internal("the document writer stopped".into())));
        match result {
            Ok(Outcome::DocumentClosed { .. }) => {
                self.inner
                    .workbench
                    .writers
                    .by_document
                    .lock()
                    .remove(&document);
                Ok(DocumentWrite::Closed)
            }
            Ok(_) => Err(IpcError::invalid("Unexpected response to closing a query")),
            Err(OxynError::Cancelled) => Ok(DocumentWrite::Superseded),
            Err(error) if is_conflict(&error) => Ok(DocumentWrite::Conflict {
                message: CONFLICT.into(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    /// Forgets the writer of a document whose console left it in conflict.
    ///
    /// The stored record is untouched; the console now writes a new document.
    pub fn release_query_document(&self, document: DocumentId) {
        self.inner
            .workbench
            .writers
            .by_document
            .lock()
            .remove(&document);
    }

    /// Deletes a saved query locally. Never a database object.
    pub async fn delete_query_document(
        &self,
        document: DocumentId,
        revision: u64,
    ) -> Result<(), IpcError> {
        let command = Command::DeleteQueryDocument {
            workspace: self.inner.executor.workspace(),
            document,
            revision,
        };
        let _local = self.local_write(&command);
        match self
            .inner
            .executor
            .dispatch_as(CommandId::new(), Actor::Human, command, &CancelToken::new())
            .await?
        {
            Outcome::DocumentClosed { .. } => {
                self.release_query_document(document);
                Ok(())
            }
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("Unexpected response to deleting a query")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts")
    }

    fn change(document: DocumentId, revision: u64, text: &str, named: bool) -> DocumentChange {
        DocumentChange {
            document: document.to_string(),
            revision,
            title: "console_1.sql".into(),
            text: text.into(),
            connection: None,
            named,
        }
    }

    /// A saved connection the documents can be written under; nothing opens.
    fn saved_connection(backend: &Backend, name: &str) -> oxyn_core::ConnectionId {
        let config = oxyn_core::ConnectionConfig::new(name, oxyn_core::DriverId::sqlite());
        backend
            .inner
            .executor
            .store()
            .connections()
            .save(backend.inner.executor.workspace(), &config)
            .expect("the connection is saved");
        config.id
    }

    fn stored(backend: &Backend, document: DocumentId) -> oxyn_store::Document {
        backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("the document is stored")
    }

    /// **An offline console writes without a session, and attaching it keeps
    /// the document** (UX-SPEC § Restauration sélective au démarrage). The
    /// restored copy keeps writing under the connection it was written for,
    /// so attaching it there resumes the same document through the same
    /// write queue: the next revision follows, with no conflict and no copy.
    #[test]
    fn an_offline_copy_attached_to_its_connection_keeps_its_identity() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let origin = saved_connection(&backend, "billing");
        let document = DocumentId::new();
        let under = |revision, text: &str| DocumentChange {
            connection: Some(origin.to_string()),
            ..change(document, revision, text, false)
        };

        // Written by the previous launch, then restored offline: no session
        // is open on `origin`, and the draft still reaches the store.
        let offline = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                Some(origin),
                under(1, "SELECT 1"),
            ))
            .expect("offline draft");
        assert!(matches!(offline, DocumentWrite::Saved { revision: 1, .. }));
        let named = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                Some(origin),
                DocumentChange {
                    named: true,
                    ..under(2, "SELECT 12")
                },
            ))
            .expect("offline named save");
        assert!(matches!(named, DocumentWrite::Saved { revision: 2, .. }));

        // Attached: the same console goes on with the next revision.
        let attached = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                Some(origin),
                under(3, "SELECT 123"),
            ))
            .expect("draft after attaching");
        assert!(matches!(attached, DocumentWrite::Saved { revision: 3, .. }));
        runtime.block_on(backend.wait_for_local_writes());

        let kept = stored(&backend, document);
        assert_eq!(kept.connection, Some(origin), "the origin is never dropped");
        assert_eq!(kept.content, "SELECT 123");
        assert_eq!(kept.saved_content.as_deref(), Some("SELECT 12"));
        assert!(kept.is_open);
    }

    /// **Attached to another connection, the copy is a new document** and the
    /// original stays as it was, still offered for recovery.
    #[test]
    fn a_copy_on_another_connection_leaves_the_original_untouched() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let origin = saved_connection(&backend, "billing");
        let elsewhere = saved_connection(&backend, "analytics");
        let original = DocumentId::new();
        runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                original,
                Some(origin),
                DocumentChange {
                    connection: Some(origin.to_string()),
                    ..change(original, 1, "SELECT 1", false)
                },
            ))
            .expect("offline draft");

        let copy = DocumentId::new();
        runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                copy,
                Some(elsewhere),
                DocumentChange {
                    connection: Some(elsewhere.to_string()),
                    ..change(copy, 1, "SELECT 1 -- edited on analytics", false)
                },
            ))
            .expect("the copy's first draft");
        runtime.block_on(backend.wait_for_local_writes());

        let kept = stored(&backend, original);
        assert_eq!(kept.connection, Some(origin));
        assert_eq!(kept.content, "SELECT 1");
        assert_eq!(kept.revision, 1);
        assert!(kept.is_open, "the original is still offered for recovery");
        assert_eq!(stored(&backend, copy).connection, Some(elsewhere));
    }

    #[test]
    fn a_second_writer_on_a_moved_document_gets_a_conflict_and_the_store_keeps_the_first() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let document = DocumentId::new();

        let first = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 1, "SELECT 1", false),
            ))
            .expect("first draft");
        assert!(matches!(first, DocumentWrite::Saved { revision: 1, .. }));

        // Another window wrote revision 2 under this console's feet.
        let workspace = backend.inner.executor.workspace();
        backend
            .inner
            .executor
            .store()
            .documents()
            .update_query(
                workspace,
                &QueryDocumentUpdate {
                    document,
                    revision: 2,
                    expected_revision: Some(1),
                    title: "elsewhere".into(),
                    language: QueryLanguage::SQL,
                    text: "SELECT 'elsewhere'".into(),
                    connection: None,
                    save_named: false,
                    is_open: true,
                    provenance: None,
                },
                &CancelToken::new(),
            )
            .expect("concurrent write");

        let conflicted = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 3, "SELECT 'mine'", true),
            ))
            .expect("answered");
        assert!(
            matches!(conflicted, DocumentWrite::Conflict { .. }),
            "got {conflicted:?}"
        );
        let stored = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("present");
        assert_eq!(stored.content, "SELECT 'elsewhere'");

        // The console preserves its text as a new document.
        let copy = DocumentId::new();
        let saved = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                copy,
                None,
                change(copy, 1, "SELECT 'mine'", true),
            ))
            .expect("copy");
        assert!(matches!(saved, DocumentWrite::Saved { is_saved: true, .. }));
    }

    #[test]
    fn a_cancelled_named_save_writes_nothing_and_leaves_the_queue_open() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let document = DocumentId::new();
        runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 1, "SELECT 1", false),
            ))
            .expect("draft");

        // The token the front reaches by its command id is the one the write
        // runs under.
        let id = CommandId::new();
        let tracked = backend.track(id).expect("a fresh id is tracked");
        assert!(backend.cancel(id), "a tracked save is reachable");
        let cancelled = runtime
            .block_on(backend.save_under(
                document,
                None,
                change(document, 2, "SELECT 'named'", true),
                CancelToken::clone(&tracked),
            ))
            .expect("answered");
        assert!(
            matches!(cancelled, DocumentWrite::Superseded),
            "got {cancelled:?}"
        );
        let stored = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("present");
        assert!(!stored.is_saved, "a cancelled save leaves no named copy");
        assert_eq!(stored.content, "SELECT 1");

        let next = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 3, "SELECT 3", true),
            ))
            .expect("answered");
        assert!(
            matches!(next, DocumentWrite::Saved { is_saved: true, .. }),
            "got {next:?}"
        );
    }

    #[test]
    fn a_cancelled_close_keeps_the_document_writable() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let document = DocumentId::new();
        runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 1, "SELECT 1", true),
            ))
            .expect("named save");

        let id = CommandId::new();
        let tracked = backend.track(id).expect("a fresh id is tracked");
        assert!(backend.cancel(id), "a tracked close is reachable");
        let cancelled = runtime
            .block_on(backend.close_under(document, 2, false, CancelToken::clone(&tracked)))
            .expect("answered");
        assert!(
            matches!(cancelled, DocumentWrite::Superseded),
            "got {cancelled:?}"
        );
        let stored = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("present");
        assert!(stored.is_open, "the working copy is still offered");

        let draft = runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 3, "SELECT 3", false),
            ))
            .expect("answered");
        assert!(
            matches!(draft, DocumentWrite::Saved { revision: 3, .. }),
            "got {draft:?}"
        );
    }

    #[test]
    fn a_finished_write_is_no_longer_cancellable() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let document = DocumentId::new();
        let id = CommandId::new();
        runtime
            .block_on(backend.save_query_document(
                id,
                document,
                None,
                change(document, 1, "SELECT 1", true),
            ))
            .expect("named save");
        assert!(!backend.cancel(id), "the token left with the write");
    }

    #[test]
    fn a_revision_that_does_not_advance_is_refused() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let document = DocumentId::new();
        runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 2, "SELECT 2", false),
            ))
            .expect("draft");
        assert!(
            runtime
                .block_on(backend.save_query_document(
                    CommandId::new(),
                    document,
                    None,
                    change(document, 2, "SELECT 2", false)
                ))
                .is_err()
        );
    }

    #[test]
    fn closing_a_draft_without_named_copy_discards_it_and_waits_for_nothing_left() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let document = DocumentId::new();
        runtime
            .block_on(backend.save_query_document(
                CommandId::new(),
                document,
                None,
                change(document, 1, "SELECT 1", false),
            ))
            .expect("draft");
        let closed = runtime
            .block_on(backend.close_query_document(CommandId::new(), document, 2, true))
            .expect("closed");
        assert!(matches!(closed, DocumentWrite::Closed));
        runtime.block_on(backend.wait_for_local_writes());
        let stored = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read");
        // Discarded: no longer offered for recovery, and no named copy left.
        assert!(stored.is_none_or(|stored| !stored.is_open && !stored.is_saved));
        let late = runtime.block_on(backend.save_query_document(
            CommandId::new(),
            document,
            None,
            change(document, 3, "SELECT 'late'", false),
        ));
        assert!(
            !matches!(late, Ok(DocumentWrite::Saved { .. })),
            "a closed document refuses a late draft, got {late:?}"
        );
    }
}
