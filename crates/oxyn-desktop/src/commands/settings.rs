//! Settings commands: display preferences and saved-connection management.
//!
//! What these widen for a script in the webview: rewriting display
//! preferences, reading the **non-secret** parameters of a saved connection,
//! and asking to edit or delete one. Every edit and deletion is a `Command` the
//! policy judges, a production connection's included; no secret is ever read
//! back ([ADR-0029](../../../../docs/adr/0029-interface-tauri-shadcn.md)).

use oxyn_core::CommandId;
use tauri::State;

use crate::backend::Backend;
use crate::commands::parse;
use crate::ipc::settings::{
    ConnectionChange, ConnectionDetails, ConnectionEdit, PreferencesChange, PreferencesSaved,
    PreferencesState,
};
use crate::ipc::{IpcError, SavedConnection};

#[tauri::command]
pub async fn read_preferences(backend: State<'_, Backend>) -> Result<PreferencesState, IpcError> {
    backend.read_preferences().await
}

#[tauri::command]
pub async fn write_preferences(
    backend: State<'_, Backend>,
    change: PreferencesChange,
) -> Result<PreferencesSaved, IpcError> {
    backend.write_preferences(change).await
}

// Async, like every command that reads the store: a synchronous command runs
// on the main thread and would freeze the window on a slow disk (I-05).
#[tauri::command]
pub async fn connection_details(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<ConnectionDetails, IpcError> {
    backend.connection_details(parse("connection", &connection)?)
}

#[tauri::command]
pub async fn update_connection(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    edit: ConnectionEdit,
) -> Result<ConnectionChange, IpcError> {
    let id: CommandId = parse("command id", &command_id)?;
    backend
        .update_connection(id, parse("connection", &connection)?, edit)
        .await
}

#[tauri::command]
pub async fn delete_connection(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<ConnectionChange, IpcError> {
    let id: CommandId = parse("command id", &command_id)?;
    backend
        .delete_connection(id, parse("connection", &connection)?)
        .await
}

#[tauri::command]
pub async fn decide_connection_change(
    backend: State<'_, Backend>,
    command: String,
    approved: bool,
) -> Result<Option<ConnectionChange>, IpcError> {
    backend
        .decide_connection_change(parse("command id", &command)?, approved)
        .await
}

#[tauri::command]
pub async fn connection_marking(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<SavedConnection, IpcError> {
    backend.connection_marking(parse("connection", &connection)?)
}
