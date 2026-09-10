use super::*;
use crate::workspace::tests::{connected_workspace, submit};
use gpui::{TestAppContext, VisualTestContext, point};

#[expect(
    clippy::disallowed_methods,
    reason = "test harness polls the independent backend runtime"
)]
fn wait_until(
    view: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    ready: impl Fn(&Workspace) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| ready(view)) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "backend operation must finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn fixture() -> (Backend, OpenConnection) {
    let (backend, open) = connected_workspace();
    for sql in [
        "CREATE TABLE ddl_items(id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
        "CREATE INDEX ddl_value_index ON ddl_items(value)",
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
    (backend, open)
}

#[gpui::test]
fn ddl_preview_is_readonly_and_its_button_opens_a_separate_unexecuted_console(
    cx: &mut TestAppContext,
) {
    let (backend, open) = fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_input("SELECT 'keep original';");
    let original = view.read_with(cx, |view, _| view.console.clone());
    view.update(cx, |view, cx| {
        view.selected_path =
            Some(CatalogPath::for_relation(None, Some("main"), "ddl_items").expect("path"));
        view.panel = WorkspacePanel::Object;
        view.select_metadata_tab(ObjectTab::Ddl, cx);
    });
    wait_until(&view, cx, |view| view.definition.active.is_none());
    let sql = view.read_with(cx, |view, cx| {
        assert!(view.definition_ready(), "{:?}", view.definition.error);
        view.definition.editor.read(cx).text()
    });
    assert!(sql.contains("CREATE INDEX"));
    cx.update(|window, cx| {
        window.focus(&view.read(cx).definition.editor.read(cx).focus_handle(cx))
    });
    cx.simulate_input("DELETE FROM ddl_items;");
    cx.simulate_keystrokes("cmd-enter cmd-a cmd-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(sql.clone())
    );
    assert_eq!(
        view.read_with(cx, |view, cx| view.definition.editor.read(cx).text()),
        sql
    );
    assert!(original.read_with(cx, |console, _| console.active.is_none()));
    let hidden_read = CancelToken::new();
    view.update(cx, |view, _| {
        view.definition.active = Some((CommandId::new(), hidden_read.clone()))
    });
    cx.run_until_parked();
    let data = cx.debug_bounds("object-data").expect("Data tab");
    cx.simulate_click(data.center(), Default::default());
    assert!(
        hidden_read.is_cancelled(),
        "leaving the DDL view stops an in-flight read"
    );
    view.update(cx, |view, cx| view.select_metadata_tab(ObjectTab::Ddl, cx));
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("open-definition-console")
        .expect("visible open action");
    cx.simulate_click(bounds.center(), Default::default());
    wait_until(&view, cx, |view| view.console_attempt.is_none());
    view.read_with(cx, |view, cx| {
        assert_eq!(view.consoles.len(), 2);
        assert_eq!(
            original.read(cx).editor.read(cx).text(),
            "SELECT 'keep original';"
        );
        let created = view.consoles.last().expect("new console").read(cx);
        assert_ne!(created.id, original.read(cx).id);
        assert_eq!(created.editor.read(cx).text(), sql);
        assert!(created.active.is_none());
        assert!(created.displayed_result.is_none());
    });
}

#[gpui::test]
fn ddl_panel_resizes_without_reloading_and_compact_mode_keeps_the_full_tab(
    cx: &mut TestAppContext,
) {
    let (backend, open) = fixture();
    let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
    cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
    view.update(cx, |view, cx| {
        view.selected_path =
            Some(CatalogPath::for_relation(None, Some("main"), "ddl_items").expect("path"));
        view.panel = WorkspacePanel::Object;
        view.select_metadata_tab(ObjectTab::Structure, cx);
    });
    wait_until(&view, cx, |view| {
        view.definition.active.is_none() && view.catalog_active.is_none()
    });
    let editor = view.read_with(cx, |view, _| view.definition.editor.clone());
    let bounds = cx
        .debug_bounds("definition-resize")
        .expect("definition resize handle");
    assert_eq!(bounds.size.width, px(8.));
    assert_eq!(view.read_with(cx, |view, _| view.definition.width), 424);
    let start = bounds.center();
    let end = point(start.x - px(50.), start.y);
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    assert_eq!(view.read_with(cx, |view, _| view.definition.width), 474);
    cx.simulate_keystrokes("home");
    assert_eq!(view.read_with(cx, |view, _| view.definition.width), 424);
    cx.simulate_resize(gpui::size(px(1024.), px(768.)));
    view.update(cx, |view, cx| view.select_metadata_tab(ObjectTab::Ddl, cx));
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.compact_layout);
        assert_eq!(
            view.definition.editor, editor,
            "resize and tab selection do not reload DDL"
        );
        assert!(view.definition.active.is_none());
    });
    assert!(cx.debug_bounds("open-definition-console").is_some());
}
