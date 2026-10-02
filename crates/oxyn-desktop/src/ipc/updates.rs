//! What crosses the boundary for automatic updates
//! ([ADR-0051](../../../../docs/adr/0051-automatic-updates-from-github-releases.md)).
//!
//! Only the backend's reading of the update crosses: no URL, no path, no
//! signature, no manifest. A script in the webview learns which version is
//! announced, and can ask for nothing but the steps Rust already allows.
//!
//! Every timestamp is an RFC 3339 string in UTC.

use serde::Serialize;

/// Where the update stands. One operation at a time: a check asked during
/// `checking`, `downloading` or `ready` changes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UpdateState {
    /// Nothing under way, nothing found yet.
    Idle,
    Checking,
    #[serde(rename_all = "camelCase")]
    UpToDate { checked_at: String },
    /// Found, not downloaded: automatic updates are off, or the download was
    /// cancelled.
    Available { version: String },
    /// `total` is what the server announced, when it did.
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    /// Downloaded and verified, waiting for the application's exit. Lasts
    /// until the restart.
    #[serde(rename_all = "camelCase")]
    Ready {
        version: String,
        /// The release notes, plain text, at most 4 KiB. Never HTML to render.
        notes: Option<String>,
        /// When the release was published, as the manifest says.
        date: Option<String>,
        ready_at: String,
        /// Whether quitting installs it. `false` where Oxyn cannot write its
        /// own bundle — a read-only volume, a translocated app, a folder the
        /// user does not own: only `Restart now` installs, and macOS may ask
        /// for an administrator there.
        install_on_quit: bool,
    },
    /// The last operation failed. `offline` and `server` are silent outside
    /// Settings; `signature` and `install` are always shown.
    Error {
        kind: UpdateErrorKind,
        message: String,
        /// Data, not a guess from the message ([I-13](../../../../CLAUDE.md#i-13)).
        retryable: bool,
        /// The version the failure concerns, once one was announced.
        version: Option<String>,
    },
    /// No update can happen in this installation, or the user turned them off.
    Disabled { reason: DisabledReason },
}

/// The family of an update failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateErrorKind {
    /// github.com could not be reached.
    Offline,
    /// It answered, but not with a usable update.
    Server,
    /// The download does not carry Oxyn's signature: discarded.
    Signature,
    /// The update was verified but could not be put in place.
    Install,
}

/// Why updates are off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DisabledReason {
    /// Automatic updates are off: Oxyn neither checks nor downloads on its
    /// own, but `Check now` still works.
    User,
    /// `OXYN_UPDATES=off`: a managed or offline workstation. No action.
    Admin,
    /// A deb or rpm package, or a platform Oxyn does not update itself on:
    /// the system's package manager does. No action.
    PackageManager,
    /// A development build. No action.
    Dev,
}

/// What `get_update_state` and the `subscribe_updates` channel carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub state: UpdateState,
    /// The running version.
    pub current_version: String,
    /// The preference, kept even where `state` is `disabled`.
    pub automatic: bool,
    /// The last check that reached the server, during this launch.
    pub last_checked_at: Option<String>,
}

/// The answer to `restart_to_update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RestartOutcome {
    /// The ordered exit began; the application restarts once installed. An
    /// open transaction still asks first, with the `restart` scope.
    Started,
    /// Work would be stopped: nothing began. Ask again with `confirmed`.
    Busy {
        /// Statements running on a server, across every window.
        running: usize,
        /// Exports being written, across every window. Stopped by the exit:
        /// the destination keeps its previous content.
        exports: usize,
    },
}

/// What the previous launch's update left to say, once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UpdateNotice {
    /// Oxyn now runs `to`.
    Installed { from: String, to: String },
    /// `version` was not installed; Oxyn still runs the previous one.
    InstallFailed { version: String, message: String },
}
