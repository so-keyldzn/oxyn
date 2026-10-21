//! Local semantic ranking's commands
//! ([ADR-0056](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md)).
//!
//! Plumbing, like the updater's: the switch is a preference written through
//! the bus as the human, the rest moves public model files. What these widen
//! for a script in the webview: turning the option on — a 220 MB download of
//! pinned files from fixed hosts, accepted on their checksum alone — or off,
//! which deletes them; and loading the model ahead of a question. None takes
//! a URL, a path or a file name.

use tauri::ipc::Channel;
use tauri::{State, Webview};

use super::subscriptions::Superseded;
use super::windows::caller;
use crate::backend::Backend;
use crate::backend::windows::Stream;
use crate::ipc::IpcError;
use crate::ipc::semantic::SemanticSnapshot;

/// The option and its model. Async: it reads the preferences and stats the
/// model's files (I-05).
#[tauri::command]
pub async fn semantic_ranking_state(
    backend: State<'_, Backend>,
) -> Result<SemanticSnapshot, IpcError> {
    backend.semantic_state().await
}

/// Every change of the option and its model — download progress included —
/// until this window subscribes again or closes. The current state first,
/// read from the disk.
#[tauri::command]
pub fn subscribe_semantic_ranking(
    webview: Webview,
    backend: State<'_, Backend>,
    channel: Channel<SemanticSnapshot>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    // A reload subscribes again and ends this task (see `subscriptions`).
    let mut superseded = Superseded::start(&backend, window, Stream::SemanticRanking)?;
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn(async move {
        // The disk is read once, so the first message is not a stale state.
        let _ = backend.semantic_state().await;
        let mut snapshots = backend.inner.semantic.subscribe();
        loop {
            let snapshot = snapshots.borrow_and_update().clone();
            if channel.send(snapshot).is_err() {
                break;
            }
            tokio::select! {
                () = superseded.wait() => break,
                changed = snapshots.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(())
}

/// Turns the option on and starts the download; the progress arrives on the
/// subscription. Also « Download again » for a failed or damaged model.
#[tauri::command]
pub async fn enable_semantic_ranking(
    backend: State<'_, Backend>,
) -> Result<SemanticSnapshot, IpcError> {
    backend.enable_semantic_ranking().await
}

/// Turns the option off: stops a download — « Cancel » is this command —,
/// drops the model and deletes its files.
#[tauri::command]
pub async fn disable_semantic_ranking(
    backend: State<'_, Backend>,
) -> Result<SemanticSnapshot, IpcError> {
    backend.disable_semantic_ranking().await
}

/// The assistant panel opened: loads the model in the background when the
/// option is on and the model ready. Synchronous: it reads memory and spawns.
#[tauri::command]
pub fn preload_semantic_model(backend: State<'_, Backend>) {
    backend.preload_semantic_model();
}
