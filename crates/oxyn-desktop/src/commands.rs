//! The IPC surface: every function the webview may call.
//!
//! Each command parses what JavaScript sent — nothing from the webview is
//! trusted to be well-formed — and hands it to [`Backend`]. None of them
//! reaches a driver, the store or the keyring on its own
//! ([I-01](../../../CLAUDE.md#i-01)). Adding a command here widens what a
//! script running in the webview can do: it is reviewed as a security change
//! ([ADR-0029](../../../docs/adr/0029-interface-tauri-shadcn.md)).

use std::str::FromStr;

use oxyn_core::{CommandId, ConnectionId, SessionId};
use tauri::ipc::Channel;
use tauri::{State, Webview};

use self::subscriptions::Superseded;
use self::windows::{adopt_open, adopt_outcome, caller, outcome_waits, run_owned};
use crate::backend::Backend;
use crate::backend::windows::Stream;
use crate::ipc::{
    CommandOutcome, ConnectResponse, ConnectionDraft, ConnectionTest, DriverChoice, ExecutionEvent,
    ExecutionEventKind, IpcError, OpenConnection,
};

fn parse<T: FromStr>(what: &str, value: &str) -> Result<T, IpcError>
where
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| IpcError::invalid(format!("invalid {what}: {error}")))
}

#[tauri::command]
pub fn list_drivers(backend: State<'_, Backend>) -> Vec<DriverChoice> {
    backend.driver_choices()
}

/// Async and on the blocking pool: it reads the store, and a synchronous
/// command would read it on the main thread (I-05).
#[tauri::command]
pub async fn list_connections(
    backend: State<'_, Backend>,
) -> Result<Vec<crate::ipc::settings::ConnectionSummary>, IpcError> {
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn_blocking(move || backend.saved_connections())
        .await
        .map_err(|_| IpcError::invalid("The connection list worker stopped"))?
}

/// The connection's catalog session and first console belong to the window
/// that opened it (ADR-0043); an approval asked for it too.
#[tauri::command]
pub async fn connect(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    draft: ConnectionDraft,
) -> Result<ConnectResponse, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let response = run_owned(
        &backend,
        window,
        id,
        backend.connect(id, draft),
        |response| matches!(response, ConnectResponse::Approval { .. }),
    )
    .await?;
    if let ConnectResponse::Open(open) = &response {
        adopt_open(&backend, window, open);
    }
    Ok(response)
}

#[tauri::command]
pub async fn test_connection(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    draft: ConnectionDraft,
) -> Result<ConnectionTest, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    run_owned(
        &backend,
        window,
        id,
        backend.test_connection(id, draft),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn decide_connection(
    webview: Webview,
    backend: State<'_, Backend>,
    command: String,
    approved: bool,
) -> Result<Option<ConnectResponse>, IpcError> {
    let window = caller(&backend, &webview)?;
    let command = parse("command id", &command)?;
    let response = run_owned(
        &backend,
        window,
        command,
        backend.decide_connection(command, approved),
        |_| false,
    )
    .await?;
    if let Some(ConnectResponse::Open(open)) = &response {
        adopt_open(&backend, window, open);
    }
    Ok(response)
}

#[tauri::command]
pub async fn reconnect(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<OpenConnection, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    let open = run_owned(
        &backend,
        window,
        id,
        backend.reconnect(id, connection),
        |_| false,
    )
    .await?;
    adopt_open(&backend, window, &open);
    Ok(open)
}

/// Closes **this window's** workspace on the connection: its sessions and
/// its assistant there. The connection itself is disconnected only when no
/// other window holds it — a window never closes what another shows
/// (ADR-0043).
#[tauri::command]
pub async fn disconnect(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    backend
        .release_connection(window, parse("connection", &connection)?)
        .await
}

#[tauri::command]
pub async fn execute(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    sql: String,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id: CommandId = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    let outcome = run_owned(
        &backend,
        window,
        id,
        backend.execute(id, connection, session, sql),
        outcome_waits,
    )
    .await?;
    adopt_outcome(&backend, window, &outcome);
    Ok(outcome)
}

/// A decision on a command this window sent — or on an agent's, which the
/// first window to decide takes.
#[tauri::command]
pub async fn decide(
    webview: Webview,
    backend: State<'_, Backend>,
    command: String,
    approved: bool,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let command = parse("command id", &command)?;
    backend.inner.windows.check_command(window, command)?;
    let outcome = run_owned(
        &backend,
        window,
        command,
        backend.decide(command, approved),
        outcome_waits,
    )
    .await?;
    adopt_outcome(&backend, window, &outcome);
    Ok(outcome)
}

/// A command of another window is refused; one not dispatched yet is
/// claimed, so that its early cancellation still reaches it.
#[tauri::command]
pub fn cancel(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
) -> Result<bool, IpcError> {
    let window = caller(&backend, &webview)?;
    let command = parse("command id", &command_id)?;
    backend.inner.windows.check_command(window, command)?;
    Ok(backend.cancel(command))
}

/// Streams this window's execution events until the channel closes.
///
/// One subscription per window load: a reload subscribes again, which ends
/// the previous task of this window at once (see [`subscriptions`]). The
/// events of other windows' commands never reach it: the filter is here, in
/// Rust, not in the webview (ADR-0043).
#[tauri::command]
pub fn subscribe_events(
    webview: Webview,
    backend: State<'_, Backend>,
    channel: Channel<ExecutionEvent>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let mut superseded = Superseded::start(&backend, window, Stream::Events)?;
    let mut events = backend.subscribe();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let received = tokio::select! {
                () = superseded.wait() => break,
                received = events.recv() => received,
            };
            match received {
                Ok(event) => {
                    if !backend.inner.windows.route(window, &event) {
                        continue;
                    }
                    let Some(kind) = ExecutionEventKind::of(&event.event) else {
                        continue;
                    };
                    let message = ExecutionEvent {
                        command: event.command.to_string(),
                        kind,
                    };
                    if channel.send(message).is_err() {
                        break;
                    }
                }
                // A slow front missed events. The outcome each command awaits
                // still arrives, so progress resumes; say it rather than hide it.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "execution events dropped for a slow webview");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Ok(())
}

pub mod ai;
pub mod consoles;
pub mod file_drops;
pub mod library;
pub mod location;
pub mod menu;
pub mod metadata;
pub mod object_operations;
pub mod recovery;
pub mod results;
pub mod settings;
mod subscriptions;
pub mod windows;
