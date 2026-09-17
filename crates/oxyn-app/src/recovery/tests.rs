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
                    provenance: None,
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
        // Les éditeurs restaurés autosauvegardent au repos de frappe : sans
        // avancer l'horloge, leur brouillon ne part jamais (ADR-0024).
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
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

/// L'écran dit ce qu'il a constaté — et rien de plus.
///
/// ADR-0021 justifie la table `app_sessions` et son battement périodique par le
/// fait que l'écran **affirme** quelque chose : « Oxyn ne s'est pas fermé
/// normalement ». Il ne l'affirmait pas. Le coût de la migration était payé, le
/// bénéfice annoncé ne l'était pas.
///
/// Le sens inverse compte autant : le même écran s'ouvre à la demande, par
/// « Saved working copies ». Y annoncer un plantage qui n'a pas eu lieu serait
/// le mensonge symétrique du silence.
#[gpui::test]
fn l_ecran_de_reprise_annonce_l_arret_anormal_et_seulement_alors(cx: &mut gpui::TestAppContext) {
    // Un backend dont le lancement précédent a laissé une session abandonnée.
    let backend = Backend::temporary_with_abandoned_session().expect("backend");
    let (recovery, cx) = cx.add_window_view(|_, cx| Recovery::new(backend, cx));
    // La lecture part sur le runtime Tokio : `run_until_parked` seul rend la
    // main pendant que le message dit encore « Loading… ». `settle` attend que
    // la lecture ait répondu, comme les autres tests de ce fichier.
    settle(&recovery, cx);
    recovery.read_with(cx, |vue, _| {
        assert!(
            vue.notice.contains("did not close normally"),
            "un arrêt anormal constaté doit être dit : {}",
            vue.notice
        );
    });

    // Le même écran, ouvert sur un lancement ordinaire.
    let ordinaire = Backend::open_temporary().expect("backend");
    let (recovery, cx) = cx.add_window_view(|_, cx| Recovery::new(ordinaire, cx));
    settle(&recovery, cx);
    recovery.read_with(cx, |vue, _| {
        assert!(
            !vue.notice.contains("did not close normally"),
            "aucun plantage n'a été constaté : ne rien affirmer — {}",
            vue.notice
        );
    });
}
