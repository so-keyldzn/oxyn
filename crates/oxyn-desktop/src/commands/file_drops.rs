//! The pipe that carries dropped files to the front
//! ([ADR-0041](../../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 9).
//!
//! Interface plumbing, like `subscribe_menu`: it emits no `Command`. The
//! command takes no path — the webview cannot ask Rust to read a file of its
//! choosing; only a drop the window received is read (`file_drop.rs`).

use tauri::State;
use tauri::ipc::Channel;

use crate::file_drop::FileDrops;
use crate::ipc::file_drops::DroppedFile;

/// Where dropped files are sent, classified. A channel rather than an event:
/// `listen` would need the `core:event` permission, which
/// `capabilities/main.json` does not grant. Synchronous: it stores a handle,
/// no I/O (I-05).
#[tauri::command]
pub fn subscribe_file_drops(drops: State<'_, FileDrops>, channel: Channel<DroppedFile>) {
    drops.subscribe(channel);
}
