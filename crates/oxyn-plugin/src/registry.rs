//! Plugin discovery, and persistence of what the user approved.
//!
//! A plugin directory looks like this:
//!
//! ```text
//! <data>/plugins/
//! ├── approvals.toml          ← what the user approved
//! ├── revue-schema/
//! │   └── plugin.toml         ← a declarative agent
//! └── duckdb/
//!     ├── plugin.toml
//!     └── duckdb.wasm
//! ```
//!
//! # The three rules that govern this module
//!
//! **Installed is not authorized.** Dropping a directory gives nothing: the
//! plugin starts in the [`Installed`](PluginState::Installed) state, its
//! permissions are presented to the user, and only the user moves it to
//! [`Approved`](PluginState::Approved)
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)).
//!
//! **Approval is about permissions, not a version.** An update that asks for
//! nothing more stays covered — asking again at every patch teaches clicking
//! without reading. An update that **widens** voids the approval and the plugin
//! falls back to `Installed`. [`PluginPermissions::is_subset_of`] decides, and
//! it is tested.
//!
//! **A broken plugin breaks no other.** An unreadable manifest produces a
//! [`Failed`](PluginState::Failed) carrying its reason; the others load. An
//! all-or-nothing discovery would make a whole workspace disappear for a
//! misplaced comma in a third-party file.
//!
//! # What this module is not
//!
//! It is **synchronous** and does I/O: none of its methods must be called from
//! the interface thread (I-05). It is up to the caller to move them onto the
//! blocking pool.
//!
//! The approvals file is TOML readable without Oxyn (I-11): what was granted can
//! be reread with a text editor, and revoked by deleting a section.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent_plugin::DeclarativeAgent;
use crate::error::{PluginError, Result};
use crate::manifest::{
    HOST_API_VERSION, MANIFEST_FILE, PluginDriverSpec, PluginKind, PluginManifest,
    PluginPermissions, PluginVersion,
};

/// Name of the approvals file, at the root of the plugin directory.
pub const APPROVALS_FILE: &str = "approvals.toml";

/// Suffix of the temporary file the approvals are written through.
const APPROVALS_TMP: &str = "approvals.toml.tmp";

// ─────────────────────────────────────────────────────────────────────────────
// State
// ─────────────────────────────────────────────────────────────────────────────

/// Where a plugin stands, from the dropped directory to the loadable component.
///
/// **Closed** enumeration: these four states are exhaustive, and adding one
/// must force rereading every place that decides whether to load, display or
/// warn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Manifest read and valid, permissions **not yet approved**.
    ///
    /// It is also the state of a plugin whose approval became void because its
    /// update asks for more.
    Installed,
    /// The user approved the permissions this manifest asks for.
    Approved,
    /// The user disabled this plugin. An update does not re-enable it.
    Disabled,
    /// The manifest is missing, unreadable or refused.
    Failed {
        /// What was refused, showable to the user.
        reason: String,
    },
}

impl PluginState {
    /// Stable name, for a log or an interface.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Approved => "approved",
            Self::Disabled => "disabled",
            Self::Failed { .. } => "failed",
        }
    }

    /// Is the plugin loadable?
    ///
    /// Only [`Approved`](Self::Approved) answers `true`. `Installed` answers
    /// `false`: it is the difference between dropping a file and granting a
    /// right.
    #[must_use]
    pub const fn is_approved(&self) -> bool {
        matches!(self, Self::Approved)
    }
}

/// What the user decided about a plugin, as persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// The plugin can be loaded.
    Approved,
    /// The plugin is disabled, without being uninstalled.
    Disabled,
}

/// A persisted decision, with **what it was about**.
///
/// Recording the approved permissions rather than a fingerprint is deliberate:
/// the file stays readable without Oxyn (I-11), the user can check what they
/// granted, and comparing with a new manifest says *what was added* rather than
/// "it changed".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    /// The decision.
    pub decision: ApprovalDecision,
    /// The plugin version at the time of the decision. Informative: it is not
    /// what voids the approval.
    pub version: PluginVersion,
    /// The permissions as they were presented and accepted.
    #[serde(default)]
    pub permissions: PluginPermissions,
}

