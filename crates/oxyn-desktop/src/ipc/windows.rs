//! What crosses the boundary for the windows
//! ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md)).

use serde::{Deserialize, Serialize};

use crate::ipc::OpenConnection;
use crate::ipc::consoles::ParameterInput;
use crate::ipc::location::ObjectPlace;

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

/// `Open in new window` on a tab: what moves (ADR-0043, « Déplacer une
/// console »).
///
/// **No `Debug` derive**: a console's bound values travel with it, and a
/// `{request:?}` added later would print them (I-03).
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum HandoffRequest {
    /// A console: its session, its document, the result it shows and its
    /// bound values move; nothing runs again.
    #[serde(rename_all = "camelCase")]
    Console {
        connection: String,
        session: String,
        document: String,
        result: Option<HandedResult>,
        parameters: Vec<ParameterInput>,
    },
    /// An object tab: the new window opens its connection and the same place,
    /// read through the bus like a selection.
    #[serde(rename_all = "camelCase")]
    Object {
        connection: String,
        place: ObjectPlace,
    },
}

/// The result a moved console shows: the new window reads the same
/// `ResultBuffer`, nothing is executed again (I-06).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandedResult {
    pub result: String,
    pub rows: u64,
    pub complete: bool,
    pub truncated: bool,
    pub cancelled: bool,
    pub elapsed_ms: u64,
}

/// What the new window adopts, once: its workspace, and the console or the
/// place it receives. Held in memory only, never written; dropped with the
/// window if it closes first.
///
/// **No `Debug` derive**, for the bound values it carries (I-03).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConsoleHandoff {
    #[serde(rename_all = "camelCase")]
    Console {
        open: OpenConnection,
        document: String,
        result: Option<HandedResult>,
        parameters: Vec<ParameterInput>,
    },
    #[serde(rename_all = "camelCase")]
    Object { open: OpenConnection },
}
