//! The restored object tab.
//!
//! What these widen for a script in the webview: reading which object was
//! shown last, and on which connection — a name the catalog tree already
//! shows — and rewriting that place. Both go through `ReadWorkspacePreferences` and
//! `WriteWorkspacePreferences`, as every preference does; neither reads a
//! server or a catalog.

use tauri::State;

use crate::backend::Backend;
use crate::commands::parse;
use crate::ipc::IpcError;
use crate::ipc::location::{ObjectPlace, SavedObjectPlace};

#[tauri::command]
pub async fn read_object_location(
    backend: State<'_, Backend>,
) -> Result<Option<SavedObjectPlace>, IpcError> {
    backend.read_object_location().await
}

#[tauri::command]
pub async fn write_object_location(
    backend: State<'_, Backend>,
    connection: String,
    place: Option<ObjectPlace>,
) -> Result<(), IpcError> {
    backend
        .write_object_location(parse("connection", &connection)?, place)
        .await
}
