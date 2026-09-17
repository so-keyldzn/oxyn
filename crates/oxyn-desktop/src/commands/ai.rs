//! The AI workspace's IPC surface.
//!
//! Each command parses what the webview sent and hands it to [`Backend`]. None
//! of them runs a model output: a tool call becomes a `Command` carrying
//! `Actor::Agent` inside the backend, and its approval goes through the
//! existing `decide` ([I-07](../../../../CLAUDE.md#i-07)).
//!
//! # What a script in the webview could do with these, and what stops it
//!
//! * read a key — impossible: no command returns one, and `DeclaredProvider`
//!   carries « configured », never a reference ([I-03](../../../../CLAUDE.md#i-03));
//! * declare a provider pointing to its own server — possible, as through the
//!   screen; the connection's tier still applies, and nothing is sent without a
//!   question typed in the panel;
//! * declare an **external agent**, which is a program Oxyn will run — held
//!   behind a native confirmation that names the exact command, drawn by the
//!   host and not by the webview ([`ai_save_external_agent`]).

use oxyn_core::ConnectionId;
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::backend::Backend;
use crate::ipc::ai::{
    AgentDraft, AgentPresetDraft, AgentSettingAnswer, AgentSettingChange, AiUpdate, AskRequest,
    AskStarted, DeclaredProvider, DestinationChoice, ExternalAgent, ModelChoice, ProposalTarget,
    ProviderDraft, SampleRequest, SchemaProposal, ThreadSummary, ThreadView,
};
use crate::ipc::{CatalogAddress, IpcError};

fn connection(value: &str) -> Result<ConnectionId, IpcError> {
    value
        .parse()
        .map_err(|error| IpcError::invalid(format!("invalid connection: {error}")))
}

#[tauri::command]
pub async fn ai_providers(backend: State<'_, Backend>) -> Result<Vec<DeclaredProvider>, IpcError> {
    backend.ai_providers().await
}

#[tauri::command]
pub async fn ai_external_agents(
    backend: State<'_, Backend>,
) -> Result<Vec<ExternalAgent>, IpcError> {
    backend.external_agents().await
}

#[tauri::command]
pub async fn ai_save_provider(
    backend: State<'_, Backend>,
    draft: ProviderDraft,
) -> Result<DeclaredProvider, IpcError> {
    backend.save_ai_provider(draft).await
}

#[tauri::command]
pub async fn ai_remove_provider(backend: State<'_, Backend>, id: String) -> Result<(), IpcError> {
    backend.remove_ai_provider(&id).await
}

#[tauri::command]
pub async fn ai_provider_models(
    backend: State<'_, Backend>,
    id: String,
) -> Result<Vec<ModelChoice>, IpcError> {
    backend.provider_models(&id).await
}

/// Declares an external agent, once the user confirmed the exact command.
///
/// The confirmation is a **native** dialog: a script injected in the webview
/// can call this command, it cannot click a window it does not draw. Without
/// it, declaring an agent then asking a question would run any program on the
/// machine ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
/// Arguments are shown in full here and nowhere else: this is the moment they
/// must be read.
#[tauri::command]
pub async fn ai_save_external_agent<R: Runtime>(
    app: AppHandle<R>,
    backend: State<'_, Backend>,
    draft: AgentDraft,
) -> Result<Option<ExternalAgent>, IpcError> {
    let mut command_line = String::new();
    for var in &draft.env {
        command_line.push_str(&format!("{}={} ", var.name.trim(), var.value));
    }
    command_line.push_str(draft.command.trim());
    for arg in &draft.args {
        command_line.push(' ');
        command_line.push_str(arg.trim());
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(format!(
            "“{}” will run this program on your machine each time you ask it a question:\n\n{}\n\n\
             Oxyn cannot see where it sends your prompts, so it never serves a local-only \
             connection.",
            draft.label.trim(),
            command_line
        ))
        .title("Declare an external agent?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Declare agent".to_owned(),
            "Cancel".to_owned(),
        ))
        .show(move |confirmed| {
            let _ = sender.send(confirmed);
        });
    if !receiver.await.unwrap_or(false) {
        return Ok(None);
    }
    backend.save_external_agent(draft).await.map(Some)
}

#[tauri::command]
pub async fn ai_remove_external_agent(
    backend: State<'_, Backend>,
    id: String,
) -> Result<(), IpcError> {
    backend.remove_external_agent(&id).await
}

/// The ready-made declarations for agents Oxyn knows, before detection.
#[tauri::command]
pub fn ai_agent_presets(backend: State<'_, Backend>) -> Vec<AgentPresetDraft> {
    backend.ai_agent_presets()
}

