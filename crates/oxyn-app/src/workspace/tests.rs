//! Interaction regressions over an isolated, real SQLite session.

use super::export::export_command;
use super::*;
use crate::backend::ConnectionResponse;
use gpui::{TestAppContext, point, px};
use oxyn_ui::{ConnectionDraft, Theme, ThemeMode};

pub(super) fn connected_workspace() -> (Backend, OpenConnection) {
    connect_test_backend(Backend::open_temporary().expect("temporary backend"))
}

pub(super) fn connect_test_backend(backend: Backend) -> (Backend, OpenConnection) {
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
pub(super) fn submit(backend: &Backend, command: oxyn_core::Command) -> Result<Outcome, OxynError> {
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
    workspace.read_with(cx, |view, cx| {
        assert!(
            view.console.read(cx).active.is_none(),
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
        assert!(view.console.read(cx).last_result.is_none());
    });

    cx.simulate_input("select 1;");
    workspace.update(cx, |view, cx| {
        view.console.update(cx, |console, _| {
            console.last_result = Some(oxyn_core::ResultId::new())
        });
        view.export
            .update(cx, |export, cx| export.set_result_ready(None, cx));
        view.execute(cx);
    });
    workspace.read_with(cx, |view, cx| {
        assert!(
            view.console.read(cx).last_result.is_none(),
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
        view.console.update(cx, |console, _| {
            console.last_result = Some(oxyn_core::ResultId::new())
        });
        view.start_export(
            ResultSource::Query,
            perime,
            oxyn_core::ExportFormat::Csv,
            destination.clone(),
            cx,
        );
    });
    cx.run_until_parked();
    workspace.read_with(cx, |view, cx| {
        assert!(view.console.read(cx).export_active.is_none())
    });
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
            Vec::new(),
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

/// `Find in loaded results…` révèle, il ne retranche pas.
///
/// C'est la propriété qui rend ce lot possible sans trancher l'arbitrage
/// export/affichage : aucune ligne ne quitte la grille, donc « ce qui est
/// exporté est ce qui est affiché » reste vrai sans qu'on y touche.
#[gpui::test]
fn la_recherche_revele_une_ligne_sans_en_retrancher_aucune(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();

    cx.simulate_input("SELECT 'alpha' AS mot UNION ALL SELECT 'beta' UNION ALL SELECT 'gamma'");
    workspace.update(cx, |view, cx| view.execute(cx));
    cx.run_until_parked();

    let lignes_avant = workspace.read_with(cx, |view, cx| {
        view.grid
            .read(cx)
            .buffer()
            .map_or(0, |tampon| tampon.row_count())
    });
    assert_eq!(lignes_avant, 3, "les trois lignes sont chargées");
    assert!(
        workspace.read_with(cx, |view, cx| view.grid.read(cx).find().is_none()),
        "aucune recherche n'a encore eu lieu : ce n'est pas « aucune correspondance »"
    );

    // L'utilisateur tape dans le champ et valide.
    workspace.update(cx, |view, cx| {
        view.find_field
            .update(cx, |champ, cx| champ.set_text("beta".to_owned(), cx));
        view.find_field
            .update(cx, |_, cx| cx.emit(oxyn_ui::FieldEvent::Submit));
    });
    cx.run_until_parked();

    let (lignes_apres, trouve, selection) = workspace.read_with(cx, |view, cx| {
        let grille = view.grid.read(cx);
        (
            grille.buffer().map_or(0, |tampon| tampon.row_count()),
            grille.find().map(|trouve| trouve.rows.clone()),
            grille.selected_row(),
        )
    });

    assert_eq!(
        lignes_apres, lignes_avant,
        "chercher ne retire aucune ligne : c'est ce qui laisse l'export intact"
    );
    assert_eq!(trouve, Some(vec![1]), "« beta » est la deuxième ligne");
    assert_eq!(selection, Some(1), "la correspondance est révélée");
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
        view.console.update(cx, |console, _| {
            console.active = Some((CommandId::new(), query_token.clone()))
        });
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
        assert!(view.console.read(cx).active.is_none());
        assert_eq!(view.draft_text(cx), "select 7;");
    });
    cx.simulate_keystrokes("cmd-j");
    assert_eq!(
        workspace.read_with(cx, |view, _| view.panel),
        WorkspacePanel::Sql
    );
}

pub(super) fn preview_fixture() -> (Backend, OpenConnection) {
    let (backend, open) = connected_workspace();
    for sql in [
        "CREATE TABLE preview_rows AS WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT x AS id FROM n",
        "CREATE TABLE preview_empty (id INTEGER)",
        // `preview_rows` is built by `CREATE TABLE … AS`, so it declares no
        // primary key: it is what an unpageable relation looks like. This one
        // declares one, which is what makes a total order — and therefore a
        // page — possible at all.
        "CREATE TABLE preview_keyed (id INTEGER PRIMARY KEY, label TEXT)",
        "INSERT INTO preview_keyed (id, label) SELECT id, 'row ' || id FROM preview_rows",
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
                    Vec::new(),
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
pub(super) fn wait_for_preview(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
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

/// Waits for a state the workspace must reach **on its own**.
///
/// A plain `run_until_parked` is not enough: the answer comes from a Tokio
/// runtime GPUI's virtual clock does not drive, so the loop yields wall time
/// between frames, with a deadline that names what never happened.
#[expect(
    clippy::disallowed_methods,
    reason = "test harness polling a wall-clock executor, not the UI thread"
)]
pub(super) fn wait_until(
    view: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    what: &str,
    mut ready: impl FnMut(&Workspace, &gpui::App) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, cx| ready(view, cx)) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{what}");
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
        assert!(view.console.read(cx).active.is_none());
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

#[gpui::test]
fn compact_layout_preserves_wide_preference_and_never_executes(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    cx.simulate_input("SELECT 'draft';");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.compact_layout);
        assert!(!view.sidebar_collapsed);
    });
    cx.simulate_resize(gpui::size(px(1024.), px(768.)));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.compact_layout);
        assert!(!view.sidebar_collapsed, "wide preference is preserved");
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
    });
    cx.simulate_keystrokes("cmd-b");
    assert!(view.read_with(cx, |view, _| view.catalog_overlay));
    cx.simulate_keystrokes("escape");
    assert!(!view.read_with(cx, |view, _| view.catalog_overlay));
    cx.simulate_resize(gpui::size(px(1200.), px(900.)));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(!view.compact_layout);
        assert!(!view.sidebar_collapsed);
        assert_eq!(view.draft_text(cx), "SELECT 'draft';");
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
    });
}

