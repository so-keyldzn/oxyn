//! IPC commands of `Drop…`, `Truncate…` and `Rename…` from the catalog
//! ([ADR-0042](../../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
//!
//! A script in the webview can call both. The review composes and runs
//! nothing; the run submits one statement of the announced kind through the
//! command bus, where the gate decides as for any SQL, and on production only
//! the host's dialog approves it ([I-01](../../../../CLAUDE.md#i-01),
//! [I-02](../../../../CLAUDE.md#i-02)).

use tauri::State;

use super::parse;
use crate::backend::Backend;
use crate::ipc::object_operations::{
    ObjectOperation, ObjectOperationOutcome, ObjectOperationReview, OperationKind,
};
use crate::ipc::{CatalogAddress, IpcError};

/// Composes the statement and reads what the box shows around it. On the
/// blocking pool: it reads the store and waits on the catalog cache's lock.
#[tauri::command]
pub async fn review_object_operation(
    backend: State<'_, Backend>,
    connection: String,
    session: String,
    address: CatalogAddress,
    operation: ObjectOperation,
) -> Result<ObjectOperationReview, IpcError> {
    let connection = parse("connection", &connection)?;
    let session = parse("session", &session)?;
    backend
        .on_blocking_pool(move |backend| {
            backend.review_object_operation(connection, session, &address, &operation)
        })
        .await
}

/// Submits a reviewed statement on a session opened for it; `cancel` with the
/// same id reaches the opening and the statement.
#[tauri::command]
pub async fn run_object_operation(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    operation: OperationKind,
    sql: String,
) -> Result<ObjectOperationOutcome, IpcError> {
    backend
        .run_object_operation(
            parse("command id", &command_id)?,
            parse("connection", &connection)?,
            operation,
            sql,
        )
        .await
}
