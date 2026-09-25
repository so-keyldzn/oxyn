//! The menu bar of macOS, built from the front's action manifest
//! ([ADR-0041](../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 4).
//!
//! The manifest is read at compile time, so the bar exists before the webview
//! loads and none of its labels come from JavaScript. This module knows the
//! ids, labels and combinations of the actions — never what they do: it builds
//! no `Command`, and a chosen entry travels to the front as its id alone. The
//! one exception is `app.quit`, which Rust runs itself so that a frozen
//! webview cannot keep the user in (ADR-0038).
//!
//! Windows and Linux get no native menu: their bar is drawn by the front
//! (point 5), and this module only answers there by refusing every id.

// The bar is described on every platform, so that its tests run on the Linux
// runners of the CI; only macOS builds it.
#![cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "the native bar is only built on macOS")
)]

use std::collections::HashMap;

use parking_lot::Mutex;
use serde::Deserialize;
use tauri::ipc::Channel;

use crate::ipc::menu::{MenuActivation, MenuEntryState};

/// The entry Rust runs itself, through the ordered shutdown (ADR-0038).
pub const QUIT: &str = "app.quit";

/// Coupled by path to the front, on purpose: moving the file breaks this
/// build, not the menu of a released version (ADR-0041, « Conséquences »).
const MANIFEST: &str = include_str!("../../../apps/desktop/src/lib/actions/actions.json");

// ---- The manifest, as far as the bar needs it --------------------------------

#[derive(Debug, Deserialize)]
struct Manifest {
    menus: Vec<MenuSpec>,
    actions: Vec<ActionSpec>,
}

#[derive(Debug, Deserialize)]
struct MenuSpec {
    id: String,
    title: String,
    #[serde(default)]
    platform: Option<Platform>,
    #[serde(default)]
    roles: Vec<RolePlacement>,
}

