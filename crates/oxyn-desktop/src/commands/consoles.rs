//! Console commands: sessions, runs and session context.
//!
//! Each parses what the webview sent and hands it to the backend, which
//! dispatches a `Command` ([I-01](../../../../CLAUDE.md#i-01)).

use oxyn_core::{ConnectionId, SessionId};
use tauri::{State, Webview};

use super::parse;
use super::windows::{adopt_outcome, caller, outcome_waits, run_owned};
use crate::backend::Backend;
use crate::ipc::consoles::{
    ConsoleRun, ConsoleSession, ContextOutcome, ParameterInput, ParameterRefusal, SessionPlace,
    bind,
};
use crate::ipc::{CommandOutcome, IpcError};

/// The console's session belongs to the window that opened it (ADR-0043).
#[tauri::command]
pub async fn open_console(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<ConsoleSession, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    // A console opens in a workspace of this window: a script cannot keep
    // open, hidden here, a connection the user closed in another window.
    if !backend.inner.windows.holds(window, connection) {
        return Err(IpcError::invalid(
            "This connection is not open in this window",
        ));
    }
    let console = run_owned(
        &backend,
        window,
        id,
        backend.open_console(id, connection),
        |_| false,
    )
    .await?;
    if let Ok(session) = console.session.parse::<SessionId>() {
        backend
            .inner
            .windows
            .claim_session(window, connection, session);
    }
    Ok(console)
}

#[tauri::command]
pub async fn close_console(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    session: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    backend
        .close_console(parse("connection", &connection)?, session)
        .await?;
    backend.inner.windows.release_session(session);
    Ok(())
}

#[tauri::command]
pub async fn run_console(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    run: ConsoleRun,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    let outcome = run_owned(
        &backend,
        window,
        id,
        backend.run_console(id, parse("connection", &connection)?, session, run),
        outcome_waits,
    )
    .await?;
    adopt_outcome(&backend, window, &outcome);
    Ok(outcome)
}

#[tauri::command]
pub async fn set_session_context(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    place: SessionPlace,
) -> Result<ContextOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.set_session_context(id, parse("connection", &connection)?, session, place),
        |_| false,
    )
    .await
}

#[tauri::command]
pub fn session_context_choices(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<Vec<SessionPlace>, IpcError> {
    backend.session_context_choices(parse("connection", &connection)?)
}

/// Whether every value converts, before a run is submitted: the console opens
/// its parameters on the refused one. The refusal names a position and a
/// type, never a value (I-03); no row is named when it is the count of
/// parameters that is refused. Pure computation on what was sent, no I/O.
#[tauri::command]
pub fn validate_parameters(parameters: Vec<ParameterInput>) -> Option<ParameterRefusal> {
    bind(&parameters).err().map(ParameterRefusal::from)
}
