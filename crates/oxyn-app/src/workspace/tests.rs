//! Interaction regressions over an isolated, real SQLite session.

use super::export::export_command;
use super::*;
use crate::backend::ConnectionResponse;
use gpui::{TestAppContext, point, px};
use oxyn_ui::{ConnectionDraft, Theme, ThemeMode};

fn connected_workspace() -> (Backend, OpenConnection) {
    let backend = Backend::open_temporary().expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "Workspace interaction test".into(),
        environment: Environment::Local,
        values: [("path".into(), ":memory:".into())].into_iter().collect(),
        secrets: Default::default(),
    };
    // Fixture setup runs before creating the UI, never in a view or render callback.
    let response = backend
        .connect(draft, CancelToken::new())
        .blocking_recv()
        .expect("executor response")
        .expect("connection setup");
    let open = match response {
        ConnectionResponse::Open(open) => open,
        ConnectionResponse::Approval {
            command, config, ..
        } => {
            let approved = backend
                .approve_connection(command, *config, CancelToken::new())
                .blocking_recv()
                .expect("approval response")
                .expect("connection approval");
            let ConnectionResponse::Open(open) = approved else {
                panic!("approved connection must open")
            };
            open
        }
    };
    (backend, open)
}

/// Submits a command and polls for the executor's answer.
///
/// Neither of the two executors in play can await the other: GPUI's test
/// executor is deterministic and never drives the Tokio runtime the backend
/// answers on, so `run_until_parked` and `BackgroundExecutor::block` both give
/// up with "parked with nothing left to run". Polling the response with a
/// bounded budget is the shape that works from a test thread, and it opens no
/// door in the product: no view calls this.
fn submit(backend: &Backend, command: oxyn_core::Command) -> Result<Outcome, OxynError> {
    // `blocking_recv` plutot qu'une attente active : le sondage en boucle
    // appelle `thread::sleep`, que le clippy.toml du depot interdit au titre
    // de I-05. Le canal rend la reponse des qu'elle existe.
    backend
        .dispatch(CommandId::new(), command, CancelToken::new())
        .blocking_recv()
        .expect("the executor dropped the response")
}

#[gpui::test]
fn an_unsupported_capability_stops_the_submission_and_says_which(cx: &mut TestAppContext) {
    // SQLite déclare EXPLAIN et pas EXPLAIN_ANALYZE. Sans ce contrôle, le
    // serveur répond `near "ANALYZE": syntax error` — un message qui ne nomme
    // aucune capacité et que personne ne relie à la session.
    let (backend, open) = connected_workspace();
    assert!(open.capabilities.contains(oxyn_core::Capabilities::EXPLAIN));
    assert!(
        !open
            .capabilities
            .contains(oxyn_core::Capabilities::EXPLAIN_ANALYZE)
    );
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("EXPLAIN ANALYZE SELECT 1");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    workspace.read_with(cx, |view, _| {
        assert!(
            view.active.is_none(),
            "rien ne doit partir sur le bus : la session ne sait pas le faire"
        );
    });

    // Ce que SQLite sait faire passe, sur la même session.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("EXPLAIN SELECT 1");
    workspace.read_with(cx, |view, cx| {
        assert!(
            capabilities::missing_for(
                &view.editor.read(cx).statement_text(),
                view.dialect,
                view.capabilities
            )
            .is_none()
        );
    });
}

#[gpui::test]
fn a_new_execution_withdraws_the_previous_result_from_export(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    workspace.read_with(cx, |view, cx| {
        // État initial : aucun résultat, donc aucune offre — et la barre le dit
        // au lieu de disparaître.
        assert!(!view.export.read(cx).is_result_ready());
        assert!(view.last_result.is_none());
    });

    cx.simulate_input("select 1;");
    workspace.update(cx, |view, cx| {
        view.last_result = Some(oxyn_core::ResultId::new());
        view.export
            .update(cx, |export, cx| export.set_result_ready(None, cx));
        view.execute(cx);
    });
    workspace.read_with(cx, |view, cx| {
        assert!(
            view.last_result.is_none(),
            "exporter le résultat précédent sous l'en-tête de la requête suivante serait faux"
        );
        assert!(!view.export.read(cx).is_result_ready());
    });
}

