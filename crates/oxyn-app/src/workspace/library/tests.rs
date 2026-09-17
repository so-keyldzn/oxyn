//! Delivered keyboard events and real local command responses.

use super::*;
use crate::workspace::library::filters::merge_history_connections;
use crate::workspace::tests::{connected_workspace, submit};
use gpui::{TestAppContext, VisualTestContext};
use oxyn_core::{QueryDocumentUpdate, QueryLanguage};

#[expect(
    clippy::disallowed_methods,
    reason = "test harness polls the separate Tokio executor with a wall-clock deadline"
)]
fn wait_for_library(view: &Entity<QueryLibrary>, cx: &mut VisualTestContext) {
    let start = std::time::Instant::now();
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            view.request.is_none()
                && view.detail_request.is_none()
                && view.result_request.is_none()
                && view.connections_request.is_none()
        }) {
            return;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "library did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Ranks are what the menu emits, so merging must never reshuffle them.
#[test]
fn merging_history_connections_appends_and_marks_without_moving_ranks() {
    let saved = ConnectionId::new();
    let created_since = ConnectionId::new();
    let deleted = ConnectionId::new();
    let mut known = vec![FilterConnection {
        id: saved,
        name: "Saved".into(),
        in_workspace: true,
    }];
    merge_history_connections(
        &mut known,
        vec![
            HistoryConnectionSummary {
                connection: created_since,
                name: Some("Created since".into()),
                last_entry: 3,
                in_workspace: true,
            },
            HistoryConnectionSummary {
                connection: saved,
                name: Some("An older name".into()),
                last_entry: 2,
                in_workspace: true,
            },
            HistoryConnectionSummary {
                connection: deleted,
                name: None,
                last_entry: 1,
                in_workspace: false,
            },
        ],
    );
    let labels = connection_labels(&known);
    assert_eq!(
        labels
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect::<Vec<_>>(),
        vec![
            "All connections",
            "Saved",
            "Created since",
            "Unnamed connection · not in this workspace",
        ],
        "the rank a menu already showed keeps pointing at the same connection"
    );
    assert_eq!(known[0].id, saved, "the saved name wins over the older one");
}

/// Registers and opens a second local connection, entirely through the bus.
fn open_named_connection(backend: &Backend, name: &str) -> (ConnectionId, oxyn_core::SessionId) {
    let mut config = oxyn_core::ConnectionConfig::new(name, oxyn_core::DriverId::sqlite())
        .with_environment(oxyn_core::Environment::Local);
    config.params.insert("path".into(), ":memory:".into());
    let connection = config.id;
    submit(
        backend,
        Command::CreateConnection {
            config: Box::new(config),
        },
    )
    .expect("registered connection");
    let Outcome::Connected { session, .. } =
        submit(backend, Command::Connect { connection }).expect("opened connection")
    else {
        panic!("a local SQLite connection opens without approval")
    };
    (connection, session)
}

/// Chooses a rank in the connection menu the way a user does: with the keyboard.
fn choose_connection(rank: usize, view: &Entity<QueryLibrary>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.focus(&view.read(cx).connection_select.read(cx).focus_handle(cx));
    });
    cx.run_until_parked();
    // "home" first: the menu opens on the current choice, so counting downs
    // from an unknown rank would land somewhere else on the second call.
    cx.simulate_keystrokes("space");
    cx.simulate_keystrokes("home");
    for _ in 0..rank {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("enter");
}

