//! Recovery status and the ordered shutdown
//! ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).

use std::time::Duration;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager as _, RunEvent, State, Webview, WindowEvent};

use crate::backend::{Backend, ExitStep, WindowKey};
use crate::commands::windows::{bring_to_front, caller, close_requested, forget_window};
use crate::ipc::IpcError;
use crate::ipc::recovery::{RecoveryStatus, ShutdownSignal};
use crate::logging::FileJournal;
use crate::menu::{self, MenuBar};

/// The journal the exit flushes last, for the exits the front asks for.
pub struct ExitJournal(pub Option<FileJournal>);

/// How long the exit waits for the journal's last lines to reach the disk.
///
/// They are the only account of how the close went: without them, the next
/// diagnosis starts from a silent journal.
const JOURNAL_GRACE: Duration = Duration::from_secs(1);

/// How long an exit macOS does not let Oxyn hold — the Dock's Quit, a logout —
/// waits for local writes and the close record
/// ([ADR-0040](../../../../docs/adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md)).
///
/// The main thread waits: the window is gone, and macOS ends the process as
/// soon as it returns. Past it, the close stays unwritten.
const FORCED_EXIT_GRACE: Duration = Duration::from_secs(2);

/// Only the window built at launch is told how the previous launch ended.
#[tauri::command]
pub fn recovery_status(
    webview: Webview,
    backend: State<'_, Backend>,
) -> Result<RecoveryStatus, IpcError> {
    Ok(backend.recovery_status(caller(&backend, &webview)?))
}

#[tauri::command]
pub fn subscribe_shutdown(
    webview: Webview,
    backend: State<'_, Backend>,
    channel: Channel<ShutdownSignal>,
) -> Result<(), IpcError> {
    backend.subscribe_shutdown(caller(&backend, &webview)?, channel);
    Ok(())
}

#[tauri::command]
pub fn shutdown_flushed(webview: Webview, backend: State<'_, Backend>) -> Result<(), IpcError> {
    backend.shutdown_flushed(caller(&backend, &webview)?);
    Ok(())
}

/// The webview shows what it was asked to resolve — the exit's transactions,
/// or its window's close
/// ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md)).
///
/// Takes nothing. Outside a wait for it, does nothing; at worst, a script
/// keeps an exit or a close the user asked for waiting on the user's
/// decision. Synchronous: it only wakes a waiting task.
#[tauri::command]
pub fn shutdown_acknowledged(
    webview: Webview,
    backend: State<'_, Backend>,
) -> Result<(), IpcError> {
    backend.shutdown_acknowledged(caller(&backend, &webview)?);
    Ok(())
}

/// `Cancel` in the dialog of an exit held by a transaction, or of this
/// window's close: abandoned before anything is flushed, recorded or closed.
///
/// Takes nothing. Outside such an exit or close, does nothing; at worst, a
/// script cancels an exit the user asked for. Synchronous: in-memory state
/// and channel messages.
#[tauri::command]
pub fn cancel_exit(webview: Webview, backend: State<'_, Backend>) -> Result<(), IpcError> {
    backend.cancel_exit(caller(&backend, &webview)?);
    Ok(())
}

/// `File ▸ Exit` of Windows and Linux, and the palette's Quit: the ordered
/// exit ⌘Q and the window's close button take
/// ([ADR-0041](../../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
/// point 5).
///
/// It takes nothing and chooses nothing — no window, no delay, no step to
/// skip — so a script in the webview can only ask for the exit that flushes
/// the drafts and records the close. Asked twice, it starts once. It emits no
/// `Command`: it acts on no data. Synchronous: it only spawns the shutdown.
#[tauri::command]
pub fn request_exit(app: AppHandle, journal: State<'_, ExitJournal>) {
    begin_exit(&app, journal.0.as_ref());
}

