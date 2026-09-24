//! Console commands: sessions, runs and session context.
//!
//! Each parses what the webview sent and hands it to the backend, which
//! dispatches a `Command` ([I-01](../../../../CLAUDE.md#i-01)).

use tauri::State;

use super::parse;
use crate::backend::Backend;
use crate::ipc::consoles::{
    ConsoleRun, ConsoleSession, ContextOutcome, ParameterInput, ParameterRefusal, SessionPlace,
    bind,
};
use crate::ipc::{CommandOutcome, IpcError};

#[tauri::command]
pub async fn open_console(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<ConsoleSession, IpcError> {
    backend
        .open_console(
            parse("command id", &command_id)?,
            parse("connection", &connection)?,
        )
        .await
}

#[tauri::command]
pub async fn close_console(
    backend: State<'_, Backend>,
    connection: String,
    session: String,
) -> Result<(), IpcError> {
    backend
        .close_console(
            parse("connection", &connection)?,
            parse("session", &session)?,
        )
        .await
}

#[tauri::command]
pub async fn run_console(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    run: ConsoleRun,
) -> Result<CommandOutcome, IpcError> {
    backend
        .run_console(
            parse("command id", &command_id)?,
            parse("connection", &connection)?,
            parse("session", &session)?,
            run,
        )
        .await
}

#[tauri::command]
pub async fn set_session_context(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    place: SessionPlace,
) -> Result<ContextOutcome, IpcError> {
    backend
        .set_session_context(
            parse("command id", &command_id)?,
            parse("connection", &connection)?,
            parse("session", &session)?,
            place,
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
