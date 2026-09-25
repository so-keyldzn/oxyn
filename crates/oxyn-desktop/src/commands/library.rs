//! Query documents and local history commands. None reaches a database.

use oxyn_core::DocumentId;
use tauri::{State, Webview};

use super::parse;
use super::windows::{caller, run_owned};
use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::library::{
    DocumentChange, DocumentList, DocumentQuery, DocumentView, DocumentWrite,
    HistoryConnectionList, HistoryDetail, HistoryList, HistoryQuery, RetainedResult,
};

/// A fresh document identity. Minted here rather than in the webview: library
/// pages order documents by identifier age, which a random UUID would break.
#[tauri::command]
pub fn new_query_document() -> String {
    oxyn_core::DocumentId::new().to_string()
}

/// Only consoles save: the document becomes this window's, and a console of
/// another window can no longer write it (ADR-0043).
#[tauri::command]
pub async fn save_query_document(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    change: DocumentChange,
) -> Result<DocumentWrite, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let document: DocumentId = parse("document", &change.document)?;
    backend.inner.windows.claim_document(window, document)?;
    let connection = change
        .connection
        .as_deref()
        .map(|connection| parse("connection", connection))
        .transpose()?;
    run_owned(
        &backend,
        window,
        id,
        backend.save_query_document(id, document, connection, change),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn close_query_document(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    document: String,
    revision: u64,
    discard: bool,
) -> Result<DocumentWrite, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let document: DocumentId = parse("document", &document)?;
    backend.inner.windows.check_document(window, document)?;
    let write = run_owned(
        &backend,
        window,
        id,
        backend.close_query_document(id, document, revision, discard),
        |_| false,
    )
    .await?;
    backend.inner.windows.release_document(window, document);
    Ok(write)
}

#[tauri::command]
pub fn release_query_document(
    webview: Webview,
    backend: State<'_, Backend>,
    document: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let document: DocumentId = parse("document", &document)?;
    backend.inner.windows.check_document(window, document)?;
    backend.release_query_document(document);
    backend.inner.windows.release_document(window, document);
    Ok(())
}

#[tauri::command]
pub async fn delete_query_document(
    webview: Webview,
    backend: State<'_, Backend>,
    document: String,
    revision: u64,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let document: DocumentId = parse("document", &document)?;
    backend.inner.windows.check_document(window, document)?;
    backend.delete_query_document(document, revision).await
}

#[tauri::command]
pub async fn open_query_document(
    backend: State<'_, Backend>,
    document: String,
) -> Result<DocumentView, IpcError> {
    backend
        .open_query_document(parse("document", &document)?)
        .await
}

#[tauri::command]
pub async fn list_query_documents(
    backend: State<'_, Backend>,
    command_id: String,
    query: DocumentQuery,
) -> Result<DocumentList, IpcError> {
    backend
        .list_query_documents(parse("command id", &command_id)?, query)
        .await
}

#[tauri::command]
pub async fn read_history(
    backend: State<'_, Backend>,
    command_id: String,
    query: HistoryQuery,
) -> Result<HistoryList, IpcError> {
    let connection = query
        .connection
        .as_deref()
        .map(|connection| parse("connection", connection))
        .transpose()?;
    backend
        .read_history(parse("command id", &command_id)?, connection, query)
        .await
}

#[tauri::command]
pub async fn read_history_entry(
    backend: State<'_, Backend>,
    entry: i64,
) -> Result<HistoryDetail, IpcError> {
    backend.read_history_entry(entry).await
}

#[tauri::command]
pub async fn reconcile_history_entry(
    backend: State<'_, Backend>,
    entry: i64,
) -> Result<(), IpcError> {
    backend.reconcile_history_entry(entry).await
}

#[tauri::command]
pub async fn list_history_connections(
    backend: State<'_, Backend>,
    before: Option<i64>,
) -> Result<HistoryConnectionList, IpcError> {
    backend.list_history_connections(before).await
}

/// A retained result is the workspace's (ADR-0017): any window may open it,
/// and then reads it as one more view of its own.
#[tauri::command]
pub async fn open_retained_result(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    result: String,
) -> Result<RetainedResult, IpcError> {
    let window = caller(&backend, &webview)?;
    let result = parse("result", &result)?;
    let opened = backend
        .open_retained_result(parse("connection", &connection)?, result)
        .await?;
    if matches!(opened, RetainedResult::Open { .. }) {
        backend.inner.windows.claim_result(window, result, true);
    }
    Ok(opened)
}
