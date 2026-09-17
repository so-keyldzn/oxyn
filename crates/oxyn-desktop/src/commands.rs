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
use tauri::State;
use tauri::ipc::Channel;

use crate::backend::Backend;
use crate::ipc::{
    CommandOutcome, ConnectResponse, ConnectionDraft, DriverChoice, ExecutionEvent,
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

#[tauri::command]
pub async fn connect(
    backend: State<'_, Backend>,
    command_id: String,
    draft: ConnectionDraft,
) -> Result<ConnectResponse, IpcError> {
    let id = parse("command id", &command_id)?;
    backend.connect(id, draft).await
}

#[tauri::command]
pub async fn decide_connection(
    backend: State<'_, Backend>,
    command: String,
    approved: bool,
) -> Result<Option<ConnectResponse>, IpcError> {
    let command = parse("command id", &command)?;
    backend.decide_connection(command, approved).await
}

#[tauri::command]
pub async fn reconnect(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
) -> Result<OpenConnection, IpcError> {
    let id = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    backend.reconnect(id, connection).await
}

#[tauri::command]
pub async fn disconnect(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<CommandOutcome, IpcError> {
    backend.disconnect(parse("connection", &connection)?).await
}

#[tauri::command]
pub async fn execute(
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    sql: String,
) -> Result<CommandOutcome, IpcError> {
    let id: CommandId = parse("command id", &command_id)?;
    let connection: ConnectionId = parse("connection", &connection)?;
    let session: SessionId = parse("session", &session)?;
    backend.execute(id, connection, session, sql).await
}

#[tauri::command]
pub async fn decide(
    backend: State<'_, Backend>,
    command: String,
    approved: bool,
) -> Result<CommandOutcome, IpcError> {
    backend
        .decide(parse("command id", &command)?, approved)
        .await
}

#[tauri::command]
pub fn cancel(backend: State<'_, Backend>, command_id: String) -> Result<bool, IpcError> {
    Ok(backend.cancel(parse("command id", &command_id)?))
}

/// Streams execution events to the front until the channel closes.
///
/// One subscription per window load: a reload subscribes again, which ends
/// the previous task at once (see [`subscriptions`]).
#[tauri::command]
pub fn subscribe_events(backend: State<'_, Backend>, channel: Channel<ExecutionEvent>) {
    static EVENTS: subscriptions::Stream = std::sync::OnceLock::new();
    let mut superseded = subscriptions::supersede(&EVENTS);
    let mut events = backend.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let received = tokio::select! {
                () = superseded.wait() => break,
                received = events.recv() => received,
            };
            match received {
                Ok(event) => {
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
}

pub mod ai;
pub mod consoles;
pub mod library;
pub mod metadata;
pub mod recovery;
pub mod results;
pub mod settings;
mod subscriptions;
