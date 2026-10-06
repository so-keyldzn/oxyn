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
//!   screen; the connection's tier still applies;
//! * point an **existing** declaration to a different endpoint (scheme, host,
//!   port, path, or query) or a different protocol — clears its key reference
//!   and forgets the key, unless a key is typed in the same request. A key only
//!   ever leaves toward the endpoint it was typed for
//!   ([I-03](../../../../CLAUDE.md#i-03));
//! * call [`ai_provider_models`] — sends no base content of its own, and a
//!   conversation's context only leaves on a question typed in the panel;
//! * call [`ai_list_draft_models`] with an address and a key of its choosing —
//!   the key goes to that address and nowhere else, which is what typing it in
//!   the form already allows; a stored key only follows its own endpoint;
//! * declare an **external agent**, which is a program Oxyn will run — held
//!   behind a native confirmation that names the exact command, drawn by the
//!   host and not by the webview ([`ai_save_external_agent`]).

use oxyn_core::ConnectionId;
use tauri::ipc::Channel;
use tauri::{Manager as _, State, Webview};

use crate::backend::Backend;
use crate::commands::windows::{bring_to_front, caller};
use crate::ipc::ai::{
    AgentDraft, AgentPresetDraft, AgentRoleOption, AgentSettingAnswer, AgentSettingChange,
    AgentStart, AgentStartRequest, AiUpdate, AskRequest, AskStarted, DeclaredProvider,
    DestinationChoice, ExternalAgent, ModelChoice, ModelListing, ModelProbe, OrphanThreadSummary,
    ProposalTarget, ProviderDraft, PrunedHistory, SampleRequest, SchemaProposal, ThreadSummary,
    ThreadView,
};
use crate::ipc::{CatalogAddress, IpcError};

fn connection(value: &str) -> Result<ConnectionId, IpcError> {
    value
        .parse()
        .map_err(|error| IpcError::invalid(format!("invalid connection: {error}")))
}

/// The connection named, once the calling window holds its assistant.
///
/// The assistant's state is kept by connection: its conversations, the
/// agent that serves them and the samples it asked for belong to one window
/// at a time (ADR-0043). Another window asking for them does not take them:
/// the window that has them comes to the front.
///
/// Only on a connection this window holds a workspace on: a script cannot
/// take the assistant of a connection another window shows.
fn assistant(backend: &Backend, webview: &Webview, value: &str) -> Result<ConnectionId, IpcError> {
    claim(backend, webview, value).map_err(|(error, _)| error)
}

