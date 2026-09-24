//! What crosses the boundary for recovery and shutdown
//! ([ADR-0021](../../../../docs/adr/0021-marqueur-d-arret.md)).

use serde::Serialize;

/// How the previous launch ended, as observed before anything was written.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    /// The previous session was left open and silent. Only then may the
    /// recovery screen say Oxyn did not close normally.
    pub abnormal: bool,
    /// History holds an execution whose server state must be inspected — the
    /// library's `needs_inspection`. Only then does the screen warn about a
    /// write with an unknown outcome.
    ///
    /// Read at startup, like `abnormal`: the screen speaks of what the launch
    /// found. A write that turns ambiguous during this session is flagged in
    /// the library instead, and re-reading history here would be a second path
    /// to it beside the command bus (I-01).
    pub unresolved_write: bool,
}

/// What the backend asks of the webview while the window closes.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShutdownSignal {
    /// Submit every draft still waiting for its typing pause, then answer
    /// `shutdown_flushed`.
    FlushDrafts,
}
