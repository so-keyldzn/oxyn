//! IPC commands over a result the backend holds: pages, search, value pages,
//! export.
//!
//! None of them runs a query: a result is read from its buffer, and one that
//! retention let go is answered « expired », never recreated (ADR-0017).

use oxyn_core::ResultId;
use tauri::{State, Webview};
use tauri_plugin_dialog::DialogExt;

use super::parse;
use super::windows::{caller, run_owned};
use crate::backend::Backend;
use crate::ipc::results::{
    CopiedRows, CopyRowsRequest, CopySpec, ExportFormatChoice, FindAnswer, ResultWindow,
    ValuePageView, export_format, suggested_file_name,
};
use crate::ipc::{CommandOutcome, IpcError, ResultColumn};

/// The result `result` names, if this window reads it (ADR-0043).
fn readable(backend: &Backend, webview: &Webview, result: &str) -> Result<ResultId, IpcError> {
    let window = caller(backend, webview)?;
    let result = parse("result", result)?;
    backend.inner.windows.check_result(window, result)?;
    Ok(result)
}

/// A window of rows through `ReadResultPage` for what spilled (ADR-0012).
#[tauri::command]
pub async fn read_result_page(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    result: String,
    offset: usize,
    limit: usize,
) -> Result<ResultWindow, IpcError> {
    let result = readable(&backend, &webview, &result)?;
    backend
        .read_result_page(parse("connection", &connection)?, result, offset, limit)
        .await
}

/// The columns of a held result, from its schema; `null` once it expired.
#[tauri::command]
pub async fn result_columns(
    webview: Webview,
    backend: State<'_, Backend>,
    result: String,
) -> Result<Option<Vec<ResultColumn>>, IpcError> {
    Ok(backend.result_columns(readable(&backend, &webview, &result)?))
}

#[tauri::command]
pub async fn forget_result(
    webview: Webview,
    backend: State<'_, Backend>,
    result: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let result = readable(&backend, &webview, &result)?;
    backend.inner.windows.forget_result(window, result);
    // Forgetting may delete spill files.
    backend
        .on_blocking_pool(move |backend| {
            backend.forget_result(result);
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn export_formats(
    backend: State<'_, Backend>,
) -> Result<Vec<ExportFormatChoice>, IpcError> {
    Ok(backend.export_formats())
}

/// Exports a result to a file the user picks in the native save dialog.
///
/// The dialog is opened **here**, not by the webview: the path goes from the
/// dialog to the writer without crossing the bridge, so a script in the
/// webview cannot choose where Oxyn writes (SECURITY, « Surface d'entrée »).
/// The webview only suggests a file name. `null` when the user dismissed the
/// dialog: nothing was written and nothing ran.
#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "the names the front sends, plus the calling webview Tauri injects (ADR-0043)"
)]
pub async fn export_result(
    app: tauri::AppHandle,
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    result: String,
    format: String,
    suggested_name: String,
) -> Result<Option<CommandOutcome>, IpcError> {
    let format = export_format(&format)
        .ok_or_else(|| IpcError::invalid(format!("unsupported export format: {format}")))?;
    if !oxyn_data::is_supported(format) {
        // Listed as unavailable, never offered: refusing before the dialog
        // leaves no empty file behind (UX-SPEC).
        return Err(IpcError::invalid(format!(
            "{format} export is not written yet"
        )));
    }
    let window = caller(&backend, &webview)?;
    let command_id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    let result = readable(&backend, &webview, &result)?;
    let extension = format.extension();
    let file_name = suggested_file_name(&suggested_name, extension);
    let label = format.to_string();
    // The dialog blocks until answered: on the blocking pool, never on a
    // runtime worker nor on the main thread (I-05).
    let chosen = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Export result")
            .set_file_name(file_name)
            .add_filter(label, &[extension])
            .blocking_save_file()
    })
    .await
    .map_err(|_| IpcError::invalid("The save dialog stopped"))?;
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let destination = chosen
        .into_path()
        .map_err(|_| IpcError::invalid("The chosen destination is not a local file"))?;
    run_owned(
        &backend,
        window,
        command_id,
        backend.export(command_id, connection, result, format, destination),
        |_| false,
    )
    .await
    .map(Some)
}

/// Rows of a held result as text for the clipboard; runs nothing.
///
/// `async`: the rows may have spilled to disk, and are read on the blocking
/// pool ([I-05](../../../../CLAUDE.md#i-05)).
#[tauri::command]
pub async fn copy_result_rows(
    webview: Webview,
    backend: State<'_, Backend>,
    request: CopyRowsRequest,
) -> Result<CopiedRows, IpcError> {
    let CopyRowsRequest {
        result,
        offset,
        count,
        columns,
        format,
        header,
        connection,
        address,
    } = request;
    let connection = connection
        .as_deref()
        .map(|connection| parse("connection", connection))
        .transpose()?;
    backend
        .copy_result_rows(
            readable(&backend, &webview, &result)?,
            connection,
            address,
            CopySpec {
                offset,
                count,
                columns,
                format,
                header,
            },
        )
        .await
}

/// `null` when the result has expired.
#[tauri::command]
pub async fn find_in_result(
    webview: Webview,
    backend: State<'_, Backend>,
    result: String,
    needle: String,
    from: usize,
    forward: bool,
) -> Result<Option<FindAnswer>, IpcError> {
    backend
        .find_in_result(
            readable(&backend, &webview, &result)?,
            needle,
            from,
            forward,
        )
        .await
}

/// `null` when the result has expired.
#[tauri::command]
pub async fn find_matches_in_window(
    webview: Webview,
    backend: State<'_, Backend>,
    result: String,
    needle: String,
    offset: usize,
    limit: usize,
) -> Result<Option<Vec<usize>>, IpcError> {
    backend
        .find_matches_in_window(
            readable(&backend, &webview, &result)?,
            needle,
            offset,
            limit,
        )
        .await
}

/// One page of one value; `null` when the result has expired. Cancellable
/// with `cancel` under the same command id.
#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "the names the front sends, plus the calling webview Tauri injects (ADR-0043)"
)]
pub async fn inspect_value(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    result: String,
    row: usize,
    column: usize,
    offset: usize,
) -> Result<Option<ValuePageView>, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let result = readable(&backend, &webview, &result)?;
    run_owned(
        &backend,
        window,
        id,
        backend.inspect_value(
            id,
            parse("connection", &connection)?,
            result,
            row,
            column,
            offset,
        ),
        |_| false,
    )
    .await
}
