//! [`HostConfirm`] on `tauri-plugin-dialog`: the dialog the user sees.
//!
//! The only part of ADR-0037 no automatic test reaches: it is checked by hand
//! in `make desktop-dev`, on each platform, whenever the plugin changes.

use std::sync::{Arc, OnceLock};

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::time::Instant;

use super::text::CANCEL;
use super::{Confirmation, HostConfirm, HostReply, Reply, Severity};

/// The native message dialog, once the application handle exists.
///
/// The backend opens before `tauri::Builder`, so that a failed start reaches
/// stderr: this port is built without a handle and receives it in `setup`.
/// Until then it refuses — not being able to open is refusing.
#[derive(Default)]
pub(crate) struct NativeDialog {
    app: OnceLock<AppHandle>,
    /// Held from when a dialog is handed to the plugin until it closes. The
    /// plugin cannot close a dialog, so one left past its deadline stays on
    /// screen: the next waits for it here, and its minimum delay starts when
    /// it is drawn, not when it was asked.
    ///
    /// Only this port's dialogs take it: the external agent's declaration
    /// (`commands/ai.rs`) still draws its own, outside the queue.
    screen: Arc<tokio::sync::Mutex<()>>,
}

impl NativeDialog {
    /// Gives the port the handle it draws with. A second call changes nothing.
    pub(crate) fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }
}

impl HostConfirm for NativeDialog {
    fn confirm(&self, confirmation: Confirmation, deadline: Instant) -> HostReply {
        let Some(app) = self.app.get().cloned() else {
            tracing::warn!("a critical confirmation was asked before the window existed: refused");
            return Box::pin(async { Reply::Refused });
        };
        let screen = Arc::clone(&self.screen);
        Box::pin(async move {
            let on_screen = match Arc::clone(&screen).try_lock_owned() {
                Ok(free) => free,
                Err(_) => {
                    tracing::info!("a critical dialog waits for an earlier one still on screen");
                    screen.lock_owned().await
                }
            };
            let dialog = app
                .dialog()
                .message(confirmation.body)
                .title(confirmation.title)
                .kind(match confirmation.severity {
                    Severity::Warning => MessageDialogKind::Warning,
                    Severity::Danger => MessageDialogKind::Error,
                })
                .buttons(MessageDialogButtons::OkCancelCustom(
                    confirmation.confirm.to_owned(),
                    CANCEL.to_owned(),
                ));
            let (shown_sender, shown_receiver) = tokio::sync::oneshot::channel();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            // Dated on the main thread, where the plugin draws: the webview can
            // keep that thread busy — synchronous commands, IPC arguments — and
            // a dialog dated before that queue would show with its second spent.
            // On the main thread already, the plugin's own hop runs in place.
            let handed = app.run_on_main_thread(move || {
                let shown = Instant::now();
                if shown >= deadline || shown_sender.is_closed() {
                    // Refused already, or nobody awaits it: drawing it would
                    // ask for an answer nobody reads, and hold the screen
                    // until it is closed.
                    return;
                }
                let _ = shown_sender.send(shown);
                dialog.show(move |confirmed| {
                    drop(on_screen);
                    let _ = sender.send(confirmed);
                });
            });
            if handed.is_err() {
                return Reply::Refused;
            }
            // A closure or callback dropped unanswered frees the screen, and
            // its dropped sender reads as a refusal.
            let Ok(shown) = shown_receiver.await else {
                return Reply::Refused;
            };
            match receiver.await {
                Ok(true) => Reply::Confirmed { shown },
                Ok(false) | Err(_) => Reply::Refused,
            }
        })
    }
}
