//! Real SQLite sessions with GPUI tab switching, including hidden responses and close.

use super::*;
use crate::workspace::tests::{connected_workspace, submit};
use gpui::{TestAppContext, VisualTestContext};

#[expect(
    clippy::disallowed_methods,
    reason = "bounded test harness polling the separate Tokio executor"
)]
fn settle(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        // Le brouillon part au **repos de frappe**, pas à la touche
        // ([ADR-0024](../../../../docs/adr/0024-autosauvegarde-au-repos-de-frappe.md)).
        // On avance l'horloge du harnais plutôt que d'attendre réellement : le
        // minuteur se déclenche, et le test reste déterministe.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        if workspace.read_with(cx, |view, cx| {
            view.console_attempt.is_none()
                && view.consoles.iter().all(|console| {
                    console.read(cx).active.is_none()
                        && console.read(cx).export_active.is_none()
                        && console.read(cx).save_active.is_none()
                        && !console.read(cx).document_closing
                        && console.read(cx).draft_pending.is_none()
                })
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "consoles did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Masquer une colonne ne la retire pas du fichier, et la barre le dit.
///
/// Le test tient les **deux** moitiés, et la seconde est celle qui compte : que
/// la mention s'affiche ne prouve rien si elle ment. On exporte donc réellement,
/// et on vérifie que la colonne masquée est bien dans le CSV. Si quelqu'un fait
/// un jour suivre la visibilité à l'export — c'est une question ouverte, voir
/// « les colonnes masquées » dans `docs/IMPLEMENTATION-PLAN.md` —, ce test
/// échoue et force à retirer la réserve en même temps.
#[gpui::test]
fn une_colonne_masquee_part_quand_meme_a_lexport_et_la_barre_lannonce(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();
    let console = view.read_with(cx, |view, _| view.console.clone());

    cx.simulate_input("SELECT 11 AS garde, 22 AS secret");
    console.update(cx, |console, cx| console.execute(cx));
    settle(&view, cx);
    let result = console.read_with(cx, |console, _| console.last_result.expect("un résultat"));

    // Rien n'est masqué : la barre n'a aucune réserve à porter.
    assert_eq!(
        console.read_with(cx, |console, cx| console.export.read(cx).hidden_columns()),
        0
    );

    // L'utilisateur masque la seconde colonne, comme le fait le panneau de
    // l'inspecteur.
    console.update(cx, |console, cx| {
        console
            .grid
            .update(cx, |grid, cx| grid.set_column_visible(1, false, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        console.read_with(cx, |console, cx| console.export.read(cx).hidden_columns()),
        1,
        "la barre doit compter la colonne masquée"
    );

    let destination =
        std::env::temp_dir().join(format!("oxyn-colonnes-masquees-{}.csv", ResultId::new()));
    console.update(cx, |console, cx| {
        console.start_export(result, ExportFormat::Csv, destination.clone(), cx);
    });
    settle(&view, cx);

    let csv = std::fs::read_to_string(&destination).expect("le fichier exporté");
    let _ = std::fs::remove_file(&destination);
    assert!(
        csv.contains("secret") && csv.contains("22"),
        "la colonne masquée est écrite : c'est ce que la réserve annonce — {csv}"
    );
}

#[gpui::test]
fn current_statement_does_not_run_neighboring_writes_and_selection_stays_explicit(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let probe = backend.clone();
    let connection = open.connection;
    let session = open.session;
    for sql in [
        "CREATE TABLE scope_sentinel(id INT)",
        "CREATE TABLE scope_scratch(id INT)",
    ] {
        submit(
            &backend,
            execution_command(
                connection,
                session,
                false,
                SqlDialect::Sqlite,
                sql.into(),
                Vec::new(),
            ),
        )
        .expect("fixture");
    }
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let script = "INSERT INTO scope_sentinel VALUES(99);\nSELECT 'é😀' AS value;\nDELETE FROM scope_scratch WHERE id = -1;";
    view.update(cx, |view, cx| view.set_draft_text(script, cx));
    cx.simulate_keystrokes("up home cmd-enter");
    settle(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.draft_text(cx), script);
        assert!(!view.console.read(cx).last_mutating);
        assert!(view.console.read(cx).last_result.is_some());
    });
    let Outcome::Executed { buffer, .. } = submit(
        &probe,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "SELECT id FROM scope_sentinel".into(),
            Vec::new(),
        ),
    )
    .expect("probe") else {
        panic!("result");
    };
    assert_eq!(
        buffer.row_count(),
        0,
        "the preceding INSERT was not executed"
    );
    cx.simulate_keystrokes("cmd-a cmd-enter");
    settle(&view, cx);
    let Outcome::Executed { buffer, .. } = submit(
        &probe,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "SELECT id FROM scope_sentinel".into(),
            Vec::new(),
        ),
    )
    .expect("probe") else {
        panic!("result");
    };
    assert_eq!(
        buffer.row_count(),
        1,
        "an explicit selection retains batch behavior when supported"
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "test harness waits for an explicit human review"
)]
fn wait_for_review(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if workspace.read_with(cx, |view, cx| {
            view.console.read(cx).approval.read(cx).is_open()
        }) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "review must appear");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn cursor_inside_a_sqlite_trigger_executes_its_whole_definition_only(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let probe = backend.clone();
    let connection = open.connection;
    let session = open.session;
    for sql in [
        "CREATE TABLE trigger_source(id INT, end INT)",
        "CREATE TABLE trigger_audit(id INT)",
    ] {
        submit(
            &backend,
            execution_command(
                connection,
                session,
                false,
                SqlDialect::Sqlite,
                sql.into(),
                Vec::new(),
            ),
        )
        .expect("fixture");
    }
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let script = "CREATE TRIGGER current_trigger AFTER INSERT ON trigger_source\nBEGIN\n  INSERT INTO trigger_audit SELECT CASE WHEN new.id > 0 THEN new.id ELSE 0 END;\n  SELECT end FROM trigger_source;\n  UPDATE trigger_audit SET id = 999;\nEND;\nINSERT INTO trigger_source(id) VALUES(42);";
    view.update(cx, |view, cx| view.set_draft_text(script, cx));
    cx.simulate_keystrokes("up up home cmd-enter");
    wait_for_review(&view, cx);
    view.read_with(cx, |view, cx| {
        let approval = view.console.read(cx).approval.read(cx);
        let statement = &approval
            .request()
            .expect("review")
            .preview
            .as_ref()
            .expect("SQL preview")
            .statement;
        assert!(
            statement.starts_with("CREATE TRIGGER current_trigger"),
            "review covers the whole definition"
        );
        assert!(statement.trim_end().ends_with("END"));
    });
    cx.simulate_keystrokes("tab space");
    settle(&view, cx);
    let Outcome::Executed { buffer, .. } = submit(
        &probe,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "SELECT name FROM sqlite_schema WHERE type='trigger' AND name='current_trigger'".into(),
            Vec::new(),
        ),
    )
    .expect("trigger metadata") else {
        panic!("result");
    };
    assert_eq!(
        buffer.row_count(),
        1,
        "the trigger was created as one statement"
    );
    let Outcome::Executed { buffer, .. } = submit(
        &probe,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "SELECT id FROM trigger_source".into(),
            Vec::new(),
        ),
    )
    .expect("probe") else {
        panic!("result");
    };
    assert_eq!(buffer.row_count(), 0, "the following INSERT did not run");
}

