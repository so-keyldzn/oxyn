use super::*;

fn entries_of<'a>(blocks: &'a [Block], id: &str) -> &'a [Entry] {
    &blocks
        .iter()
        .find(|block| block.id == id)
        .unwrap_or_else(|| panic!("the bar has a {id} menu"))
        .entries
}

/// Every action entry of the bar, submenus included.
fn actions(blocks: &[Block]) -> Vec<&ActionEntry> {
    fn walk<'a>(entries: &'a [Entry], found: &mut Vec<&'a ActionEntry>) {
        for entry in entries {
            match entry {
                Entry::Action(action) => found.push(action),
                Entry::Submenu(block) => walk(&block.entries, found),
                Entry::Separator | Entry::Role(_) => {}
            }
        }
    }
    let mut found = Vec::new();
    for block in blocks {
        walk(&block.entries, &mut found);
    }
    found
}

fn action<'a>(blocks: &'a [Block], id: &str) -> &'a ActionEntry {
    actions(blocks)
        .into_iter()
        .find(|action| action.id == id)
        .unwrap_or_else(|| panic!("the bar has {id}"))
}

#[test]
fn the_shipped_manifest_describes_the_macos_bar() {
    let blocks = describe().expect("the manifest shipped with the front is valid");
    let titles: Vec<&str> = blocks.iter().map(|block| block.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Oxyn", "File", "Edit", "View", "Query", "Window", "Help"]
    );
    let quit = action(&blocks, QUIT);
    assert_eq!(quit.labels, ["Quit Oxyn"]);
    assert_eq!(quit.accelerator.as_deref(), Some("CmdOrCtrl+Q"));
    assert_eq!(
        action(&blocks, "console.runAll").accelerator.as_deref(),
        Some("CmdOrCtrl+Shift+Enter")
    );
    assert_eq!(
        action(&blocks, "view.sidePanel").accelerator.as_deref(),
        Some("CmdOrCtrl+Alt+B")
    );
}

#[test]
fn close_tab_owns_cmd_w_and_no_close_window_remains() {
    let blocks = describe().expect("valid manifest");
    assert_eq!(
        action(&blocks, "tab.close").accelerator.as_deref(),
        Some("CmdOrCtrl+W")
    );
    let others: Vec<&str> = actions(&blocks)
        .into_iter()
        .filter(|action| {
            action.id != "tab.close" && action.accelerator.as_deref() == Some("CmdOrCtrl+W")
        })
        .map(|action| action.id.as_str())
        .collect();
    assert!(others.is_empty(), "⌘W is bound once: {others:?}");
}

#[test]
fn escape_is_never_a_native_accelerator() {
    let blocks = describe().expect("valid manifest");
    let cancel = action(&blocks, "console.cancel");
    assert_eq!(cancel.accelerator, None, "Cancel is shown without Esc");
    assert_eq!(to_accelerator("Escape"), Ok(None));
}

#[test]
fn the_system_roles_are_kept_where_the_manifest_places_them() {
    let blocks = describe().expect("valid manifest");
    let edit = entries_of(&blocks, "edit");
    for role in [Role::Cut, Role::Copy, Role::Paste, Role::SelectAll] {
        assert!(edit.contains(&Entry::Role(role)), "Edit keeps {role:?}");
    }
    let app = entries_of(&blocks, "app");
    assert_eq!(app.first(), Some(&Entry::Role(Role::About)));
    assert!(
        matches!(app.last(), Some(Entry::Action(action)) if action.id == QUIT),
        "Quit ends the application menu"
    );
    assert!(
        !edit
            .windows(2)
            .any(|pair| pair == [Entry::Separator, Entry::Separator])
    );
}

#[test]
fn web_only_actions_stay_out_of_the_native_bar() {
    let blocks = describe().expect("valid manifest");
    let ids: Vec<&str> = actions(&blocks)
        .into_iter()
        .map(|action| action.id.as_str())
        .collect();
    assert!(!ids.contains(&"edit.copy"), "macOS copies through its role");
}

