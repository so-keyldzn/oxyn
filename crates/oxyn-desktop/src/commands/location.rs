//! The restored object tab, per window.
//!
//! What these widen for a script in the webview: reading which object its
//! own window showed last, and on which connection — a name the catalog tree
//! already shows — and rewriting that place. The write goes through
//! `WriteWindowLayout`, as every layout write does; neither reads a server
//! or a catalog, and neither reaches another window's line.

use tauri::{State, Webview};

use crate::backend::Backend;
use crate::commands::parse;
use crate::commands::windows::caller;
use crate::ipc::IpcError;
use crate::ipc::location::{ObjectPlace, SavedObjectPlace};

#[tauri::command]
pub fn read_object_location(
    webview: Webview,
    backend: State<'_, Backend>,
) -> Result<Option<SavedObjectPlace>, IpcError> {
    let window = caller(&backend, &webview)?;
    Ok(backend.read_object_location(window))
}

#[tauri::command]
pub async fn write_object_location(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    place: Option<ObjectPlace>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    backend
        .write_object_location(window, parse("connection", &connection)?, place)
        .await
}