#[gpui::test]
fn each_tab_keeps_its_result_and_session_when_responses_arrive_off_screen(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();
    let first = view.read_with(cx, |view, _| view.console.clone());
    let master = view.read_with(cx, |view, _| view.session);
    assert_ne!(
        first.read_with(cx, |console, _| console.session),
        Some(master)
    );
    cx.simulate_input("SELECT 11");
    cx.simulate_keystrokes("cmd-t");
    settle(&view, cx);
    let second = view.read_with(cx, |view, _| view.console.clone());
    assert_ne!(first, second);
    assert_ne!(
        first.read_with(cx, |console, _| console.session),
        second.read_with(cx, |console, _| console.session)
    );
    cx.simulate_input("SELECT 22");
    first.update(cx, |console, cx| console.execute(cx));
    second.update(cx, |console, cx| console.execute(cx));
    settle(&view, cx);
    let first_result = first.read_with(cx, |console, _| console.last_result.expect("first result"));
    let second_result =
        second.read_with(cx, |console, _| console.last_result.expect("second result"));
    assert_ne!(first_result, second_result);
    assert_eq!(
        first.read_with(cx, |console, cx| console.editor.read(cx).text()),
        "SELECT 11"
    );
    assert_eq!(
        second.read_with(cx, |console, cx| console.editor.read(cx).text()),
        "SELECT 22"
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.select_console(first.clone(), window, cx)
        })
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(view.grid, first.read(cx).grid);
        assert_eq!(view.console.read(cx).last_result, Some(first_result));
    });
    let destination = std::env::temp_dir().join(format!("oxyn-console-{}.csv", ResultId::new()));
    first.update(cx, |console, cx| {
        console.start_export(first_result, ExportFormat::Csv, destination.clone(), cx)
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.select_console(second.clone(), window, cx)
        })
    });
    settle(&view, cx);
    assert!(first.read_with(cx, |console, _| console.export_active.is_none()));
    let csv = std::fs::read_to_string(&destination).expect("exported first tab");
    assert!(csv.contains("11"));
    assert!(!csv.contains("22"));
    std::fs::remove_file(destination).expect("fixture cleanup");
}

