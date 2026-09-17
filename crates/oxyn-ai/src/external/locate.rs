//! Finding an agent's program on the machine, when the user asks.
//!
//! # Only on request
//!
//! Nothing here runs at startup, and nothing here writes a declaration: a
//! « Detect » button calls it, the user reads what was found, and confirms.
//! Reading the machine without being asked is how a tool ends up declaring an
//! agent nobody chose (ADR-0023, IMPLEMENTATION-PLAN).
//!
//! # Why the usual places, and not only `PATH`
//!
//! A desktop application started from the Finder or a launcher does not get
//! the `PATH` of the user's shell: `/usr/bin:/bin` and little more. The
//! programs of a Claude Code or Codex installation live in documented places
//! outside it. They are listed in RESEARCH-NOTES with their sources; a place
//! no official page names is not searched.
//!
//! Every function here touches the file system: never on the IPC or UI thread
//! (I-05).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Where to look for a program, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchPath {
    dirs: Vec<PathBuf>,
}

impl SearchPath {
    /// The entries of a `PATH` value, then the documented installation places
    /// under `home`.
    #[must_use]
    pub fn usual(path_var: Option<&OsStr>, home: Option<&Path>) -> Self {
        let mut dirs: Vec<PathBuf> = path_var
            .map(|value| std::env::split_paths(value).collect())
            .unwrap_or_default();
        if let Some(home) = home {
            // Claude Code's native installer, and Codex's (RESEARCH-NOTES).
            dirs.push(home.join(".local").join("bin"));
            // Claude Code's former local npm installation.
            dirs.push(home.join(".claude").join("local"));
        }
        #[cfg(unix)]
        {
            // Homebrew on Apple Silicon, on Intel macs, on Linux; npm's usual
            // global prefix.
            dirs.push(PathBuf::from("/opt/homebrew/bin"));
            dirs.push(PathBuf::from("/usr/local/bin"));
            dirs.push(PathBuf::from("/home/linuxbrew/.linuxbrew/bin"));
        }
        let mut seen = Vec::with_capacity(dirs.len());
        for dir in dirs {
            if !dir.as_os_str().is_empty() && !seen.contains(&dir) {
                seen.push(dir);
            }
        }
        Self { dirs: seen }
    }

    /// Exactly these directories.
    #[must_use]
    pub fn of(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    /// The first executable file named `program`.
    ///
    /// A name with a separator is a path, and is checked as it is.
    #[must_use]
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        let as_path = Path::new(program);
        if as_path.components().count() > 1 || as_path.is_absolute() {
            return is_executable(as_path).then(|| as_path.to_path_buf());
        }
        self.dirs.iter().find_map(|dir| {
            candidates(program)
                .into_iter()
                .map(|name| dir.join(name))
                .find(|path| is_executable(path))
        })
    }
}

#[cfg(windows)]
fn candidates(program: &str) -> Vec<String> {
    ["", ".exe", ".cmd", ".bat"]
        .iter()
        .map(|extension| format!("{program}{extension}"))
        .collect()
}

#[cfg(not(windows))]
fn candidates(program: &str) -> Vec<String> {
    vec![program.to_owned()]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oxyn-locate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    #[cfg(unix)]
    fn program(dir: &Path, name: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        path
    }

    #[cfg(unix)]
    #[test]
    fn the_first_executable_wins_and_a_plain_file_is_not_one() {
        let first = scratch("first");
        let second = scratch("second");
        program(&first, "codex", 0o644);
        let expected = program(&second, "codex", 0o755);
        let search = SearchPath::of(vec![first, second]);
        assert_eq!(search.find("codex"), Some(expected));
        assert_eq!(search.find("claude"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_is_checked_as_given_not_searched() {
        let dir = scratch("path");
        let expected = program(&dir, "npx", 0o755);
        let search = SearchPath::of(Vec::new());
        assert_eq!(
            search.find(expected.to_str().expect("utf-8")),
            Some(expected)
        );
        assert_eq!(search.find("./npx"), None);
    }

    #[test]
    fn the_usual_places_follow_path_without_repeating_it() {
        let home = Path::new("/home/someone");
        let path = std::env::join_paths([home.join(".local/bin"), PathBuf::from("/usr/bin")])
            .expect("join");
        let search = SearchPath::usual(Some(&path), Some(home));
        assert_eq!(search.dirs.first(), Some(&home.join(".local/bin")));
        assert_eq!(
            search
                .dirs
                .iter()
                .filter(|d| **d == home.join(".local/bin"))
                .count(),
            1
        );
        assert!(search.dirs.contains(&home.join(".claude").join("local")));
    }
}
