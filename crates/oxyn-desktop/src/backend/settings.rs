//! Settings: the workspace display preferences, and the management of saved
//! connections (edit, delete).
//!
//! Both go through the command bus — `ReadWorkspacePreferences`,
//! `WriteWorkspacePreferences`, `UpdateConnection`, `DeleteConnection` — and
//! nothing here writes the store directly ([I-01](../../../../CLAUDE.md#i-01)).

mod connections;
mod location;
mod preferences;

#[cfg(test)]
mod location_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use oxyn_core::CommandId;
use parking_lot::Mutex;

pub(crate) use connections::PendingChange;
pub(crate) use preferences::PreferenceState;

/// The state this feature adds to [`Inner`](crate::backend::Inner).
#[derive(Default)]
pub(crate) struct SettingsState {
    pub(crate) preferences: PreferenceState,
    /// Connection changes the policy held back, by the command id the
    /// decision answers. Kept here: an edit may carry retyped secrets, which
    /// the webview has no reason to hold again.
    pub(crate) pending_changes: Mutex<HashMap<CommandId, PendingChange>>,
    /// Held while a connection change is computed and saved, or checked
    /// against the saved configuration and applied: two approvals must not
    /// both pass the check, nor a direct save slip between check and save.
    pub(crate) decisions: tokio::sync::Mutex<()>,
}

// Not derived: a pending edit carries secrets in clear (I-03).
impl std::fmt::Debug for SettingsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsState").finish_non_exhaustive()
    }
}