#[gpui::test]
fn closing_a_dirty_console_requires_a_choice_and_leaves_catalog_and_siblings_usable(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let master = open.session;
    let connection = open.connection;
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    cx.simulate_input("SELECT 'keep me'");
    cx.simulate_keystrokes("cmd-t");
    settle(&view, cx);
    let target = view.read_with(cx, |view, _| view.console.clone());
    cx.simulate_input("SELECT 'discard me'");
    cx.simulate_keystrokes("cmd-w");
    assert!(view.read_with(cx, |view, _| view.console_close.is_some()));
    cx.simulate_keystrokes("enter");
    assert_eq!(view.read_with(cx, |view, _| view.consoles.len()), 2);
    cx.simulate_keystrokes("cmd-w");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |view, _| view.consoles.len()), 1);
    assert!(target.read_with(cx, |console, _| console.closed));
    assert_eq!(
        view.read_with(cx, |view, cx| view.editor.read(cx).text()),
        "SELECT 'keep me'"
    );
    submit(
        &backend,
        execution_command(
            connection,
            master,
            false,
            SqlDialect::Sqlite,
            "SELECT 7".into(),
            Vec::new(),
        ),
    )
    .expect("catalog session remains usable");
    view.update(cx, |view, cx| view.execute(cx));
    settle(&view, cx);
    assert!(view.read_with(cx, |view, cx| view.console.read(cx).last_result.is_some()));
}

#[gpui::test]
fn named_save_preserves_newer_edits_and_discard_keeps_the_saved_copy(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    let console = view.read_with(cx, |view, _| view.console.clone());
    cx.update(|window, cx| window.focus(&console.read(cx).name.read(cx).focus_handle(cx)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Revenue.sql");
    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| {
        assert!(
            !console
                .read(cx)
                .name
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
    cx.simulate_keystrokes("cmd-j");
    cx.simulate_input("SELECT 1");
    cx.simulate_keystrokes("cmd-s");
    console.update(cx, |console, cx| {
        console
            .editor
            .update(cx, |editor, cx| editor.set_text("SELECT 2", cx))
    });
    settle(&view, cx);
    let document = console.read_with(cx, |console, _| console.id);
    console.read_with(cx, |console, cx| {
        assert!(console.has_saved_copy && console.dirty);
        assert_eq!(console.saved_text, "SELECT 1");
        assert_eq!(console.editor.read(cx).text(), "SELECT 2");
    });
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("SELECT 1");
    cx.run_until_parked();
    console.read_with(cx, |console, _| {
        assert!(!console.dirty);
        assert_eq!(console.save_notice, "Matches the saved version.");
    });
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    assert!(console.read_with(cx, |console, _| console.dirty));
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("read saved query");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.saved_content.as_deref() == Some("SELECT 1") && document.title == "Revenue.sql")
    );
    cx.simulate_keystrokes("cmd-w");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    assert!(console.read_with(cx, |console, _| console.closed));
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("saved copy survives closing");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if !document.is_open && document.saved_content.as_deref() == Some("SELECT 1"))
    );
}