#[derive(Debug, Deserialize)]
struct RolePlacement {
    role: Role,
    group: u32,
    order: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Platform {
    Mac,
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Label {
    Same(String),
    PerPlatform {
        mac: Option<String>,
        other: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct Variant {
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Shortcuts {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct PerPlatform<T> {
    mac: Option<T>,
}

#[derive(Debug, Deserialize)]
struct Placement {
    menu: String,
    group: u32,
    order: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Binding {
    #[default]
    Dispatcher,
    Component,
    System,
}

#[derive(Debug, Deserialize)]
struct ActionSpec {
    id: String,
    label: Label,
    #[serde(default)]
    variants: Vec<Variant>,
    #[serde(default)]
    platform: Option<Platform>,
    #[serde(default)]
    binding: Binding,
    #[serde(default)]
    shortcut: Option<PerPlatform<Shortcuts>>,
    #[serde(default)]
    menu: Option<PerPlatform<Placement>>,
}

/// Why the manifest cannot give a bar. Caught by this module's tests: at
/// launch it only degrades the bar to Quit.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ManifestError {
    #[error("the action manifest is not valid JSON of the expected shape: {0}")]
    Unreadable(#[from] serde_json::Error),
    #[error("action {action} is placed in menu {menu}, which the manifest does not declare")]
    UnknownMenu { action: String, menu: String },
    #[error("action {0} is declared twice")]
    Duplicate(String),
    #[error("shortcut {shortcut} of {action} cannot be a native accelerator")]
    Accelerator { action: String, shortcut: String },
}

// ---- The description of the macOS bar -----------------------------------------

/// A system item of AppKit: it acts on the application, the window or the
/// focused text field, never on data (I-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Role {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Fullscreen,
    Minimize,
    Zoom,
}

/// An action entry of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionEntry {
    pub(crate) id: String,
    /// The default label, then the manifest's variants in order.
    pub(crate) labels: Vec<String>,
    /// In `muda`'s syntax; `None` for an entry shown without a combination.
    pub(crate) accelerator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Separator,
    Role(Role),
    Action(ActionEntry),
}

/// One menu of the bar, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) entries: Vec<Entry>,
}

/// Parses the manifest shipped with this build and describes the macOS bar.
///
/// # Errors
/// If the manifest is malformed, places an action in an undeclared menu,
/// declares an id twice, or gives an entry a combination `muda` cannot bind.
pub(crate) fn describe() -> Result<Vec<Block>, ManifestError> {
    describe_from(MANIFEST)
}

fn describe_from(source: &str) -> Result<Vec<Block>, ManifestError> {
    let manifest: Manifest = serde_json::from_str(source)?;
    let mut seen = std::collections::HashSet::new();
    for action in &manifest.actions {
        if !seen.insert(action.id.as_str()) {
            return Err(ManifestError::Duplicate(action.id.clone()));
        }
    }
    let mac_menus: Vec<&MenuSpec> = manifest
        .menus
        .iter()
        .filter(|menu| {
            menu.platform
                .is_none_or(|platform| platform == Platform::Mac)
        })
        .collect();
    let mut placed: HashMap<&str, Vec<(u32, u32, Entry)>> = HashMap::new();
    for action in &manifest.actions {
        if action
            .platform
            .is_some_and(|platform| platform != Platform::Mac)
        {
            continue;
        }
        let Some(placement) = action.menu.as_ref().and_then(|menu| menu.mac.as_ref()) else {
            continue;
        };
        if !mac_menus.iter().any(|menu| menu.id == placement.menu) {
            return Err(ManifestError::UnknownMenu {
                action: action.id.clone(),
                menu: placement.menu.clone(),
            });
        }
        let entry = Entry::Action(ActionEntry {
            id: action.id.clone(),
            labels: labels_of(action),
            accelerator: accelerator_of(action)?,
        });
        placed.entry(placement.menu.as_str()).or_default().push((
            placement.group,
            placement.order,
            entry,
        ));
    }
    let blocks = mac_menus
        .into_iter()
        .map(|menu| {
            let mut entries = placed.remove(menu.id.as_str()).unwrap_or_default();
            entries.extend(
                menu.roles
                    .iter()
                    .map(|role| (role.group, role.order, Entry::Role(role.role))),
            );
            entries.sort_by_key(|(group, order, _)| (*group, *order));
            let mut flat = Vec::with_capacity(entries.len());
            let mut current = None;
            for (group, _, entry) in entries {
                if current.is_some_and(|previous| previous != group) {
                    flat.push(Entry::Separator);
                }
                current = Some(group);
                flat.push(entry);
            }
            Block {
                id: menu.id.clone(),
                title: menu.title.clone(),
                entries: flat,
            }
        })
        .filter(|block| !block.entries.is_empty())
        .collect();
    Ok(blocks)
}

fn labels_of(action: &ActionSpec) -> Vec<String> {
    let label = match &action.label {
        Label::Same(label) => label.clone(),
        Label::PerPlatform { mac, other } => {
            mac.clone().or_else(|| other.clone()).unwrap_or_default()
        }
    };
    std::iter::once(label)
        .chain(action.variants.iter().map(|variant| variant.label.clone()))
        .collect()
}

/// The native accelerator of an action, in `muda`'s syntax.
///
/// Only a combination the dispatcher would bind becomes one: a `component`
/// or `system` binding is shown by its zone, and `Escape` never is an
/// accelerator — `Query ▸ Cancel` would take it from every dialog and
/// completion of the window (ADR-0041, point 4).
fn accelerator_of(action: &ActionSpec) -> Result<Option<String>, ManifestError> {
    if action.binding != Binding::Dispatcher {
        return Ok(None);
    }
    let first = match action
        .shortcut
        .as_ref()
        .and_then(|shortcut| shortcut.mac.as_ref())
    {
        None => return Ok(None),
        Some(Shortcuts::One(one)) => one.as_str(),
        Some(Shortcuts::Many(many)) => match many.first() {
            Some(first) => first.as_str(),
            None => return Ok(None),
        },
    };
    to_accelerator(first).map_err(|()| ManifestError::Accelerator {
        action: action.id.clone(),
        shortcut: first.to_owned(),
    })
}

/// `Mod+Shift+Enter` → `CmdOrCtrl+Shift+Enter`; `None` for `Escape`.
fn to_accelerator(shortcut: &str) -> Result<Option<String>, ()> {
    let mut parts: Vec<&str> = shortcut.split('+').collect();
    let key = parts.pop().filter(|key| !key.is_empty()).ok_or(())?;
    if key == "Escape" {
        return Ok(None);
    }
    let mut accelerator = Vec::with_capacity(parts.len() + 1);
    for part in parts {
        accelerator.push(match part {
            "Mod" => "CmdOrCtrl",
            "Ctrl" => "Ctrl",
            "Alt" => "Alt",
            "Shift" => "Shift",
            _ => return Err(()),
        });
    }
    let key = if key.chars().count() == 1 {
        key.to_uppercase()
    } else {
        key.to_owned()
    };
    Ok(Some(
        format!("{}+{key}", accelerator.join("+"))
            .trim_start_matches('+')
            .to_owned(),
    ))
}

// ---- The live bar --------------------------------------------------------------

/// What `set_menu_state` may change on one entry.
#[derive(Debug, Clone)]
struct Known {
    labels: Vec<String>,
    accelerator: Option<String>,
}

/// One change `set_menu_state` applies, once every entry has been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Change {
    id: String,
    enabled: bool,
    label: String,
    accelerator: Option<String>,
}

/// Why `set_menu_state` refused a whole update. Not retryable: the same state
/// is refused again.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum MenuStateError {
    #[error("the menu bar has no entry {0}")]
    UnknownEntry(String),
    #[error("the menu entry {id} has no label variant {variant}")]
    UnknownVariant { id: String, variant: usize },
}

/// The menu bar the application manages: the front's channel, and the
/// entries the front may switch.
#[derive(Default)]
pub struct MenuBar {
    channel: Mutex<Option<Channel<MenuActivation>>>,
    known: Mutex<HashMap<String, Known>>,
    #[cfg(target_os = "macos")]
    items: Mutex<HashMap<String, tauri::menu::MenuItem<tauri::Wry>>>,
}

impl MenuBar {
    /// Remembers the action entries of `blocks` as the ones the front may switch.
    fn learn(&self, blocks: &[Block]) {
        let mut known = self.known.lock();
        for block in blocks {
            for entry in &block.entries {
                if let Entry::Action(action) = entry {
                    known.insert(
                        action.id.clone(),
                        Known {
                            labels: action.labels.clone(),
                            accelerator: action.accelerator.clone(),
                        },
                    );
                }
            }
        }
    }

    /// Checks every entry before changing any: an update is applied whole or
    /// not at all, and an id the bar does not have is an error, not a skip.
    fn plan(&self, entries: &[MenuEntryState]) -> Result<Vec<Change>, MenuStateError> {
        let known = self.known.lock();
        entries
            .iter()
            .map(|entry| {
                let item = known
                    .get(&entry.id)
                    .ok_or_else(|| MenuStateError::UnknownEntry(entry.id.clone()))?;
                let label = item.labels.get(entry.variant).ok_or_else(|| {
                    MenuStateError::UnknownVariant {
                        id: entry.id.clone(),
                        variant: entry.variant,
                    }
                })?;
                // Quit stays enabled and bound to ⌘Q whatever the webview
                // says: a script there must not take the ordered exit away
                // (ADR-0038).
                let quit = entry.id == QUIT;
                Ok(Change {
                    id: entry.id.clone(),
                    enabled: entry.enabled || quit,
                    label: label.clone(),
                    accelerator: if entry.shortcut || quit {
                        item.accelerator.clone()
                    } else {
                        None
                    },
                })
            })
            .collect()
    }

    /// Applies the front's state to the native entries.
    ///
    /// Called on the main thread by a synchronous command: only `muda`
    /// setters, no I/O ([I-05](../../../CLAUDE.md#i-05)).
    ///
    /// # Errors
    /// [`MenuStateError`] for an unknown id or variant; nothing is changed then.
    pub(crate) fn apply(&self, entries: &[MenuEntryState]) -> Result<(), MenuStateError> {
        let changes = self.plan(entries)?;
        #[cfg(target_os = "macos")]
        {
            let items = self.items.lock();
            for change in changes {
                let Some(item) = items.get(&change.id) else {
                    continue;
                };
                // A setter that fails leaves that entry as it was; the next
                // context change sends its state again.
                if let Err(error) = item
                    .set_enabled(change.enabled)
                    .and_then(|()| item.set_text(&change.label))
                    .and_then(|()| item.set_accelerator(change.accelerator.as_deref()))
                {
                    tracing::warn!(entry = %change.id, %error, "could not update a menu entry");
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        drop(changes);
        Ok(())
    }

    /// The front's channel, replacing the previous one: a reloaded page
    /// subscribes again, and its predecessor's callbacks are gone.
    pub fn subscribe(&self, channel: Channel<MenuActivation>) {
        *self.channel.lock() = Some(channel);
    }

    /// Sends a chosen entry to the front. An id the bar did not build — a
    /// system item, another menu — is not the front's to run.
    pub fn forward(&self, id: &str) {
        if id == QUIT || !self.known.lock().contains_key(id) {
            return;
        }
        let Some(channel) = self.channel.lock().clone() else {
            tracing::debug!(entry = %id, "a menu entry was chosen before the front listened");
            return;
        };
        if let Err(error) = channel.send(MenuActivation { id: id.to_owned() }) {
            tracing::warn!(entry = %id, %error, "could not hand a menu entry to the front");
        }
    }
}

/// The macOS bar, from the manifest.
///
/// Every action entry starts disabled but Quit: the front enables what it can
/// run once it has loaded, and an entry chosen before that would reach no one.
/// If the manifest cannot be read — its tests make that a CI failure — the bar
/// keeps the application menu with Quit, without which ⌘Q would fall back to
/// `terminate:` and go unrecorded (ADR-0038).
///
/// # Errors
/// If a menu item cannot be created.
#[cfg(target_os = "macos")]
pub fn application_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::Manager as _;
    use tauri::menu::{
        AboutMetadata, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu, WINDOW_SUBMENU_ID,
    };

    let blocks = describe().unwrap_or_else(|error| {
        tracing::error!(%error, "the menu bar keeps Quit only");
        vec![Block {
            id: "app".to_owned(),
            title: String::new(),
            entries: vec![Entry::Role(Role::About), Entry::Separator, quit_entry()],
        }]
    });
    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        authors: app.config().bundle.publisher.clone().map(|p| vec![p]),
        ..AboutMetadata::default()
    };
    let menu = Menu::new(app)?;
    let mut items = HashMap::new();
    for block in &blocks {
        let title = if block.id == "app" {
            package.name.clone()
        } else {
            block.title.clone()
        };
        let submenu = if block.id == "window" {
            // Under this id, AppKit lists the open windows in it.
            Submenu::with_id(app, WINDOW_SUBMENU_ID, title, true)?
        } else {
            Submenu::with_id(app, block.id.as_str(), title, true)?
        };
        for entry in &block.entries {
            let item: Box<dyn IsMenuItem<tauri::Wry>> = match entry {
                Entry::Separator => Box::new(PredefinedMenuItem::separator(app)?),
                Entry::Role(role) => Box::new(match role {
                    Role::About => PredefinedMenuItem::about(app, None, Some(about.clone()))?,
                    Role::Services => PredefinedMenuItem::services(app, None)?,
                    Role::Hide => PredefinedMenuItem::hide(app, None)?,
                    Role::HideOthers => PredefinedMenuItem::hide_others(app, None)?,
                    Role::ShowAll => PredefinedMenuItem::show_all(app, None)?,
                    Role::Undo => PredefinedMenuItem::undo(app, None)?,
                    Role::Redo => PredefinedMenuItem::redo(app, None)?,
                    Role::Cut => PredefinedMenuItem::cut(app, None)?,
                    Role::Copy => PredefinedMenuItem::copy(app, None)?,
                    Role::Paste => PredefinedMenuItem::paste(app, None)?,
                    Role::SelectAll => PredefinedMenuItem::select_all(app, None)?,
                    Role::Fullscreen => PredefinedMenuItem::fullscreen(app, None)?,
                    Role::Minimize => PredefinedMenuItem::minimize(app, None)?,
                    Role::Zoom => PredefinedMenuItem::maximize(app, None)?,
                }),
                Entry::Action(action) => {
                    let label = action.labels.first().cloned().unwrap_or_default();
                    let item = MenuItem::with_id(
                        app,
                        action.id.as_str(),
                        label,
                        action.id == QUIT,
                        action.accelerator.as_deref(),
                    )?;
                    items.insert(action.id.clone(), item.clone());
                    Box::new(item)
                }
            };
            submenu.append(item.as_ref())?;
        }
        menu.append(&submenu)?;
    }
    if let Some(bar) = app.try_state::<MenuBar>() {
        bar.learn(&blocks);
        *bar.items.lock() = items;
    }
    Ok(menu)
}

#[cfg(target_os = "macos")]
fn quit_entry() -> Entry {
    Entry::Action(ActionEntry {
        id: QUIT.to_owned(),
        labels: vec!["Quit Oxyn".to_owned()],
        accelerator: Some("CmdOrCtrl+Q".to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries_of<'a>(blocks: &'a [Block], id: &str) -> &'a [Entry] {
        &blocks
            .iter()
            .find(|block| block.id == id)
            .unwrap_or_else(|| panic!("the bar has a {id} menu"))
            .entries
    }

    fn action<'a>(blocks: &'a [Block], id: &str) -> &'a ActionEntry {
        blocks
            .iter()
            .flat_map(|block| &block.entries)
            .find_map(|entry| match entry {
                Entry::Action(action) if action.id == id => Some(action),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the bar has {id}"))
    }

    #[test]
    fn the_shipped_manifest_describes_the_macos_bar() {
        let blocks = describe().expect("the manifest shipped with the front is valid");
        let titles: Vec<&str> = blocks.iter().map(|block| block.title.as_str()).collect();
        assert_eq!(titles, ["Oxyn", "File", "Edit", "View", "Query", "Window"]);
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
        let others: Vec<&str> = blocks
            .iter()
            .flat_map(|block| &block.entries)
            .filter_map(|entry| match entry {
                Entry::Action(action)
                    if action.id != "tab.close"
                        && action.accelerator.as_deref() == Some("CmdOrCtrl+W") =>
                {
                    Some(action.id.as_str())
                }
                _ => None,
            })
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
        let ids: Vec<&str> = blocks
            .iter()
            .flat_map(|block| &block.entries)
            .filter_map(|entry| match entry {
                Entry::Action(action) => Some(action.id.as_str()),
                _ => None,
            })
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
        };
        let changes = bar.plan(&[quit]).expect("Quit is in the bar");
        let change = changes.first().expect("one change");
        assert!(change.enabled);
        assert_eq!(change.accelerator.as_deref(), Some("CmdOrCtrl+Q"));
    }

    #[test]
    fn an_unknown_bar_forwards_nothing() {
        // Without a native bar (Windows, Linux), no id is the bar's.
        let bar = MenuBar::default();
        assert!(bar.apply(&[state("console.run", 0, true)]).is_err());
        bar.forward("console.run");
    }
}