#[gpui::test]
fn an_export_chosen_before_a_new_execution_is_dropped_not_written(cx: &mut TestAppContext) {
    // Le sélecteur de fichier n'est pas modal pour Oxyn : une exécution a pu
    // partir pendant qu'il était ouvert. Écrire quand même produirait un fichier
    // dont le nom désigne une requête et dont les lignes viennent d'une autre.
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let perime = oxyn_core::ResultId::new();
    let destination = std::env::temp_dir().join("oxyn-export-jamais-ecrit.csv");
    let _ = std::fs::remove_file(&destination);
    workspace.update(cx, |view, cx| {
        view.last_result = Some(oxyn_core::ResultId::new());
        view.start_export(
            perime,
            oxyn_core::ExportFormat::Csv,
            destination.clone(),
            cx,
        );
    });
    cx.run_until_parked();
    workspace.read_with(cx, |view, _| assert!(view.export_active.is_none()));
    assert!(
        !destination.exists(),
        "aucun fichier ne doit être écrit pour un résultat qui n'est plus affiché"
    );
}

#[test]
fn the_exported_file_is_written_from_the_result_the_view_designates() {
    // Le bout en bout : la commande que la vue émet produit un fichier lisible
    // sans Oxyn ([I-11](../../../CLAUDE.md#i-11)).
    let (backend, open) = connected_workspace();
    let outcome = submit(
        &backend,
        execution_command(
            open.connection,
            open.session,
            false,
            open.dialect,
            "SELECT 1 AS n, 'deux' AS mot".to_owned(),
        ),
    )
    .expect("statement runs");
    let Outcome::Executed { result, buffer, .. } = outcome else {
        panic!("l'exécution doit produire un résultat")
    };
    assert!(buffer.is_complete(), "le résultat doit être clos");

    let destination = std::env::temp_dir().join(format!(
        "oxyn-export-{}-{:?}.csv",
        std::process::id(),
        std::thread::current().id()
    ));
    let outcome = submit(
        &backend,
        export_command(
            open.connection,
            result,
            oxyn_core::ExportFormat::Csv,
            destination.clone(),
        ),
    )
    .expect("export runs");

    let Outcome::Exported { rows, bytes, .. } = outcome else {
        panic!("l'export doit rendre Exported, pas {outcome:?}")
    };
    assert_eq!(rows, 1);
    assert!(bytes > 0);
    let ecrit = std::fs::read_to_string(&destination).expect("le fichier existe");
    assert!(ecrit.contains("n,mot"), "en-tête absent : {ecrit}");
    assert!(ecrit.contains("deux"), "valeur absente : {ecrit}");
    let _ = std::fs::remove_file(&destination);
}