#[gpui::test]
fn preview_export_tracks_only_the_current_complete_preview(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("path");
    view.update(cx, |view, cx| view.select_object(path, cx));
    wait_for_preview(&view, cx);
    let preview_result = view.read_with(cx, |view, cx| {
        assert!(view.preview_export.read(cx).is_result_ready());
        assert!(!view.export.read(cx).is_result_ready());
        view.preview_result.expect("complete preview is exportable")
    });
    let empty = CatalogPath::for_relation(None, Some("main"), "preview_empty").expect("path");
    view.update(cx, |view, cx| {
        view.select_object(empty, cx);
        assert!(view.preview_result.is_none());
        assert!(!view.preview_export.read(cx).is_result_ready());
        view.start_export(
            ResultSource::Preview,
            preview_result,
            ExportFormat::Csv,
            PathBuf::from("obsolete-preview-must-not-be-written.csv"),
            cx,
        );
        assert!(
            view.export_active.is_none(),
            "stale file dialog cannot export the previous table"
        );
    });
    wait_for_preview(&view, cx);
    view.read_with(cx, |view, cx| {
        assert!(
            view.preview_export.read(cx).is_result_ready(),
            "empty success can export its header"
        );
        assert_ne!(view.preview_result, Some(preview_result));
        assert!(view.console.read(cx).last_result.is_none());
    });
}

#[gpui::test]
fn changing_result_keeps_an_in_flight_export_cancellable(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let token = CancelToken::new();
    view.update(cx, |view, cx| {
        view.export_active = Some((CommandId::new(), token.clone(), ResultSource::Preview));
        view.preview_export.update(cx, |export, cx| {
            export.set_result_ready(None, cx);
            export.running(cx);
        });
        let path = CatalogPath::for_relation(None, Some("main"), "unloaded").expect("path");
        view.select_object(path, cx);
        assert!(view.preview_export_open);
        assert_eq!(
            view.preview_export.read(cx).phase(),
            &oxyn_ui::ExportPhase::Running
        );
        view.cancel_export(cx);
    });
    assert!(token.is_cancelled());
}

#[test]
fn exporting_a_complete_preview_writes_only_its_bounded_rows() {
    let (backend, open) = preview_fixture();
    let outcome = submit(
        &backend,
        Command::PreviewRelation {
            connection: open.connection,
            session: open.session,
            catalog: None,
            namespace: Some("main".into()),
            relation: "preview_rows".into(),
            limit: preview::PREVIEW_ROWS,
            shape: oxyn_core::PreviewShape::unordered(),
        },
    )
    .expect("preview");
    let Outcome::Executed { result, buffer, .. } = outcome else {
        panic!("preview result")
    };
    assert_eq!(buffer.row_count(), 200);
    assert!(!buffer.stats().truncated);
    let destination = std::env::temp_dir().join(format!("oxyn-preview-{}.csv", result));
    let exported = submit(
        &backend,
        export_command(
            open.connection,
            result,
            ExportFormat::Csv,
            destination.clone(),
        ),
    )
    .expect("preview export");
    assert!(matches!(exported, Outcome::Exported { rows: 200, .. }));
    let csv = std::fs::read_to_string(&destination).expect("written CSV");
    assert_eq!(
        csv.lines().count(),
        201,
        "header plus 200 rows, not the 1000-row table"
    );
    std::fs::remove_file(destination).expect("fixture cleanup");
}

