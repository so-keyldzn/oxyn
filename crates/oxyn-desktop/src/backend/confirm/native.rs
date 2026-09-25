//! [`HostConfirm`] on `tauri-plugin-dialog`: the dialog the user sees.
//!
//! The only part of ADR-0037 no automatic test reaches: it is checked by hand
//! in `make desktop-dev`, on each platform, whenever the plugin changes.

use std::sync::OnceLock;

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use super::text::CANCEL;
use super::{Confirmation, HostConfirm, HostReply, Severity};

/// The native message dialog, once the application handle exists.
///
/// The backend opens before `tauri::Builder`, so that a failed start reaches
/// stderr: this port is built without a handle and receives it in `setup`.
/// Until then it answers `false` — not being able to open is refusing.
#[derive(Default)]
pub(crate) struct NativeDialog {
    app: OnceLock<AppHandle>,
}

impl NativeDialog {
    /// Gives the port the handle it draws with. A second call changes nothing.
    pub(crate) fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }
}

impl HostConfirm for NativeDialog {
    fn confirm(&self, confirmation: Confirmation) -> HostReply {
        let Some(app) = self.app.get() else {
            tracing::warn!("a critical confirmation was asked before the window existed: refused");
            return Box::pin(async { false });
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .message(confirmation.body)
            .title(confirmation.title)
            .kind(match confirmation.severity {
                Severity::Warning => MessageDialogKind::Warning,
                Severity::Danger => MessageDialogKind::Error,
            })
            .buttons(MessageDialogButtons::OkCancelCustom(
                confirmation.confirm.to_owned(),
                CANCEL.to_owned(),
            ))
            .show(move |confirmed| {
                let _ = sender.send(confirmed);
            });
        // The plugin ignores a failed `run_on_main_thread`: the callback is
        // then dropped unanswered, and the dropped sender reads as a refusal.
        Box::pin(async move { receiver.await.unwrap_or(false) })
    }
}
