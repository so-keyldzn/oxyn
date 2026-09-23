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
    AgentDraft, AgentPresetDraft, AgentSettingAnswer, AgentSettingChange, AgentStart,
    AgentStartRequest, AiUpdate, AskRequest, AskStarted, DeclaredProvider, DestinationChoice,
    ExternalAgent, ModelChoice, ProposalTarget, ProviderDraft, SampleRequest, SchemaProposal,
    ThreadSummary, ThreadView, shell_quote,
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
    // Validated before the dialog, and shown as it will be saved: the user
    // confirms the declaration itself, not a draft that may still be refused.
    let agent = backend.review_external_agent(&draft).await?;
    let (title, confirm) = if draft.id.is_some() {
        ("Replace this external agent?", "Replace agent")
    } else {
        ("Declare an external agent?", "Declare agent")
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(format!(
            "“{}” will run this program on your machine each time you ask it a question:\n\n{}\n\n\
             Oxyn cannot see where it sends your prompts, so it never serves a local-only \
             connection.",
            agent.label,
            command_line(&agent)
        ))
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            confirm.to_owned(),
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

/// The declaration as one shell line, each word quoted as a shell reads it
/// back: joined by spaces, `["-y foo"]` and `["-y", "foo"]` would read the same
/// in the dialog and run differently.
fn command_line(agent: &oxyn_core::ExternalAgentConfig) -> String {
    let env = agent
        .env
        .iter()
        .map(|(name, value)| format!("{}={}", shell_quote(name), shell_quote(value)));
    let words = std::iter::once(&agent.command)
        .chain(&agent.args)
        .map(|word| shell_quote(word));
    env.chain(words).collect::<Vec<_>>().join(" ")
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

/// Looks for a known agent on the machine, when the AI settings screen opens.
/// Runs nothing and saves nothing:
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

/// The user's answer to an agent's request for a row sample: the columns
/// ticked, or `null` to decline. The **only** way such a request is approved
/// (ADR-0034): the agent's call waits on it, and does the read itself.
///
/// In memory only, and synchronous for that: it hands the answer to the call
/// waiting on it and returns — the read runs on that call's task.
#[tauri::command]
pub fn ai_answer_sample(
    backend: State<'_, Backend>,
    connection: String,
    request: String,
    columns: Option<Vec<String>>,
) -> Result<(), IpcError> {
    backend.ai_answer_sample(self::connection(&connection)?, &request, columns.as_deref())
}

#[tauri::command]
pub async fn ai_threads(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<Vec<ThreadSummary>, IpcError> {
    Ok(backend.ai_threads(self::connection(&connection)?).await)
}

/// A read, not an action: the conversation so far comes back, and what follows
/// streams on `channel`. Nothing is asked again.
#[tauri::command]
pub async fn ai_open_thread(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    channel: Channel<AiUpdate>,
) -> Result<ThreadView, IpcError> {
    backend
        .ai_open_thread(self::connection(&connection)?, &thread, channel)
        .await
}

#[tauri::command]
pub async fn ai_rename_thread(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    title: String,
) -> Result<(), IpcError> {
    backend
        .ai_rename_thread(self::connection(&connection)?, &thread, &title)
        .await
}

#[tauri::command]
pub async fn ai_delete_thread(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
) -> Result<(), IpcError> {
    backend
        .ai_delete_thread(self::connection(&connection)?, &thread)
        .await
}

#[tauri::command]
pub async fn ai_select_version(
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    node: u32,
) -> Result<(), IpcError> {
    backend
        .ai_select_version(self::connection(&connection)?, &thread, node)
        .await
}

/// Asks the conversation's agent to sign its user in, by a method it offered.
/// The agent does it; Oxyn neither reads nor keeps a token.
///
/// `thread` is `None` before the first question: the sign-in then goes to the
/// agent the panel started for the connection.
#[tauri::command]
pub async fn ai_authenticate(
    backend: State<'_, Backend>,
    connection: String,
    thread: Option<String>,
    method: String,
) -> Result<(), IpcError> {
    backend
        .ai_authenticate(self::connection(&connection)?, thread.as_deref(), &method)
        .await
}

/// Starts the external agent the next question would use, before it is asked,
/// and answers with the models, efforts and options it declares.
///
/// What a script in the webview gains: launching an agent the user declared —
/// behind the native confirmation — on a connection it chooses, which the
/// panel already does on a question. The refusal under `Local` falls before
/// anything starts, the tools and the confinement are a question's, and no
/// prompt is sent: nothing of the database leaves.
#[tauri::command]
pub async fn ai_start_agent(
    backend: State<'_, Backend>,
    request: AgentStartRequest,
) -> Result<AgentStart, IpcError> {
    backend.ai_start_agent(request).await
}

/// Stops the agent started ahead of a question, if no question took it.
/// Asynchronous although in memory: releasing the agent withdraws its requests
/// at the executor.
#[tauri::command]
pub async fn ai_stop_agent_start(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<bool, IpcError> {
    Ok(backend.ai_stop_agent_start(self::connection(&connection)?))
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

/// Asks the conversation's agent to change a mode or an option it declared —
/// or, before the first question (`thread` `None`), the agent the panel
/// started for the connection.
#[tauri::command]
pub async fn ai_set_agent_setting(
    backend: State<'_, Backend>,
    connection: String,
    thread: Option<String>,
    change: AgentSettingChange,
) -> Result<AgentSettingAnswer, IpcError> {
    backend
        .ai_set_agent_setting(self::connection(&connection)?, thread.as_deref(), change)
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

#[cfg(test)]
mod tests {
    use oxyn_core::{ExternalAgentConfig, ProviderId};

    use super::command_line;

    #[test]
    fn the_dialog_tells_one_argument_from_two() {
        let agent = |args: &[&str]| {
            ExternalAgentConfig::new(ProviderId::for_new_agent(), "Agent", "npx")
                .with_args(args.iter().copied())
        };
        let one = command_line(&agent(&["-y foo"]));
        let two = command_line(&agent(&["-y", "foo"]));
        assert_eq!(one, "npx '-y foo'");
        assert_eq!(two, "npx -y foo");
    }

    #[test]
    fn the_environment_is_quoted_before_the_command() {
        let mut agent = ExternalAgentConfig::new(
            ProviderId::for_new_agent(),
            "Agent",
            "/Users/me/.nvm/versions/node/v22.23.2/bin/npx",
        )
        .with_args(["-y", "@agentclientprotocol/claude-agent-acp@0.78.0"]);
        agent.env = vec![("PATH".to_owned(), "/opt/my bin:/usr/bin".to_owned())];
        assert_eq!(
            command_line(&agent),
            "PATH='/opt/my bin:/usr/bin' /Users/me/.nvm/versions/node/v22.23.2/bin/npx -y \
             @agentclientprotocol/claude-agent-acp@0.78.0"
        );
    }
}