/// The defect this path exists for: `query_history` has no foreign key, so the
/// entries of a deleted connection survive — but a menu built from the saved
/// connections alone could never name it, which left them unreachable.
#[gpui::test]
fn a_deleted_connection_stays_choosable_in_the_history_filter(cx: &mut TestAppContext) {
    let (backend, removed) = connected_workspace();
    submit(
        &backend,
        crate::workspace::execution_command(
            removed.connection,
            removed.session,
            false,
            removed.dialect,
            "SELECT 'ran on the removed connection'".into(),
            Vec::new(),
        ),
    )
    .expect("execution on the connection about to disappear");
    let (kept, kept_session) = open_named_connection(&backend, "Kept connection");
    submit(
        &backend,
        crate::workspace::execution_command(
            kept,
            kept_session,
            false,
            removed.dialect,
            "SELECT 'ran on the kept connection'".into(),
            Vec::new(),
        ),
    )
    .expect("execution on the connection that stays");
    submit(
        &backend,
        Command::DeleteConnection {
            connection: removed.connection,
        },
    )
    .expect("connection removed from the workspace");
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| {
        QueryLibrary::new(backend, Some((kept, "Kept connection".into())), cx)
    });
    view.update(cx, |view, cx| view.reload(cx));
    wait_for_library(&view, cx);
    let labels = view.read_with(cx, |view, _| connection_labels(&view.connections));
    assert_eq!(
        labels.len(),
        3,
        "every connection, the kept one, and the one only history remembers: {labels:?}"
    );
    assert_eq!(labels[1].as_ref(), "Kept connection");
    assert!(
        labels[2].as_ref().starts_with("Workspace interaction test")
            && labels[2].as_ref().contains("not in this workspace"),
        "a connection this workspace no longer holds must say so: {labels:?}"
    );
    choose_connection(2, &view, cx);
    wait_for_library(&view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.rows.len(),
            1,
            "only what ran on the deleted connection: {:?}",
            view.rows.iter().map(|row| &row.title).collect::<Vec<_>>()
        );
        assert!(view.rows[0].title.contains("removed connection"));
        assert_eq!(view.rows[0].connection, "Workspace interaction test");
    });
    choose_connection(0, &view, cx);
    wait_for_library(&view, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.rows.len()),
        2,
        "returning to every connection restores the unfiltered page"
    );
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(
                event.event,
                oxyn_core::Event::SchemaReady { .. }
                    | oxyn_core::Event::BatchReady { .. }
                    | oxyn_core::Event::Progress { .. }
                    | oxyn_core::Event::Completed { .. }
            ),
            "browsing history reaches no database: {:?}",
            event.event
        );
    }
}

#[gpui::test]
/// La garantie d'[I-13](../../../../CLAUDE.md#i-13) se lit sur l'Historique, et
/// nulle part ailleurs.
///
/// Relevé `268:36866`. Le texte existait, noyé en fin d'une ligne grise ; ce
/// qu'il permet — parcourir son historique sans craindre qu'un clic rejoue une
/// écriture — ne se lit pas dans une note de bas de ligne. Et il n'a rien à
/// faire sur les requêtes enregistrées ni les résultats retenus : rien n'y a
/// jamais été écrit, l'avertissement y inquiéterait sans objet.
#[gpui::test]
fn le_bandeau_des_ecritures_ambigues_ne_vit_que_sur_lhistorique(cx: &mut TestAppContext) {
    let backend = Backend::open_temporary().expect("backend");
    let (view, cx) = cx.add_window_view(|_, cx| QueryLibrary::new(backend, None, cx));
    wait_for_library(&view, cx);

    let bandeau = |vue: &Entity<QueryLibrary>, cx: &mut VisualTestContext| {
        vue.update(cx, |vue, cx| vue.ambiguous_writes_banner(cx).is_some())
    };

    assert!(
        bandeau(&view, cx),
        "l'onglet Historique liste des exécutions réelles : la garantie s'y lit"
    );

    for onglet in [Tab::Saved, Tab::Results] {
        view.update(cx, |vue, cx| vue.activate(Action::Tab(onglet), cx));
        cx.run_until_parked();
        assert!(
            !bandeau(&view, cx),
            "{onglet:?} ne liste aucune écriture : l'avertissement y serait sans objet"
        );
    }

    view.update(cx, |vue, cx| vue.activate(Action::Tab(Tab::History), cx));
    cx.run_until_parked();
    assert!(bandeau(&view, cx), "et il revient avec l'Historique");
}

