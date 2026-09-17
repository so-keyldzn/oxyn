//! One bounded, backend-owned persistence queue per editable document.

use super::*;
use oxyn_core::{DocumentId, QueryDocumentUpdate};
use parking_lot::Mutex;

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
struct Pending {
    operation: Operation,
    cancel: CancelToken,
    reply: oneshot::Sender<Result<Outcome, OxynError>>,
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

/// Clones refer to the same queue; its task retains no GPUI entity or runtime owner.
#[derive(Clone)]
pub(crate) struct DocumentWriter {
    backend: Backend,
    document: DocumentId,
    queue: Arc<Mutex<Queue>>,
}
impl Backend {
    pub(crate) fn document_writer(
        &self,
        document: DocumentId,
        known_revision: u64,
    ) -> DocumentWriter {
        DocumentWriter {
            backend: self.clone(),
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
}
impl DocumentWriter {
    pub(crate) fn save(
        &self,
        update: QueryDocumentUpdate,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<Outcome, OxynError>> {
        self.enqueue(Operation::Save(Box::new(update)), cancel)
    }
    pub(crate) fn close(
        &self,
        revision: u64,
        discard: bool,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<Outcome, OxynError>> {
        self.enqueue(Operation::Close { revision, discard }, cancel)
    }
    fn enqueue(
        &self,
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
            let _ = reply.send(Err(OxynError::Serialization(
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
            let inner = self.backend.inner.clone();
            inner
                .pending_local_writes
                .count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let guard = LocalWriteGuard(inner.pending_local_writes.clone());
            let state = self.queue.clone();
            let document = self.document;
            self.backend.runtime.spawn(async move {
                Self::drain(inner, state, document, guard).await;
            });
        }
        receiver
    }

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
                .filter_map(|(index, pending)| {
                    pending.map(|pending| (index, pending.operation.revision()))
                })
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
            let command = match pending.operation {
                Operation::Save(mut update) => {
                    update.expected_revision = Some(expected);
                    Command::SaveQueryDocument {
                        workspace: inner.executor.workspace(),
                        update,
                    }
                }
                Operation::Close { revision, discard } => Command::CloseQueryDocument {
                    workspace: inner.executor.workspace(),
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
                    .dispatch(Actor::Human, command, &pending.cancel)
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
                Ok(Outcome::DocumentClosed { document: closed })
                    if is_close && closed == document =>
                {
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
                    for pending in [
                        &mut queue.draft.take(),
                        &mut queue.named.take(),
                        &mut queue.close.take(),
                        &mut queue.suspended_draft.take(),
                    ] {
                        if let Some(pending) = pending.take() {
                            let error = if queue.blocked {
                                OxynError::Serialization(
                                    "document changed elsewhere; pending writes were stopped"
                                        .into(),
                                )
                            } else {
                                OxynError::Cancelled
                            };
                            let _ = pending.reply.send(Err(error));
                        }
                    }
                }
            }
            let _ = pending.reply.send(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::QueryLanguage;

    fn update(document: DocumentId, revision: u64, named: bool) -> QueryDocumentUpdate {
        QueryDocumentUpdate {
            document,
            revision,
            expected_revision: None,
            title: "Draft.sql".into(),
            language: QueryLanguage::SQL,
            text: format!("SELECT {revision}"),
            connection: None,
            save_named: named,
            is_open: true,
            provenance: None,
        }
    }

    #[test]
    fn coalesced_drafts_and_named_save_survive_dropped_receivers() {
        let mut backend = Backend::open_temporary().expect("backend");
        backend.runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("controlled runtime"),
        );
        let document = DocumentId::new();
        let writer = backend.document_writer(document, 0);
        for revision in 1..=200 {
            drop(writer.save(
                update(document, revision, revision == 50),
                CancelToken::new(),
            ));
            let state = writer.queue.lock();
            assert!(
                state
                    .draft
                    .as_ref()
                    .is_none_or(|pending| pending.operation.revision() <= revision)
            );
        }
        {
            let state = writer.queue.lock();
            assert_eq!(
                state
                    .draft
                    .as_ref()
                    .expect("latest draft")
                    .operation
                    .revision(),
                200
            );
            assert_eq!(
                state
                    .named
                    .as_ref()
                    .expect("explicit save")
                    .operation
                    .revision(),
                50
            );
        }
        drop(writer);
        backend.runtime.block_on(backend.wait_for_local_writes());
        let doc = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("draft");
        assert_eq!(doc.content, "SELECT 200");
        assert_eq!(doc.saved_content.as_deref(), Some("SELECT 50"));
        assert_eq!(doc.revision, 200);
    }

    #[test]
    fn two_writers_cannot_overwrite_each_others_revision() {
        let backend = Backend::open_temporary().expect("backend");
        let document = DocumentId::new();
        let first = backend.document_writer(document, 0);
        let second = backend.document_writer(document, 0);
        first
            .save(update(document, 1, false), CancelToken::new())
            .blocking_recv()
            .expect("reply")
            .expect("first draft");
        assert!(
            second
                .save(update(document, 5, false), CancelToken::new())
                .blocking_recv()
                .expect("reply")
                .is_err()
        );
        assert!(
            second
                .save(update(document, 6, false), CancelToken::new())
                .blocking_recv()
                .expect("reply")
                .is_err()
        );
        let doc = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("draft");
        assert_eq!(doc.content, "SELECT 1");
    }

    #[test]
    fn closing_before_first_save_keeps_a_tombstone_and_stops_late_writes() {
        let backend = Backend::open_temporary().expect("backend");
        let document = DocumentId::new();
        let writer = backend.document_writer(document, 0);
        writer
            .close(2, true, CancelToken::new())
            .blocking_recv()
            .expect("reply")
            .expect("close");
        assert!(
            writer
                .save(update(document, 3, false), CancelToken::new())
                .blocking_recv()
                .expect("reply")
                .is_err()
        );
        assert!(
            backend
                .inner
                .executor
                .store()
                .documents()
                .get(document)
                .expect("read")
                .is_none()
        );
        assert!(
            backend
                .inner
                .executor
                .store()
                .documents()
                .update_query(
                    backend.workspace_id(),
                    &update(document, 1, false),
                    &CancelToken::new()
                )
                .is_err()
        );
    }
    #[test]
    fn cancelling_a_queued_close_restores_the_last_draft_without_a_view() {
        let mut backend = Backend::open_temporary().expect("backend");
        backend.runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("controlled runtime"),
        );
        let document = DocumentId::new();
        let writer = backend.document_writer(document, 0);
        let draft = writer.save(update(document, 1, false), CancelToken::new());
        let cancel = CancelToken::new();
        let close = writer.close(2, true, cancel.clone());
        cancel.cancel();
        drop(writer);
        backend.runtime.block_on(backend.wait_for_local_writes());
        assert!(matches!(
            close.blocking_recv().expect("close reply"),
            Err(OxynError::Cancelled)
        ));
        draft
            .blocking_recv()
            .expect("draft reply")
            .expect("restored draft");
        let doc = backend
            .inner
            .executor
            .store()
            .documents()
            .get(document)
            .expect("read")
            .expect("draft");
        assert!(doc.is_open);
        assert_eq!(doc.content, "SELECT 1");
    }
}
