//! Window/session ownership is checked before any schema-map provider access.
use crate::backend::Backend;
use crate::commands::parse;
use crate::commands::windows::{caller, run_owned};
use crate::ipc::IpcError;
use tauri::{State, Webview};

#[tauri::command]
pub async fn load_schema_map(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    request: oxyn_core::SchemaMapRequest,
) -> Result<oxyn_catalog::schema_map::SchemaMapLoaded, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let _using = backend.inner.workbench.consoles.run_on(request.session)?;
    backend
        .inner
        .windows
        .check_session(window, request.session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.load_schema_map(id, request),
        |_| false,
    )
    .await
}
