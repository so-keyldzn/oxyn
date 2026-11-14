//! Local grouping only; every request is attributed to its calling window.

use oxyn_core::{CommandId, ServerProfile, ServerProfileSnapshot};
use tauri::{State, Webview};

use crate::backend::Backend;
use crate::commands::parse;
use crate::commands::windows::{caller, run_owned};
use crate::ipc::IpcError;

#[tauri::command]
pub async fn list_server_profiles(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
) -> Result<Vec<ServerProfileSnapshot>, IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    run_owned(
        &backend,
        window,
        id,
        backend.list_server_profiles(id),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn save_server_profile(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    profile: ServerProfile,
    expected_revision: Option<u64>,
) -> Result<ServerProfileSnapshot, IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    run_owned(
        &backend,
        window,
        id,
        backend.save_server_profile(id, profile, expected_revision),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn remove_server_profile(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    profile: String,
    expected_revision: u64,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    let profile = parse("server profile", &profile)?;
    run_owned(
        &backend,
        window,
        id,
        backend.remove_server_profile(id, profile, expected_revision),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn set_server_profile_membership(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    profile: Option<String>,
    expected_profile: Option<String>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    let profile = profile
        .as_deref()
        .map(|value| parse("server profile", value))
        .transpose()?;
    let expected_profile = expected_profile
        .as_deref()
        .map(|value| parse("previous server profile", value))
        .transpose()?;
    run_owned(
        &backend,
        window,
        id,
        backend.set_server_profile_membership(id, connection, profile, expected_profile),
        |_| false,
    )
    .await
}

/// Discover through an open seed owned by this workspace.
#[tauri::command]
pub async fn discover_databases(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<Vec<oxyn_core::DiscoveredDatabase>, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    backend.check_discovery_owner(window, connection)?;
    run_owned(
        &backend,
        window,
        id,
        backend.discover_databases(id, connection),
        |_| false,
    )
    .await
}

/// Reserve a window slot and create/open one independently reviewed child.
#[tauri::command]
pub async fn open_database(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    profile: String,
    seed: String,
    draft: crate::ipc::ConnectionDraft,
) -> Result<crate::ipc::ConnectResponse, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let profile = parse("server profile", &profile)?;
    let seed = parse("seed connection", &seed)?;
    backend.check_discovery_owner(window, seed)?;
    let opening = backend.inner.windows.begin_opening(window, id)?;
    let response = run_owned(
        &backend,
        window,
        id,
        backend.open_database(id, profile, seed, draft),
        |response| matches!(response, crate::ipc::ConnectResponse::Approval { .. }),
    )
    .await?;
    if let crate::ipc::ConnectResponse::Open(open) = &response {
        backend.adopt_open(opening, open).await?;
    }
    Ok(response)
}

/// Discard exactly a cancelled opening's owned sessions; repeat is harmless.
#[tauri::command]
pub async fn discard_database_opening(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    catalog: String,
    console: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    backend
        .discard_opening(
            window,
            parse("connection", &connection)?,
            parse("catalog session", &catalog)?,
            parse("console session", &console)?,
        )
        .await
}

/// Lifecycle facts only: refuse disconnect while operations/transactions remain.
#[tauri::command]
pub async fn connection_disconnect_ready(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    run_owned(
        &backend,
        window,
        id,
        backend.connection_disconnect_ready(window, id, connection),
        |_| false,
    )
    .await
}