#[expect(
    clippy::disallowed_methods,
    reason = "test harness waits for the separate Tokio executor"
)]
fn wait_for_metadata(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            view.catalog_active.is_none() && view.catalog_pending.is_none()
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "metadata must finish");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn metadata_tabs_load_real_indexes_and_keys_without_running_the_draft(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    for sql in [
        "CREATE TABLE parents (id INTEGER PRIMARY KEY)",
        "CREATE TABLE children (id INTEGER, parent_id INTEGER REFERENCES parents(id) ON DELETE CASCADE)",
        "CREATE INDEX child_parent_lookup ON children(parent_id)",
        "CREATE INDEX child_id_lookup ON children(id)",
    ] {
        submit(
            &backend,
            execution_command(
                open.connection,
                open.session,
                false,
                open.dialect,
                sql.into(),
                Vec::new(),
            ),
        )
        .expect("fixture DDL");
    }
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("SELECT 'unsaved';");
    let path = CatalogPath::for_relation(None, Some("main"), "children").expect("path");
    view.update(cx, |view, cx| {
        view.selected_path = Some(path.clone());
        view.panel = WorkspacePanel::Object;
        view.select_metadata_tab(ObjectTab::Indexes, cx);
    });
    wait_for_metadata(&view, cx);
    cx.update(|window, cx| window.focus(&view.read(cx).metadata_focus));
    cx.simulate_keystrokes("end");
    assert_eq!(view.read_with(cx, |view, _| view.metadata_selected), 1);
    cx.simulate_keystrokes("home");
    assert_eq!(view.read_with(cx, |view, _| view.metadata_selected), 0);
    view.update(cx, |view, cx| {
        {
            let cache = view.catalog_cache.read();
            assert!(
                cache
                    .indexes(&path)
                    .expect("indexes read")
                    .iter()
                    .any(|index| index.name == "child_parent_lookup")
            );
            let keys = cache.foreign_keys(&path).expect("keys read");
            assert_eq!(keys.len(), 1);
            assert_eq!(
                keys.first().expect("key").references.relation.relation(),
                Some("parents")
            );
        }
        view.select_metadata_tab(ObjectTab::Relations, cx);
        assert!(
            view.catalog_active.is_none(),
            "cached tab does not repeat introspection"
        );
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
        assert_eq!(view.draft_text(cx), "SELECT 'unsaved';");
        assert!(view.catalog_cache.read().constraints(&path).is_none());
        view.select_metadata_tab(ObjectTab::Constraints, cx);
        assert!(
            view.catalog_active.is_some(),
            "constraints are loaded only on demand"
        );
    });
    wait_for_metadata(&view, cx);
    view.update(cx, |view, cx| {
        let cache = view.catalog_cache.read();
        let constraints = cache
            .constraints(&path)
            .expect("constraints read through the bus");
        assert_eq!(constraints.len(), 1);
        let key = constraints.first().expect("foreign key");
        assert!(
            key.name.is_empty(),
            "SQLite does not invent a declaration name"
        );
        assert_eq!(key.fields, ["parent_id"]);
        assert_eq!(
            key.expression.as_deref(),
            Some("REFERENCES parents(id) ON DELETE CASCADE")
        );
        assert_eq!(view.draft_text(cx), "SELECT 'unsaved';");
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
    });
}

/// `Open Indexes` mène où sa phrase dit d'aller, et disparaît sans la capacité.
///
/// Relevé `229:8021` : la maquette place ce bouton sous « Unique indexes ». Le
/// code n'avait que la phrase « listed under Indexes », qui envoie chercher
/// sans mener — juste après avoir dit qu'on regardait au mauvais endroit. Le
/// test porte aussi sur l'absence : un bouton qui mène à un onglet qu'un driver
/// sans introspection d'index ne peut pas remplir promet une vue vide
/// ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
#[gpui::test]
fn depuis_les_contraintes_un_geste_mene_aux_index(cx: &mut TestAppContext) {
    use oxyn_catalog::{Relation, RelationKind};
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let path = CatalogPath::for_relation(None, Some("main"), "items").expect("path");
    view.update(cx, |view, cx| {
        view.capabilities
            .insert(Capabilities::CONSTRAINTS | Capabilities::INDEXES);
        view.selected_path = Some(path.clone());
        view.panel = WorkspacePanel::Object;
        view.catalog_cache
            .write()
            .set_relation(&path, Relation::new("items", RelationKind::Table))
            .expect("relation");
        view.select_metadata_tab(ObjectTab::Constraints, cx);
    });
    cx.run_until_parked();

    let bouton = cx
        .debug_bounds("constraints-open-indexes")
        .expect("le geste est proposé quand la capacité existe");
    cx.simulate_click(bouton.center(), Default::default());
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.object_tab,
            ObjectTab::Indexes,
            "le bouton doit mener là où sa phrase renvoie"
        );
    });
}

