//! Putting a verified update in place at the exit, and what the next launch
//! is told about it.
//!
//! The outcome crosses the restart in one open JSON file next to the
//! preference ([I-11](../../../../CLAUDE.md#i-11)), read and removed at the
//! next launch:
//!
//! ```json
//! {"format":1,"type":"installed","from":"0.0.2","to":"0.0.3"}
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri_plugin_updater::Update;

use super::failure;
use super::preference::{read_bounded, write_atomically};
use crate::ipc::updates::{UpdateErrorKind, UpdateNotice};

pub(crate) const NOTICE_FILE: &str = "update-notice.json";

const FORMAT: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Outcome {
    Installed { from: String, to: String },
    InstallFailed { version: String, message: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    format: u32,
    #[serde(flatten)]
    outcome: Outcome,
}

/// What the stored outcome says to a launch of `current_version`. An
/// installation that reports success but did not bring that version — an
/// older copy launched from elsewhere — says nothing.
pub(crate) fn notice_of(bytes: &[u8], current_version: &str) -> Option<UpdateNotice> {
    let stored: Stored = match serde_json::from_slice(bytes) {
        Ok(stored) => stored,
        Err(error) => {
            tracing::warn!(%error, "unreadable update notice; ignored");
            return None;
        }
    };
    match stored.outcome {
        Outcome::Installed { from, to } => {
            (to == current_version).then_some(UpdateNotice::Installed { from, to })
        }
        Outcome::InstallFailed { version, message } => {
            Some(UpdateNotice::InstallFailed { version, message })
        }
    }
}

/// Reads and removes the notice the previous exit left in `folder`.
/// Blocking: called once, at launch.
pub(crate) fn take_notice(folder: &Path, current_version: &str) -> Option<UpdateNotice> {
    let path = folder.join(NOTICE_FILE);
    let bytes = match read_bounded(&path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, "update notice not read");
            return None;
        }
    };
    // Removed before it is shown: a notice is said once, even if this
    // launch then crashes.
    if let Err(error) = std::fs::remove_file(&path) {
        tracing::warn!(%error, "update notice not removed; it may be shown again");
    }
    notice_of(&bytes, current_version)
}

/// A verified update, its bytes, and where to record the outcome.
pub(crate) struct Install {
    pub(crate) update: Update,
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) from: String,
    pub(crate) folder: Option<PathBuf>,
}

impl std::fmt::Debug for Install {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Install")
            .field("from", &self.from)
            .field("to", &self.update.version)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Install {
    /// Replaces the installation, then records the outcome. Blocking and
    /// without a time limit — interrupted midway, the swap could leave no
    /// application at all — so never on the main thread, except through
    /// [`Self::run_within`]. An error is recorded, never raised: the exit
    /// goes on.
    pub(crate) fn run(self) {
        let to = self.update.version.clone();
        let started = Instant::now();
        let outcome = match self.update.install(&*self.bytes) {
            Ok(()) => {
                tracing::info!(
                    from = %self.from,
                    %to,
                    elapsed_ms = started.elapsed().as_millis(),
                    "update installed"
                );
                Outcome::Installed {
                    from: self.from,
                    to,
                }
            }
            Err(error) => {
                tracing::error!(%error, version = %to, "the update could not be installed");
                Outcome::InstallFailed {
                    message: failure::message(UpdateErrorKind::Install, &error),
                    version: to,
                }
            }
        };
        let Some(folder) = self.folder else { return };
        let stored = Stored {
            format: FORMAT,
            outcome,
        };
        let written = serde_json::to_vec(&stored)
            .map_err(std::io::Error::other)
            .and_then(|bytes| write_atomically(&folder, NOTICE_FILE, &bytes));
        if let Err(error) = written {
            tracing::warn!(%error, "update outcome not recorded for the next launch");
        }
    }

    /// [`Self::run`] on a thread of its own, waited for at most `grace`.
    ///
    /// For the exits that run on the main thread: past `grace` the thread is
    /// left to finish while the process ends. The plugin asks macOS for an
    /// administrator on the main thread, which then waits here: that request
    /// only follows a refused move of the bundle, before anything was moved.
    pub(crate) fn run_within(self, grace: Duration) {
        let (done, finished) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("oxyn-update-install".into())
            .spawn(move || {
                self.run();
                let _ = done.send(());
            });
        match spawned {
            Ok(_) => {
                if finished.recv_timeout(grace).is_err() {
                    tracing::warn!("the update installation was still running at exit");
                }
            }
            Err(error) => tracing::error!(%error, "the update installation could not start"),
        }
    }
}
