//! What crosses the boundary for Settings → About: which build is running.
//!
//! Nothing here is read at run time from the machine — no path, no
//! environment variable, no checkout: the revision and its modified state are
//! what `build.rs` recorded, the platform is the target the binary was
//! compiled for. What the user copies from it goes into a public bug report
//! (I-03).

use serde::Serialize;

/// Whether the binary was compiled with optimizations, as `cargo` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BuildProfile {
    Development,
    Release,
}

/// The identity of the running build. A `None` is something the build could
/// not say — the screen writes « Unknown », never a guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildIdentity {
    /// The package version, as the bundle and the native About box show it.
    pub version: String,
    /// The commit the sources were at, full length.
    pub revision: Option<String>,
    /// Whether tracked files differed from that commit.
    pub modified: Option<bool>,
    pub os: &'static str,
    pub arch: &'static str,
    pub profile: BuildProfile,
}

impl BuildIdentity {
    /// `version` comes from the Tauri configuration, which the bundle and
    /// the native About box read too.
    pub fn of_this_build(version: String) -> Self {
        Self::recorded(
            version,
            option_env!("OXYN_SOURCE_REVISION"),
            option_env!("OXYN_SOURCE_MODIFIED"),
        )
    }

    fn recorded(version: String, revision: Option<&str>, modified: Option<&str>) -> Self {
        Self {
            version,
            revision: revision.map(str::to_owned),
            modified: modified.and_then(|modified| modified.parse().ok()),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            profile: if cfg!(debug_assertions) {
                BuildProfile::Development
            } else {
                BuildProfile::Release
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The version shown is the bundle's: the native About box, the
    /// installer and Settings → About must name the same one.
    #[test]
    fn the_bundle_version_is_the_package_version() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
            .expect("tauri.conf.json is the configuration the build embeds");
        assert_eq!(config["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn what_the_build_did_not_record_stays_unknown() {
        let identity = BuildIdentity::recorded("0.0.1".to_owned(), None, None);
        assert_eq!(identity.revision, None);
        assert_eq!(identity.modified, None);

        let unreadable = BuildIdentity::recorded("0.0.1".to_owned(), None, Some("maybe"));
        assert_eq!(unreadable.modified, None);
    }

    #[test]
    fn a_recorded_revision_is_carried_as_is() {
        let sha = "8359edb9ecae7f7bc5fa0fd0006b48634a2694c6";
        let identity = BuildIdentity::recorded("0.0.1".to_owned(), Some(sha), Some("true"));
        assert_eq!(identity.revision.as_deref(), Some(sha));
        assert_eq!(identity.modified, Some(true));
    }
}