/// Sans introspection d'index, le geste n'est pas proposé du tout.
///
/// Une fenêtre neuve plutôt qu'une capacité retirée en cours de route :
/// `debug_bounds` rend ce que le dernier dessin a posé, si bien qu'une
/// transition testerait le harnais plutôt que la vue. Et c'est de toute façon
/// le cas réel — un driver sans `INDEXES` ne l'a jamais eue
/// ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
#[gpui::test]
fn sans_introspection_dindex_le_geste_nest_pas_propose(cx: &mut TestAppContext) {
    use oxyn_catalog::{Relation, RelationKind};
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let path = CatalogPath::for_relation(None, Some("main"), "items").expect("path");
    view.update(cx, |view, cx| {
        view.capabilities.insert(Capabilities::CONSTRAINTS);
        view.capabilities.remove(Capabilities::INDEXES);
        view.selected_path = Some(path.clone());
        view.panel = WorkspacePanel::Object;
        view.catalog_cache
            .write()
            .set_relation(&path, Relation::new("items", RelationKind::Table))
            .expect("relation");
        view.select_metadata_tab(ObjectTab::Constraints, cx);
    });
    cx.run_until_parked();

    assert!(
        cx.debug_bounds("constraints-open-indexes").is_none(),
        "mener vers un onglet qu'aucun driver ne peut remplir promet une vue vide"
    );
}

#[gpui::test]
fn constraints_are_read_only_and_the_selected_definition_can_be_copied(cx: &mut TestAppContext) {
    use oxyn_catalog::{Constraint, ConstraintKind, Relation, RelationKind};
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("SELECT 'unsaved';");
    let path = CatalogPath::for_relation(None, Some("main"), "items").expect("path");
    view.update(cx, |view, cx| {
        view.capabilities.insert(Capabilities::CONSTRAINTS);
        view.selected_path = Some(path.clone());
        view.panel = WorkspacePanel::Object;
        {
            let mut cache = view.catalog_cache.write();
            cache
                .set_relation(&path, Relation::new("items", RelationKind::Table))
                .expect("relation");
            cache
                .set_constraints(
                    &path,
                    vec![
                        Constraint::new("first", ConstraintKind::Check, vec![])
                            .with_expression("CHECK (id > 0)"),
                        Constraint::new("second", ConstraintKind::Unique, vec!["name".into()])
                            .with_expression("UNIQUE (name)"),
                    ],
                )
                .expect("constraints");
        }
        view.select_metadata_tab(ObjectTab::Constraints, cx);
        assert!(view.catalog_active.is_none());
    });
    cx.update(|window, cx| window.focus(&view.read(cx).metadata_focus));
    cx.simulate_keystrokes("end cmd-c");
    assert_eq!(view.read_with(cx, |view, _| view.metadata_selected), 1);
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("UNIQUE (name)".into())
    );
    view.update(cx, |view, cx| {
        assert_eq!(view.draft_text(cx), "SELECT 'unsaved';");
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
        view.capabilities.remove(Capabilities::CONSTRAINTS);
        view.select_metadata_tab(ObjectTab::Constraints, cx);
        assert!(view.catalog_active.is_none());
    });
}

#[gpui::test]
fn incoming_relationships_open_the_source_without_running_or_replacing_the_draft(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    for sql in [
        "CREATE TABLE incoming_parent(id INTEGER PRIMARY KEY)",
        "CREATE TABLE incoming_child(id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES incoming_parent(id))",
    ] {
        submit(
            &backend,
            execution_command(
                open.connection,
                open.session,
                false,
                open.dialect,
                sql.into(),
                Vec::new(),
            ),
        )
        .expect("fixture DDL");
    }
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("SELECT 'untouched';");
    let parent = CatalogPath::for_relation(None, Some("main"), "incoming_parent").expect("path");
    view.update(cx, |view, cx| {
        view.selected_path = Some(parent.clone());
        view.panel = WorkspacePanel::Object;
        view.select_metadata_tab(ObjectTab::IncomingRelations, cx);
    });
    wait_for_metadata(&view, cx);
    view.update(cx, |view, cx| {
        assert_eq!(
            view.catalog_cache
                .read()
                .incoming_foreign_keys(&parent)
                .expect("bus read")
                .len(),
            1
        );
        assert!(view.preview_active.is_none());
        assert_eq!(view.draft_text(cx), "SELECT 'untouched';");
    });
    cx.update(|window, cx| window.focus(&view.read(cx).metadata_focus));
    cx.simulate_keystrokes("enter");
    wait_for_metadata(&view, cx);
    wait_for_preview(&view, cx);
    view.update(cx, |view, cx| {
        assert_eq!(
            view.selected_path.as_ref().and_then(CatalogPath::relation),
            Some("incoming_child")
        );
        assert_eq!(view.object_tab, ObjectTab::Data);
        assert!(
            view.catalog_cache
                .read()
                .relation_summary(view.selected_path.as_ref().expect("source"))
                .is_some(),
            "source is discovered even without prior tree expansion"
        );
        assert!(
            view.preview_path.is_some(),
            "normal table preview is requested after discovery"
        );
        assert!(
            view.preview_result.is_some(),
            "the source preview is actually loaded"
        );
        assert!(view.console.read(cx).active.is_none());
        assert_eq!(view.draft_text(cx), "SELECT 'untouched';");
    });
}

