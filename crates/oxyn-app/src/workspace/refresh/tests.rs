//! What a real change on a real session does to the views on screen.
//!
//! Every test here drives the executor of a temporary SQLite workspace and
//! never calls the refresh path itself: what must be proven is that the **bus**
//! moved the view, not that a function does what its name says. The one
//! exception is the lag catch-up, which no test can provoke honestly, and it is
//! called directly and said so.

use super::*;
use crate::workspace::tests::{
    connect_test_backend, preview_fixture, submit, wait_for_preview, wait_until,
};
use gpui::{Entity, TestAppContext, VisualTestContext};

/// Rows currently on screen in the preview, or `None` when it holds no buffer.
fn rows_on_screen(view: &Workspace, cx: &gpui::App) -> Option<usize> {
    view.preview_grid
        .read(cx)
        .state()
        .buffer()
        .map(|buffer| buffer.row_count())
}

/// Gives the workspace every chance to react, so that « nothing happened »
/// means nothing happened rather than « not yet ».
#[expect(
    clippy::disallowed_methods,
    reason = "test harness polling a wall-clock executor, not the UI thread"
)]
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    cx.run_until_parked();
}

/// Opens the Data tab of a relation and waits for its first rows.
fn show_table(view: &Entity<Workspace>, cx: &mut VisualTestContext, relation: &str) {
    let path = CatalogPath::for_relation(None, Some("main"), relation).expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.panel, WorkspacePanel::Object);
        assert_eq!(view.object_tab, ObjectTab::Data);
    });
}

#[test]
fn only_a_read_leaves_the_rows_on_screen_believable() {
    assert!(!stales_displayed_rows(StatementIntent::Read));
    for intent in [
        StatementIntent::Write,
        StatementIntent::Ddl,
        StatementIntent::Grant,
        StatementIntent::Unknown,
    ] {
        assert!(
            stales_displayed_rows(intent),
            "an unclassified or mutating statement must never be trusted to have changed nothing"
        );
    }
}

/// **Une écriture atteint l'aperçu affiché sans clic.**
///
/// C'est le défaut que corrige ADR-0022 : l'`INSERT` part d'ailleurs, et la
/// ligne doit apparaître sans que personne ne touche `Refresh data`.
#[gpui::test]
fn a_write_reaches_the_preview_on_screen_without_a_click(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_empty");
    view.read_with(cx, |view, cx| {
        assert_eq!(rows_on_screen(view, cx), Some(0), "the table starts empty");
    });

    submit(
        &submitter,
        execution_command(
            connection,
            session,
            false,
            dialect,
            "INSERT INTO preview_empty (id) VALUES (7)".into(),
            Vec::new(),
        ),
    )
    .expect("insert");

    wait_until(
        &view,
        cx,
        "the inserted row must reach the preview without a click",
        |view, cx| view.preview_active.is_none() && rows_on_screen(view, cx) == Some(1),
    );
}

/// **La forme demandée survit au rafraîchissement.**
///
/// Un prédicat posé par l'utilisateur doit revenir avec les lignes à jour, pas
/// être remplacé par une première page sans filtre : le retour à zéro serait
/// une perte de travail silencieuse.
#[gpui::test]
fn a_write_re_reads_with_the_filter_the_user_applied(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    view.update(cx, |view, cx| {
        view.preview_controls
            .predicate
            .update(cx, |field, cx| field.set_text("id > 990".into(), cx));
        view.apply_preview_predicate(cx);
    });
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.preview_controls.applied.predicate(), Some("id > 990"));
        assert_eq!(rows_on_screen(view, cx), Some(10));
    });

    submit(
        &submitter,
        execution_command(
            connection,
            session,
            false,
            dialect,
            "INSERT INTO preview_keyed (id, label) VALUES (2000, 'new')".into(),
            Vec::new(),
        ),
    )
    .expect("insert");

    wait_until(
        &view,
        cx,
        "the refreshed preview must still be the filtered one",
        |view, cx| view.preview_active.is_none() && rows_on_screen(view, cx) == Some(11),
    );
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.preview_controls.applied.predicate(),
            Some("id > 990"),
            "an unfiltered 200-row page would mean the user's predicate was dropped"
        );
        assert_eq!(view.preview_controls.applied.offset, 0);
    });
}

