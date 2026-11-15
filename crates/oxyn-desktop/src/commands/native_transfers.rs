//! Native selections are made by the host; all program execution remains an owned human bus command.
use crate::backend::Backend;
use crate::commands::{
    parse,
    windows::{caller, run_owned},
};
use crate::ipc::{
    IpcError,
    native_transfers::{NativePreparationDraft, SelectedNativeFile},
};
use oxyn_core::{
    ConnectionId, NativeToolInfo, NativeToolRole, NativeTransferFormat, NativeTransferId,
    NativeTransferJob, NativeTransferOperation, NativeTransferRequest, NativeTransferReview,
    NativeTransferSupport, SessionId,
};
use tauri::{AppHandle, State, Webview};
use tauri_plugin_dialog::DialogExt;

fn checked(
    backend: &Backend,
    webview: &Webview,
    connection: &str,
    session: &str,
) -> Result<(crate::backend::windows::WindowKey, ConnectionId, SessionId), IpcError> {
    let window = caller(backend, webview)?;
    let connection = parse("connection", connection)?;
    let session = parse("session", session)?;
    backend.inner.windows.check_session(window, session)?;
    backend
        .inner
        .executor
        .session_catalog(connection, session)?;
    Ok((window, connection, session))
}
#[tauri::command]
pub async fn native_transfer_support(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
) -> Result<NativeTransferSupport, IpcError> {
    let (window, connection, session) = checked(&backend, &webview, &connection, &session)?;
    let id = parse("command id", &command_id)?;
    let _using = backend.inner.workbench.consoles.run_on(session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.native_transfer_support(id, connection, session),
        |_| false,
    )
    .await
}
#[tauri::command]
pub async fn select_native_tool(
    app: AppHandle,
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    role: NativeToolRole,
) -> Result<Option<NativeToolInfo>, IpcError> {
    let (window, connection, session) = checked(&backend, &webview, &connection, &session)?;
    let id = parse("command id", &command_id)?;
    let _using = backend.inner.workbench.consoles.run_on(session)?;
    let path = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Select a trusted installed native tool; Oxyn runs --version to verify it")
            .blocking_pick_file()
    })
    .await
    .map_err(|_| IpcError::invalid("The native tool picker stopped"))?;
    let Some(path) = path else { return Ok(None) };
    let path = path
        .into_path()
        .map_err(|_| IpcError::invalid("Select an installed local executable"))?;
    backend.inner.windows.check_session(window, session)?;
    let grant = backend
        .on_blocking_pool(move |backend| {
            let config = backend.config(connection)?;
            backend
                .inner
                .native_runner
                .select_tool(window.id(), config.driver, role, &path)
                .map_err(|error| IpcError::invalid(error.to_string()))
        })
        .await?;
    run_owned(
        &backend,
        window,
        id,
        backend.inspect_native_tool(id, window.id(), grant),
        |_| false,
    )
    .await
    .map(Some)
}
#[tauri::command]
pub async fn select_native_transfer_file(
    app: AppHandle,
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    session: String,
    operation: NativeTransferOperation,
    format: NativeTransferFormat,
) -> Result<Option<SelectedNativeFile>, IpcError> {
    let (window, connection, session) = checked(&backend, &webview, &connection, &session)?;
    let _using = backend.inner.workbench.consoles.run_on(session)?;
    if operation == NativeTransferOperation::Restore && format == NativeTransferFormat::Sql {
        return Err(IpcError::invalid(
            "SQL-script restore remains unavailable; select a supported native archive",
        ));
    }
    let path = tokio::task::spawn_blocking(move || match operation {
        NativeTransferOperation::Export => {
            let extension = if format == NativeTransferFormat::Sql {
                "sql"
            } else {
                "dump"
            };
            app.dialog()
                .file()
                .set_title("Export database to a new file; existing paths are refused")
                .set_file_name(format!("database-export.{extension}"))
                .add_filter("Database export", &[extension])
                .blocking_save_file()
        }
        NativeTransferOperation::Restore => app
            .dialog()
            .file()
            .set_title("Select a trusted native archive; its contents are not certified")
            .add_filter("Native database archive", &["dump", "backup", "archive"])
            .blocking_pick_file(),
        _ => None,
    })
    .await
    .map_err(|_| IpcError::invalid("The native file picker stopped"))?;
    let Some(path) = path else { return Ok(None) };
    let path = path
        .into_path()
        .map_err(|_| IpcError::invalid("Select a local native transfer file"))?;
    backend.inner.windows.check_session(window, session)?;
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.len() <= 4096)
        .ok_or_else(|| IpcError::invalid("The selected filename cannot be represented safely"))?
        .to_owned();
    backend
        .on_blocking_pool(move |backend| {
            let source = oxyn_transfer::SourceBinding {
                connection,
                session,
            };
            let grant = match operation {
                NativeTransferOperation::Export => {
                    backend
                        .inner
                        .native_runner
                        .select_output(window.id(), source, format, &path)
                }
                NativeTransferOperation::Restore => {
                    backend
                        .inner
                        .native_runner
                        .select_archive(window.id(), source, &path)
                }
                _ => return Err(IpcError::invalid("Unsupported native operation")),
            }
            .map_err(|error| IpcError::invalid(error.to_string()))?;
            Ok(Some(SelectedNativeFile { grant, name: label }))
        })
        .await
}
#[tauri::command]
pub async fn prepare_native_transfer(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    request: NativePreparationDraft,
) -> Result<NativeTransferReview, IpcError> {
    let window = caller(&backend, &webview)?;
    backend
        .inner
        .windows
        .check_session(window, request.session)?;
    backend
        .inner
        .executor
        .session_catalog(request.connection, request.session)?;
    let _using = backend.inner.workbench.consoles.run_on(request.session)?;
    let id = parse("command id", &command_id)?;
    let request = NativeTransferRequest {
        connection: request.connection,
        session: request.session,
        owner: window.id(),
        operation: request.operation,
        format: request.format,
        tool: request.tool,
        file: request.file,
        no_owner: request.no_owner,
        no_privileges: request.no_privileges,
    };
    run_owned(
        &backend,
        window,
        id,
        backend.prepare_native_transfer(id, request),
        |_| false,
    )
    .await
}
#[tauri::command]
pub async fn start_native_transfer(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    transfer: String,
) -> Result<NativeTransferJob, IpcError> {
    let (window, connection, session) = checked(&backend, &webview, &connection, &session)?;
    let id = parse("command id", &command_id)?;
    let transfer: NativeTransferId = parse("native transfer", &transfer)?;
    let review = backend
        .inner
        .executor
        .native_review(connection, window.id(), transfer)?;
    if review.job.session != session {
        return Err(IpcError::invalid(
            "The native preparation belongs to another original session",
        ));
    }
    let _using = backend.inner.workbench.consoles.run_on(session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.start_native_transfer(id, connection, window.id(), transfer),
        |_| false,
    )
    .await
}
#[tauri::command]
pub async fn discard_native_transfer(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    transfer: String,
) -> Result<NativeTransferJob, IpcError> {
    let window = caller(&backend, &webview)?;
    let connection = parse("connection", &connection)?;
    let _original_session: SessionId = parse("session", &session)?;
    // Releasing this window's plan must remain possible after its console
    // moves or retires. The bus checks the immutable job/connection/window
    // identity; cleanup grants no session access or execution authority.
    let id = parse("command id", &command_id)?;
    let transfer = parse("native transfer", &transfer)?;
    run_owned(
        &backend,
        window,
        id,
        backend.discard_native_transfer(id, connection, window.id(), transfer),
        |_| false,
    )
    .await
}
#[tauri::command]
pub async fn native_transfer_jobs(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
) -> Result<Vec<NativeTransferJob>, IpcError> {
    let (window, connection, _) = checked(&backend, &webview, &connection, &session)?;
    let id = parse("command id", &command_id)?;
    run_owned(
        &backend,
        window,
        id,
        backend.native_transfer_jobs(id, connection),
        |_| false,
    )
    .await
}
#[tauri::command]
pub async fn reconcile_native_transfer(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    transfer: String,
) -> Result<NativeTransferJob, IpcError> {
    let (window, connection, _) = checked(&backend, &webview, &connection, &session)?;
    let id = parse("command id", &command_id)?;
    let transfer = parse("native transfer", &transfer)?;
    run_owned(
        &backend,
        window,
        id,
        backend.reconcile_native_transfer(id, connection, transfer),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn revoke_native_selection(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    grant: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let grant = parse("native selection", &grant)?;
    run_owned(
        &backend,
        window,
        id,
        backend.revoke_native_selection(id, window.id(), grant),
        |_| false,
    )
    .await
}
#[tauri::command]
pub async fn native_transfer_progress(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    transfer: String,
) -> Result<Option<NativeTransferJob>, IpcError> {
    let window = caller(&backend, &webview)?;
    let connection = parse("connection", &connection)?;
    let transfer = parse("native transfer", &transfer)?;
    backend
        .on_blocking_pool(move |backend| {
            Ok(backend
                .inner
                .executor
                .native_transfer_progress(connection, window.id(), transfer))
        })
        .await
}
