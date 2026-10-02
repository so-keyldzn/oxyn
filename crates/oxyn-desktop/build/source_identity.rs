//! Which source revision a build comes from, asked of git at build time.
//! Kept out of `build.rs` so that `tests/source_identity.rs` can exercise it:
//! a build script has no test harness of its own.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The revision of the Oxyn workspace, and the files whose change moves it.
#[derive(Debug)]
pub struct SourceIdentity {
    pub revision: String,
    /// `None` when git gave a revision but could not compare the tree to it.
    pub modified: Option<bool>,
    /// Absolute paths of `HEAD`, the index, the packed refs and the branch.
    pub watched: Vec<PathBuf>,
}

/// The identity of the git repository rooted at `workspace`, or `None` when
/// `workspace` is not the top level of one. An archive has no `.git`, and git
/// walks up from it: unpacked inside another repository, it would name that
/// repository's commit — often clean, since the archive is untracked there.
pub fn read(workspace: &Path) -> Option<SourceIdentity> {
    let workspace = workspace.canonicalize().ok()?;
    let top_level = git(&workspace, &["rev-parse", "--show-toplevel"])?;
    if Path::new(&top_level).canonicalize().ok()? != workspace {
        return None;
    }
    let revision = git(&workspace, &["rev-parse", "HEAD"]).filter(|sha| is_object_id(sha))?;
    // Untracked files left out: the build itself leaves some behind.
    let modified = git(
        &workspace,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .map(|status| !status.is_empty());
    let branch = git(&workspace, &["symbolic-ref", "-q", "HEAD"]);
    let watched = ["HEAD", "index", "packed-refs"]
        .into_iter()
        .chain(branch.as_deref())
        // `--git-path` answers relative to the directory git ran in.
        .filter_map(|path| git(&workspace, &["rev-parse", "--git-path", path]))
        .map(|path| workspace.join(path))
        .collect();
    Some(SourceIdentity {
        revision,
        modified,
        watched,
    })
}

/// What git printed, trimmed, or `None` when it is absent or failed.
fn git(directory: &Path, arguments: &[&str]) -> Option<String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "build time, on the machine that compiles: git reads the repository being built, and only a hash, a path or a status comes back"
    )]
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

/// A SHA-1 or SHA-256 object name: anything else is not a revision.
fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