#[gpui::test]
fn save_and_close_waits_for_storage_before_removing_the_console(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    cx.simulate_input("SELECT 'keep this'");
    let document = view.read_with(cx, |view, cx| view.console.read(cx).id);
    cx.simulate_keystrokes("cmd-w");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    assert!(view.read_with(cx, |view, _| view.consoles.is_empty()));
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("saved document");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if !document.is_open && document.saved_content.as_deref() == Some("SELECT 'keep this'"))
    );
}

#[gpui::test]
fn a_save_conflict_preserves_the_external_copy_and_offers_a_new_identity(
    app_cx: &mut TestAppContext,
) {
    for revision_jump in [1, 50] {
        let (backend, open) = connected_workspace();
        let connection = open.connection;
        let (view, cx) = app_cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
        cx.run_until_parked();
        cx.simulate_input("SELECT 'mine'");
        cx.simulate_keystrokes("cmd-s");
        settle(&view, cx);
        let console = view.read_with(cx, |view, _| view.console.clone());
        let original = console.read_with(cx, |console, _| console.id);
        let external_revision =
            console.read_with(cx, |console, _| console.document_revision + revision_jump);
        submit(
            &backend,
            Command::SaveQueryDocument {
                workspace: backend.workspace_id(),
                update: Box::new(oxyn_core::QueryDocumentUpdate {
                    expected_revision: None,
                    document: original,
                    revision: external_revision,
                    title: "External.sql".into(),
                    language: QueryLanguage::Sql(SqlDialect::Sqlite),
                    text: "SELECT 'external'".into(),
                    connection: Some(connection),
                    save_named: true,
                    is_open: true,
                    provenance: None,
                }),
            },
        )
        .expect("external edit");
        cx.simulate_keystrokes("cmd-s");
        settle(&view, cx);
        assert!(console.read_with(cx, |console, _| console.save_conflict));
        console.update(cx, |console, cx| console.save_as_new_query(cx));
        settle(&view, cx);
        assert_ne!(console.read_with(cx, |console, _| console.id), original);
        let stored = submit(
            &backend,
            Command::OpenDocument {
                workspace: backend.workspace_id(),
                document: original,
            },
        )
        .expect("external document retained");
        assert!(
            matches!(stored, Outcome::DocumentOpened { document } if document.saved_content.as_deref() == Some("SELECT 'external'"))
        );
    }
}

#[gpui::test]
fn cancelling_save_and_close_never_closes_after_a_late_save_acknowledgement(
    cx: &mut TestAppContext,
) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.run_until_parked();
    cx.simulate_input("SELECT 'stay open'");
    let console = view.read_with(cx, |view, _| view.console.clone());
    console.update(cx, |console, cx| {
        console.save_and_close(cx);
        console.cancel_save(cx);
    });
    settle(&view, cx);
    assert!(!console.read_with(cx, |console, _| console.closed));
    assert_eq!(view.read_with(cx, |view, _| view.consoles.len()), 1);
    assert_eq!(
        console.read_with(cx, |console, cx| console.editor.read(cx).text()),
        "SELECT 'stay open'"
    );
}

