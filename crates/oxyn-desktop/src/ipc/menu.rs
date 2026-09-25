//! What crosses the boundary for the menu bar
//! ([ADR-0041](../../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md),
//! point 4).
//!
//! Both directions are deliberately poor. Towards the front, an activation is
//! an action id and nothing else: Rust does not know what the action does.
//! Towards Rust, an entry's state is a switch, a variant index and a switch:
//! no text and no combination, so a script in the webview can neither rename
//! « Quit » nor bind a key.

use serde::{Deserialize, Serialize};

/// An entry of the native bar was chosen, by click or by its accelerator.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuActivation {
    /// An action id of the manifest, never `app.quit`, which Rust runs itself.
    pub id: String,
}

/// The state the front computed for one entry of the bar.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEntryState {
    pub id: String,
    pub enabled: bool,
    /// 0 for the default label, `n` for the manifest's `n`-th variant.
    pub variant: usize,
    /// Whether the entry carries its accelerator: a zone with the focus may
    /// bind the same combination to another action.
    pub shortcut: bool,
}
