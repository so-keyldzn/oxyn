//! Revision ordering, lifecycle barriers and bounded library reads.

use super::*;
use oxyn_core::{CancelToken, DocumentFilter, MAX_QUERY_DOCUMENT_BYTES, QueryDocumentUpdate};

fn setup() -> (Store, WorkspaceId, QueryDocumentUpdate) {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("library").expect("workspace").id;
    let update = QueryDocumentUpdate {
        expected_revision: None,
        document: DocumentId::new(),
        revision: 1,
        title: "Saved query".into(),
        language: QueryLanguage::SQL,
        text: "SELECT 1".into(),
        connection: None,
        save_named: true,
        is_open: true,
    };
    (store, workspace, update)
}

#[test]
fn delayed_named_save_preserves_the_newer_working_copy() {
    let (store, workspace, mut update) = setup();
    let cancel = CancelToken::new();
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("initial save");
    update.revision = 3;
    update.save_named = false;
    update.text = "SELECT 3".into();
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("newer draft");
    update.revision = 2;
    update.save_named = true;
    update.text = "SELECT 2".into();
    let saved = store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("delayed save");
    assert_eq!(saved.content, "SELECT 3");
    assert_eq!(saved.revision, 3);
    assert_eq!(saved.saved_content.as_deref(), Some("SELECT 2"));
    assert_eq!(saved.saved_revision, 2);
    update.text = "SELECT 22".into();
    assert!(
        store
            .documents()
            .update_query(workspace, &update, &cancel)
            .is_err()
    );
    let page = store
        .documents()
        .page(workspace, &DocumentFilter::default(), &cancel)
        .expect("summaries");
    assert!(page.entries[0].has_changes);
}

#[test]
fn closing_requires_a_choice_and_blocks_late_saves() {
    let (store, workspace, mut update) = setup();
    let cancel = CancelToken::new();
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("saved");
    update.revision = 2;
    update.save_named = false;
    update.text = "SELECT 2".into();
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("draft");
    assert!(
        store
            .documents()
            .close_query(workspace, update.document, 3, false, false, &cancel)
            .is_err()
    );
    store
        .documents()
        .close_query(workspace, update.document, 3, true, false, &cancel)
        .expect("discard");
    update.save_named = true;
    let closed = store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("stale save ignored");
    assert!(!closed.is_open);
    assert_eq!(closed.content, "SELECT 1");
    assert_eq!(closed.saved_content.as_deref(), Some("SELECT 1"));
    update.revision = 4;
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("explicit reopen");
    store
        .documents()
        .close_query(workspace, update.document, 5, true, true, &cancel)
        .expect("delete");
    update.revision = 6;
    assert!(
        store
            .documents()
            .update_query(workspace, &update, &cancel)
            .is_err()
    );
    assert!(
        store
            .documents()
            .get(update.document)
            .expect("read")
            .is_none()
    );
    assert!(
        store
            .documents()
            .page(workspace, &DocumentFilter::default(), &cancel)
            .expect("list")
            .entries
            .is_empty()
    );
}

#[test]
fn workspace_and_legacy_writes_cannot_bypass_revision_barriers() {
    let (store, workspace, update) = setup();
    let cancel = CancelToken::new();
    let saved = store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("saved");
    assert!(store.documents().save(&saved).is_err());
    let other = store.workspaces().create("other").expect("workspace").id;
    assert!(
        store
            .documents()
            .update_query(other, &update, &cancel)
            .is_err()
    );
    assert!(
        store
            .documents()
            .close_query(other, update.document, 2, true, true, &cancel)
            .is_err()
    );
    assert!(store.documents().delete(update.document).expect("delete"));
    assert!(
        !store
            .documents()
            .delete(update.document)
            .expect("repeat delete")
    );
    assert!(store.documents().save(&saved).is_err());
    assert!(
        store
            .documents()
            .update_query(workspace, &update, &cancel)
            .is_err()
    );
}

#[test]
fn document_pages_are_bounded_literal_and_do_not_open_oversized_bodies() {
    let (store, workspace, mut update) = setup();
    let cancel = CancelToken::new();
    for title in ["alpha", "beta", "literal_%"] {
        update.document = DocumentId::new();
        update.title = title.into();
        store
            .documents()
            .update_query(workspace, &update, &cancel)
            .expect("save");
    }
    let mut filter = DocumentFilter {
        limit: 2,
        ..Default::default()
    };
    let first = store
        .documents()
        .page(workspace, &filter, &cancel)
        .expect("first");
    assert_eq!(first.entries.len(), 2);
    filter.before = first.next;
    let second = store
        .documents()
        .page(workspace, &filter, &cancel)
        .expect("second");
    assert_eq!(second.entries.len(), 1);
    assert!(second.next.is_none());
    assert!(
        !first
            .entries
            .iter()
            .any(|entry| entry.id == second.entries[0].id)
    );
    filter.before = None;
    filter.search = "_%".into();
    let found = store
        .documents()
        .page(workspace, &filter, &cancel)
        .expect("literal");
    assert_eq!(found.entries.len(), 1);
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE documents SET saved_content=?1 WHERE id=?2",
                params![
                    "x".repeat(MAX_QUERY_DOCUMENT_BYTES + 1),
                    update.document.to_string()
                ],
            )?;
            Ok(())
        })
        .expect("legacy oversized copy");
    assert_eq!(
        store
            .documents()
            .page(workspace, &filter, &cancel)
            .expect("bounded summary")
            .entries
            .len(),
        1
    );
    assert!(store.documents().get(update.document).is_err());
    assert!(
        store
            .documents()
            .close_query(workspace, update.document, 2, true, false, &cancel)
            .is_err()
    );
}

#[test]
fn legacy_titles_remain_readable_with_a_bound_before_rust_materialization() {
    let (store, workspace, update) = setup();
    store
        .documents()
        .update_query(workspace, &update, &CancelToken::new())
        .expect("save");
    for (length, readable) in [(1024, true), (4097, false)] {
        store
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE documents SET title=?1, saved_title=?1 WHERE id=?2",
                    params!["a".repeat(length), update.document.to_string()],
                )?;
                Ok(())
            })
            .expect("legacy title");
        assert_eq!(store.documents().get(update.document).is_ok(), readable);
    }
}

#[test]
fn compare_and_swap_checks_the_stored_revision_before_writing_or_discarding() {
    let (store, workspace, mut update) = setup();
    let cancel = CancelToken::new();
    update.expected_revision = Some(0);
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("first revision");
    update.revision = 20;
    update.text = "SELECT 'stale writer'".into();
    assert!(
        store
            .documents()
            .update_query(workspace, &update, &cancel)
            .is_err()
    );
    assert!(
        store
            .documents()
            .close_query_checked(
                workspace,
                update.document,
                DocumentRevision {
                    next: 21,
                    expected: Some(0)
                },
                true,
                false,
                &cancel
            )
            .is_err()
    );
    assert_eq!(
        store
            .documents()
            .get(update.document)
            .expect("read")
            .expect("present")
            .content,
        "SELECT 1"
    );
    update.expected_revision = Some(1);
    store
        .documents()
        .update_query(workspace, &update, &cancel)
        .expect("fresh writer");
}