#[gpui::test]
fn edits_are_recoverable_without_replacing_the_named_copy(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    cx.simulate_input("SELECT 'draft one'");
    settle(&view, cx);
    let console = view.read_with(cx, |view, _| view.console.clone());
    let document = console.read_with(cx, |console, _| console.id);
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("recoverable draft");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.is_open && !document.is_saved && document.content == "SELECT 'draft one'")
    );
    cx.simulate_keystrokes("cmd-s");
    settle(&view, cx);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("SELECT 'draft two'");
    settle(&view, cx);
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("newer draft");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.content == "SELECT 'draft two'" && document.saved_content.as_deref() == Some("SELECT 'draft one'"))
    );
    cx.update(|window, cx| window.focus(&console.read(cx).name.read(cx).focus_handle(cx)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_keystrokes("backspace");
    settle(&view, cx);
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("empty working name");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if document.title.is_empty() && document.saved_title.as_deref() == Some("console_1.sql"))
    );
}

#[gpui::test]
fn discarding_an_autosaved_unnamed_console_leaves_no_open_draft(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend.clone(), open, cx));
    cx.run_until_parked();
    cx.simulate_input("SELECT 'discard this'");
    settle(&view, cx);
    let document = view.read_with(cx, |view, cx| view.console.read(cx).id);
    cx.simulate_keystrokes("cmd-w");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    let stored = submit(
        &backend,
        Command::OpenDocument {
            workspace: backend.workspace_id(),
            document,
        },
    )
    .expect("closed draft");
    assert!(
        matches!(stored, Outcome::DocumentOpened { document } if !document.is_open && !document.is_saved && document.content.is_empty())
    );
}

/// Explain describes the statement; it must never be the statement.
///
/// `EXPLAIN QUERY PLAN DELETE …` is a plan, but one missing guard away from a
/// real delete — and the prefix helper is what stands between the two. The
/// draft is left untouched so the user can still run what they wrote.
#[gpui::test]
fn explain_describes_a_delete_without_deleting_anything(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let probe = backend.clone();
    let (connection, session) = (open.connection, open.session);
    for sql in [
        "CREATE TABLE explain_target(id INT)",
        "INSERT INTO explain_target VALUES(1)",
    ] {
        submit(
            &backend,
            execution_command(
                connection,
                session,
                false,
                SqlDialect::Sqlite,
                sql.into(),
                Vec::new(),
            ),
        )
        .expect("fixture");
    }
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let draft = "DELETE FROM explain_target";
    view.update(cx, |view, cx| view.set_draft_text(draft, cx));
    cx.run_until_parked();

    let control = cx.debug_bounds("explain-query").expect("Explain control");
    cx.simulate_click(control.center(), Default::default());
    settle(&view, cx);

    view.read_with(cx, |view, cx| {
        assert_eq!(view.draft_text(cx), draft, "le brouillon reste celui écrit");
        assert!(
            view.console.read(cx).last_result.is_some(),
            "le plan est un résultat comme un autre"
        );
        assert!(
            !view.console.read(cx).last_mutating,
            "un plan ne demande aucune revue d'écriture"
        );
    });

    let Outcome::Executed { buffer, .. } = submit(
        &probe,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "SELECT id FROM explain_target".into(),
            Vec::new(),
        ),
    )
    .expect("probe") else {
        panic!("result");
    };
    assert_eq!(buffer.row_count(), 1, "la ligne n'a pas été supprimée");
}

