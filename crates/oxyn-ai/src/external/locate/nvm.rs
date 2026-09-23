//! Node installed by nvm.
//!
//! nvm puts no `node` in any fixed place: each version lives in
//! `$NVM_DIR/versions/node/v<x.y.z>/bin`, and a shell finds one only because
//! nvm's profile line prepends it to `PATH`. An application started from the
//! Finder never runs that line. So the version a new shell would use is read
//! the way nvm reads it — the `default` alias, a file under `$NVM_DIR/alias` —
//! and resolved against what is installed (RESEARCH-NOTES, nvm 0.40.8).
//!
//! Everything read here was written by nvm or by the user's own hand: a
//! malformed file is a missing answer, never a panic (I-09), and every read is
//! bounded.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

/// How many entries of `versions/node` are looked at. A few dozen is already
/// a lot; the bound keeps a pathological directory from stalling detection.
const MAX_VERSIONS: usize = 256;

/// How many aliases are followed. `default` → `lts/*` → `lts/<name>` is three;
/// nvm itself stops on a cycle, this stops on a cycle or a long chain.
const MAX_ALIAS_HOPS: usize = 8;

/// An alias file holds one short line.
const MAX_ALIAS_BYTES: u64 = 256;

/// `major.minor.patch`, compared in that order.
type Version = (u64, u64, u64);

/// The `bin` directory of the Node nvm would start in a new shell, if its
/// major is at least `minimum_major`; otherwise of the most recent installed
/// version that is. An older default would start the launcher and fail in the
/// adapter, with a message about Node the user did not ask about.
///
/// Reads the file system; `None` when nvm is absent or nothing qualifies.
pub(super) fn bin_dir(nvm_dir: &Path, minimum_major: u64) -> Option<PathBuf> {
    let versions = nvm_dir.join("versions").join("node");
    let installed = installed(&versions);
    let recent = |version: &Version| version.0 >= minimum_major;
    let chosen = default_version(&nvm_dir.join("alias"), &installed)
        .filter(recent)
        .or_else(|| {
            installed
                .iter()
                .map(|(version, _)| *version)
                .filter(recent)
                .max()
        })?;
    let (_, name) = installed.iter().find(|(version, _)| *version == chosen)?;
    Some(versions.join(name).join("bin"))
}

/// The installed versions, with the directory name each was read from.
fn installed(versions: &Path) -> Vec<(Version, String)> {
    let Ok(entries) = std::fs::read_dir(versions) else {
        return Vec::new();
    };
    entries
        .take(MAX_VERSIONS)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let version = parse_installed(&name)?;
            Some((version, name))
        })
        .collect()
}

/// `v22.23.2`, the only shape nvm gives an installed version's directory.
fn parse_installed(name: &str) -> Option<Version> {
    match *pattern(name.strip_prefix('v')?)?.as_slice() {
        [major, minor, patch] => Some((major, minor, patch)),
        _ => None,
    }
}

/// A version pattern as nvm accepts one in an alias: `22`, `22.23`, `v22.23.2`.
fn pattern(text: &str) -> Option<Vec<u64>> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let parts: Vec<u64> = text
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    (1..=3).contains(&parts.len()).then_some(parts)
}

/// What `default` resolves to among the installed versions.
///
/// `node` and `stable` mean the latest installed; a pattern means the latest
/// installed that matches it; anything else is the name of another alias —
/// `lts/*`, then `lts/<name>` — followed. `system`, `iojs` and aliases nvm
/// computes without a file resolve to nothing, and the caller falls back.
fn default_version(alias_dir: &Path, installed: &[(Version, String)]) -> Option<Version> {
    let mut name = "default".to_owned();
    for _ in 0..MAX_ALIAS_HOPS {
        let target = read_alias(alias_dir, &name)?;
        let prefix = match target.as_str() {
            "node" | "stable" => Vec::new(),
            other => match pattern(other) {
                Some(prefix) => prefix,
                None => {
                    name = target;
                    continue;
                }
            },
        };
        return installed
            .iter()
            .map(|(version, _)| *version)
            .filter(|version| {
                let parts = [version.0, version.1, version.2];
                parts.iter().zip(&prefix).all(|(have, want)| have == want)
            })
            .max();
    }
    None
}