#[test]
fn a_manifest_that_misplaces_an_action_is_refused() {
    let source = r#"{"menus": [{"id": "file", "title": "File"}],
            "actions": [{"id": "a.b", "label": "B", "zone": "global",
                         "menu": {"mac": {"menu": "nowhere", "group": 0, "order": 0}}}]}"#;
    assert!(matches!(
        describe_from(source),
        Err(ManifestError::UnknownMenu { .. })
    ));
    let twice = r#"{"menus": [], "actions": [
            {"id": "a.b", "label": "B", "zone": "global"},
            {"id": "a.b", "label": "C", "zone": "global"}]}"#;
    assert!(matches!(
        describe_from(twice),
        Err(ManifestError::Duplicate(_))
    ));
    assert!(matches!(
        describe_from("{"),
        Err(ManifestError::Unreadable(_))
    ));
}

#[test]
fn text_size_and_theme_are_submenus_of_view_around_full_screen() {
    let blocks = describe().expect("valid manifest");
    let view = entries_of(&blocks, "view");
    let position = |wanted: &Entry| view.iter().position(|entry| entry == wanted);
    let submenu = |id: &str| {
        view.iter()
            .find_map(|entry| match entry {
                Entry::Submenu(block) if block.id == id => Some(block),
                _ => None,
            })
            .unwrap_or_else(|| panic!("View has {id}"))
    };
    let text_size = submenu("textSize");
    let theme = submenu("theme");
    assert_eq!(text_size.title, "Text size");
    let fullscreen = position(&Entry::Role(Role::Fullscreen));
    assert!(position(&Entry::Submenu(text_size.clone())) < fullscreen);
    assert!(fullscreen < position(&Entry::Submenu(theme.clone())));
    for entry in text_size.entries.iter().chain(&theme.entries) {
        assert!(
            matches!(entry, Entry::Action(action) if action.check),
            "{entry:?} is a checked choice"
        );
    }
}

#[test]
fn a_submenu_nests_one_level_in_a_menu_of_the_bar() {
    let source = r#"{"menus": [
                {"id": "view", "title": "View"},
                {"id": "size", "title": "Size",
                 "parent": {"mac": {"menu": "view", "group": 0, "order": 0}}},
                {"id": "deeper", "title": "Deeper",
                 "parent": {"mac": {"menu": "size", "group": 0, "order": 0}}}],
            "actions": [{"id": "a.b", "label": "B", "zone": "global",
                         "menu": {"mac": {"menu": "deeper", "group": 0, "order": 0}}}]}"#;
    assert!(matches!(
        describe_from(source),
        Err(ManifestError::UnknownParent { .. })
    ));
}

#[test]
fn a_shortcut_muda_cannot_read_is_refused() {
    assert_eq!(to_accelerator("Hyper+K"), Err(()));
    assert_eq!(to_accelerator("Mod+"), Err(()));
    assert_eq!(to_accelerator("Mod+/"), Ok(Some("CmdOrCtrl+/".to_owned())));
}

fn bar() -> MenuBar {
    let bar = MenuBar::default();
    bar.learn(&describe().expect("valid manifest"));
    bar
}

fn state(id: &str, variant: usize, shortcut: bool) -> MenuEntryState {
    MenuEntryState {
        id: id.to_owned(),
        enabled: true,
        variant,
        shortcut,
        checked: None,
    }
}

#[test]
fn set_menu_state_refuses_an_id_the_bar_does_not_have() {
    let bar = bar();
    assert_eq!(
        bar.plan(&[
            state("console.run", 0, true),
            state("app.rename-quit", 0, true)
        ]),
        Err(MenuStateError::UnknownEntry("app.rename-quit".to_owned()))
    );
    // A known id that is not in the native bar is unknown too.
    assert!(bar.apply(&[state("edit.copy", 0, true)]).is_err());
}