/// [`assistant`], with the window that has it when it is another's.
fn claim(
    backend: &Backend,
    webview: &Webview,
    value: &str,
) -> Result<ConnectionId, (IpcError, Option<crate::backend::WindowKey>)> {
    let window = caller(backend, webview).map_err(|error| (error, None))?;
    let connection = connection(value).map_err(|error| (error, None))?;
    let windows = &backend.inner.windows;
    if !windows.holds(window, connection) {
        return Err((
            IpcError::invalid("This connection is not open in this window"),
            None,
        ));
    }
    windows
        .claim_assistant(window, connection)
        .map_err(|owner| {
            (
                IpcError::invalid("The assistant of this connection is open in another window"),
                Some(owner),
            )
        })?;
    Ok(connection)
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

/// Lists the models of the endpoint a provider form describes, before it is
/// saved.
///
/// Sends the typed key — or, on an unchanged endpoint, the stored one — to
/// that endpoint only; persists nothing, writes nothing to the keychain. A
/// provider's refusal comes back as a `failed` listing; an `Err` means the
/// request itself was malformed.
#[tauri::command]
pub async fn ai_list_draft_models(
    backend: State<'_, Backend>,
    probe: ModelProbe,
    refresh: bool,
) -> Result<ModelListing, IpcError> {
    backend.list_draft_models(probe, refresh).await
}

/// Declares an external agent, once the user confirmed the exact command.
///
/// The confirmation is a **native** dialog: a script injected in the webview
/// can call this command, it cannot click a window it does not draw. Without
/// it, declaring an agent then asking a question would run any program on the
/// machine ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
/// The backend composes the dialog and applies the shared timing safeguards.
#[tauri::command]
pub async fn ai_save_external_agent(
    backend: State<'_, Backend>,
    draft: AgentDraft,
) -> Result<Option<ExternalAgent>, IpcError> {
    backend.save_external_agent(draft).await
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
    webview: Webview,
    backend: State<'_, Backend>,
    request: AskRequest,
    channel: Channel<AiUpdate>,
) -> Result<AskStarted, IpcError> {
    assistant(&backend, &webview, &request.connection)?;
    // The session the agent works in is this window's: a console moved to
    // another window keeps its connection, not its assistant (ADR-0043).
    let session = crate::commands::parse("session", &request.session)?;
    backend
        .inner
        .windows
        .check_session(caller(&backend, &webview)?, session)?;
    backend.ai_ask(request, channel).await
}

#[tauri::command]
pub fn ai_cancel(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
) -> Result<bool, IpcError> {
    backend.ai_cancel(assistant(&backend, &webview, &connection)?, &thread)
}

/// In memory only: forgets a grant, reads nothing.
#[tauri::command]
pub fn ai_withdraw_sample(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    request: String,
) -> Result<(), IpcError> {
    backend.ai_withdraw_sample(assistant(&backend, &webview, &connection)?, &request);
    Ok(())
}

/// The user's answer to an agent's request for a row sample: the columns
/// ticked, or `null` to decline. The **only** way such a request is approved
/// (ADR-0034): the agent's call waits on it, and does the read itself.
///
/// The native confirmation is awaited off the UI thread before the answer
/// reaches the waiting call, which then performs the read through the bus.
#[tauri::command]
pub async fn ai_answer_sample(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    request: String,
    columns: Option<Vec<String>>,
) -> Result<(), IpcError> {
    backend
        .ai_answer_sample(
            assistant(&backend, &webview, &connection)?,
            &request,
            columns.as_deref(),
        )
        .await
}

#[tauri::command]
pub async fn ai_threads(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<Vec<ThreadSummary>, IpcError> {
    Ok(backend.ai_threads(self::connection(&connection)?).await)
}

/// The agents the picker offers on a connection; with `destination`, for that
/// destination only. Async: it reads the connection's file and the declared
/// destinations.
#[tauri::command]
pub async fn ai_list_agents(
    backend: State<'_, Backend>,
    connection_id: String,
    destination: Option<DestinationChoice>,
) -> Result<Vec<AgentRoleOption>, IpcError> {
    backend
        .ai_list_agents(self::connection(&connection_id)?, destination.as_ref())
        .await
}

/// Reads the user's agents directory again, then answers as
/// [`ai_list_agents`] does without a destination. The webview names a
/// connection, never a path: the directory is the backend's. Async: the read
/// runs on the blocking pool.
#[tauri::command]
pub async fn ai_reload_agents(
    backend: State<'_, Backend>,
    connection_id: String,
) -> Result<Vec<AgentRoleOption>, IpcError> {
    backend
        .ai_reload_agents(self::connection(&connection_id)?)
        .await
}

/// In memory only, and synchronous for that: what the launch's prune removed.
#[tauri::command]
pub fn ai_pruned_history(backend: State<'_, Backend>) -> Option<PrunedHistory> {
    backend.ai_pruned_history()
}

/// A read, and the only thing a webview can do with these: a thread whose
/// connection was deleted has no command that opens, renames or deletes it.
#[tauri::command]
pub async fn ai_orphan_threads(
    backend: State<'_, Backend>,
) -> Result<Vec<OrphanThreadSummary>, IpcError> {
    Ok(backend.ai_orphan_threads().await)
}

/// A read, not an action: the conversation so far comes back, and what follows
/// streams on `channel`. Nothing is asked again.
#[tauri::command]
pub async fn ai_open_thread(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    channel: Channel<AiUpdate>,
) -> Result<ThreadView, IpcError> {
    // Opening a conversation is the gesture that brings the window holding
    // it to the front; no other refusal moves the focus, or a script looping
    // on it would steal the keyboard of another window.
    let connection = claim(&backend, &webview, &connection).map_err(|(error, owner)| {
        if let Some(owner) = owner {
            bring_to_front(webview.app_handle(), owner);
        }
        error
    })?;
    backend.ai_open_thread(connection, &thread, channel).await
}

#[tauri::command]
pub async fn ai_rename_thread(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    title: String,
) -> Result<(), IpcError> {
    backend
        .ai_rename_thread(assistant(&backend, &webview, &connection)?, &thread, &title)
        .await
}

#[tauri::command]
pub async fn ai_delete_thread(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
) -> Result<(), IpcError> {
    backend
        .ai_delete_thread(assistant(&backend, &webview, &connection)?, &thread)
        .await
}

#[tauri::command]
pub async fn ai_select_version(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: String,
    node: u32,
) -> Result<(), IpcError> {
    backend
        .ai_select_version(assistant(&backend, &webview, &connection)?, &thread, node)
        .await
}

/// Asks the conversation's agent to sign its user in, by a method it offered.
/// The agent does it; Oxyn neither reads nor keeps a token.
///
/// `thread` is `None` before the first question: the sign-in then goes to the
/// agent the panel started for the connection.
#[tauri::command]
pub async fn ai_authenticate(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: Option<String>,
    method: String,
) -> Result<(), IpcError> {
    backend
        .ai_authenticate(
            assistant(&backend, &webview, &connection)?,
            thread.as_deref(),
            &method,
        )
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
    webview: Webview,
    backend: State<'_, Backend>,
    request: AgentStartRequest,
) -> Result<AgentStart, IpcError> {
    assistant(&backend, &webview, &request.connection)?;
    backend.ai_start_agent(request).await
}

/// Stops the agent started ahead of a question, if no question took it.
/// Asynchronous although in memory: releasing the agent withdraws its requests
/// at the executor.
#[tauri::command]
pub async fn ai_stop_agent_start(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
) -> Result<bool, IpcError> {
    Ok(backend.ai_stop_agent_start(assistant(&backend, &webview, &connection)?))
}

/// Offers a row sample of a relation for the next question — what would be
/// read and where it would go, never a value. `Sampled` only.
#[tauri::command]
pub async fn ai_request_sample(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: Option<String>,
    parent: Option<u32>,
    address: CatalogAddress,
    destination: DestinationChoice,
) -> Result<SampleRequest, IpcError> {
    backend
        .ai_request_sample(
            assistant(&backend, &webview, &connection)?,
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
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
    thread: Option<String>,
    change: AgentSettingChange,
) -> Result<AgentSettingAnswer, IpcError> {
    backend
        .ai_set_agent_setting(
            assistant(&backend, &webview, &connection)?,
            thread.as_deref(),
            change,
        )
        .await
}

/// Reads from the server the names `@` offers and the cache lacks — never a
/// row, never a column. Resolves once they are in the cache.
#[tauri::command]
pub async fn ai_list_mentionable(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<(), IpcError> {
    backend
        .list_mentionable(self::connection(&connection)?)
        .await
}

/// Only the window that holds the assistant of the connection forgets its
/// conversation; from another, nothing happens.
#[tauri::command]
pub fn ai_forget(
    webview: Webview,
    backend: State<'_, Backend>,
    connection: String,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let connection = self::connection(&connection)?;
    let windows = &backend.inner.windows;
    if windows.has_assistant(window, connection) {
        backend.close_ai_conversation(connection);
        windows.release_assistant(window, connection);
    }
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