/// Holds the window close and the application exit until drafts are flushed,
/// local writes are finished and the session close is recorded — then exits.
///
/// Every wait is bounded in [`Backend::shutdown`]: a dead webview cannot keep
/// the window open, and an unconfirmed close is left unrecorded.
pub fn on_run_event(app: &AppHandle, event: &RunEvent, journal: Option<&FileJournal>) {
    match event {
        RunEvent::WindowEvent { label, event, .. } => on_window_event(app, label, event, journal),
        // Quit is the one entry Rust runs itself: a frozen webview must not
        // keep the user in (ADR-0038). Every other entry is the front's, and
        // goes to the window that has the focus now, to it alone (ADR-0043).
        RunEvent::MenuEvent(event) if event.id() == menu::QUIT => begin_exit(app, journal),
        RunEvent::MenuEvent(event) => {
            if let Some(bar) = app.try_state::<MenuBar>() {
                let windows = &app.state::<Backend>().inner.windows;
                let focused = windows.focused().map(WindowKey::label);
                let first = windows.keys().first().map(|key| key.label());
                bar.forward(event.id().as_ref(), focused.as_deref(), first.as_deref());
            }
        }
        RunEvent::ExitRequested { api, .. } => {
            if app.state::<Backend>().shutdown_finished() {
                return;
            }
            api.prevent_exit();
            begin_exit(app, journal);
        }
        RunEvent::Exit => {
            // The Dock's Quit, a logout: macOS ends the loop without asking.
            // The close is recorded after local writes, drafts unflushed; past
            // the grace it stays unrecorded, and the next launch offers
            // recovery (ADR-0040). The window is gone: these waits freeze
            // nothing (I-05).
            let backend = app.state::<Backend>();
            if !backend.shutdown_finished() && !backend.close_on_forced_exit(FORCED_EXIT_GRACE) {
                tracing::warn!(
                    "exiting without the ordered shutdown; the close is not recorded yet"
                );
            }
            if let Some(journal) = journal {
                journal.flush(JOURNAL_GRACE);
            }
        }
        _ => {}
    }
}

/// A window's events. Each only updates the registry in memory or spawns a
/// task: nothing here writes, or waits for a lock a task holds (I-05).
fn on_window_event(
    app: &AppHandle,
    label: &str,
    event: &WindowEvent,
    journal: Option<&FileJournal>,
) {
    let backend = app.state::<Backend>();
    match event {
        // The last window's close is the application's exit; another's closes
        // its consoles, after its webview has asked (ADR-0043).
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            match backend.inner.windows.key_of(label) {
                Ok(window) => {
                    close_requested(app, backend.inner().clone(), window, journal.cloned());
                }
                // Not one of ours: nothing of Oxyn's to keep it for.
                Err(_) => begin_exit(app, journal),
            }
        }
        WindowEvent::Focused(focused) => {
            let windows = &backend.inner.windows;
            windows.focus(label, *focused);
            if *focused && let Some(bar) = app.try_state::<MenuBar>() {
                bar.refocus(Some(label));
            }
        }
        // Destroyed by a path that did not release it — the system, a crash
        // of the webview: what it held is let go all the same.
        WindowEvent::Destroyed => {
            if let Ok(window) = backend.inner.windows.key_of(label) {
                forget_window(app, label);
                let backend = backend.inner().clone();
                tauri::async_runtime::spawn(async move { backend.release_window(window).await });
            }
        }
        _ => {}
    }
}

/// Lists the open transactions, then begins the ordered shutdown once and
/// exits — unless a transaction holds the exit for the user to resolve
/// (ADR-0043). Asked again while the dialog is open, it lists again.
pub(crate) fn begin_exit(app: &AppHandle, journal: Option<&FileJournal>) {
    let backend = app.state::<Backend>().inner().clone();
    let app = app.clone();
    let journal = journal.cloned();
    // Off the main thread: the listing waits for the executor's events and
    // the webview's acknowledgement (I-05).
    tauri::async_runtime::spawn(async move {
        match backend.exit_step().await {
            ExitStep::Proceed => {}
            ExitStep::Held => return,
            ExitStep::Asked(windows) => {
                // The dialog is in each window that holds a transaction:
                // those come to the front, since the exit may have been asked
                // from the Dock or another app — and they alone.
                for window in windows {
                    bring_to_front(&app, window);
                }
                return;
            }
        }
        if !backend.begin_shutdown() {
            return;
        }
        backend.shutdown().await;
        if let Some(journal) = journal {
            let _ = tokio::task::spawn_blocking(move || journal.flush(JOURNAL_GRACE)).await;
        }
        app.exit(0);
    });
}
