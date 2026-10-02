//! The update's commands
//! ([ADR-0051](../../../../docs/adr/0051-automatic-updates-from-github-releases.md)).
//!
//! Interface plumbing, like the menu and the exit: none emits a `Command`,
//! because none acts on data. None takes a URL, a path or a version — the
//! endpoint, the release page and the version installed are Rust's — so a
//! script in the webview can at worst check, download, cancel, or restart
//! into an update already verified.

use tauri::ipc::Channel;
use tauri::{AppHandle, State, Webview};

use super::recovery::{ExitJournal, begin_exit};
use super::subscriptions::Superseded;
use super::windows::caller;
use crate::backend::Backend;
use crate::backend::windows::Stream;
use crate::ipc::IpcError;
use crate::ipc::updates::{RestartOutcome, UpdateNotice, UpdateSnapshot};
use crate::updates::Updates;

/// Synchronous: a reading in memory.
#[tauri::command]
pub fn get_update_state(updates: State<'_, Updates>) -> UpdateSnapshot {
    updates.snapshot()
}

/// Every change of the update's state, the current one first, until this
/// window subscribes again or closes.
#[tauri::command]
pub fn subscribe_updates(
    webview: Webview,
    backend: State<'_, Backend>,
    updates: State<'_, Updates>,
    channel: Channel<UpdateSnapshot>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    // A reload subscribes again and ends this task (see `subscriptions`).
    let mut superseded = Superseded::start(&backend, window, Stream::Updates)?;
    let mut snapshots = updates.subscribe();
    tauri::async_runtime::spawn(async move {
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

/// `Check now`, and the menu's `Check for updates…`. Synchronous: it only
/// spawns the request.
#[tauri::command]
pub fn check_for_updates(app: AppHandle, updates: State<'_, Updates>) -> UpdateSnapshot {
    updates.check(&app)
}

/// Aborts the check or the download under way, its HTTP request with it.
#[tauri::command]
pub fn cancel_update(updates: State<'_, Updates>) -> UpdateSnapshot {
    updates.cancel()
}

/// Downloads the update a check announced, when automatic updates are off.
#[tauri::command]
pub fn download_update(updates: State<'_, Updates>) -> UpdateSnapshot {
    updates.download()
}

/// Async: it writes `updates.json` (I-05).
///
/// # Errors
/// The preference could not be written: nothing changed, `Save again` may
/// succeed.
#[tauri::command]
pub async fn set_automatic_updates(
    app: AppHandle,
    enabled: bool,
) -> Result<UpdateSnapshot, IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager as _;
        app.state::<Updates>().set_automatic(enabled)
    })
    .await
    .map_err(|_| IpcError::invalid("The preference worker stopped"))?
}

/// `Restart now`: the ordered exit, then the installation, then a new
/// launch. Without `confirmed`, statements or exports running anywhere stop
/// it with `busy`; an open transaction still asks, with the `restart` scope.
///
/// # Errors
/// No update is ready.
#[tauri::command]
pub fn restart_to_update(
    app: AppHandle,
    backend: State<'_, Backend>,
    updates: State<'_, Updates>,
    journal: State<'_, ExitJournal>,
    confirmed: bool,
) -> Result<RestartOutcome, IpcError> {
    let running = backend.inner.executor.running().len();
    let outcome = updates.ask_restart(running, confirmed)?;
    if outcome == RestartOutcome::Started {
        tracing::info!("restart to update asked");
        begin_exit(&app, journal.0.as_ref());
    }
    Ok(outcome)
}

/// Opens the release notes of the version announced — or of the one
/// running — in the system browser. The URL is built here.
///
/// # Errors
/// The version is not a plain semantic version, or the system could not
/// open the browser.
#[tauri::command]
pub async fn open_release_page(updates: State<'_, Updates>) -> Result<(), IpcError> {
    let page = updates
        .release_page()
        .ok_or_else(|| IpcError::invalid("This version has no release page."))?;
    tauri::async_runtime::spawn_blocking(move || {
        tauri_plugin_opener::open_url(&page, None::<&str>)
            .map_err(|error| IpcError::invalid(format!("The browser could not be opened: {error}")))
    })
    .await
    .map_err(|_| IpcError::invalid("The browser worker stopped"))?
}

/// What the previous launch's update left to say: shown once, by the
/// first window that asks.
#[tauri::command]
pub fn take_update_notice(updates: State<'_, Updates>) -> Option<UpdateNotice> {
    updates.take_notice()
}