#[gpui::test]
fn saved_and_working_copies_are_inspected_without_modification(cx: &mut TestAppContext) {
    let backend = Backend::open_temporary().expect("backend");
    let workspace = backend.workspace_id();
    let mut update = QueryDocumentUpdate {
        expected_revision: None,
        document: DocumentId::new(),
        revision: 1,
        title: "Saved report".into(),
        language: QueryLanguage::SQL,
        text: "SELECT 'saved'".into(),
        connection: None,
        save_named: true,
        is_open: true,
        provenance: None,
    };
    submit(
        &backend,
        Command::SaveQueryDocument {
            workspace,
            update: Box::new(update.clone()),
        },
    )
    .expect("saved");
    update.revision = 2;
    update.save_named = false;
    update.text = "SELECT 'draft'".into();
    submit(
        &backend,
        Command::SaveQueryDocument {
            workspace,
            update: Box::new(update.clone()),
        },
    )
    .expect("draft");
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = QueryLibrary::new(backend, None, cx);
        view.activate(Action::Tab(Tab::Saved), cx);
        window.focus(&view.focus);
        view
    });
    wait_for_library(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.rows.len()), 1);
    cx.simulate_keystrokes("down");
    wait_for_library(&view, cx);
    assert_eq!(
        view.read_with(cx, |view, cx| view.reader.read(cx).text()),
        "SELECT 'saved'"
    );
    view.update(cx, |view, cx| view.activate(Action::WorkingCopy, cx));
    assert_eq!(
        view.read_with(cx, |view, cx| view.reader.read(cx).text()),
        "SELECT 'draft'"
    );
    cx.update(|window, cx| window.focus(&view.read(cx).reader.read(cx).focus_handle(cx)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("DELETE FROM important");
    cx.simulate_keystrokes("cmd-enter");
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        view.read_with(cx, |view, cx| view.reader.read(cx).text()),
        "SELECT 'draft'"
    );
    view.update(cx, |view, cx| view.activate(Action::Copy, cx));
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("SELECT 'draft'")
        )
    });
}

#[gpui::test]
fn stale_lists_and_cancelled_inspections_cannot_replace_current_state(cx: &mut TestAppContext) {
    let backend = Backend::open_temporary().expect("backend");
    for title in ["First report", "Second report"] {
        submit(
            &backend,
            Command::SaveQueryDocument {
                workspace: backend.workspace_id(),
                update: Box::new(QueryDocumentUpdate {
                    expected_revision: None,
                    document: DocumentId::new(),
                    revision: 1,
                    title: title.into(),
                    language: QueryLanguage::SQL,
                    text: "SELECT 1".into(),
                    connection: None,
                    save_named: true,
                    is_open: false,
                    provenance: None,
                }),
            },
        )
        .expect("saved");
    }
    let (view, cx) = cx.add_window_view(|_, cx| QueryLibrary::new(backend, None, cx));
    view.update(cx, |view, cx| {
        view.tab = Tab::Saved;
        view.search
            .update(cx, |search, cx| search.set_text("First".into(), cx));
        view.reload(cx);
        view.search
            .update(cx, |search, cx| search.set_text("Second".into(), cx));
        view.reload(cx);
    });
    wait_for_library(&view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].title, "Second report");
    });
    view.update(cx, |view, cx| {
        view.inspect(0, cx);
        view.cancel(cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.detail.is_none());
        assert_eq!(view.reader.read(cx).text(), "");
        assert_eq!(view.detail_notice, "Inspection cancelled.");
    });
}

#[gpui::test]
fn history_and_recent_results_read_the_same_execution_without_replaying_it(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let command = crate::workspace::execution_command(
        open.connection,
        open.session,
        false,
        open.dialect,
        "SELECT 73".into(),
        Vec::new(),
    );
    submit(&backend, command).expect("execution");
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = QueryLibrary::new(backend, None, cx);
        view.reload(cx);
        window.focus(&view.focus);
        view
    });
    wait_for_library(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.rows.len()), 1);
    cx.simulate_keystrokes("down");
    wait_for_library(&view, cx);
    assert_eq!(
        view.read_with(cx, |view, cx| view.reader.read(cx).text()),
        "SELECT 73"
    );
    view.update(cx, |view, cx| view.activate(Action::Tab(Tab::Results), cx));
    wait_for_library(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.rows.len()), 1);
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}

