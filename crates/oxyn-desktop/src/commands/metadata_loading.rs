//! Window-attributed loading preferences, through typed executor commands.

use oxyn_core::{MetadataLoadingPolicy, MetadataPolicySnapshot};
use tauri::{State, Webview};

use crate::backend::Backend;
use crate::commands::parse;
use crate::commands::windows::{caller, run_owned};
use crate::ipc::IpcError;

#[tauri::command]
pub async fn read_metadata_policy(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<MetadataPolicySnapshot, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    run_owned(
        &backend,
        window,
        id,
        backend.read_metadata_policy(id, connection),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn write_metadata_policy(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    policy: MetadataLoadingPolicy,
    expected_revision: u64,
) -> Result<MetadataPolicySnapshot, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    run_owned(
        &backend,
        window,
        id,
        backend.write_metadata_policy(id, connection, policy, expected_revision),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn preload_metadata(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    request: oxyn_core::MetadataPreloadRequest,
) -> Result<oxyn_core::MetadataPreloadProgress, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    backend
        .inner
        .windows
        .check_session(window, request.session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.preload_metadata(id, request),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn metadata_preload_progress(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
) -> Result<Option<oxyn_core::MetadataPreloadProgress>, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let progress = backend.metadata_preload_progress(id);
    if progress.is_some() {
        backend.inner.windows.check_command(window, id)?;
    }
    Ok(progress)
}