#[test]
fn set_menu_state_takes_a_variant_index_never_a_text() {
    let bar = bar();
    let changes = bar
        .plan(&[state("edit.find", 2, false)])
        .expect("edit.find has two variants");
    assert_eq!(
        changes.first().map(|change| change.label.as_str()),
        Some("Find in results")
    );
    assert_eq!(
        changes
            .first()
            .and_then(|change| change.accelerator.clone()),
        None
    );
    assert_eq!(
        bar.plan(&[state("edit.find", 3, true)]),
        Err(MenuStateError::UnknownVariant {
            id: "edit.find".to_owned(),
            variant: 3
        })
    );
}

#[test]
fn the_webview_cannot_disable_or_unbind_quit() {
    let bar = bar();
    let quit = MenuEntryState {
        id: QUIT.to_owned(),
        enabled: false,
        variant: 0,
        shortcut: false,
        checked: None,
    };
    let changes = bar.plan(&[quit]).expect("Quit is in the bar");
    let change = changes.first().expect("one change");
    assert!(change.enabled);
    assert_eq!(change.accelerator.as_deref(), Some("CmdOrCtrl+Q"));
}

#[test]
fn set_menu_state_marks_only_a_check_entry() {
    let bar = bar();
    let checked = |id: &str, mark: Option<bool>| {
        bar.plan(&[MenuEntryState {
            checked: mark,
            ..state(id, 0, true)
        }])
        .expect("a known entry")
        .first()
        .and_then(|change| change.checked)
    };
    assert_eq!(checked("view.theme.dark", Some(true)), Some(true));
    assert_eq!(checked("view.theme.dark", None), Some(false));
    assert_eq!(checked("console.run", Some(true)), None);
}

#[test]
fn an_unknown_bar_forwards_nothing() {
    // Without a native bar (Windows, Linux), no id is the bar's.
    let bar = MenuBar::default();
    assert!(bar.apply(&[state("console.run", 0, true)]).is_err());
    assert_eq!(bar.target("console.run", Some("a"), Some("b")), None);
    bar.forward("console.run", Some("a"), Some("b"));
}

#[test]
fn an_entry_goes_to_the_focused_window_only() {
    let bar = bar();
    assert_eq!(bar.target("console.run", Some("a"), Some("b")), Some("a"));
    // No window has the focus: a window action has no target.
    assert_eq!(bar.target("console.run", None, Some("b")), None);
    // An application action has none either, so any window may run it.
    assert_eq!(bar.target("window.new", None, Some("b")), Some("b"));
    assert_eq!(bar.target("window.new", Some("a"), Some("b")), Some("a"));
    assert_eq!(bar.target(QUIT, Some("a"), Some("b")), None);
}

#[test]
fn only_the_focused_window_state_is_kept_for_the_bar() {
    let bar = bar();
    let disabled = MenuEntryState {
        enabled: false,
        ..state("console.run", 0, true)
    };
    bar.report("a", &[state("console.run", 0, true)], Some("b"))
        .expect("a known entry");
    bar.report("b", &[disabled], Some("b"))
        .expect("a known entry");
    assert_eq!(
        bar.states
            .lock()
            .get("a")
            .and_then(|changes| changes.first())
            .map(|c| c.enabled),
        Some(true)
    );
    // An unknown id records nothing for that window.
    assert!(
        bar.report("a", &[state("nope", 0, true)], Some("a"))
            .is_err()
    );
    assert!(
        bar.states
            .lock()
            .get("a")
            .is_some_and(|changes| changes.len() == 1)
    );
    bar.forget("a");
    assert!(bar.states.lock().get("a").is_none());
}

#[test]
fn without_focus_only_the_application_entries_stay_enabled() {
    let bar = bar();
    let enabled: Vec<String> = bar
        .without_focus()
        .into_iter()
        .filter(|change| change.enabled)
        .map(|change| change.id)
        .collect();
    let mut enabled = enabled;
    enabled.sort();
    assert_eq!(
        enabled,
        [
            "app.checkForUpdates",
            "app.quit",
            "app.settings",
            "window.new"
        ]
    );
}