/// The first line of the alias file `name`, if it is a plain relative name.
fn read_alias(alias_dir: &Path, name: &str) -> Option<String> {
    let relative = Path::new(name);
    // nvm refuses a `..` component for the same reason: the name comes from a
    // file, and must not lead out of the alias directory.
    if !relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(alias_dir.join(relative))
        .ok()?
        .take(MAX_ALIAS_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nvm(name: &str, versions: &[&str], aliases: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oxyn-nvm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for version in versions {
            std::fs::create_dir_all(dir.join("versions/node").join(version).join("bin"))
                .expect("version directory");
        }
        for (alias, target) in aliases {
            let path = dir.join("alias").join(alias);
            std::fs::create_dir_all(path.parent().expect("an alias has a parent"))
                .expect("alias directory");
            std::fs::write(path, format!("{target}\n")).expect("alias file");
        }
        dir
    }

    fn bin(dir: &Path, version: &str) -> Option<PathBuf> {
        Some(dir.join("versions/node").join(version).join("bin"))
    }

    #[test]
    fn a_partial_default_takes_the_latest_matching_version() {
        // The shape found on the development machine: `default` holds `22`.
        let dir = nvm(
            "partial",
            &["v22.17.0", "v22.23.2", "v24.1.0"],
            &[("default", "22")],
        );
        assert_eq!(bin_dir(&dir, 22), bin(&dir, "v22.23.2"));
    }

    #[test]
    fn a_default_too_old_gives_way_to_the_latest_recent_enough() {
        let dir = nvm(
            "old",
            &["v18.20.8", "v22.1.0", "v23.4.0"],
            &[("default", "v18.20.8")],
        );
        assert_eq!(bin_dir(&dir, 22), bin(&dir, "v23.4.0"));
    }

    #[test]
    fn an_lts_default_is_followed_through_its_aliases() {
        let dir = nvm(
            "lts",
            &["v22.9.0", "v24.2.0", "v25.0.0"],
            &[
                ("default", "lts/*"),
                ("lts/*", "lts/krypton"),
                ("lts/krypton", "v24.2.0"),
            ],
        );
        assert_eq!(bin_dir(&dir, 22), bin(&dir, "v24.2.0"));
    }

    #[test]
    fn a_cycle_or_an_escaping_alias_resolves_to_nothing_and_falls_back() {
        let cycle = nvm(
            "cycle",
            &["v22.0.0", "v22.10.1"],
            &[("default", "loop"), ("loop", "default")],
        );
        assert_eq!(bin_dir(&cycle, 22), bin(&cycle, "v22.10.1"));
        let escape = nvm("escape", &["v22.0.0"], &[("default", "../../etc/passwd")]);
        assert_eq!(read_alias(&escape.join("alias"), "../../etc/passwd"), None);
        assert_eq!(bin_dir(&escape, 22), bin(&escape, "v22.0.0"));
    }

    #[test]
    fn nothing_recent_enough_is_nothing_found() {
        let dir = nvm("none", &["v20.1.0", "system", "v22.x.0"], &[]);
        assert_eq!(bin_dir(&dir, 22), None);
        assert_eq!(bin_dir(&dir.join("absent"), 22), None);
    }

    #[test]
    fn each_agent_asks_for_its_own_minimum() {
        // Codex runs on 20 where Claude needs 22: one Node does not serve both.
        let dir = nvm("per-agent", &["v20.18.0", "v22.1.0"], &[("default", "20")]);
        assert_eq!(bin_dir(&dir, 20), bin(&dir, "v20.18.0"));
        assert_eq!(bin_dir(&dir, 22), bin(&dir, "v22.1.0"));
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        let dir = nvm("numeric", &["v22.9.0", "v22.10.0"], &[("default", "node")]);
        assert_eq!(bin_dir(&dir, 22), bin(&dir, "v22.10.0"));
        assert_eq!(pattern("99999999999999999999"), None);
        assert_eq!(pattern("22.1.2.3"), None);
        assert_eq!(pattern(""), None);
    }
}
