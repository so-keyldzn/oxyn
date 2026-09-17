//! Recovery status and the ordered shutdown
//! ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager as _, RunEvent, State, WindowEvent};

use crate::backend::Backend;
use crate::ipc::recovery::{RecoveryStatus, ShutdownSignal};

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

/// Holds the window close and the application exit until drafts are flushed,
/// local writes are finished and the session close is recorded — then exits.
///
/// Every wait is bounded in [`Backend::shutdown`]: a dead webview cannot keep
/// the window open, and an unconfirmed close is left unrecorded.
pub fn on_run_event(app: &AppHandle, event: &RunEvent) {
    let held = match event {
        RunEvent::WindowEvent {
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            api.prevent_close();
            true
        }
        RunEvent::ExitRequested { api, .. } => {
            let backend = app.state::<Backend>();
            if backend.shutdown_finished() {
                return;
            }
            api.prevent_exit();
            true
        }
        _ => false,
    };
    if !held {
        return;
    }
    let backend = app.state::<Backend>().inner().clone();
    if !backend.begin_shutdown() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        backend.shutdown().await;
        app.exit(0);
    });
}