#[gpui::test]
fn sidebar_shortcuts_work_from_editor_without_consuming_native_editing(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("select 1;");
    cx.simulate_keystrokes("cmd-b");
    assert!(workspace.read_with(cx, |view, _| view.sidebar_collapsed));
    assert_eq!(
        workspace.read_with(cx, |view, cx| view.draft_text(cx)),
        "select 1;"
    );
    cx.simulate_keystrokes("cmd-b cmd-a");
    cx.simulate_input("select 2;");
    assert_eq!(
        workspace.read_with(cx, |view, cx| view.draft_text(cx)),
        "select 2;"
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        workspace.read_with(cx, |view, cx| view.draft_text(cx)),
        "select 2"
    );
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(
        workspace.read_with(cx, |view, cx| view.draft_text(cx)),
        "select 2;"
    );
    cx.simulate_keystrokes("cmd-shift-l");
    cx.update(|_, cx| assert_eq!(Theme::of(cx).mode, ThemeMode::Light));
    // The trigger's geometry is explicit, independent of the fake text metrics.
    cx.simulate_click(point(px(316.), px(36.)), Default::default());
    assert!(workspace.read_with(cx, |view, _| view.sidebar_collapsed));
    cx.simulate_keystrokes("cmd-j");
    cx.update(|window, cx| {
        assert!(
            workspace
                .read(cx)
                .editor
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
}

#[gpui::test]
fn catalog_cancellation_signals_its_own_token_and_preserves_the_query(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let catalog_token = CancelToken::new();
    let query_token = CancelToken::new();
    workspace.update(cx, |view, cx| {
        view.active = Some((CommandId::new(), query_token.clone()));
        view.catalog_active = Some((CommandId::new(), catalog_token.clone()));
        view.catalog_state = CatalogState::Loading;
        view.cancel_catalog(cx);
    });
    assert!(catalog_token.is_cancelled());
    assert!(!query_token.is_cancelled());
    workspace.update(cx, |view, cx| view.cancel(cx));
    assert!(query_token.is_cancelled());
}

#[gpui::test]
fn catalog_selection_changes_context_without_executing_or_replacing_the_draft(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("select 7;");
    let path =
        CatalogPath::for_relation(None, None, "odd\"; table.name").expect("valid object name");
    let tree = workspace.read_with(cx, |view, _| view.catalog.clone().expect("catalog entity"));
    workspace.update(cx, |view, cx| {
        view.catalog_state = CatalogState::Ready;
        view.set_draft_text("select 7;", cx);
        cx.notify();
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.focus(&tree.read(cx).focus_handle(cx)));
    tree.update(cx, |tree, cx| tree.select(path.clone(), cx));
    cx.run_until_parked();
    workspace.read_with(cx, |view, cx| {
        assert_eq!(view.selected_path.as_ref(), Some(&path));
        assert_eq!(view.panel, WorkspacePanel::Object);
        assert!(view.active.is_none());
        assert_eq!(view.draft_text(cx), "select 7;");
    });
    cx.simulate_keystrokes("cmd-j");
    assert_eq!(
        workspace.read_with(cx, |view, _| view.panel),
        WorkspacePanel::Sql
    );
}

fn preview_fixture() -> (Backend, OpenConnection) {
    let (backend, open) = connected_workspace();
    for sql in [
        "CREATE TABLE preview_rows AS WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT x AS id FROM n",
        "CREATE TABLE preview_empty (id INTEGER)",
    ] {
        backend
            .dispatch(
                CommandId::new(),
                execution_command(
                    open.connection,
                    open.session,
                    false,
                    open.dialect,
                    sql.into(),
                ),
                CancelToken::new(),
            )
            .blocking_recv()
            .expect("execution response")
            .expect("fixture table");
    }
    backend
        .dispatch(
            CommandId::new(),
            Command::RefreshCatalogScope {
                connection: open.connection,
                scope: oxyn_core::CatalogRefreshScope::Relations {
                    catalog: None,
                    namespace: Some("main".into()),
                },
            },
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("catalog response")
        .expect("fixture metadata");
    (backend, open)
}

// The interdict in `clippy.toml` targets the product's UI thread, where a
// blocking sleep freezes the window (I-05). This is a test harness polling an
// executor that GPUI's virtual clock does not drive, and the loop right below
// carries its own deadline. `expect` rather than `allow`: if the sleep ever
// goes away, the exemption must go with it.
#[expect(
    clippy::disallowed_methods,
    reason = "test harness polling a wall-clock executor, not the UI thread"
)]
fn wait_for_preview(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| view.preview_active.is_none()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "preview must finish");
        // The driver's Tokio executor uses wall time, outside GPUI's virtual clock.
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn selecting_a_table_loads_bounded_rows_without_replacing_sql(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("SELECT 'keep my draft';");
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
    tree.update(cx, |tree, cx| tree.select(path, cx));
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.object_tab, ObjectTab::Data);
        assert_eq!(
            view.preview_grid
                .read(cx)
                .state()
                .buffer()
                .expect("real rows")
                .row_count(),
            200
        );
        assert_eq!(view.draft_text(cx), "SELECT 'keep my draft';");
        assert!(
            view.grid.read(cx).state().buffer().is_none(),
            "SQL results stay separate"
        );
        assert!(view.active.is_none());
    });
    let empty = CatalogPath::for_relation(None, Some("main"), "preview_empty").expect("table path");
    tree.update(cx, |tree, cx| tree.select(empty, cx));
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        let buffer = view
            .preview_grid
            .read(cx)
            .state()
            .buffer()
            .expect("empty success is not an error");
        assert_eq!(buffer.row_count(), 0);
        assert!(buffer.is_complete());
        assert!(view.preview_notice.starts_with("0 rows shown"));
    });
}

#[gpui::test]
fn changing_table_cancels_old_preview_and_rejects_its_late_error(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let old_id = CommandId::new();
    let old_cancel = CancelToken::new();
    let new_path =
        CatalogPath::for_relation(None, Some("main"), "preview_empty").expect("table path");
    view.update(cx, |view, cx| {
        view.preview_active = Some((old_id, old_cancel.clone()));
        view.select_object(new_path.clone(), cx);
        let current_notice = view.preview_notice.clone();
        view.complete_preview(
            old_id,
            Err(OxynError::Internal("obsolete failure".into())),
            cx,
        );
        assert_eq!(view.preview_notice, current_notice);
        assert_eq!(view.preview_path.as_ref(), Some(&new_path));
    });
    assert!(old_cancel.is_cancelled());
    wait_for_preview(&view, cx);
    assert!(view.read_with(cx, |view, cx| {
        view.preview_grid.read(cx).state().buffer().is_some()
    }));
}
