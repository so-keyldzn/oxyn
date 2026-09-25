//! What crosses the boundary for the windows
//! ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md)).

use serde::{Deserialize, Serialize};

/// What the backend tells one window, on that window's own channel.
///
/// A notice, never data: the window reads what it shows again through the
/// bus.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WindowSignal {
    /// The user asked to close this window, which is not the last, and no
    /// transaction holds it: answer `shutdown_acknowledged` at once, then ask
    /// about the consoles that would lose work, and answer
    /// `confirm_window_close` or `cancel_exit`.
    CloseRequested,
    /// A saved connection this window holds was edited — marking, tier,
    /// read-only — in this window or another: read it again. A stale tier
    /// badge is a false one.
    #[serde(rename_all = "camelCase")]
    ConnectionChanged { connection: String },
    /// The workspace preferences were written by another window: read them
    /// again.
    PreferencesChanged,
}

/// The consoles a window shows, as its webview reports them: document ids
/// in tab order, and the one in front. Validated in Rust, never trusted: at
/// most 256, each a document no other window writes.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowConsoles {
    pub documents: Vec<String>,
    pub active: Option<String>,
}