/// Shape of the `approvals.toml` file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ApprovalsFile {
    #[serde(default)]
    approvals: BTreeMap<String, ApprovalRecord>,
}

// ─────────────────────────────────────────────────────────────────────────────
// An installed plugin
// ─────────────────────────────────────────────────────────────────────────────

/// A plugin directory, as discovery found it.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    slug: String,
    directory: PathBuf,
    manifest: Option<PluginManifest>,
    state: PluginState,
}

impl InstalledPlugin {
    /// The directory name, which is also the plugin identifier when the
    /// manifest could be read.
    ///
    /// It is the only thing always known, including of a failed plugin — and
    /// it is therefore the registry key.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The plugin directory.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The manifest, when it could be read and validated.
    #[must_use]
    pub const fn manifest(&self) -> Option<&PluginManifest> {
        self.manifest.as_ref()
    }

    /// The plugin state.
    #[must_use]
    pub const fn state(&self) -> &PluginState {
        &self.state
    }

    /// Is the plugin approved?
    #[must_use]
    pub const fn is_approved(&self) -> bool {
        self.state.is_approved()
    }

    /// Showable name: the manifest's, or the directory name otherwise.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.manifest
            .as_ref()
            .map_or(self.slug.as_str(), |m| m.name.as_str())
    }

    /// The surface occupied, when the manifest is readable.
    #[must_use]
    pub fn kind(&self) -> Option<PluginKind> {
        self.manifest.as_ref().map(|m| m.kind)
    }

    /// The **effective** permissions.
    ///
    /// They are the manifest's, not the approved ones: discovery guarantees the
    /// former are included in the latter, otherwise the plugin would not be
    /// [`Approved`](PluginState::Approved). The intersection is therefore the
    /// manifest itself.
    ///
    /// # Errors
    /// [`PluginError::NotApproved`] if the plugin is not approved;
    /// [`PluginError::InvalidManifest`] if it has no readable manifest.
    pub fn effective_permissions(&self) -> Result<&PluginPermissions> {
        let manifest = self.manifest.as_ref().ok_or_else(|| {
            PluginError::invalid_manifest(self.slug.as_str(), "no readable manifest")
        })?;
        if !self.state.is_approved() {
            return Err(PluginError::NotApproved {
                plugin: self.slug.clone(),
            });
        }
        Ok(&manifest.permissions)
    }

    /// Can this plugin be put into service by **this build** of Oxyn?
    ///
    /// A declarative agent can as soon as it is approved. A plugin that runs
    /// code requires the WebAssembly host: without the `wasm-host` feature, it
    /// is installable and approvable but **unusable**, and it is better to say
    /// so than to display it as active — a plugin that does nothing without
    /// explaining why reads as an Oxyn failure.
    ///
    /// # Errors
    /// [`PluginError::InvalidManifest`] if the manifest is unreadable,
    /// [`PluginError::NotApproved`] if it is not approved,
    /// [`PluginError::WasmHostUnavailable`] if it requires a host this build
    /// does not provide.
    pub fn check_usable(&self) -> Result<()> {
        self.effective_permissions()?;

        #[cfg(not(feature = "wasm-host"))]
        {
            if self
                .manifest
                .as_ref()
                .is_some_and(PluginManifest::requires_wasm)
            {
                return Err(PluginError::WasmHostUnavailable {
                    plugin: self.slug.clone(),
                });
            }
        }

        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The registry
// ─────────────────────────────────────────────────────────────────────────────

/// The discovered plugins, and their approvals.
///
/// A [`BTreeMap`] and not a hash table: the order is alphabetical, hence
/// reproducible from one machine to another. A plugin list that reorders
/// between two openings reads as a change.
#[derive(Debug, Clone)]
pub struct PluginRegistry {
    root: PathBuf,
    plugins: BTreeMap<String, InstalledPlugin>,
    approvals: BTreeMap<String, ApprovalRecord>,
}

impl PluginRegistry {
    /// An empty registry, backed by a plugin directory.
    ///
    /// The path must be **absolute**: the confinement checks of file
    /// permissions are lexical, and a relative path would make them depend on
    /// the process's current directory. Nothing enforces it here — the caller
    /// resolves the system's data directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            plugins: BTreeMap::new(),
            approvals: BTreeMap::new(),
        }
    }

    /// The plugin directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of the approvals file.
    #[must_use]
    pub fn approvals_path(&self) -> PathBuf {
        self.root.join(APPROVALS_FILE)
    }

    /// Rereads the plugin directory and the approvals file.
    ///
    /// A missing directory is **not** an error: it is the state of a fresh
    /// install, and Oxyn must start without it. A subdirectory without
    /// `plugin.toml` is ignored: it is not a broken plugin, it is not a plugin.
    ///
    /// **Synchronous and blocking.** Do not call from the interface thread
    /// (I-05).
    ///
    /// # Errors
    /// [`PluginError::Approvals`] if the approvals file exists but is
    /// unreadable — silently losing the approvals would make every plugin fall
    /// back to `Installed` without anyone knowing why;
    /// [`PluginError::Directory`] if the directory exists but cannot be
    /// traversed.
    pub fn discover(&mut self) -> Result<()> {
        self.approvals = read_approvals(&self.approvals_path())?;

        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                self.plugins.clear();
                return Ok(());
            }
            Err(source) => {
                return Err(PluginError::Directory {
                    path: self.root.clone(),
                    source,
                });
            }
        };

        let mut found = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|source| PluginError::Directory {
                path: self.root.clone(),
                source,
            })?;
            let directory = entry.path();
            if !directory.is_dir() {
                continue;
            }
            let Some(slug) = directory.file_name().and_then(|ident| ident.to_str()) else {
                tracing::warn!("plugin directory with a non-UTF-8 name: ignored");
                continue;
            };
            let slug = slug.to_owned();
            let manifest_path = directory.join(MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            found.insert(slug.clone(), self.load_one(slug, directory, &manifest_path));
        }

        self.plugins = found;
        Ok(())
    }

    /// Reads and classifies a plugin. Never returns an error: a faulty plugin
    /// becomes [`Failed`](PluginState::Failed) and leaves the others alone.
    fn load_one(&self, slug: String, directory: PathBuf, manifest_path: &Path) -> InstalledPlugin {
        let failure = |slug: String, directory: PathBuf, reason: String| {
            tracing::warn!(plugin = %slug, motif = %reason, "plugin refused");
            InstalledPlugin {
                slug,
                directory,
                manifest: None,
                state: PluginState::Failed { reason },
            }
        };

        let text = match fs::read_to_string(manifest_path) {
            Ok(text) => text,
            Err(err) => {
                return failure(
                    slug,
                    directory,
                    format!("`{MANIFEST_FILE}` unreadable: {err}"),
                );
            }
        };
        let manifest = match PluginManifest::from_toml(&text) {
            Ok(manifest) => manifest,
            Err(err) => return failure(slug, directory, err.to_string()),
        };

        // The directory carries the identifier. Without this rule, two
        // directories could claim the same identifier, and the approval of one
        // would cover the other.
        if manifest.id.as_str() != slug {
            let reason = format!(
                "the manifest declares itself `{}` but its directory is named `{slug}`: \
                 its approval would cover another plugin",
                manifest.id
            );
            return failure(slug, directory, reason);
        }

        // A file permission that encompassed the plugin directory would give
        // the plugin control over `approvals.toml`, hence over its own approval
        // and over the others'.
        for root_dir in &manifest.permissions.filesystem {
            if self.root.starts_with(root_dir) {
                let reason = format!(
                    "the file root `{}` contains the plugin directory: \
                     the plugin could rewrite the approvals",
                    root_dir.display()
                );
                return failure(slug, directory, reason);
            }
        }

        let state = self.state_for(&manifest);
        InstalledPlugin {
            slug,
            directory,
            manifest: Some(manifest),
            state,
        }
    }

    /// Crosses the manifest with the persisted approval.
    fn state_for(&self, manifest: &PluginManifest) -> PluginState {
        let Some(record) = self.approvals.get(manifest.id.as_str()) else {
            return PluginState::Installed;
        };
        // A deactivation is a decision, not a consequence: an update does not
        // lift it.
        if record.decision == ApprovalDecision::Disabled {
            return PluginState::Disabled;
        }
        if manifest.permissions.is_subset_of(&record.permissions) {
            PluginState::Approved
        } else {
            tracing::warn!(
                plugin = %manifest.id,
                "stale approval: the update asks for more"
            );
            PluginState::Installed
        }
    }

    /// The plugin carrying this directory name.
    #[must_use]
    pub fn get(&self, slug: &str) -> Option<&InstalledPlugin> {
        self.plugins.get(slug)
    }

    /// The plugin carrying this name, or a showable error.
    ///
    /// # Errors
    /// [`PluginError::Unknown`] if no plugin carries this name.
    pub fn require(&self, slug: &str) -> Result<&InstalledPlugin> {
        self.get(slug).ok_or_else(|| PluginError::Unknown {
            plugin: slug.to_owned(),
        })
    }

    /// The plugins, in the alphabetical order of their directory.
    pub fn iter(&self) -> impl Iterator<Item = &InstalledPlugin> {
        self.plugins.values()
    }

    /// Only the approved plugins.
    pub fn approved(&self) -> impl Iterator<Item = &InstalledPlugin> {
        self.plugins.values().filter(|p| p.is_approved())
    }

    /// Number of discovered plugins, failed states included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// No plugin was discovered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// The declarative agents provided by **approved** plugins.
    ///
    /// An unapproved plugin does not appear: dropping a file is not enough to
    /// add an agent to the AI workspace.
    ///
    /// Allocates on every call; it is a window-opening path.
    #[must_use]
    pub fn declarative_agents(&self) -> Vec<DeclarativeAgent> {
        self.approved()
            .filter(|plugin| plugin.kind() == Some(PluginKind::Agent))
            .filter_map(|plugin| {
                let manifest = plugin.manifest()?;
                DeclarativeAgent::from_manifest(manifest).ok()
            })
            .collect()
    }

    /// The protocols claimed by **approved** driver plugins.
    ///
    /// Used by `oxyn-desktop` to refuse a plugin that would claim an identifier
    /// already held by a native driver, before running a single instruction of
    /// the component.
    #[must_use]
    pub fn driver_specs(&self) -> Vec<&PluginDriverSpec> {
        self.approved()
            .filter_map(|plugin| plugin.manifest()?.driver.as_ref())
            .collect()
    }

    /// Records the user's approval for the permissions this manifest asks for
    /// **today**.
    ///
    /// Writes nothing to disk: call [`save`](Self::save) afterwards.
    ///
    /// # Errors
    /// [`PluginError::Unknown`] if the plugin does not exist;
    /// [`PluginError::InvalidManifest`] if it has no readable manifest;
    /// [`PluginError::IncompatibleInterface`] if it targets an interface
    /// version this host does not provide — having approved what can never run
    /// teaches clicking without reading.
    pub fn approve(&mut self, slug: &str) -> Result<()> {
        let record = {
            let plugin = self.require(slug)?;
            let manifest = plugin.manifest().ok_or_else(|| {
                PluginError::invalid_manifest(slug, "no readable manifest to approve")
            })?;
            if !manifest.is_api_compatible() {
                return Err(PluginError::IncompatibleInterface {
                    plugin: slug.to_owned(),
                    declared: manifest.api_version,
                    host: HOST_API_VERSION,
                });
            }
            ApprovalRecord {
                decision: ApprovalDecision::Approved,
                version: manifest.version,
                permissions: manifest.permissions.clone(),
            }
        };

        self.approvals.insert(slug.to_owned(), record);
        if let Some(plugin) = self.plugins.get_mut(slug) {
            plugin.state = PluginState::Approved;
        }
        Ok(())
    }

    /// Disables a plugin without uninstalling it.
    ///
    /// The decision survives updates: it is what distinguishes "I do not want
    /// this plugin" from "I have not looked yet".
    ///
    /// # Errors
    /// [`PluginError::Unknown`] if the plugin does not exist.
    pub fn disable(&mut self, slug: &str) -> Result<()> {
        let record = {
            let plugin = self.require(slug)?;
            let (version, permissions) = plugin.manifest().map_or_else(
                || (PluginVersion::new(0, 0, 0), PluginPermissions::default()),
                |manifest| (manifest.version, manifest.permissions.clone()),
            );
            ApprovalRecord {
                decision: ApprovalDecision::Disabled,
                version,
                permissions,
            }
        };

        self.approvals.insert(slug.to_owned(), record);
        if let Some(plugin) = self.plugins.get_mut(slug) {
            plugin.state = PluginState::Disabled;
        }
        Ok(())
    }

    /// Forgets any decision taken on this plugin: it starts over from
    /// [`Installed`](PluginState::Installed).
    ///
    /// # Errors
    /// [`PluginError::Unknown`] if the plugin does not exist.
    pub fn forget(&mut self, slug: &str) -> Result<()> {
        self.require(slug)?;
        self.approvals.remove(slug);
        // A failed plugin stays failed: forgetting a decision does not repair a
        // manifest.
        if let Some(plugin) = self.plugins.get_mut(slug)
            && plugin.manifest.is_some()
        {
            plugin.state = PluginState::Installed;
        }
        Ok(())
    }

    /// The persisted approval of a plugin, as it was recorded.
    #[must_use]
    pub fn approval(&self, slug: &str) -> Option<&ApprovalRecord> {
        self.approvals.get(slug)
    }

    /// Does the persisted approval still cover what the manifest asks for
    /// **today**?
    ///
    /// It is what the interface calls to word its reapproval request: the
    /// error names the added grants, one by one. Saying "the permissions
    /// changed" helps nobody decide.
    ///
    /// # Errors
    /// [`PluginError::Unknown`], [`PluginError::InvalidManifest`],
    /// [`PluginError::NotApproved`] if nothing was ever approved, or
    /// [`PluginError::ApprovalStale`] listing what was added.
    pub fn check_approval(&self, slug: &str) -> Result<()> {
        let plugin = self.require(slug)?;
        let manifest = plugin.manifest().ok_or_else(|| {
            PluginError::invalid_manifest(slug, "no readable manifest to compare")
        })?;
        let Some(record) = self.approvals.get(slug) else {
            return Err(PluginError::NotApproved {
                plugin: slug.to_owned(),
            });
        };

        let additions = manifest.permissions.additions_over(&record.permissions);
        if additions.is_empty() {
            return Ok(());
        }
        Err(PluginError::ApprovalStale {
            plugin: slug.to_owned(),
            detail: format!("this version also asks for {}", additions.join(", ")),
        })
    }

    /// Writes the approvals file.
    ///
    /// The write goes through a temporary file then a rename: an interrupted
    /// write would leave a truncated `approvals.toml`, and every plugin would
    /// fall back to `Installed` at the next start.
    ///
    /// **Synchronous and blocking.** Do not call from the interface thread
    /// (I-05).
    ///
    /// # Errors
    /// [`PluginError::Approvals`] if the directory cannot be created, if
    /// rendering fails, or if the write fails.
    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(|err| PluginError::Approvals {
            detail: format!("directory `{}`: {err}", self.root.display()),
        })?;

        let approvals_file = ApprovalsFile {
            approvals: self.approvals.clone(),
        };
        let text = toml::to_string(&approvals_file).map_err(|err| PluginError::Approvals {
            detail: err.to_string(),
        })?;

        let temporary = self.root.join(APPROVALS_TMP);
        fs::write(&temporary, text).map_err(|err| PluginError::Approvals {
            detail: format!("writing `{}`: {err}", temporary.display()),
        })?;
        fs::rename(&temporary, self.approvals_path()).map_err(|err| PluginError::Approvals {
            detail: format!("renaming `{}`: {err}", temporary.display()),
        })
    }
}

/// Reads the approvals file. Its absence is the normal state of a fresh
/// install; its unreadability is not.
fn read_approvals(path: &Path) -> Result<BTreeMap<String, ApprovalRecord>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => {
            return Err(PluginError::Approvals {
                detail: format!("reading `{}`: {err}", path.display()),
            });
        }
    };
    let approvals_file: ApprovalsFile =
        toml::from_str(&text).map_err(|err| PluginError::Approvals {
            detail: format!("`{}`: {err}", path.display()),
        })?;
    Ok(approvals_file.approvals)
}

#[cfg(test)]
mod tests;
