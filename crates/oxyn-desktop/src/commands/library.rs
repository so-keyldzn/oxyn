//! Query documents and local history commands. None reaches a database.

use tauri::State;

use super::parse;
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

#[tauri::command]
pub async fn save_query_document(
    backend: State<'_, Backend>,
    command_id: String,
    change: DocumentChange,
) -> Result<DocumentWrite, IpcError> {
    let id = parse("command id", &command_id)?;
    let document = parse("document", &change.document)?;
    let connection = change
        .connection
        .as_deref()
        .map(|connection| parse("connection", connection))
        .transpose()?;
    backend
        .save_query_document(id, document, connection, change)
        .await
}

#[tauri::command]
pub async fn close_query_document(
    backend: State<'_, Backend>,
    command_id: String,
    document: String,
    revision: u64,
    discard: bool,
) -> Result<DocumentWrite, IpcError> {
    backend
        .close_query_document(
            parse("command id", &command_id)?,
            parse("document", &document)?,
            revision,
            discard,
        )
        .await
}

#[tauri::command]
pub fn release_query_document(
    backend: State<'_, Backend>,
    document: String,
) -> Result<(), IpcError> {
    backend.release_query_document(parse("document", &document)?);
    Ok(())
}

#[tauri::command]
pub async fn delete_query_document(
    backend: State<'_, Backend>,
    document: String,
    revision: u64,
) -> Result<(), IpcError> {
    backend
        .delete_query_document(parse("document", &document)?, revision)
        .await
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

#[tauri::command]
pub async fn open_retained_result(
    backend: State<'_, Backend>,
    connection: String,
    result: String,
) -> Result<RetainedResult, IpcError> {
    backend
        .open_retained_result(parse("connection", &connection)?, parse("result", &result)?)
        .await
}
