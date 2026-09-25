//! The pipe that carries dropped files to the front
//! ([ADR-0041](../../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 9).
//!
//! Interface plumbing, like `subscribe_menu`: it emits no `Command`. The
//! command takes no path — the webview cannot ask Rust to read a file of its
//! choosing; only a drop the window received is read (`file_drop.rs`).

use tauri::ipc::Channel;
use tauri::{State, Webview};

use crate::backend::Backend;
use crate::commands::windows::caller;
use crate::file_drop::FileDrops;
use crate::ipc::IpcError;
use crate::ipc::file_drops::DroppedFile;

/// Where files dropped on this window are sent, classified, and only those.
/// A channel rather than an event:
/// `listen` would need the `core:event` permission, which
/// `capabilities/main.json` does not grant. Synchronous: it stores a handle,
/// no I/O (I-05).
#[tauri::command]
pub fn subscribe_file_drops(
    webview: Webview,
    backend: State<'_, Backend>,
    drops: State<'_, FileDrops>,
    channel: Channel<DroppedFile>,
) -> Result<(), IpcError> {
    caller(&backend, &webview)?;
    drops.subscribe(webview.label(), channel);
    Ok(())
}
