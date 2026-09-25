//! Settings commands: display preferences and saved-connection management.
//!
//! What these widen for a script in the webview: rewriting display
//! preferences, reading the **non-secret** parameters of a saved connection,
//! and asking to edit or delete one. Every edit and deletion is a `Command` the
//! policy judges, a production connection's included; no secret is ever read
//! back ([ADR-0029](../../../../docs/adr/0029-interface-tauri-shadcn.md)).

use oxyn_core::{CommandId, ConnectionId};
use tauri::{State, Webview};

use crate::backend::{Backend, WindowKey};
use crate::commands::parse;
use crate::commands::windows::{caller, run_owned};
use crate::ipc::settings::{
    ConnectionChange, ConnectionDetails, ConnectionEdit, PreferencesChange, PreferencesSaved,
    PreferencesState,
};
use crate::ipc::windows::WindowSignal;
use crate::ipc::{IpcError, SavedConnection};

#[tauri::command]
pub async fn read_preferences(backend: State<'_, Backend>) -> Result<PreferencesState, IpcError> {
    backend.read_preferences().await
}

/// The other windows are told, on their own channel, to read the
/// preferences again (ADR-0043).
#[tauri::command]
pub async fn write_preferences(
    webview: Webview,
    backend: State<'_, Backend>,
    change: PreferencesChange,
) -> Result<PreferencesSaved, IpcError> {
    let window = caller(&backend, &webview)?;
    let saved = backend.write_preferences(change).await?;
    let windows = &backend.inner.windows;
    for other in windows.keys().into_iter().filter(|key| *key != window) {
        windows.signal(other, WindowSignal::PreferencesChanged);
    }
    Ok(saved)
}

/// An edit of a saved connection holds at once for every window: each one
/// that holds it is told to read it again (ADR-0043).
fn announce_change(backend: &Backend, connection: ConnectionId, change: Option<&ConnectionChange>) {
    if !matches!(change, Some(ConnectionChange::Saved { .. })) {
        return;
    }
    let windows = &backend.inner.windows;
    for holder in windows.holders(connection) {
        windows.signal(
            holder,
            WindowSignal::ConnectionChanged {
                connection: connection.to_string(),
            },
        );
    }
}

// Async, like every command that reads the store: a synchronous command runs
// on the main thread and would freeze the window on a slow disk (I-05).
#[tauri::command]
pub async fn connection_details(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<ConnectionDetails, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.connection_details(connection))
        .await
}

#[tauri::command]
pub async fn update_connection(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    edit: ConnectionEdit,
) -> Result<Option<ConnectionChange>, IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    let change = run_owned(
        &backend,
        window,
        id,
        backend.update_connection(id, connection, edit),
        |change| matches!(change, Some(ConnectionChange::Approval { .. })),
    )
    .await?;
    announce_change(&backend, connection, change.as_ref());
    Ok(change)
}

/// Refused while another window holds the connection: its consoles would
/// lose the configuration they run on (ADR-0043).
#[tauri::command]
pub async fn delete_connection(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<ConnectionChange, IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    refuse_if_held_elsewhere(&backend, window, connection)?;
    run_owned(
        &backend,
        window,
        id,
        backend.delete_connection(id, connection),
        |change| matches!(change, ConnectionChange::Approval { .. }),
    )
    .await
}

fn refuse_if_held_elsewhere(
    backend: &Backend,
    window: WindowKey,
    connection: ConnectionId,
) -> Result<(), IpcError> {
    if backend
        .inner
        .windows
        .other_holders(window, connection)
        .is_empty()
    {
        Ok(())
    } else {
        Err(IpcError::invalid(
            "This connection is open in another window. Close it there first.",
        ))
    }
}

#[tauri::command]
pub async fn decide_connection_change(
    webview: Webview,
    backend: State<'_, Backend>,
    command: String,
    approved: bool,
) -> Result<Option<ConnectionChange>, IpcError> {
    let window = caller(&backend, &webview)?;
    let command: CommandId = parse("command id", &command)?;
    backend.inner.windows.check_command(window, command)?;
    let target = backend.pending_change_connection(command);
    if approved && let Some((connection, true)) = target {
        refuse_if_held_elsewhere(&backend, window, connection)?;
    }
    let change = run_owned(
        &backend,
        window,
        command,
        backend.decide_connection_change(command, approved),
        |_| false,
    )
    .await?;
    if let Some((connection, _)) = target {
        announce_change(&backend, connection, change.as_ref());
    }
    Ok(change)
}

#[tauri::command]
pub async fn connection_marking(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<SavedConnection, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.connection_marking(connection))
        .await
}
