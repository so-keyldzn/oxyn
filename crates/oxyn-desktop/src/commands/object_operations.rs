//! IPC commands of `Drop…`, `Truncate…` and `Rename…` from the catalog
//! ([ADR-0042](../../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md)).
//!
//! A script in the webview can call both. The review composes and runs
//! nothing; the run submits one statement of the announced kind through the
//! command bus, where the gate decides as for any SQL, and on production only
//! the host's dialog approves it ([I-01](../../../../CLAUDE.md#i-01),
//! [I-02](../../../../CLAUDE.md#i-02)).

use oxyn_core::{ConnectionId, SessionId};
use tauri::{State, Webview};

use super::parse;
use super::windows::{caller, run_owned};
use crate::backend::Backend;
use crate::ipc::object_operations::{
    ObjectOperation, ObjectOperationOutcome, ObjectOperationReview, OperationKind,
};
use crate::ipc::{CatalogAddress, IpcError};

/// Composes the statement and reads what the box shows around it. On the
/// blocking pool: it reads the store and waits on the catalog cache's lock.
#[tauri::command]
pub async fn review_object_operation(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    session: String,
    address: CatalogAddress,
    operation: ObjectOperation,
) -> Result<ObjectOperationReview, IpcError> {
    let window = caller(&backend, &webview)?;
    let connection = parse("connection", &connection)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    backend
        .on_blocking_pool(move |backend| {
            backend.review_object_operation(connection, session, &address, &operation)
        })
        .await
}

/// Submits a reviewed statement on a session opened for it; `cancel` with the
/// same id reaches the opening and the statement.
///
/// The review in progress belongs to this window: its pending approval is
/// rejected if the window closes (ADR-0042, ADR-0043).
#[tauri::command]
pub async fn run_object_operation(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    operation: OperationKind,
    sql: String,
) -> Result<ObjectOperationOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    if !backend.inner.windows.holds(window, connection) {
        return Err(IpcError::invalid(
            "This connection is not open in this window",
        ));
    }
    run_owned(
        &backend,
        window,
        id,
        backend.run_object_operation(id, connection, operation, sql),
        |outcome| matches!(outcome, ObjectOperationOutcome::NeedsApproval { .. }),
    )
    .await
}
