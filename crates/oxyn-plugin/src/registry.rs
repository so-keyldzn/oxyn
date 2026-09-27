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

        let mut trouves = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|source| PluginError::Directory {
                path: self.root.clone(),
                source,
            })?;
            let directory = entry.path();
            if !directory.is_dir() {
                continue;
            }
            let Some(slug) = directory.file_name().and_then(|nom| nom.to_str()) else {
                tracing::warn!("plugin directory with a non-UTF-8 name: ignored");
                continue;
            };
            let slug = slug.to_owned();
            let manifest_path = directory.join(MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            trouves.insert(slug.clone(), self.load_one(slug, directory, &manifest_path));
        }

        self.plugins = trouves;
        Ok(())
    }

    /// Reads and classifies a plugin. Never returns an error: a faulty plugin
    /// becomes [`Failed`](PluginState::Failed) and leaves the others alone.
    fn load_one(&self, slug: String, directory: PathBuf, manifest_path: &Path) -> InstalledPlugin {
        let echec = |slug: String, directory: PathBuf, reason: String| {
            tracing::warn!(plugin = %slug, motif = %reason, "plugin refused");
            InstalledPlugin {
                slug,
                directory,
                manifest: None,
                state: PluginState::Failed { reason },
            }
        };

        let texte = match fs::read_to_string(manifest_path) {
            Ok(texte) => texte,
            Err(err) => {
                return echec(
                    slug,
                    directory,
                    format!("`{MANIFEST_FILE}` unreadable: {err}"),
                );
            }
        };
        let manifest = match PluginManifest::from_toml(&texte) {
            Ok(manifest) => manifest,
            Err(err) => return echec(slug, directory, err.to_string()),
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
            return echec(slug, directory, reason);
        }

        // A file permission that encompassed the plugin directory would give
        // the plugin control over `approvals.toml`, hence over its own approval
        // and over the others'.
        for racine in &manifest.permissions.filesystem {
            if self.root.starts_with(racine) {
                let reason = format!(
                    "the file root `{}` contains the plugin directory: \
                     the plugin could rewrite the approvals",
                    racine.display()
                );
                return echec(slug, directory, reason);
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

        let ajouts = manifest.permissions.additions_over(&record.permissions);
        if ajouts.is_empty() {
            return Ok(());
        }
        Err(PluginError::ApprovalStale {
            plugin: slug.to_owned(),
            detail: format!("this version also asks for {}", ajouts.join(", ")),
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

        let fichier = ApprovalsFile {
            approvals: self.approvals.clone(),
        };
        let texte = toml::to_string(&fichier).map_err(|err| PluginError::Approvals {
            detail: err.to_string(),
        })?;

        let temporaire = self.root.join(APPROVALS_TMP);
        fs::write(&temporaire, texte).map_err(|err| PluginError::Approvals {
            detail: format!("writing `{}`: {err}", temporaire.display()),
        })?;
        fs::rename(&temporaire, self.approvals_path()).map_err(|err| PluginError::Approvals {
            detail: format!("renaming `{}`: {err}", temporaire.display()),
        })
    }
}

/// Reads the approvals file. Its absence is the normal state of a fresh
/// install; its unreadability is not.
fn read_approvals(path: &Path) -> Result<BTreeMap<String, ApprovalRecord>> {
    let texte = match fs::read_to_string(path) {
        Ok(texte) => texte,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => {
            return Err(PluginError::Approvals {
                detail: format!("reading `{}`: {err}", path.display()),
            });
        }
    };
    let fichier: ApprovalsFile = toml::from_str(&texte).map_err(|err| PluginError::Approvals {
        detail: format!("`{}`: {err}", path.display()),
    })?;
    Ok(fichier.approvals)
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{TempDir, agent_toml, driver_toml, export_toml};
    use crate::manifest::{ConnectionAccess, HostPort};

    use super::*;

    #[test]
    fn a_missing_plugin_directory_is_not_a_failure() {
        // It is the state of a fresh install: Oxyn must start.
        let mut registre = PluginRegistry::new(std::env::temp_dir().join("oxyn-plugin-inexistant"));
        registre
            .discover()
            .expect("a missing directory is legitimate");
        assert!(registre.is_empty());
        assert!(registre.declarative_agents().is_empty());
    }

    #[test]
    fn dropping_a_directory_authorizes_nothing() {
        // ADR-0005: installed is not authorized.
        let racine = TempDir::new("depot");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        let plugin = registre.require("revue").expect("discovered plugin");
        assert_eq!(*plugin.state(), PluginState::Installed);
        assert!(!plugin.is_approved());
        assert!(
            plugin.effective_permissions().is_err(),
            "an unapproved plugin has no effective permission"
        );
        assert!(
            registre.declarative_agents().is_empty(),
            "an unapproved agent does not enter the AI workspace"
        );
    }

    #[test]
    fn the_approval_survives_a_restart() {
        let racine = TempDir::new("persistance");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        registre.approve("revue").expect("approval");
        registre.save().expect("writing the approvals");

        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("discovery");
        let plugin = relu.require("revue").expect("discovered plugin");
        assert!(plugin.is_approved(), "{:?}", plugin.state());
        assert_eq!(
            plugin
                .effective_permissions()
                .expect("effective permissions")
                .connections,
            ConnectionAccess::ReadOnly
        );

        let agents = relu.declarative_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].plugin().as_str(), "revue");
        assert_eq!(agents[0].spec().name, "Schema");
    }

    #[test]
    fn the_approvals_file_is_readable_without_oxyn() {
        // I-11: what Oxyn writes can be reread with a text editor.
        let racine = TempDir::new("lisible");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        registre.approve("csv").expect("approval");
        registre.save().expect("write");

        let texte = fs::read_to_string(registre.approvals_path()).expect("the file was written");
        assert!(texte.contains("csv"), "{texte}");
        assert!(texte.contains("approved"), "{texte}");
        assert!(
            texte.contains("a.example:443"),
            "the user must be able to reread what they granted: {texte}"
        );
    }

    #[test]
    fn an_update_that_widens_voids_the_approval() {
        let racine = TempDir::new("elargissement");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        registre.approve("csv").expect("approval");
        registre.save().expect("write");

        // The plugin updates and asks for one more host.
        racine.plugin(
            "csv",
            &export_toml("csv", "\"a.example:443\", \"exfiltration.example:443\""),
        );

        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("discovery");
        let plugin = relu.require("csv").expect("discovered plugin");
        assert_eq!(
            *plugin.state(),
            PluginState::Installed,
            "an added permission must ask for approval again"
        );
        assert!(plugin.effective_permissions().is_err());
    }

    #[test]
    fn an_update_that_asks_nothing_more_stays_approved() {
        // Asking again at every patch teaches clicking without reading.
        let racine = TempDir::new("correctif");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        registre.approve("csv").expect("approval");
        registre.save().expect("write");

        let mise_a_jour = export_toml("csv", "\"a.example:443\"")
            .replace("version     = \"1.0.0\"", "version     = \"1.4.2\"");
        racine.plugin("csv", &mise_a_jour);

        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("discovery");
        assert!(relu.require("csv").expect("plugin").is_approved());

        // And an update that asks for **less** stays covered too.
        racine.plugin("csv", &export_toml("csv", ""));
        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("discovery");
        assert!(relu.require("csv").expect("plugin").is_approved());
    }

    #[test]
    fn a_deactivation_does_not_lift_by_itself() {
        let racine = TempDir::new("desactivation");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        registre.disable("csv").expect("deactivation");
        registre.save().expect("write");

        racine.plugin("csv", &export_toml("csv", ""));
        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("discovery");
        assert_eq!(
            *relu.require("csv").expect("plugin").state(),
            PluginState::Disabled,
            "an update does not re-enable what the user set aside"
        );
    }

    #[test]
    fn forgetting_a_decision_brings_back_to_installed() {
        let racine = TempDir::new("oubli");
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        registre.approve("csv").expect("approval");
        assert!(registre.require("csv").expect("plugin").is_approved());

        registre.forget("csv").expect("forget");
        assert_eq!(
            *registre.require("csv").expect("plugin").state(),
            PluginState::Installed
        );
        assert!(registre.approval("csv").is_none());
    }

    #[test]
    fn a_reapproval_names_what_was_added() {
        // "The permissions changed" helps nobody decide.
        let racine = TempDir::new("reapprobation");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        // Nothing was ever approved.
        let err = registre
            .check_approval("csv")
            .expect_err("refusal expected");
        assert!(matches!(err, PluginError::NotApproved { .. }), "{err}");

        registre.approve("csv").expect("approval");
        registre.save().expect("write");
        registre.check_approval("csv").expect("nothing changed");

        racine.plugin(
            "csv",
            &export_toml("csv", "\"a.example:443\", \"exfiltration.example:443\"")
                .replace("network = [", "connections = \"read_write\"\nnetwork = ["),
        );
        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("discovery");

        let err = relu.check_approval("csv").expect_err("refusal expected");
        let message = err.to_string();
        assert!(err.needs_user_decision());
        assert!(message.contains("exfiltration.example:443"), "{message}");
        assert!(message.contains("read_write"), "{message}");
        assert!(
            !message.contains("a.example:443"),
            "what was already granted is not presented as new: {message}"
        );
    }

    #[test]
    fn a_usable_plugin_is_approved_and_supported_by_this_build() {
        let racine = TempDir::new("utilisable");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        // Not approved: unusable, whatever the build.
        assert!(
            registre
                .require("revue")
                .expect("plugin")
                .check_usable()
                .is_err()
        );

        registre.approve("revue").expect("approval");
        registre.approve("csv").expect("approval");

        // A declarative agent never needs the WebAssembly host.
        registre
            .require("revue")
            .expect("plugin")
            .check_usable()
            .expect("an approved agent is usable everywhere");

        // A plugin that runs code does depend on the build.
        let export = registre.require("csv").expect("plugin");
        #[cfg(feature = "wasm-host")]
        {
            export
                .check_usable()
                .expect("the WebAssembly host is compiled");
        }
        #[cfg(not(feature = "wasm-host"))]
        {
            let err = export.check_usable().expect_err("refusal expected");
            assert!(
                matches!(err, PluginError::WasmHostUnavailable { .. }),
                "{err}"
            );
        }
    }

    #[test]
    fn a_broken_plugin_breaks_no_other() {
        let racine = TempDir::new("casse");
        racine.plugin("bon", &export_toml("bon", ""));
        racine.plugin("casse", "ceci n'est pas du TOML = = =");
        racine.plugin(
            "vide",
            "id = \"vide\"\nname = \"\"\nversion = \"1.0.0\"\n\
             api_version = \"0.1.0\"\nkind = \"export\"\nentrypoint = \"v.wasm\"\n",
        );

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        assert_eq!(registre.len(), 3);
        assert_eq!(
            *registre.require("bon").expect("plugin").state(),
            PluginState::Installed
        );
        for casse in ["casse", "vide"] {
            let plugin = registre.require(casse).expect("discovered plugin");
            assert!(
                matches!(plugin.state(), PluginState::Failed { .. }),
                "{casse}: {:?}",
                plugin.state()
            );
            assert!(plugin.manifest().is_none());
            assert_eq!(
                plugin.display_name(),
                casse,
                "the fallback name is the slug"
            );
        }
    }

    #[test]
    fn a_manifest_lying_about_its_directory_is_refused() {
        // Otherwise the approval of one plugin would cover another directory.
        let racine = TempDir::new("mensonge");
        racine.plugin("innocent", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        let plugin = registre.require("innocent").expect("discovered plugin");
        match plugin.state() {
            PluginState::Failed { reason } => {
                assert!(reason.contains("innocent"), "{reason}");
                assert!(reason.contains("approval"), "{reason}");
            }
            autre => panic!("unexpected state: {autre:?}"),
        }
    }

    #[test]
    fn a_plugin_cannot_ask_for_the_plugin_directory() {
        // It would rewrite its own approval there, and the others'.
        let racine = TempDir::new("autoapprobation");
        let manifeste = format!(
            "id = \"glouton\"\nname = \"Glouton\"\nversion = \"1.0.0\"\n\
             api_version = \"0.1.0\"\nkind = \"export\"\nentrypoint = \"g.wasm\"\n\
             \n[permissions]\nfilesystem = [\"{}\"]\n",
            racine.path().display()
        );
        racine.plugin("glouton", &manifeste);

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        let plugin = registre.require("glouton").expect("discovered plugin");
        match plugin.state() {
            PluginState::Failed { reason } => {
                assert!(reason.contains("approvals"), "{reason}");
            }
            autre => panic!("unexpected state: {autre:?}"),
        }
    }

    #[test]
    fn what_can_never_run_is_not_approved() {
        let racine = TempDir::new("incompatible");
        let manifeste =
            export_toml("futur", "").replace("api_version = \"0.1.0\"", "api_version = \"9.9.9\"");
        racine.plugin("futur", &manifeste);

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        let err = registre.approve("futur").expect_err("refusal expected");
        assert!(
            matches!(err, PluginError::IncompatibleInterface { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_directory_without_manifest_is_not_a_plugin() {
        let racine = TempDir::new("intrus");
        fs::create_dir_all(racine.path().join("notes")).expect("directory");
        fs::write(racine.path().join("lisez-moi.txt"), "bonjour").expect("file");
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        assert_eq!(registre.len(), 1);
        assert!(registre.get("notes").is_none());
    }

    #[test]
    fn an_unreadable_approvals_file_is_reported_not_ignored() {
        // Silently losing it would make every plugin fall back to `Installed`
        // without anyone knowing why.
        let racine = TempDir::new("approbations-cassees");
        racine.plugin("csv", &export_toml("csv", ""));
        fs::write(racine.path().join(APPROVALS_FILE), "= = =").expect("write");

        let mut registre = PluginRegistry::new(racine.path());
        let err = registre.discover().expect_err("refusal expected");
        assert!(matches!(err, PluginError::Approvals { .. }), "{err}");
    }

    #[test]
    fn claimed_protocols_only_come_from_approved_plugins() {
        let racine = TempDir::new("drivers");
        racine.plugin("duckdb", &driver_toml("duckdb"));
        racine.file("duckdb", "duckdb.wasm", b"\0asm\x0d\x00\x01\x00");

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        assert!(registre.driver_specs().is_empty());

        registre.approve("duckdb").expect("approval");
        let specs = registre.driver_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id.as_str(), "duckdb");
    }

    #[test]
    fn the_order_is_alphabetical_hence_reproducible() {
        let racine = TempDir::new("ordre");
        for slug in ["zeta", "alpha", "mu"] {
            racine.plugin(slug, &export_toml(slug, ""));
        }
        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        let slugs: Vec<&str> = registre.iter().map(InstalledPlugin::slug).collect();
        assert_eq!(slugs, ["alpha", "mu", "zeta"]);
    }

    #[test]
    fn approving_an_unknown_plugin_gives_a_message() {
        let racine = TempDir::new("inconnu");
        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");
        let err = registre.approve("fantome").expect_err("refusal expected");
        assert!(matches!(err, PluginError::Unknown { .. }), "{err}");
    }

    #[test]
    fn the_permission_summary_is_the_one_shown_before_approving() {
        let racine = TempDir::new("resume");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("discovery");

        let plugin = registre.require("csv").expect("plugin");
        let manifeste = plugin.manifest().expect("manifest");
        let resume = manifeste.permissions.summary();
        assert_eq!(resume.len(), 1);
        assert!(resume[0].contains("a.example:443"), "{resume:?}");

        // And the grant is indeed about this host, not a subdomain.
        let accord = HostPort::new("a.example:443").expect("host");
        assert!(manifeste.permissions.network.contains(&accord));
        assert!(!manifeste.permissions.allows_host("evil.a.example", 443));
    }
}