#[gpui::test]
fn workspace_navigation_preserves_the_current_editor_and_result(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let outcome = submit(
        &backend,
        crate::workspace::execution_command(
            open.connection,
            open.session,
            false,
            open.dialect,
            "SELECT 73".into(),
            Vec::new(),
        ),
    )
    .expect("execute");
    let Outcome::Executed { result, buffer, .. } = outcome else {
        panic!("executed result");
    };
    let mut events = backend.subscribe();
    let (workspace, cx) =
        cx.add_window_view(|_, cx| crate::workspace::Workspace::new(backend, open, cx));
    workspace.update(cx, |workspace, cx| {
        workspace.set_draft_text("SELECT 'unsaved work'", cx);
        workspace
            .grid
            .update(cx, |grid, cx| grid.set_buffer(buffer, cx));
        workspace
            .console
            .update(cx, |console, _| console.last_result = Some(result));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-h");
    let library = workspace.read_with(cx, |workspace, _| workspace.library.clone());
    wait_for_library(&library, cx);
    cx.simulate_keystrokes("down");
    wait_for_library(&library, cx);
    assert_eq!(
        library.read_with(cx, |library, cx| library.reader.read(cx).text()),
        "SELECT 73"
    );
    cx.simulate_keystrokes("cmd-j");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.panel,
            crate::workspace::layout::WorkspacePanel::Sql
        );
        assert_eq!(workspace.draft_text(cx), "SELECT 'unsaved work'");
        assert_eq!(workspace.console.read(cx).last_result, Some(result));
    });
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "bounded harness waits for the real backend runtime"
)]
fn settle_open(view: &Entity<crate::workspace::Workspace>, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, cx| {
            view.console_attempt.is_none()
                && view.consoles.iter().all(|console| {
                    console.read(cx).save_active.is_none()
                        && console.read(cx).draft_pending.is_none()
                })
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "opening did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn history_opens_a_copy_without_running_it_or_replacing_the_existing_draft(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    submit(
        &backend,
        crate::workspace::execution_command(
            open.connection,
            open.session,
            false,
            open.dialect,
            "SELECT 81".into(),
            Vec::new(),
        ),
    )
    .expect("history fixture");
    let mut events = backend.subscribe();
    let (view, cx) =
        cx.add_window_view(|_, cx| crate::workspace::Workspace::new(backend, open, cx));
    cx.run_until_parked();
    cx.simulate_input("SELECT 'original draft'");
    let original = view.read_with(cx, |view, _| view.console.clone());
    cx.simulate_keystrokes("cmd-shift-h");
    let library = view.read_with(cx, |view, _| view.library.clone());
    wait_for_library(&library, cx);
    library.update(cx, |library, cx| library.inspect(0, cx));
    wait_for_library(&library, cx);
    library.update(cx, |library, cx| library.activate(Action::OpenCopy, cx));
    settle_open(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.consoles.len(), 2);
        assert_ne!(view.console, original);
        assert_eq!(view.editor.read(cx).text(), "SELECT 81");
        assert!(view.console.read(cx).active.is_none());
        assert!(view.console.read(cx).last_result.is_none());
        assert_eq!(
            original.read(cx).editor.read(cx).text(),
            "SELECT 'original draft'"
        );
    });
    library.update(cx, |library, cx| {
        if let Some(Detail::History(entry)) = &mut library.detail {
            entry.record.error_class = Some(oxyn_core::ErrorClass::Ambiguous);
        }
        library.activate(Action::OpenCopy, cx);
    });
    settle_open(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.consoles.len()), 2);
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}