#[gpui::test]
fn unavailable_metadata_never_dispatches_and_catalog_cancel_clears_queued_work(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    view.update(cx, |view, cx| {
        view.capabilities.remove(Capabilities::INDEXES);
        view.selected_path =
            Some(CatalogPath::for_relation(None, Some("main"), "items").expect("path"));
        view.select_metadata_tab(ObjectTab::Indexes, cx);
        assert!(view.catalog_active.is_none());
        let token = CancelToken::new();
        view.catalog_active = Some((CommandId::new(), token.clone()));
        view.catalog_scope = CatalogScope::Server;
        let requested = CatalogScope::Relation(view.selected_path.clone().expect("selection"));
        view.refresh_catalog(requested.clone(), cx);
        assert_eq!(view.catalog_pending, Some(requested));
        view.cancel_catalog(cx);
        assert!(view.catalog_pending.is_none());
        assert!(token.is_cancelled());
    });
}

/// **Un DDL lancé ailleurs rafraîchit le catalogue sans clic Refresh.**
///
/// La commande n'est jamais soumise par cette vue : c'est le signal reçu par
/// `on_exec_event` sur le bus d'événements, pas un appel que le test se
/// contenterait de rejouer, qui doit déclencher `refresh_catalog`.
#[gpui::test]
fn a_ddl_from_elsewhere_refreshes_the_catalog_without_a_click(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let submitter = backend.clone();
    let connection = open.connection;
    let session = open.session;
    let dialect = open.dialect;
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));

    // Like a workspace whose catalog tree is already open: a node never read
    // stays `Never`, never `Invalidated` (`CatalogCache`'s module docs).
    // Completion always settles `catalog_scope` back to `Server` (see
    // `refresh_catalog`), so `Server` is also the scope the auto-refresh
    // below will actually re-fetch — the same one the Refresh button uses.
    view.update(cx, |view, cx| {
        view.refresh_catalog(CatalogScope::Server, cx);
    });
    wait_for_metadata(&view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.catalog_scope, CatalogScope::Server);
        assert!(
            matches!(
                view.catalog_cache.read().freshness(&CatalogScope::Server),
                oxyn_catalog::Freshness::Fetched(_)
            ),
            "the initial refresh must have left the scope fresh"
        );
    });

    // A different actor on the same connection, out of band: this view never
    // dispatches this command itself.
    submit(
        &submitter,
        execution_command(
            connection,
            session,
            false,
            dialect,
            "CREATE TABLE probe (id INTEGER)".into(),
            Vec::new(),
        ),
    )
    .expect("fixture ddl");

    wait_for_metadata(&view, cx);
    view.read_with(cx, |view, _| {
        assert!(
            matches!(
                view.catalog_cache.read().freshness(&CatalogScope::Server),
                oxyn_catalog::Freshness::Fetched(_)
            ),
            "the workspace must have re-fetched on its own after the external DDL, \
             without this test ever calling refresh_catalog a second time"
        );
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "test harness waits for the independent executor"
)]
fn wait_for_result_pages(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, cx| {
            view.console.read(cx).active.is_none()
                && view.console.read(cx).page_active.is_none()
                && view.page_reads.is_empty()
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "query and local pages must complete"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn scrolling_to_spilled_rows_loads_local_pages_without_reexecuting_sql(cx: &mut TestAppContext) {
    let backend = Backend::temporary_with_memory_budget(512 * 1024).expect("bounded backend");
    let (backend, open) = connect_test_backend(backend);
    let mut events = backend.subscribe();
    let sql = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x FROM n";
    let mut command = execution_command(
        open.connection,
        open.session,
        false,
        open.dialect,
        sql.into(),
        Vec::new(),
    );
    if let Command::Execute { request, .. } = &mut command {
        request.limits.max_rows = Some(100_000);
    }
    let Outcome::Executed { result, buffer, .. } =
        submit(&backend, command).expect("large bounded result")
    else {
        panic!("result required")
    };
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    view.update(cx, |view, cx| {
        view.set_draft_text(sql, cx);
        view.console
            .update(cx, |console, _| console.displayed_result = Some(result));
        view.grid
            .update(cx, |grid, cx| grid.set_buffer(buffer.clone(), cx));
    });
    wait_for_result_pages(&view, cx);
    let target = (0..buffer.batch_count())
        .map(oxyn_data::BatchIndex::new)
        .find(|index| !buffer.is_resident(*index))
        .expect("fixture contains spilled pages");
    let row = buffer.batch_start(target).expect("page start");
    assert!(buffer.cached_batch(target).is_none());
    let grid = view.read_with(cx, |view, _| view.grid.clone());
    grid.update(cx, |grid, cx| grid.select_row(row, cx));
    wait_for_result_pages(&view, cx);
    let batch = buffer
        .cached_batch(target)
        .expect("visible spilled page was loaded");
    let value = oxyn_data::format_cell(&batch, 0, 0, &oxyn_data::FormatOptions::default());
    assert_eq!(value.text(), Some((row + 1).to_string().as_str()));
    assert!(buffer.resident_bytes() + buffer.cached_bytes() <= buffer.limits().memory_budget);
    let mut schema_events = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event.event, Event::SchemaReady { .. }) {
            schema_events += 1;
        }
    }
    assert_eq!(
        schema_events, 1,
        "scrolling must not start another SQL execution"
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "test harness waits for the independent value worker"
)]
fn wait_for_value(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            view.value_inspection
                .as_ref()
                .is_none_or(|value| value.active.is_none())
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "value inspection must complete"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn full_value_inspection_pages_real_data_with_keyboard_and_cancels_on_replacement(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let mut events = backend.subscribe();
    let Outcome::Executed { result, buffer, .. } = submit(
        &backend,
        execution_command(
            open.connection,
            open.session,
            false,
            open.dialect,
            "SELECT printf('%030000d',1) AS document, NULL AS absent, 'NULL' AS literal".into(),
            Vec::new(),
        ),
    )
    .expect("query") else {
        panic!("result")
    };
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    view.update(cx, |view, cx| {
        view.console
            .update(cx, |console, _| console.displayed_result = Some(result));
        view.grid.update(cx, |grid, cx| {
            grid.set_buffer(buffer, cx);
            grid.select_row(0, cx);
        });
        view.inspect_selected_value(cx);
    });
    wait_for_value(&view, cx);
    let first = view.read_with(cx, |view, _| {
        view.value_inspection
            .as_ref()
            .expect("inspection")
            .page
            .as_ref()
            .expect("page")
            .clone()
    });
    assert_eq!(first.text.len(), 16 * 1024);
    assert_eq!(first.total_bytes, 30_000);
    cx.update(|window, cx| window.focus(view.read(cx).value_buttons.get(1).expect("next button")));
    cx.simulate_keystrokes("enter");
    wait_for_value(&view, cx);
    view.read_with(cx, |view, cx| {
        let page = view
            .value_inspection
            .as_ref()
            .expect("inspection")
            .page
            .as_ref()
            .expect("second page");
        assert_eq!(page.offset, 16 * 1024);
        assert!(page.next_offset.is_none());
        assert!(page.text.ends_with('1'));
        assert!(
            view.console.read(cx).active.is_none(),
            "inspection does not execute SQL"
        );
    });
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |view, _| view.value_inspection.is_none()));
    view.update(cx, |view, cx| {
        view.inspected_column = 2;
        view.inspect_selected_value(cx);
        let cancel = view
            .value_inspection
            .as_ref()
            .expect("inspection")
            .active
            .as_ref()
            .expect("request")
            .1
            .clone();
        view.set_draft_text("SELECT 42", cx);
        view.execute(cx);
        assert!(view.value_inspection.is_none());
        assert!(cancel.is_cancelled());
    });
    wait_for_result_pages(&view, cx);
    assert!(view.read_with(cx, |view, _| view.value_inspection.is_none()));
    let mut executions = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event.event, Event::SchemaReady { .. }) {
            executions += 1;
        }
    }
    assert_eq!(
        executions, 2,
        "only the initial query and the explicit replacement execute"
    );
}

