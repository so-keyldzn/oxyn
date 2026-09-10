//! Recovery never opens a database session or executes SQL until a later explicit action.

use super::*;
use gpui::{TestAppContext, VisualTestContext};
use oxyn_core::{QueryDocumentUpdate, QueryLanguage};

fn fixture(backend: &Backend, title: &str, text: &str) -> DocumentId {
    let document = DocumentId::new();
    backend
        .dispatch(
            CommandId::new(),
            Command::SaveQueryDocument {
                workspace: backend.workspace_id(),
                update: Box::new(QueryDocumentUpdate {
                    document,
                    revision: 1,
                    expected_revision: Some(0),
                    title: title.into(),
                    language: QueryLanguage::SQL,
                    text: text.into(),
                    connection: None,
                    save_named: false,
                    is_open: true,
                }),
            },
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("response")
        .expect("draft");
    document
}

#[expect(
    clippy::disallowed_methods,
    reason = "bounded test harness polls the independent Tokio runtime"
)]
fn settle(view: &Entity<Recovery>, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, cx| {
            view.request.is_none()
                && view.editors.values().all(|console| {
                    console.read(cx).draft_pending.is_none()
                        && console.read(cx).save_active.is_none()
                        && !console.read(cx).document_closing
                })
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "recovery did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn checkbox_selection_restores_only_chosen_editors_offline_and_keeps_them_editable(
    cx: &mut TestAppContext,
) {
    let backend = Backend::open_temporary().expect("backend");
    let first = fixture(&backend, "First.sql", "SELECT 'first'");
    let second = fixture(&backend, "Second.sql", "SELECT 'second'");
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| Recovery::new(backend.clone(), cx));
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.selected.len()), 2);
    // The newest document is first; Space deselects it through the delivered event path.
    cx.simulate_keystrokes("space");
    assert!(!view.read_with(cx, |view, _| view.selected.contains_key(&second)));
    view.update(cx, |view, cx| view.action(Action::Restore, cx));
    settle(&view, cx);
    let console = view.read_with(cx, |view, cx| {
        assert_eq!(view.editors.len(), 1);
        let console = view.editors.get(&first).expect("selected editor");
        assert!(console.read(cx).open.is_none());
        assert_eq!(console.read(cx).editor.read(cx).text(), "SELECT 'first'");
        console.clone()
    });
    cx.simulate_keystrokes("cmd-enter");
    assert!(console.read_with(cx, |console, _| console.active.is_none()));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("SELECT 'edited offline'");
    settle(&view, cx);
    let stored = backend
        .dispatch(
            CommandId::new(),
            Command::OpenDocument {
                workspace: backend.workspace_id(),
                document: first,
            },
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("response")
        .expect("stored draft");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.content == "SELECT 'edited offline'" && document.connection.is_none())
    );
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}

#[gpui::test]
fn recovered_query_can_be_saved_and_closed_without_a_connection(cx: &mut TestAppContext) {
    let backend = Backend::open_temporary().expect("backend");
    let document = fixture(&backend, "Offline.sql", "SELECT 5");
    let (view, cx) = cx.add_window_view(|_, cx| Recovery::new(backend.clone(), cx));
    settle(&view, cx);
    view.update(cx, |view, cx| view.action(Action::Restore, cx));
    settle(&view, cx);
    cx.simulate_keystrokes("cmd-w");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    assert!(view.read_with(cx, |view, _| view.restored.is_empty()));
    let stored = backend
        .dispatch(
            CommandId::new(),
            Command::OpenDocument {
                workspace: backend.workspace_id(),
                document,
            },
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("response")
        .expect("saved");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if !document.is_open && document.saved_content.as_deref() == Some("SELECT 5"))
    );
}
