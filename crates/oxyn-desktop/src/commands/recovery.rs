//! Recovery status and the ordered shutdown
//! ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).

use std::time::Duration;

use tauri::ipc::Channel;
#[cfg(target_os = "macos")]
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager as _, RunEvent, State, WindowEvent};

use crate::backend::Backend;
use crate::ipc::recovery::{RecoveryStatus, ShutdownSignal};
use crate::logging::FileJournal;

/// The menu item that replaces the predefined Quit.
#[cfg(target_os = "macos")]
const QUIT: &str = "oxyn-quit";

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

/// Tauri's default macOS menu, with a Quit that goes through the ordered
/// shutdown.
///
/// The predefined Quit sends `terminate:` to the application: macOS then ends
/// the event loop without an `ExitRequested`, nothing can hold it, and the
/// close was never recorded — every ⌘Q looked like a crash at the next launch.
/// Elsewhere Tauri sets no menu, and the window close is the way out
/// ([ADR-0038](../../../../docs/adr/0038-un-plantage-s-annonce-une-fois.md)).
///
/// # Errors
/// If a menu item cannot be created.
#[cfg(target_os = "macos")]
pub fn application_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        authors: app.config().bundle.publisher.clone().map(|p| vec![p]),
        ..AboutMetadata::default()
    };
    let quit = MenuItem::with_id(
        app,
        QUIT,
        format!("Quit {}", package.name),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    // The application submenu comes first; it is rebuilt as Tauri builds it,
    // but for its last item.
    let application = Submenu::with_items(
        app,
        package.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    menu.remove_at(0)?;
    menu.prepend(&application)?;
    Ok(menu)
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
            request_exit(app, journal);
        }
        #[cfg(target_os = "macos")]
        RunEvent::MenuEvent(menu) if menu.id() == QUIT => request_exit(app, journal),
        RunEvent::ExitRequested { api, .. } => {
            if app.state::<Backend>().shutdown_finished() {
                return;
            }
            api.prevent_exit();
            request_exit(app, journal);
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
fn request_exit(app: &AppHandle, journal: Option<&FileJournal>) {
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