#[gpui::test]
fn inspector_width_changes_restore_wide_preference_without_reading_again(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.inspector_open && !view.compact_layout));
    cx.simulate_resize(gpui::size(px(1024.), px(768.)));
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.inspector_open, "wide preference is retained");
        assert!(!view.inspector_overlay, "compact inspector starts closed");
        assert!(view.page_reads.is_empty());
    });
    view.update(cx, |view, cx| {
        view.inspector_overlay = true;
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.inspector_open);
        assert!(!view.inspector_overlay);
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
        assert!(view.page_reads.is_empty());
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "test harness waits for the independent preference worker"
)]
fn wait_for_preferences(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            !matches!(
                view.preference_state,
                preferences::PreferenceSaveState::Saving
            )
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "preferences must finish saving"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    view.read_with(cx, |view, _| {
        assert!(
            matches!(
                view.preference_state,
                preferences::PreferenceSaveState::Saved
            ),
            "{:?}",
            view.preference_state
        )
    });
}

#[gpui::test]
fn reading_density_changes_geometry_and_persists_without_querying_again(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let saved_backend = backend.clone();
    let mut events = backend.subscribe();
    let Outcome::Executed { result, buffer, .. } = submit(
        &backend,
        execution_command(
            open.connection,
            open.session,
            false,
            open.dialect,
            "SELECT 7 AS n".into(),
            Vec::new(),
        ),
    )
    .expect("query") else {
        panic!("result")
    };
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    view.update(cx, |view, cx| {
        view.console
            .update(cx, |console, _| console.displayed_result = Some(result));
        view.grid
            .update(cx, |grid, cx| grid.set_buffer(buffer.clone(), cx));
        view.set_draft_text("SELECT 'keep draft'", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("grid-first-row")
            .expect("row geometry")
            .size
            .height,
        px(24.)
    );
    view.update(cx, |view, cx| {
        view.set_reading_density(oxyn_core::ReadingDensity::Comfortable, cx);
        view.set_reading_density(oxyn_core::ReadingDensity::Compact, cx);
        view.set_reading_density(oxyn_core::ReadingDensity::Comfortable, cx);
    });
    wait_for_preferences(&view, cx);
    assert_eq!(
        cx.debug_bounds("grid-first-row")
            .expect("comfortable row")
            .size
            .height,
        px(28.)
    );
    cx.simulate_keystrokes("cmd-shift-l");
    wait_for_preferences(&view, cx);
    let revision = view.read_with(cx, |view, _| view.preference_revision);
    cx.simulate_resize(gpui::size(px(1024.), px(768.)));
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.preference_revision, revision,
            "viewport does not persist a layout override"
        );
        assert_eq!(
            Theme::of(cx).reading_density,
            oxyn_core::ReadingDensity::Comfortable
        );
        assert_eq!(Theme::of(cx).mode, ThemeMode::Light);
        assert_eq!(view.draft_text(cx), "SELECT 'keep draft'");
        assert!(std::sync::Arc::ptr_eq(
            view.grid.read(cx).state().buffer().expect("same buffer"),
            &buffer
        ));
        assert!(view.console.read(cx).active.is_none());
    });
    let loaded = submit(
        &saved_backend,
        Command::ReadWorkspacePreferences {
            workspace: saved_backend.workspace_id(),
        },
    )
    .expect("read saved preferences");
    let Outcome::WorkspacePreferences { snapshot } = loaded else {
        panic!("snapshot")
    };
    assert_eq!(
        snapshot.preferences.reading_density,
        oxyn_core::ReadingDensity::Comfortable
    );
    assert_eq!(
        snapshot.preferences.appearance,
        oxyn_core::Appearance::Light
    );
    let mut queries = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event.event, Event::SchemaReady { .. }) {
            queries += 1;
        }
    }
    assert_eq!(queries, 1, "preference changes never execute SQL");
}

