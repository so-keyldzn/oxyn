//! Recovery status and the ordered shutdown
//! ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).

use std::time::Duration;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager as _, RunEvent, State, WindowEvent};

use crate::backend::Backend;
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

#[tauri::command]
pub fn recovery_status(backend: State<'_, Backend>) -> RecoveryStatus {
    backend.recovery_status()
}

#[tauri::command]
pub fn subscribe_shutdown(backend: State<'_, Backend>, channel: Channel<ShutdownSignal>) {
    backend.subscribe_shutdown(channel);
}

#[tauri::command]
pub fn shutdown_flushed(backend: State<'_, Backend>) {
    backend.shutdown_flushed();
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
        RunEvent::WindowEvent {
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            api.prevent_close();
            begin_exit(app, journal);
        }
        // Quit is the one entry Rust runs itself: a frozen webview must not
        // keep the user in (ADR-0038). Every other entry is the front's.
        RunEvent::MenuEvent(event) if event.id() == menu::QUIT => begin_exit(app, journal),
        RunEvent::MenuEvent(event) => {
            if let Some(bar) = app.try_state::<MenuBar>() {
                bar.forward(event.id().as_ref());
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

/// Begins the ordered shutdown once, then exits.
fn begin_exit(app: &AppHandle, journal: Option<&FileJournal>) {
    let backend = app.state::<Backend>().inner().clone();
    if !backend.begin_shutdown() {
        return;
    }
    let app = app.clone();
    let journal = journal.cloned();
    tauri::async_runtime::spawn(async move {
        backend.shutdown().await;
        if let Some(journal) = journal {
            let _ = tokio::task::spawn_blocking(move || journal.flush(JOURNAL_GRACE)).await;
        }
        app.exit(0);
    });
}