/// **Une lecture ne relit rien.**
///
/// Le compte se fait sur le bus : rien de ce que ce test n'a pas soumis ne doit
/// atteindre l'exécuteur.
#[gpui::test]
fn a_read_never_re_reads_the_preview(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let mut events = submitter.subscribe();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    while events.try_recv().is_ok() {}

    submit(
        &submitter,
        execution_command(
            connection,
            session,
            false,
            dialect,
            "SELECT id FROM preview_keyed LIMIT 1".into(),
            Vec::new(),
        ),
    )
    .expect("select");
    // Drained before any frame runs, so whatever appears below can only come
    // from the workspace reacting.
    while events.try_recv().is_ok() {}
    settle(cx);

    assert!(
        events.try_recv().is_err(),
        "a read changed nothing: re-reading 200 rows is work nobody asked for"
    );
    view.read_with(cx, |view, _| assert!(view.preview_active.is_none()));
}

/// **Rien ne suit une erreur, et l'aperçu précédent reste affiché.**
///
/// Une relecture après un échec remplacerait le message par des lignes, et
/// l'utilisateur ne saurait jamais que son instruction n'a pas tourné.
#[gpui::test]
fn a_failed_execution_refreshes_nothing_and_keeps_the_rows_shown(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let mut events = submitter.subscribe();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    let (rows, notice) = view.read_with(cx, |view, cx| {
        (rows_on_screen(view, cx), view.preview_notice.clone())
    });
    assert_eq!(rows, Some(200));
    while events.try_recv().is_ok() {}

    assert!(
        submit(
            &submitter,
            execution_command(
                connection,
                session,
                false,
                dialect,
                "INSERT INTO no_such_table (id) VALUES (1)".into(),
                Vec::new(),
            ),
        )
        .is_err(),
        "the fixture must really fail on the server"
    );
    while events.try_recv().is_ok() {}
    settle(cx);

    assert!(
        events.try_recv().is_err(),
        "a failure must trigger no read at all"
    );
    view.read_with(cx, |view, cx| {
        assert_eq!(rows_on_screen(view, cx), rows);
        assert_eq!(view.preview_notice, notice);
    });
}

/// **Rien ne se relit derrière un onglet qui n'est pas à l'écran.**
#[gpui::test]
fn a_write_behind_a_hidden_data_tab_dispatches_nothing(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let mut events = submitter.subscribe();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1060.)));
    show_table(&view, cx, "preview_keyed");
    // The real gesture that hides the tab, not a field assignment — and the
    // focus the gesture needs, since a shortcut only fires from inside the view.
    cx.update(|window, cx| window.focus(&view.read(cx).focus_handle(cx)));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-j");
    view.read_with(cx, |view, _| assert_eq!(view.panel, WorkspacePanel::Sql));
    while events.try_recv().is_ok() {}

    submit(
        &submitter,
        execution_command(
            connection,
            session,
            false,
            dialect,
            "INSERT INTO preview_keyed (id, label) VALUES (3000, 'hidden')".into(),
            Vec::new(),
        ),
    )
    .expect("insert");
    while events.try_recv().is_ok() {}
    settle(cx);

    assert!(
        events.try_recv().is_err(),
        "invisible rows are re-read when the tab comes back, not before"
    );
    view.read_with(cx, |view, _| assert!(view.preview_active.is_none()));
}

/// **Un événement d'une autre connexion ne touche à rien.**
#[gpui::test]
fn an_event_from_another_connection_is_ignored(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    // A second session on the same backend: same bus, another connection.
    let (backend, other) = connect_test_backend(backend);
    let submitter = backend.clone();
    let mut events = submitter.subscribe();
    assert_ne!(open.connection, other.connection);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    while events.try_recv().is_ok() {}

    // A DDL *and* a write, both on the other connection.
    for sql in [
        "CREATE TABLE elsewhere (id INTEGER)",
        "INSERT INTO elsewhere (id) VALUES (1)",
    ] {
        submit(
            &submitter,
            execution_command(
                other.connection,
                other.session,
                false,
                other.dialect,
                sql.into(),
                Vec::new(),
            ),
        )
        .expect("statement on the other connection");
    }
    while events.try_recv().is_ok() {}
    settle(cx);

    assert!(
        events.try_recv().is_err(),
        "another connection's changes are not this workspace's business"
    );
    view.read_with(cx, |view, _| {
        assert!(view.preview_active.is_none());
        assert!(view.catalog_active.is_none());
    });
}