#[gpui::test]
fn inspector_drag_is_delivered_and_saves_only_when_released(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let saved_backend = backend.clone();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("inspector-resize").expect("resize handle");
    assert_eq!(bounds.size.width, px(8.));
    let origin = bounds.center();
    let target = point(origin.x - px(60.), origin.y);
    let revision = view.read_with(cx, |view, _| view.preference_revision);
    cx.simulate_mouse_down(origin, gpui::MouseButton::Left, Default::default());
    assert!(view.read_with(cx, |view, _| view.inspector_drag.is_some()));
    cx.simulate_mouse_move(target, gpui::MouseButton::Left, Default::default());
    view.read_with(cx, |view, _| {
        assert_eq!(view.inspector_width, 340);
        assert_eq!(
            view.preference_revision, revision,
            "drag frames do not write preferences"
        );
    });
    cx.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
    wait_for_preferences(&view, cx);
    cx.simulate_keystrokes("left");
    wait_for_preferences(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.inspector_width), 350);
    cx.simulate_keystrokes("home");
    wait_for_preferences(&view, cx);
    let loaded = submit(
        &saved_backend,
        Command::ReadWorkspacePreferences {
            workspace: saved_backend.workspace_id(),
        },
    )
    .expect("saved width");
    assert!(
        matches!(loaded, Outcome::WorkspacePreferences { snapshot } if snapshot.preferences.inspector_width == 280)
    );
    assert!(
        view.read_with(cx, |view, cx| view.console.read(cx).active.is_none()
            && view.preview_active.is_none())
    );
}

#[gpui::test]
fn collapsing_sidebar_keeps_table_context_and_settings_take_keyboard_focus(
    cx: &mut TestAppContext,
) {
    let (backend, open) = preview_fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    view.update(cx, |view, cx| {
        view.select_object(
            CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table"),
            cx,
        )
    });
    wait_for_preview(&view, cx);
    cx.update(|window, cx| window.focus(&view.read(cx).preview_grid.read(cx).focus_handle(cx)));
    let before = view.read_with(cx, |view, _| view.preview_result);
    cx.simulate_keystrokes("cmd-b");
    wait_for_preferences(&view, cx);
    view.read_with(cx, |view, cx| {
        assert!(view.sidebar_collapsed);
        assert_eq!(view.panel, WorkspacePanel::Object);
        assert_eq!(view.preview_result, before);
        assert!(view.console.read(cx).active.is_none());
        assert!(view.preview_active.is_none());
    });
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert_eq!(view.read(cx).panel, WorkspacePanel::Preferences);
        assert!(view.read(cx).preferences_focus.is_focused(window));
    });
}