/// Looks for a known agent on the machine, on the user's click. Saves nothing:
/// the draft comes back to be read, and declaring it goes through
/// [`ai_save_external_agent`] and its native confirmation.
#[tauri::command]
pub async fn ai_detect_agent(
    backend: State<'_, Backend>,
    id: String,
) -> Result<AgentPresetDraft, IpcError> {
    backend.ai_detect_agent(&id).await
}

/// Asks a question; returns once it is accepted, the answer streams on `channel`.
#[tauri::command]
pub async fn ai_ask(
    backend: State<'_, Backend>,
    request: AskRequest,
    channel: Channel<AiUpdate>,
) -> Result<AskStarted, IpcError> {
    backend.ai_ask(request, channel).await
}

#[tauri::command]
pub fn ai_cancel(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
) -> Result<bool, IpcError> {
    backend.ai_cancel(self::connection(&connection)?, &thread)
}

/// In memory only: forgets a grant, reads nothing.
#[tauri::command]
pub fn ai_withdraw_sample(
    backend: State<'_, Backend>,
    connection: String,
    request: String,
) -> Result<(), IpcError> {
    backend.ai_withdraw_sample(self::connection(&connection)?, &request);
    Ok(())
}

#[tauri::command]
pub fn ai_threads(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<Vec<ThreadSummary>, IpcError> {
    Ok(backend.ai_threads(self::connection(&connection)?))
}

/// A read, not an action: the conversation so far comes back, and what follows
/// streams on `channel`. Nothing is asked again.
#[tauri::command]
pub fn ai_open_thread(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    channel: Channel<AiUpdate>,
) -> Result<ThreadView, IpcError> {
    backend.ai_open_thread(self::connection(&connection)?, &thread, channel)
}

#[tauri::command]
pub fn ai_rename_thread(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    title: String,
) -> Result<(), IpcError> {
    backend.ai_rename_thread(self::connection(&connection)?, &thread, &title)
}

#[tauri::command]
pub fn ai_delete_thread(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
) -> Result<(), IpcError> {
    backend.ai_delete_thread(self::connection(&connection)?, &thread)
}

#[tauri::command]
pub fn ai_select_version(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    node: u32,
) -> Result<(), IpcError> {
    backend.ai_select_version(self::connection(&connection)?, &thread, node)
}

/// Asks the conversation's agent to sign its user in, by a method it offered.
/// The agent does it; Oxyn neither reads nor keeps a token.
#[tauri::command]
pub async fn ai_authenticate(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    method: String,
) -> Result<(), IpcError> {
    backend
        .ai_authenticate(self::connection(&connection)?, &thread, &method)
        .await
}

/// Offers a row sample of a relation for the next question — what would be
/// read and where it would go, never a value. `Sampled` only.
#[tauri::command]
pub async fn ai_request_sample(
    backend: State<'_, Backend>,
    connection: String,
    thread: Option<String>,
    parent: Option<u32>,
    address: CatalogAddress,
    destination: DestinationChoice,
) -> Result<SampleRequest, IpcError> {
    backend
        .ai_request_sample(
            self::connection(&connection)?,
            thread,
            parent,
            address,
            destination,
        )
        .await
}

/// Asks the conversation's agent to change a mode or an option it declared.
#[tauri::command]
pub async fn ai_set_agent_setting(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    change: AgentSettingChange,
) -> Result<AgentSettingAnswer, IpcError> {
    backend
        .ai_set_agent_setting(self::connection(&connection)?, &thread, change)
        .await
}

#[tauri::command]
pub fn ai_forget(backend: State<'_, Backend>, connection: String) -> Result<(), IpcError> {
    backend.close_ai_conversation(self::connection(&connection)?);
    Ok(())
}

/// Asynchronous, and the work itself off the async workers: a proposal reads
/// the connection from the store, SQLite under the store's lock. A synchronous
/// command runs on the main thread, where a lock held by a pruning or a history
/// write freezes the window for as long ([I-05](../../../../CLAUDE.md#i-05)).
#[tauri::command]
pub async fn ai_propose_schema_change(
    backend: State<'_, Backend>,
    connection: String,
    address: CatalogAddress,
    target: ProposalTarget,
) -> Result<Option<SchemaProposal>, IpcError> {
    let connection = self::connection(&connection)?;
    let backend = Backend::clone(&backend);
    tokio::task::spawn_blocking(move || {
        backend.propose_schema_change(connection, &address, &target)
    })
    .await
    .map_err(|_| IpcError::invalid("The proposal stopped before it finished"))?
}