/// **Une relecture en cours absorbe les demandes qui arrivent pendant elle.**
///
/// Déterministe, contrairement au test de rafale ci-dessous : les demandes sont
/// posées pendant qu'une lecture est réellement sur le fil, ce qu'aucun
/// enchaînement de `submit` ne garantit. Ce qu'il prouve tient en trois
/// points — la lecture en cours n'est ni remplacée ni interrompue, trois
/// demandes n'en valent qu'une, et cette une-là part bien.
#[gpui::test]
fn requests_arriving_during_a_read_collapse_into_exactly_one(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    while events.try_recv().is_ok() {}

    view.update(cx, |view, cx| {
        view.load_preview(cx);
        let running = view.preview_active.as_ref().map(|run| run.0);
        assert!(running.is_some(), "a read must really be on the wire");
        for _ in 0..3 {
            view.refresh_visible_preview(cx);
        }
        assert_eq!(
            view.preview_active.as_ref().map(|run| run.0),
            running,
            "the read in flight is neither replaced nor interrupted"
        );
        assert!(view.preview_refresh_owed);
    });
    wait_for_preview(&view, cx);
    settle(cx);

    let mut reads = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.event,
            Event::Completed {
                intent: StatementIntent::Read,
                ..
            }
        ) {
            reads += 1;
        }
    }
    assert_eq!(
        reads, 2,
        "the read in flight plus exactly one owed to the three requests"
    );
    view.read_with(cx, |view, _| assert!(!view.preview_refresh_owed));
}

/// **Plusieurs écritures rapprochées ne produisent pas une relecture chacune.**
///
/// Les relectures se comptent par leur intention : les trois `INSERT` de ce
/// test sont classés `Write`, une relecture d'aperçu est classée `Read`. Le
/// compte exact dépend du moment où la première réponse revient ; ce qui ne
/// doit jamais arriver, c'est une relecture par écriture — un script qui écrit
/// cent fois ne doit pas produire cent lectures.
#[gpui::test]
fn several_writes_in_a_row_do_not_each_re_read_the_preview(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let mut events = submitter.subscribe();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    while events.try_recv().is_ok() {}

    let writes = 3;
    for id in 4001..=4000 + writes {
        submit(
            &submitter,
            execution_command(
                connection,
                session,
                false,
                dialect,
                format!("INSERT INTO preview_keyed (id, label) VALUES ({id}, 'burst')"),
                Vec::new(),
            ),
        )
        .expect("insert");
    }
    settle(cx);

    let mut re_reads = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.event,
            Event::Completed {
                intent: StatementIntent::Read,
                ..
            }
        ) {
            re_reads += 1;
        }
    }
    assert!(re_reads >= 1, "the burst must reach the preview at all");
    assert!(
        re_reads < writes,
        "one re-read per write is exactly what coalescing must prevent, got {re_reads}"
    );
    view.read_with(cx, |view, cx| {
        assert_eq!(
            rows_on_screen(view, cx),
            Some(200),
            "the shape in force is still a bounded first page"
        );
    });
}

/// **La bibliothèque ouverte suit les exécutions.**
#[gpui::test]
fn an_execution_adds_its_entry_to_the_open_library(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let submitter = backend.clone();
    let (connection, session, dialect) = (open.connection, open.session, open.dialect);
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1060.)));
    cx.update(|window, cx| window.focus(&view.read(cx).focus_handle(cx)));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-h");
    wait_until(
        &view,
        cx,
        "the library must load its first page",
        |view, cx| view.panel == WorkspacePanel::Library && view.library.read(cx).entry_count() > 0,
    );
    let before = view.read_with(cx, |view, cx| view.library.read(cx).entry_count());

    submit(
        &submitter,
        execution_command(
            connection,
            session,
            false,
            dialect,
            "SELECT 'recorded'".into(),
            Vec::new(),
        ),
    )
    .expect("select");

    wait_until(
        &view,
        cx,
        "the executed statement must appear in local history without a click",
        |view, cx| view.library.read(cx).entry_count() > before,
    );
}

/// **Après un `Lagged`, on relit comme si tout avait été manqué.**
///
/// Provoquer un vrai `RecvError::Lagged` demanderait de saturer un canal de 256
/// places pendant qu'un abonné est bloqué : c'est fabriquer une panne, pas
/// l'observer. Ce test appelle donc **la fonction de rattrapage** et prouve ce
/// qu'elle relance ; la branche `Lagged` de la boucle d'abonnement, elle, n'est
/// pas couverte.
#[gpui::test]
fn catching_up_after_a_lag_re_reads_the_catalog_and_the_visible_preview(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    show_table(&view, cx, "preview_keyed");
    view.read_with(cx, |view, _| {
        assert!(view.preview_active.is_none());
        assert!(view.catalog_active.is_none());
    });

    view.update(cx, Workspace::catch_up_after_lag);

    view.read_with(cx, |view, _| {
        assert!(
            view.catalog_active.is_some(),
            "a dropped event may have been a DDL, so the tree is re-fetched"
        );
        assert!(
            view.preview_active.is_some(),
            "a dropped event may have been a write, so the visible rows are re-read"
        );
    });
    wait_until(&view, cx, "the catch-up must settle", |view, _| {
        view.catalog_active.is_none() && view.preview_active.is_none()
    });
}