/// A bound value travels in `ExecRequest::params`, never inside the SQL text.
///
/// The value here is written to be destructive if it were ever concatenated:
/// a table would be gone by the end of the test ([I-10](../../../../CLAUDE.md#i-10)).
#[gpui::test]
fn a_hostile_bound_value_is_bound_and_never_spliced_into_the_sql(cx: &mut TestAppContext) {
    const HOSTILE: &str = "S3NT1NELLE'); DROP TABLE bound_target;--";
    let (backend, open) = connected_workspace();
    let probe = backend.clone();
    let (connection, session) = (open.connection, open.session);
    submit(
        &backend,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "CREATE TABLE bound_target(id INT)".into(),
            Vec::new(),
        ),
    )
    .expect("fixture");
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));

    let control = cx
        .debug_bounds("query-parameters")
        .expect("Parameters control");
    cx.simulate_click(control.center(), Default::default());
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.console.read(cx).parameters_open);
        assert!(view.console.read(cx).parameters.read(cx).is_empty());
    });

    let editor = view.read_with(cx, |view, cx| view.console.read(cx).parameters.clone());
    editor.update(cx, |editor, cx| {
        editor.add_parameter(cx);
        // Une ligne neuve vaut NULL et refuse la saisie : choisir le type est
        // le premier geste, comme dans le panneau.
        editor.set_parameter_type(0, oxyn_core::ParameterType::Text, cx);
    });
    cx.run_until_parked();
    let field = editor
        .read_with(cx, |editor, _| editor.value_field(0))
        .expect("value field");
    // Saisie réelle plutôt qu'une écriture directe : c'est le chemin que prend
    // l'utilisateur, protection du presse-papiers comprise.
    cx.update(|window, cx| window.focus(&field.read(cx).focus_handle(cx)));
    cx.run_until_parked();
    cx.simulate_input(HOSTILE);
    cx.run_until_parked();
    assert_eq!(
        field.read_with(cx, |field, _| field.text().to_owned()),
        HOSTILE
    );

    view.update(cx, |view, cx| view.set_draft_text("SELECT ? AS bound", cx));
    cx.run_until_parked();
    // Le clavier appartient encore au champ de valeur : c'est le bouton Run que
    // l'utilisateur atteint depuis là, et c'est lui qu'on exerce.
    let run = cx.debug_bounds("run-query").expect("Run control");
    cx.simulate_click(run.center(), Default::default());
    settle(&view, cx);

    let Outcome::Executed { buffer, .. } = submit(
        &probe,
        execution_command(
            connection,
            session,
            false,
            SqlDialect::Sqlite,
            "SELECT count(*) AS present FROM sqlite_schema WHERE name='bound_target'".into(),
            Vec::new(),
        ),
    )
    .expect("probe") else {
        panic!("result");
    };
    assert_eq!(
        buffer.row_count(),
        1,
        "la table doit toujours exister : la valeur n'a pas été concaténée"
    );
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.draft_text(cx),
            "SELECT ? AS bound",
            "le SQL de l'utilisateur part tel quel"
        );
        assert_eq!(view.console.read(cx).parameter_count(cx), 1);
        // Sans valeur liée, SQLite refuse le `?` : un résultat prouve la liaison.
        assert!(
            view.console.read(cx).last_result.is_some(),
            "la valeur a bien été liée : {:?}",
            view.console.read(cx).status.read(cx).notice()
        );
    });
}

/// Each console owns its bound values; a neighbour never inherits them.
#[gpui::test]
fn bound_values_do_not_cross_between_consoles(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let first = view.read_with(cx, |view, _| view.console.clone());
    first.update(cx, |console, cx| {
        console.parameters.update(cx, |editor, cx| {
            editor.add_parameter(cx);
            editor.add_parameter(cx);
        });
        console.toggle_parameters(cx);
    });
    cx.simulate_keystrokes("cmd-t");
    settle(&view, cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(view.consoles.len(), 2);
        let second = view.console.read(cx);
        assert_eq!(
            second.parameter_count(cx),
            0,
            "la nouvelle console part vide"
        );
        assert!(!second.parameters_open, "et son panneau est fermé");
    });
    first.read_with(cx, |console, cx| {
        assert_eq!(
            console.parameter_count(cx),
            2,
            "la première garde les siennes"
        );
        assert!(console.parameters_open);
    });
}

