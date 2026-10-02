//! Generates the Tauri context: configuration, capabilities and embedded icons.
//! Also records which source revision this binary was built from, for
//! Settings → About.

use std::process::Command;

fn main() {
    source_identity();
    tauri_build::build();
}

/// Sets `OXYN_SOURCE_REVISION` and `OXYN_SOURCE_MODIFIED` for the crate, or
/// leaves them unset when git cannot say — a build from an archive has no
/// `.git`, a machine may have no git. Unset reads as « Unknown » on screen:
/// a revision is captured here, never guessed later from whatever checkout
/// sits on the user's machine.
fn source_identity() {
    let Some(revision) = git(&["rev-parse", "HEAD"]).filter(|sha| is_object_id(sha)) else {
        return;
    };
    println!("cargo:rustc-env=OXYN_SOURCE_REVISION={revision}");
    // Untracked files left out: the build itself leaves some behind.
    if let Some(status) = git(&["status", "--porcelain", "--untracked-files=no"]) {
        let modified = !status.is_empty();
        println!("cargo:rustc-env=OXYN_SOURCE_MODIFIED={modified}");
    }
    // Cargo reruns this script when the commit, the branch or the index
    // moves. An edit nobody staged moves none of them: the modified state of
    // a development build is that of its last rerun. A release is built from
    // a fresh checkout, where both are exact.
    for path in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &branch])
    {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// What git printed, trimmed, or `None` when it is absent or failed.
fn git(arguments: &[&str]) -> Option<String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "build time, on the machine that compiles: git reads the repository being built, and only a hash or a status comes back"
    )]
    let output = Command::new("git").args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

/// A SHA-1 or SHA-256 object name: anything else is not a revision.
fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
