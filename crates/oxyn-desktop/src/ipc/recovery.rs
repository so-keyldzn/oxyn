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
}

/// What the backend asks of the webview while the window closes.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShutdownSignal {
    /// Submit every draft still waiting for its typing pause, then answer
    /// `shutdown_flushed`.
    FlushDrafts,
}