/// A value that cannot be converted stops the run before anything is dispatched.
///
/// The notice names the position and the expected type. It never repeats what
/// was typed ([I-03](../../../../CLAUDE.md#i-03)).
#[gpui::test]
fn an_unconvertible_value_stops_the_run_without_echoing_it(cx: &mut TestAppContext) {
    const TYPED: &str = "S3NT1NELLE-not-a-number";
    let (backend, open) = connected_workspace();
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    let editor = view.read_with(cx, |view, cx| view.console.read(cx).parameters.clone());
    editor.update(cx, |editor, cx| {
        editor.add_parameter(cx);
        editor.set_parameter_type(0, oxyn_core::ParameterType::Int64, cx);
        if let Some(field) = editor.value_field(0) {
            field.update(cx, |field, cx| field.set_text(TYPED.into(), cx));
        }
    });
    view.update(cx, |view, cx| view.set_draft_text("SELECT ?", cx));
    cx.simulate_keystrokes("cmd-enter");
    settle(&view, cx);

    view.read_with(cx, |view, cx| {
        let console = view.console.read(cx);
        assert!(console.active.is_none(), "rien ne part sur le bus");
        assert!(console.last_result.is_none());
        assert!(
            console.parameters_open,
            "le panneau s'ouvre sur la valeur à corriger"
        );
        let notice = console
            .status
            .read(cx)
            .notice()
            .unwrap_or_default()
            .to_owned();
        assert!(!notice.is_empty(), "l'utilisateur doit savoir pourquoi");
        assert!(
            !notice.contains(TYPED),
            "le texte saisi ne se répète jamais dans un message"
        );
    });
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
    }
}

/// Stop only exists while something runs, and Run only while nothing does.
///
/// Figma `273:37036` greys out the segment that has nothing to do. A greyed
/// control that still acts is worse than none: the click that was meant to stop
/// a long query would start it again.
#[gpui::test]
fn run_and_stop_are_inert_when_they_have_nothing_to_do(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    view.update(cx, |view, cx| view.set_draft_text("SELECT 1", cx));
    cx.run_until_parked();

    // Rien ne tourne : Stop ne doit rien déclencher, ni exécution ni erreur.
    let stop = cx.debug_bounds("stop-query").expect("Stop control");
    cx.simulate_click(stop.center(), Default::default());
    settle(&view, cx);
    view.read_with(cx, |view, cx| {
        assert!(view.console.read(cx).last_result.is_none());
        assert!(view.console.read(cx).active.is_none());
    });

    // Une exécution en vol : c'est Stop qui l'annule, et Run reste sans effet.
    let token = CancelToken::new();
    view.update(cx, |view, cx| {
        view.console.update(cx, |console, _| {
            console.active = Some((CommandId::new(), token.clone()));
        });
        cx.notify();
    });
    cx.run_until_parked();
    let run = cx.debug_bounds("run-query").expect("Run control");
    cx.simulate_click(run.center(), Default::default());
    cx.run_until_parked();
    assert!(
        !token.is_cancelled(),
        "Run ne relance ni n'interrompt une exécution en cours"
    );
    let stop = cx.debug_bounds("stop-query").expect("Stop control");
    cx.simulate_click(stop.center(), Default::default());
    cx.run_until_parked();
    assert!(token.is_cancelled(), "Stop atteint l'annulation réelle");
}

/// The resolution context belongs to the session, so it belongs to the tab
/// ([ADR-0015](../../../../docs/adr/0015-consoles-independantes.md)).
#[gpui::test]
fn each_tab_shows_its_own_context_and_switching_follows_it(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let mut events = backend.subscribe();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1060.)));
    cx.run_until_parked();
    let first = view.read_with(cx, |view, _| view.console.clone());
    cx.simulate_keystrokes("cmd-t");
    settle(&view, cx);
    let second = view.read_with(cx, |view, _| view.console.clone());
    assert_ne!(first, second, "cmd-T opens a tab with its own session");

    // Only the interface is told the sessions can move: SQLite cannot, and the
    // executor refuses. What is under test is which tab the bar reads.
    for (console, namespace) in [(&first, "public"), (&second, "analytics")] {
        console.update(cx, |console, cx| {
            console.capabilities |= Capabilities::SESSION_CONTEXT;
            console.context = Some(oxyn_driver::SessionContext::new(
                None,
                Some(namespace.to_owned()),
            ));
            cx.notify();
        });
    }
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("session-context").is_some(),
        "a session that declares the capability gets the selector"
    );
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.console.read(cx).context_summary().as_deref(),
            Some("Workspace interaction test / analytics"),
            "the bar reads the tab in front"
        );
    });

    for (target, expected) in [
        (&first, "Workspace interaction test / public"),
        (&second, "Workspace interaction test / analytics"),
    ] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.select_console(target.clone(), window, cx);
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.console.read(cx).context_summary().as_deref(),
                Some(expected),
                "changing tab shows the context of that tab, not the last one seen"
            );
        });
    }

    let mut executed = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event.event, Event::SchemaReady { .. }) {
            executed += 1;
        }
    }
    assert_eq!(executed, 0, "showing a context runs no SQL");
}

