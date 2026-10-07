//! What crosses the boundary for local semantic ranking
//! ([ADR-0056](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md)).
//!
//! Only the backend's reading of the option crosses: whether it is on, and
//! where the model stands. No path, no URL, no file name: the webview cannot
//! choose what is downloaded nor where it is written.

use serde::Serialize;

/// Where the local embedding model stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelState {
    /// This workspace has no data directory to hold the model.
    Unavailable,
    /// Not downloaded, removed, or a download stopped before its end.
    Absent,
    /// Hashing the files already present, to keep those that are valid.
    Verifying,
    /// `total` is what this download fetches: less than the full model when
    /// a file already present was valid.
    Downloading { received: u64, total: u64 },
    /// Writing the local model file; a few seconds, not cancellable.
    Converting,
    /// On disk and verified by size; loaded on the next question.
    Ready,
    /// A file is present but is not the pinned one: download it again.
    Corrupt,
    /// The last download or load failed. The message names no local path.
    Failed { message: String, retryable: bool },
}

/// The option and its model, as the settings show them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSnapshot {
    /// The workspace preference, off by default.
    pub enabled: bool,
    pub model: ModelState,
}

impl SemanticSnapshot {
    /// Whether a download is under way: turning the option on again changes
    /// nothing until it ends.
    pub const fn downloading(&self) -> bool {
        matches!(
            self.model,
            ModelState::Verifying | ModelState::Downloading { .. } | ModelState::Converting
        )
    }
}
