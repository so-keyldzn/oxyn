//! Settings → About: which build is running.
//!
//! What this widens for a script in the webview: reading the version, the
//! source revision, the platform and the profile of the binary — constants
//! fixed at compile time, which the bundle already exposes to anyone who
//! opens it. No store, no connection, no environment is read.

use tauri::AppHandle;

use crate::ipc::about::BuildIdentity;

/// Synchronous: everything it returns is a constant already in memory.
#[tauri::command]
pub fn build_identity(app: AppHandle) -> BuildIdentity {
    BuildIdentity::of_this_build(app.package_info().version.to_string())
}
