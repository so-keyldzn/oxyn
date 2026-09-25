//! The menu bar's two pipes to the front
//! ([ADR-0041](../../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 4).
//!
//! Interface plumbing, like `subscribe_events`: neither command emits a
//! `Command`, because neither acts on data. A chosen entry reaches the front
//! as an id, and the front runs the action through the same IPC functions as
//! its button (I-01).

use tauri::ipc::Channel;
use tauri::{State, Webview};

use crate::backend::{Backend, WindowKey};
use crate::commands::windows::caller;
use crate::ipc::IpcError;
use crate::ipc::menu::{MenuActivation, MenuEntryState};
use crate::menu::MenuBar;

/// Where the native bar sends the entries chosen in it while this window has
/// the focus. A channel rather than an event: `listen` would need the
/// `core:event` permission, which `capabilities/main.json` does not grant.
#[tauri::command]
pub fn subscribe_menu(
    webview: Webview,
    backend: State<'_, Backend>,
    bar: State<'_, MenuBar>,
    channel: Channel<MenuActivation>,
) -> Result<(), IpcError> {
    caller(&backend, &webview)?;
    bar.subscribe(webview.label(), channel);
    Ok(())
}

/// This window's state of the native entries, as its context says: recorded,
/// and shown while it has the focus (ADR-0043). Synchronous, so on the main
/// thread where `muda` wants it: the body is setters on items already in
/// memory, no I/O (I-05).
///
/// # Errors
/// An id the bar does not have, or a label variant it does not declare —
/// the whole update is refused, and resending it changes nothing.
#[tauri::command]
pub fn set_menu_state(
    webview: Webview,
    backend: State<'_, Backend>,
    bar: State<'_, MenuBar>,
    entries: Vec<MenuEntryState>,
) -> Result<(), IpcError> {
    caller(&backend, &webview)?;
    let focused = backend.inner.windows.focused().map(WindowKey::label);
    bar.report(webview.label(), &entries, focused.as_deref())
        .map_err(|error| IpcError {
            message: error.to_string(),
            retryable: false,
        })
}