/// **Where you were comes back; nothing you were reading does.**
///
/// UX-SPEC promises a restored object tab that keeps its location « sans charger
/// ses données avant une reconnexion explicite ». The event count is the half
/// that matters: a restoration that reloaded the preview would put the first
/// request of the session on the wire for a window nobody has looked at yet.
#[gpui::test]
fn a_restored_location_and_its_sub_tab_come_back_without_reading_anything(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let path = CatalogPath::for_relation(None, Some("main"), "preview_rows").expect("table path");
    {
        let (view, cx) = cx.add_window_view({
            let backend = backend.clone();
            let open = open.clone();
            |_, cx| Workspace::new(backend, open, cx)
        });
        let tree = view.read_with(cx, |view, _| view.catalog.clone().expect("catalog"));
        tree.update(cx, |tree, cx| tree.select(path.clone(), cx));
        wait_for_preview(&view, cx);
        view.update(cx, |view, cx| {
            view.select_metadata_tab(ObjectTab::Indexes, cx);
        });
        wait_for_metadata(&view, cx);
        wait_for_preferences(&view, cx);
    }

    // The location reached the store, not only the in-memory snapshot.
    let saved = submit(
        &backend,
        Command::ReadWorkspacePreferences {
            workspace: backend.workspace_id(),
        },
    )
    .expect("preferences read");
    let Outcome::WorkspacePreferences { snapshot } = saved else {
        panic!("preferences outcome required")
    };
    let location = snapshot
        .preferences
        .object_location
        .expect("the browsing location is saved with the other preferences");
    assert_eq!(location.path, path.to_string());
    assert_eq!(location.section, oxyn_core::ObjectSection::Indexes);
    assert_eq!(location.connection, open.connection);

    let mut events = backend.subscribe();
    let (restored, cx) = cx.add_window_view({
        let backend = backend.clone();
        let open = open.clone();
        |_, cx| Workspace::new(backend, open, cx)
    });
    cx.run_until_parked();
    restored.read_with(cx, |view, cx| {
        assert_eq!(view.selected_path.as_ref(), Some(&path));
        assert_eq!(view.object_tab, ObjectTab::Indexes);
        assert!(view.preview_active.is_none(), "no preview was started");
        assert!(view.preview_path.is_none(), "no preview was even requested");
        assert!(view.catalog_active.is_none(), "no catalog read was started");
        assert!(view.console.read(cx).active.is_none());
        assert_eq!(
            view.panel,
            WorkspacePanel::Sql,
            "restoring a location does not open the object panel"
        );
    });
    let mut reads = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.event,
            Event::SchemaReady { .. } | Event::Completed { .. }
        ) {
            reads += 1;
        }
    }
    assert_eq!(reads, 0, "restoring a location must execute nothing");
}

/// **An object dropped between two sessions is said, not silently forgotten.**
#[gpui::test]
fn a_restored_object_that_vanished_is_explained_and_never_erased(cx: &mut TestAppContext) {
    let (backend, open) = preview_fixture();
    let gone =
        CatalogPath::for_relation(None, Some("main"), "dropped_between_sessions").expect("path");
    {
        let (view, cx) = cx.add_window_view({
            let backend = backend.clone();
            let open = open.clone();
            |_, cx| Workspace::new(backend, open, cx)
        });
        view.update(cx, |view, cx| view.select_object(gone.clone(), cx));
        wait_for_preferences(&view, cx);
    }
    let (restored, cx) = cx.add_window_view({
        let backend = backend.clone();
        let open = open.clone();
        |_, cx| Workspace::new(backend, open, cx)
    });
    cx.run_until_parked();
    restored.read_with(cx, |view, _| {
        assert_eq!(view.selected_path.as_ref(), Some(&gone));
        assert!(
            view.console_notice.is_none(),
            "an unread catalog is not evidence of a missing object"
        );
    });
    // Listing the schema that would hold it is what turns « not read » into
    // « not there » — the workspace never starts this read on its own.
    let namespace = gone.parent().expect("a relation has a container");
    restored.update(cx, |view, cx| {
        view.refresh_catalog(CatalogScope::Namespace(namespace), cx);
    });
    wait_for_metadata(&restored, cx);
    restored.read_with(cx, |view, cx| {
        assert_eq!(
            view.selected_path.as_ref(),
            Some(&gone),
            "the location is kept: the breadcrumb is what gives the notice a subject"
        );
        let notice = view
            .console_notice
            .clone()
            .expect("a missing object is explained, not shown as an empty tab");
        assert!(notice.contains("dropped_between_sessions"));
        assert!(view.preview_active.is_none());
        assert!(view.console.read(cx).active.is_none());
    });
}

/// **A very long object name costs its own location, never the preferences.**
#[gpui::test]
fn an_object_name_too_long_to_store_still_saves_every_other_preference(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let long = "n".repeat(oxyn_core::ObjectLocation::MAX_PATH_BYTES);
    let path = CatalogPath::for_relation(None, Some("main"), long).expect("legal table name");
    assert!(path.to_string().len() > oxyn_core::ObjectLocation::MAX_PATH_BYTES);
    let (view, cx) = cx.add_window_view({
        let backend = backend.clone();
        let open = open.clone();
        |_, cx| Workspace::new(backend, open, cx)
    });
    view.update(cx, |view, cx| {
        view.select_object(path.clone(), cx);
        view.sidebar_collapsed = true;
        view.persist_preferences(cx);
    });
    // `wait_for_preferences` asserts the save succeeded rather than failed.
    wait_for_preferences(&view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.selected_path.as_ref(), Some(&path));
    });
    let saved = submit(
        &backend,
        Command::ReadWorkspacePreferences {
            workspace: backend.workspace_id(),
        },
    )
    .expect("preferences read");
    let Outcome::WorkspacePreferences { snapshot } = saved else {
        panic!("preferences outcome required")
    };
    assert!(
        snapshot.preferences.object_location.is_none(),
        "an unstorable location is dropped"
    );
    assert!(
        snapshot.preferences.sidebar_collapsed,
        "the preference saved alongside it is unaffected"
    );
}