/// Une frappe continue n'écrit rien ; son repos écrit une fois.
///
/// C'est la décision d'[ADR-0024], et elle vient d'une mesure : copier le
/// document entier à chaque touche produisait 17 à-coups en 14 s de frappe, dont
/// un de 50 ms pour un budget de 8 ms. `editor.text()` est un `join` sur toutes
/// les lignes — jusqu'à un mégaoctet, sur le fil d'interface, par caractère.
///
/// Le test porte sur le **nombre de révisions** : c'est ce qui compte, parce que
/// chaque révision est une copie et une écriture.
///
/// [ADR-0024]: ../../../../docs/adr/0024-autosauvegarde-au-repos-de-frappe.md
#[gpui::test]
fn une_frappe_continue_n_ecrit_qu_au_repos(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    settle(&workspace, cx);

    let revision = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |view, cx| {
            view.consoles
                .first()
                .expect("une console est ouverte")
                .read(cx)
                .document_revision
        })
    };
    let depart = revision(cx);

    // Dix touches sans pause : le minuteur est relancé à chaque fois.
    workspace.update(cx, |view, cx| {
        let console = view.consoles.first().expect("console").clone();
        console.update(cx, |console, cx| {
            for lettre in "SELECT 1;\n".chars() {
                console
                    .editor
                    .update(cx, |editor, cx| editor.insert(&lettre.to_string(), cx));
            }
        });
    });
    cx.run_until_parked();
    assert_eq!(
        revision(cx),
        depart,
        "dix touches sans pause ne doivent produire aucune écriture"
    );

    // Le repos, et une seule écriture pour les dix touches.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(
        revision(cx),
        depart + 1,
        "le repos de frappe écrit une fois, pas dix"
    );
}

/// Un plan d'exécution se distingue des données qu'il décrit.
///
/// `EXPLAIN` rend des lignes comme n'importe quelle requête : rien dans la
/// grille ne dit qu'on lit un plan et non le contenu de la table. Le relevé
/// `191:1521` le dit par un onglet `Explain plan` à côté de `Result 1`.
///
/// Le piège que ce test ferme est de l'ordre du contresens : un utilisateur qui
/// lance `Explain` sur un `SELECT`, s'absente, revient, et lit sa grille comme
/// des données — alors qu'il regarde un plan.
#[gpui::test]
fn un_plan_ne_se_lit_pas_comme_des_donnees(cx: &mut TestAppContext) {
    let (backend, open) = connected_workspace();
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    settle(&workspace, cx);

    let console = workspace.read_with(cx, |view, _| {
        view.consoles.first().expect("une console").clone()
    });

    // Une exécution ordinaire ne montre aucune mention de plan.
    console.update(cx, |console, cx| {
        console
            .editor
            .update(cx, |editor, cx| editor.set_text("SELECT 1", cx));
        console.execute(cx);
    });
    settle(&workspace, cx);
    console.read_with(cx, |console, _| {
        assert!(
            !console.showing_plan,
            "un SELECT ordinaire ne produit pas de plan"
        );
    });

    // `Explain` sur la même instruction, si la session le permet.
    console.update(cx, |console, cx| console.execute_explain(cx));
    settle(&workspace, cx);
    console.read_with(cx, |console, _| {
        assert!(
            console.showing_plan,
            "après Explain, la vue doit dire que ces lignes sont un plan"
        );
    });
}
