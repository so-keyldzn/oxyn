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
/// the document** (UX-SPEC § Selective restore at startup). The
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