#[gpui::test]
fn resuming_a_saved_query_preserves_its_working_copy_identity_and_existing_console(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let workspace = backend.workspace_id();
    let document = DocumentId::new();
    let mut update = QueryDocumentUpdate {
        expected_revision: None,
        document,
        revision: 1,
        title: "Report.sql".into(),
        language: QueryLanguage::SQL,
        text: "SELECT 'saved'".into(),
        connection: Some(open.connection),
        save_named: true,
        is_open: false,
        provenance: None,
    };
    submit(
        &backend,
        Command::SaveQueryDocument {
            workspace,
            update: Box::new(update.clone()),
        },
    )
    .expect("named copy");
    update.revision = 2;
    update.save_named = false;
    update.text = "SELECT 'working'".into();
    submit(
        &backend,
        Command::SaveQueryDocument {
            workspace,
            update: Box::new(update),
        },
    )
    .expect("working copy");
    let (view, cx) =
        cx.add_window_view(|_, cx| crate::workspace::Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    let library = view.read_with(cx, |view, _| view.library.clone());
    library.update(cx, |library, cx| {
        library.activate(Action::Tab(Tab::Saved), cx)
    });
    wait_for_library(&library, cx);
    library.update(cx, |library, cx| library.inspect(0, cx));
    wait_for_library(&library, cx);
    library.update(cx, |library, cx| library.activate(Action::EditOriginal, cx));
    settle_open(&view, cx);
    let resumed = view.read_with(cx, |view, cx| {
        assert_eq!(view.console.read(cx).id, document);
        assert_eq!(view.editor.read(cx).text(), "SELECT 'working'");
        assert!(view.console.read(cx).dirty);
        view.console.clone()
    });
    resumed.update(cx, |console, cx| {
        console
            .editor
            .update(cx, |editor, cx| editor.set_text("SELECT 'new edit'", cx))
    });
    library.update(cx, |library, cx| library.activate(Action::EditOriginal, cx));
    settle_open(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.consoles.len(), 2);
        assert_eq!(view.console, resumed);
        assert_eq!(view.editor.read(cx).text(), "SELECT 'new edit'");
    });
    cx.simulate_keystrokes("cmd-s");
    settle_open(&view, cx);
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace,
            document,
        },
    )
    .expect("stored original");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.saved_content.as_deref() == Some("SELECT 'new edit'") && document.language == QueryLanguage::SQL)
    );
}

#[gpui::test]
fn a_query_from_another_connection_is_opened_as_a_new_copy(cx: &mut TestAppContext) {
    let (backend, original_connection) = connected_workspace();
    let document = DocumentId::new();
    submit(
        &backend,
        Command::SaveQueryDocument {
            workspace: backend.workspace_id(),
            update: Box::new(QueryDocumentUpdate {
                expected_revision: None,
                document,
                revision: 1,
                title: "Original.sql".into(),
                language: QueryLanguage::SQL,
                text: "SELECT 'original'".into(),
                connection: Some(original_connection.connection),
                save_named: true,
                is_open: false,
                provenance: None,
            }),
        },
    )
    .expect("original query");
    let target = backend
        .connect(
            oxyn_ui::ConnectionDraft {
                driver: "sqlite".into(),
                name: "Destination".into(),
                environment: oxyn_core::Environment::Local,
                values: [("path".into(), ":memory:".into())].into_iter().collect(),
                secrets: Default::default(),
            },
            CancelToken::new(),
        )
        .blocking_recv()
        .expect("response")
        .expect("target connection");
    let crate::backend::ConnectionResponse::Open(target) = target else {
        panic!("local connection");
    };
    let target_id = target.connection;
    let (view, cx) =
        cx.add_window_view(|_, cx| crate::workspace::Workspace::new(backend.clone(), target, cx));
    cx.run_until_parked();
    let library = view.read_with(cx, |view, _| view.library.clone());
    library.update(cx, |library, cx| {
        library.activate(Action::Tab(Tab::Saved), cx)
    });
    wait_for_library(&library, cx);
    library.update(cx, |library, cx| library.inspect(0, cx));
    wait_for_library(&library, cx);
    assert!(!library.read_with(cx, |library, _| library.can_edit_original()));
    library.update(cx, |library, cx| library.activate(Action::OpenCopy, cx));
    settle_open(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_ne!(view.console.read(cx).id, document);
        assert_eq!(
            view.console
                .read(cx)
                .open
                .as_ref()
                .expect("connected")
                .connection,
            target_id
        );
        assert_eq!(view.editor.read(cx).text(), "SELECT 'original'");
    });
    cx.simulate_keystrokes("cmd-s");
    settle_open(&view, cx);
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("original remains");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.connection == Some(original_connection.connection) && document.saved_content.as_deref() == Some("SELECT 'original'"))
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "bounded test harness polling result page and export workers"
)]
fn settle_retained(
    view: &Entity<crate::workspace::console::QueryConsole>,
    cx: &mut VisualTestContext,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            view.page_active.is_none() && view.export_active.is_none()
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "retained result did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn reopening_scrolls_existing_spill_pages_and_exports_without_reexecuting(cx: &mut TestAppContext) {
    let backend = Backend::temporary_with_memory_budget(512 * 1024).expect("bounded backend");
    let (backend, open) = crate::workspace::tests::connect_test_backend(backend);
    let sql = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT x FROM n";
    let mut command = crate::workspace::execution_command(
        open.connection,
        open.session,
        false,
        open.dialect,
        sql.into(),
        Vec::new(),
    );
    if let Command::Execute { request, .. } = &mut command {
        request.limits.max_rows = Some(200_000);
    }
    let Outcome::Executed { result, buffer, .. } =
        submit(&backend, command).expect("fixture query")
    else {
        panic!("result");
    };
    assert!(!buffer.stats().truncated);
    let mut events = backend.subscribe();
    let (library, cx) = cx.add_window_view(|_, cx| {
        QueryLibrary::new(backend, Some((open.connection, "Fixture".into())), cx)
    });
    library.update(cx, |view, cx| view.reload(cx));
    wait_for_library(&library, cx);
    library.update(cx, |view, cx| view.inspect(0, cx));
    wait_for_library(&library, cx);
    library.update(cx, |view, cx| view.activate(Action::OpenResult, cx));
    wait_for_library(&library, cx);
    let retained = library.read_with(cx, |view, _| {
        view.retained.clone().expect("retained viewer")
    });
    retained.read_with(cx, |view, cx| {
        assert_eq!(view.displayed_result, Some(result));
        assert!(std::sync::Arc::ptr_eq(
            view.grid.read(cx).state().buffer().expect("buffer"),
            &buffer
        ));
        assert!(view.export.read(cx).is_result_ready());
    });
    retained.update(cx, |view, cx| {
        view.grid.update(cx, |grid, cx| {
            grid.select_row(99_999, cx);
            grid.request_row_page(99_999, cx);
        })
    });
    settle_retained(&retained, cx);
    assert!(oxyn_ui::data_grid::row_batch(&buffer, 99_999).is_some());
    let destination =
        std::env::temp_dir().join(format!("oxyn-retained-{}.csv", oxyn_core::ResultId::new()));
    retained.update(cx, |view, cx| {
        view.start_export(
            result,
            oxyn_core::ExportFormat::Csv,
            destination.clone(),
            cx,
        )
    });
    library.update(cx, |view, cx| view.activate(Action::BackToLibrary, cx));
    settle_retained(&retained, cx);
    let csv = std::fs::read_to_string(&destination).expect("export");
    assert!(csv.lines().last().is_some_and(|line| line == "100000"));
    std::fs::remove_file(destination).expect("cleanup");
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}

#[gpui::test]
fn expired_results_are_reported_and_truncated_results_cannot_be_exported(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let mut command = crate::workspace::execution_command(
        open.connection,
        open.session,
        false,
        open.dialect,
        "SELECT 1 UNION ALL SELECT 2".into(),
        Vec::new(),
    );
    if let Command::Execute { request, .. } = &mut command {
        request.limits.max_rows = Some(1);
    }
    submit(&backend, command).expect("truncated result");
    let mut events = backend.subscribe();
    let (library, cx) = cx.add_window_view(|_, cx| {
        QueryLibrary::new(backend, Some((open.connection, "Fixture".into())), cx)
    });
    library.update(cx, |view, cx| view.reload(cx));
    wait_for_library(&library, cx);
    library.update(cx, |view, cx| view.inspect(0, cx));
    wait_for_library(&library, cx);
    library.update(cx, |view, cx| view.activate(Action::OpenResult, cx));
    wait_for_library(&library, cx);
    library.read_with(cx, |view, cx| {
        assert!(view.retained_notice.contains("Truncated"));
        let retained = view.retained.as_ref().expect("viewer").read(cx);
        assert!(!retained.export.read(cx).is_result_ready());
        assert!(retained.last_result.is_none());
    });
    library.update(cx, |view, cx| {
        view.activate(Action::CloseRetained, cx);
        if let Some(Detail::History(entry)) = &mut view.detail {
            entry.record.result = Some(oxyn_core::ResultId::new());
        }
        view.activate(Action::OpenResult, cx);
    });
    wait_for_library(&library, cx);
    library.read_with(cx, |view, _| {
        assert!(view.retained.is_none());
        assert!(!view.show_retained);
        assert!(view.detail_notice.contains("unavailable"));
    });
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}
