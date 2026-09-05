//! Découverte des plugins, et persistance de ce que l'utilisateur a approuvé.
//!
//! Un répertoire de plugins ressemble à ceci :
//!
//! ```text
//! <données>/plugins/
//! ├── approvals.toml          ← ce que l'utilisateur a approuvé
//! ├── revue-schema/
//! │   └── plugin.toml         ← un agent déclaratif
//! └── duckdb/
//!     ├── plugin.toml
//!     └── duckdb.wasm
//! ```
//!
//! # Les trois règles qui gouvernent ce module
//!
//! **Installé n'est pas autorisé.** Déposer un répertoire ne donne rien : le
//! plugin part à l'état [`Installed`](PluginState::Installed), ses permissions
//! sont présentées à l'utilisateur, et lui seul le fait passer à
//! [`Approved`](PluginState::Approved)
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)).
//!
//! **L'approbation porte sur des permissions, pas sur une version.** Une mise à
//! jour qui ne demande rien de plus reste couverte — redemander à chaque
//! correctif apprend à cliquer sans lire. Une mise à jour qui **élargit** rend
//! l'approbation caduque et le plugin retombe à `Installed`. C'est
//! [`PluginPermissions::is_subset_of`] qui tranche, et c'est testé.
//!
//! **Un plugin cassé n'en casse aucun autre.** Un manifeste illisible produit
//! un [`Failed`](PluginState::Failed) portant son motif ; les autres se
//! chargent. Une découverte tout-ou-rien ferait disparaître un workspace entier
//! pour une virgule mal placée dans un fichier tiers.
//!
//! # Ce que ce module n'est pas
//!
//! Il est **synchrone** et fait des entrées-sorties : aucune de ses méthodes ne
//! doit être appelée depuis le thread d'interface (I-05). C'est à l'appelant de
//! les porter sur le pool bloquant.
//!
//! Le fichier d'approbations est du TOML lisible sans Oxyn (I-11) : ce qui a
//! été accordé se relit avec un éditeur de texte, et se révoque en supprimant
//! une section.

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

/// Nom du fichier d'approbations, à la racine du répertoire de plugins.
pub const APPROVALS_FILE: &str = "approvals.toml";

/// Suffixe du fichier temporaire d'écriture des approbations.
const APPROVALS_TMP: &str = "approvals.toml.tmp";

// ─────────────────────────────────────────────────────────────────────────────
// État
// ─────────────────────────────────────────────────────────────────────────────

/// Où en est un plugin, du répertoire déposé au composant chargeable.
///
/// Énumération **fermée** : ces quatre états sont exhaustifs, et en ajouter un
/// doit forcer la relecture de chaque endroit qui décide s'il faut charger,
/// afficher ou avertir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Manifeste lu et valide, permissions **pas encore approuvées**.
    ///
    /// C'est aussi l'état d'un plugin dont l'approbation est devenue caduque
    /// parce que sa mise à jour demande davantage.
    Installed,
    /// L'utilisateur a approuvé les permissions que ce manifeste demande.
    Approved,
    /// L'utilisateur a désactivé ce plugin. Une mise à jour ne le réactive pas.
    Disabled,
    /// Le manifeste est absent, illisible ou refusé.
    Failed {
        /// Ce qui a été refusé, montrable à l'utilisateur.
        reason: String,
    },
}

impl PluginState {
    /// Nom stable, pour un journal ou une interface.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Approved => "approved",
            Self::Disabled => "disabled",
            Self::Failed { .. } => "failed",
        }
    }

    /// Le plugin est-il chargeable ?
    ///
    /// Seul [`Approved`](Self::Approved) répond `true`. `Installed` répond
    /// `false` : c'est la différence entre déposer un fichier et accorder un
    /// droit.
    #[must_use]
    pub const fn is_approved(&self) -> bool {
        matches!(self, Self::Approved)
    }
}

/// Ce que l'utilisateur a décidé d'un plugin, tel qu'il est persisté.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Le plugin peut être chargé.
    Approved,
    /// Le plugin est désactivé, sans être désinstallé.
    Disabled,
}

/// Une décision persistée, avec **ce sur quoi elle portait**.
///
/// Enregistrer les permissions approuvées plutôt qu'une empreinte est
/// délibéré : le fichier reste lisible sans Oxyn (I-11), l'utilisateur peut
/// vérifier ce qu'il a accordé, et la comparaison avec un nouveau manifeste dit
/// *ce qui a été ajouté* plutôt que « ça a changé ».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    /// La décision.
    pub decision: ApprovalDecision,
    /// La version du plugin au moment de la décision. Informatif : ce n'est pas
    /// elle qui rend l'approbation caduque.
    pub version: PluginVersion,
    /// Les permissions telles qu'elles ont été présentées et acceptées.
    #[serde(default)]
    pub permissions: PluginPermissions,
}

/// Forme du fichier `approvals.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ApprovalsFile {
    #[serde(default)]
    approvals: BTreeMap<String, ApprovalRecord>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Un plugin installé
// ─────────────────────────────────────────────────────────────────────────────

/// Un répertoire de plugin, tel que la découverte l'a trouvé.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    slug: String,
    directory: PathBuf,
    manifest: Option<PluginManifest>,
    state: PluginState,
}

impl InstalledPlugin {
    /// Le nom du répertoire, qui est aussi l'identifiant du plugin quand le
    /// manifeste a pu être lu.
    ///
    /// C'est la seule chose qu'on connaisse toujours, y compris d'un plugin en
    /// échec — et c'est donc la clé du registre.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// Le répertoire du plugin.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Le manifeste, quand il a pu être lu et validé.
    #[must_use]
    pub const fn manifest(&self) -> Option<&PluginManifest> {
        self.manifest.as_ref()
    }

    /// L'état du plugin.
    #[must_use]
    pub const fn state(&self) -> &PluginState {
        &self.state
    }

    /// Le plugin est-il approuvé ?
    #[must_use]
    pub const fn is_approved(&self) -> bool {
        self.state.is_approved()
    }

    /// Nom montrable : celui du manifeste, ou le nom du répertoire à défaut.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.manifest
            .as_ref()
            .map_or(self.slug.as_str(), |m| m.name.as_str())
    }

    /// La surface occupée, quand le manifeste est lisible.
    #[must_use]
    pub fn kind(&self) -> Option<PluginKind> {
        self.manifest.as_ref().map(|m| m.kind)
    }

    /// Les permissions **effectives**.
    ///
    /// Ce sont celles du manifeste, et non celles qui ont été approuvées : la
    /// découverte garantit que les premières sont incluses dans les secondes,
    /// faute de quoi le plugin ne serait pas [`Approved`](PluginState::Approved).
    /// L'intersection est donc le manifeste lui-même.
    ///
    /// # Erreurs
    /// [`PluginError::NotApproved`] si le plugin n'est pas approuvé ;
    /// [`PluginError::InvalidManifest`] s'il n'a pas de manifeste lisible.
    pub fn effective_permissions(&self) -> Result<&PluginPermissions> {
        let manifest = self.manifest.as_ref().ok_or_else(|| {
            PluginError::invalid_manifest(self.slug.as_str(), "aucun manifeste lisible")
        })?;
        if !self.state.is_approved() {
            return Err(PluginError::NotApproved {
                plugin: self.slug.clone(),
            });
        }
        Ok(&manifest.permissions)
    }

    /// Ce plugin peut-il être mis en service par **cette compilation** d'Oxyn ?
    ///
    /// Un agent déclaratif le peut dès qu'il est approuvé. Un plugin qui
    /// exécute du code exige l'hôte WebAssembly : sans la feature `wasm-host`,
    /// il est installable et approuvable mais **inutilisable**, et il vaut
    /// mieux le dire que l'afficher comme actif — un plugin qui ne fait rien
    /// sans expliquer pourquoi se lit comme une panne d'Oxyn.
    ///
    /// # Erreurs
    /// [`PluginError::InvalidManifest`] si le manifeste est illisible,
    /// [`PluginError::NotApproved`] s'il n'est pas approuvé,
    /// [`PluginError::WasmHostUnavailable`] s'il exige un hôte que cette
    /// compilation ne fournit pas.
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
// Le registre
// ─────────────────────────────────────────────────────────────────────────────

/// Les plugins découverts, et leurs approbations.
///
/// Un [`BTreeMap`] et non une table de hachage : l'ordre est alphabétique, donc
/// reproductible d'une machine à l'autre. Une liste de plugins qui se réordonne
/// entre deux ouvertures se lit comme un changement.
#[derive(Debug, Clone)]
pub struct PluginRegistry {
    root: PathBuf,
    plugins: BTreeMap<String, InstalledPlugin>,
    approvals: BTreeMap<String, ApprovalRecord>,
}

impl PluginRegistry {
    /// Un registre vide, adossé à un répertoire de plugins.
    ///
    /// Le chemin doit être **absolu** : les contrôles de confinement des
    /// permissions de fichiers sont lexicaux, et un chemin relatif les rendrait
    /// dépendants du répertoire courant du processus. Rien ne l'impose ici —
    /// c'est l'appelant qui résout le répertoire de données du système.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            plugins: BTreeMap::new(),
            approvals: BTreeMap::new(),
        }
    }

    /// Le répertoire de plugins.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Chemin du fichier d'approbations.
    #[must_use]
    pub fn approvals_path(&self) -> PathBuf {
        self.root.join(APPROVALS_FILE)
    }

    /// Relit le répertoire de plugins et le fichier d'approbations.
    ///
    /// Un répertoire absent n'est **pas** une erreur : c'est l'état d'une
    /// installation neuve, et Oxyn doit démarrer sans lui. Un sous-répertoire
    /// sans `plugin.toml` est ignoré : ce n'est pas un plugin cassé, ce n'est
    /// pas un plugin.
    ///
    /// **Synchrone et bloquant.** Ne pas appeler depuis le thread d'interface
    /// (I-05).
    ///
    /// # Erreurs
    /// [`PluginError::Approvals`] si le fichier d'approbations existe mais est
    /// illisible — perdre les approbations en silence ferait retomber tous les
    /// plugins à `Installed` sans que personne ne sache pourquoi ;
    /// [`PluginError::Directory`] si le répertoire existe mais n'est pas
    /// parcourable.
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
                tracing::warn!("répertoire de plugin au nom non UTF-8 : ignoré");
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

    /// Lit et classe un plugin. Ne renvoie jamais d'erreur : un plugin fautif
    /// devient [`Failed`](PluginState::Failed) et laisse les autres tranquilles.
    fn load_one(&self, slug: String, directory: PathBuf, manifest_path: &Path) -> InstalledPlugin {
        let echec = |slug: String, directory: PathBuf, reason: String| {
            tracing::warn!(plugin = %slug, motif = %reason, "plugin refusé");
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
                    format!("`{MANIFEST_FILE}` illisible : {err}"),
                );
            }
        };
        let manifest = match PluginManifest::from_toml(&texte) {
            Ok(manifest) => manifest,
            Err(err) => return echec(slug, directory, err.to_string()),
        };

        // Le répertoire porte l'identifiant. Sans cette règle, deux répertoires
        // pourraient revendiquer le même identifiant, et l'approbation de l'un
        // couvrirait l'autre.
        if manifest.id.as_str() != slug {
            let reason = format!(
                "le manifeste se déclare `{}` mais son répertoire s'appelle `{slug}` : \
                 son approbation couvrirait un autre plugin",
                manifest.id
            );
            return echec(slug, directory, reason);
        }

        // Une permission de fichiers qui engloberait le répertoire de plugins
        // donnerait au plugin la main sur `approvals.toml`, donc sur sa propre
        // approbation et sur celle des autres.
        for racine in &manifest.permissions.filesystem {
            if self.root.starts_with(racine) {
                let reason = format!(
                    "la racine de fichiers `{}` contient le répertoire de plugins : \
                     le plugin pourrait réécrire les approbations",
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

    /// Croise le manifeste avec l'approbation persistée.
    fn state_for(&self, manifest: &PluginManifest) -> PluginState {
        let Some(record) = self.approvals.get(manifest.id.as_str()) else {
            return PluginState::Installed;
        };
        // Une désactivation est une décision, pas une conséquence : une mise à
        // jour ne la lève pas.
        if record.decision == ApprovalDecision::Disabled {
            return PluginState::Disabled;
        }
        if manifest.permissions.is_subset_of(&record.permissions) {
            PluginState::Approved
        } else {
            tracing::warn!(
                plugin = %manifest.id,
                "approbation caduque : la mise à jour demande davantage"
            );
            PluginState::Installed
        }
    }

    /// Le plugin portant ce nom de répertoire.
    #[must_use]
    pub fn get(&self, slug: &str) -> Option<&InstalledPlugin> {
        self.plugins.get(slug)
    }

    /// Le plugin portant ce nom, ou une erreur montrable.
    ///
    /// # Erreurs
    /// [`PluginError::Unknown`] si aucun plugin ne porte ce nom.
    pub fn require(&self, slug: &str) -> Result<&InstalledPlugin> {
        self.get(slug).ok_or_else(|| PluginError::Unknown {
            plugin: slug.to_owned(),
        })
    }

    /// Les plugins, dans l'ordre alphabétique de leur répertoire.
    pub fn iter(&self) -> impl Iterator<Item = &InstalledPlugin> {
        self.plugins.values()
    }

    /// Les seuls plugins approuvés.
    pub fn approved(&self) -> impl Iterator<Item = &InstalledPlugin> {
        self.plugins.values().filter(|p| p.is_approved())
    }

    /// Nombre de plugins découverts, états d'échec compris.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Aucun plugin n'a été découvert.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Les agents déclaratifs fournis par les plugins **approuvés**.
    ///
    /// Un plugin non approuvé n'apparaît pas : il ne suffit pas de déposer un
    /// fichier pour ajouter un agent au workspace IA.
    ///
    /// Alloue à chaque appel ; c'est un chemin d'ouverture de fenêtre.
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

    /// Les protocoles revendiqués par les plugins de type driver **approuvés**.
    ///
    /// Sert à `oxyn-app` pour refuser un plugin qui revendiquerait un
    /// identifiant déjà tenu par un driver natif, avant d'exécuter la moindre
    /// instruction du composant.
    #[must_use]
    pub fn driver_specs(&self) -> Vec<&PluginDriverSpec> {
        self.approved()
            .filter_map(|plugin| plugin.manifest()?.driver.as_ref())
            .collect()
    }

    /// Enregistre l'approbation de l'utilisateur pour les permissions que ce
    /// manifeste demande **aujourd'hui**.
    ///
    /// N'écrit rien sur disque : appeler [`save`](Self::save) ensuite.
    ///
    /// # Erreurs
    /// [`PluginError::Unknown`] si le plugin n'existe pas ;
    /// [`PluginError::InvalidManifest`] s'il n'a pas de manifeste lisible ;
    /// [`PluginError::IncompatibleInterface`] s'il vise une version d'interface
    /// que cet hôte ne fournit pas — faire approuver ce qui ne pourra jamais
    /// tourner apprend à cliquer sans lire.
    pub fn approve(&mut self, slug: &str) -> Result<()> {
        let record = {
            let plugin = self.require(slug)?;
            let manifest = plugin.manifest().ok_or_else(|| {
                PluginError::invalid_manifest(slug, "aucun manifeste lisible à approuver")
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

    /// Désactive un plugin sans le désinstaller.
    ///
    /// La décision survit aux mises à jour : c'est ce qui distingue « je ne veux
    /// pas de ce plugin » de « je n'ai pas encore regardé ».
    ///
    /// # Erreurs
    /// [`PluginError::Unknown`] si le plugin n'existe pas.
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

    /// Oublie toute décision prise sur ce plugin : il repart de
    /// [`Installed`](PluginState::Installed).
    ///
    /// # Erreurs
    /// [`PluginError::Unknown`] si le plugin n'existe pas.
    pub fn forget(&mut self, slug: &str) -> Result<()> {
        self.require(slug)?;
        self.approvals.remove(slug);
        // Un plugin en échec le reste : oublier une décision ne répare pas un
        // manifeste.
        if let Some(plugin) = self.plugins.get_mut(slug)
            && plugin.manifest.is_some()
        {
            plugin.state = PluginState::Installed;
        }
        Ok(())
    }

    /// L'approbation persistée d'un plugin, telle qu'elle a été enregistrée.
    #[must_use]
    pub fn approval(&self, slug: &str) -> Option<&ApprovalRecord> {
        self.approvals.get(slug)
    }

    /// L'approbation persistée couvre-t-elle encore ce que le manifeste demande
    /// **aujourd'hui** ?
    ///
    /// C'est ce que l'interface appelle pour rédiger sa demande de
    /// réapprobation : l'erreur nomme les accords ajoutés, un par un. Dire
    /// « les permissions ont changé » n'aide personne à décider.
    ///
    /// # Erreurs
    /// [`PluginError::Unknown`], [`PluginError::InvalidManifest`],
    /// [`PluginError::NotApproved`] si rien n'a jamais été approuvé, ou
    /// [`PluginError::ApprovalStale`] en énumérant ce qui a été ajouté.
    pub fn check_approval(&self, slug: &str) -> Result<()> {
        let plugin = self.require(slug)?;
        let manifest = plugin.manifest().ok_or_else(|| {
            PluginError::invalid_manifest(slug, "aucun manifeste lisible à comparer")
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
            detail: format!("cette version demande aussi {}", ajouts.join(", ")),
        })
    }

    /// Écrit le fichier d'approbations.
    ///
    /// L'écriture passe par un fichier temporaire puis un renommage : une
    /// écriture interrompue laisserait un `approvals.toml` tronqué, et tous les
    /// plugins retomberaient à `Installed` au prochain démarrage.
    ///
    /// **Synchrone et bloquant.** Ne pas appeler depuis le thread d'interface
    /// (I-05).
    ///
    /// # Erreurs
    /// [`PluginError::Approvals`] si le répertoire ne peut pas être créé, si le
    /// rendu échoue, ou si l'écriture échoue.
    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(|err| PluginError::Approvals {
            detail: format!("répertoire `{}` : {err}", self.root.display()),
        })?;

        let fichier = ApprovalsFile {
            approvals: self.approvals.clone(),
        };
        let texte = toml::to_string(&fichier).map_err(|err| PluginError::Approvals {
            detail: err.to_string(),
        })?;

        let temporaire = self.root.join(APPROVALS_TMP);
        fs::write(&temporaire, texte).map_err(|err| PluginError::Approvals {
            detail: format!("écriture de `{}` : {err}", temporaire.display()),
        })?;
        fs::rename(&temporaire, self.approvals_path()).map_err(|err| PluginError::Approvals {
            detail: format!("renommage de `{}` : {err}", temporaire.display()),
        })
    }
}

/// Lit le fichier d'approbations. Son absence est l'état normal d'une
/// installation neuve ; son illisibilité ne l'est pas.
fn read_approvals(path: &Path) -> Result<BTreeMap<String, ApprovalRecord>> {
    let texte = match fs::read_to_string(path) {
        Ok(texte) => texte,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => {
            return Err(PluginError::Approvals {
                detail: format!("lecture de `{}` : {err}", path.display()),
            });
        }
    };
    let fichier: ApprovalsFile = toml::from_str(&texte).map_err(|err| PluginError::Approvals {
        detail: format!("`{}` : {err}", path.display()),
    })?;
    Ok(fichier.approvals)
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{TempDir, agent_toml, driver_toml, export_toml};
    use crate::manifest::{ConnectionAccess, HostPort};

    use super::*;

    #[test]
    fn un_repertoire_de_plugins_absent_n_est_pas_une_panne() {
        // C'est l'état d'une installation neuve : Oxyn doit démarrer.
        let mut registre = PluginRegistry::new(std::env::temp_dir().join("oxyn-plugin-inexistant"));
        registre
            .discover()
            .expect("un répertoire absent est licite");
        assert!(registre.is_empty());
        assert!(registre.declarative_agents().is_empty());
    }

    #[test]
    fn deposer_un_repertoire_n_autorise_rien() {
        // ADR-0005 : installé n'est pas autorisé.
        let racine = TempDir::new("depot");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        let plugin = registre.require("revue").expect("plugin découvert");
        assert_eq!(*plugin.state(), PluginState::Installed);
        assert!(!plugin.is_approved());
        assert!(
            plugin.effective_permissions().is_err(),
            "un plugin non approuvé n'a aucune permission effective"
        );
        assert!(
            registre.declarative_agents().is_empty(),
            "un agent non approuvé n'entre pas dans le workspace IA"
        );
    }

    #[test]
    fn l_approbation_survit_a_un_redemarrage() {
        let racine = TempDir::new("persistance");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre.approve("revue").expect("approbation");
        registre.save().expect("écriture des approbations");

        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("découverte");
        let plugin = relu.require("revue").expect("plugin découvert");
        assert!(plugin.is_approved(), "{:?}", plugin.state());
        assert_eq!(
            plugin
                .effective_permissions()
                .expect("permissions effectives")
                .connections,
            ConnectionAccess::ReadOnly
        );

        let agents = relu.declarative_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].plugin().as_str(), "revue");
        assert_eq!(agents[0].spec().name, "Schema");
    }

    #[test]
    fn le_fichier_d_approbations_est_relisible_sans_oxyn() {
        // I-11 : ce qu'Oxyn écrit se relit avec un éditeur de texte.
        let racine = TempDir::new("lisible");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre.approve("csv").expect("approbation");
        registre.save().expect("écriture");

        let texte = fs::read_to_string(registre.approvals_path()).expect("le fichier a été écrit");
        assert!(texte.contains("csv"), "{texte}");
        assert!(texte.contains("approved"), "{texte}");
        assert!(
            texte.contains("a.example:443"),
            "l'utilisateur doit pouvoir relire ce qu'il a accordé : {texte}"
        );
    }

    #[test]
    fn une_mise_a_jour_qui_elargit_rend_l_approbation_caduque() {
        let racine = TempDir::new("elargissement");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre.approve("csv").expect("approbation");
        registre.save().expect("écriture");

        // Le plugin se met à jour et réclame un hôte de plus.
        racine.plugin(
            "csv",
            &export_toml("csv", "\"a.example:443\", \"exfiltration.example:443\""),
        );

        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("découverte");
        let plugin = relu.require("csv").expect("plugin découvert");
        assert_eq!(
            *plugin.state(),
            PluginState::Installed,
            "une permission ajoutée doit redemander l'approbation"
        );
        assert!(plugin.effective_permissions().is_err());
    }

    #[test]
    fn une_mise_a_jour_qui_ne_demande_rien_de_plus_reste_approuvee() {
        // Redemander à chaque correctif apprend à cliquer sans lire.
        let racine = TempDir::new("correctif");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre.approve("csv").expect("approbation");
        registre.save().expect("écriture");

        let mise_a_jour = export_toml("csv", "\"a.example:443\"")
            .replace("version     = \"1.0.0\"", "version     = \"1.4.2\"");
        racine.plugin("csv", &mise_a_jour);

        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("découverte");
        assert!(relu.require("csv").expect("plugin").is_approved());

        // Et une mise à jour qui demande **moins** reste couverte elle aussi.
        racine.plugin("csv", &export_toml("csv", ""));
        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("découverte");
        assert!(relu.require("csv").expect("plugin").is_approved());
    }

    #[test]
    fn une_desactivation_ne_se_leve_pas_toute_seule() {
        let racine = TempDir::new("desactivation");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre.disable("csv").expect("désactivation");
        registre.save().expect("écriture");

        racine.plugin("csv", &export_toml("csv", ""));
        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("découverte");
        assert_eq!(
            *relu.require("csv").expect("plugin").state(),
            PluginState::Disabled,
            "une mise à jour ne réactive pas ce que l'utilisateur a écarté"
        );
    }

    #[test]
    fn oublier_une_decision_ramene_a_installed() {
        let racine = TempDir::new("oubli");
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        registre.approve("csv").expect("approbation");
        assert!(registre.require("csv").expect("plugin").is_approved());

        registre.forget("csv").expect("oubli");
        assert_eq!(
            *registre.require("csv").expect("plugin").state(),
            PluginState::Installed
        );
        assert!(registre.approval("csv").is_none());
    }

    #[test]
    fn une_reapprobation_nomme_ce_qui_a_ete_ajoute() {
        // « Les permissions ont changé » n'aide personne à décider.
        let racine = TempDir::new("reapprobation");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        // Rien n'a jamais été approuvé.
        let err = registre.check_approval("csv").expect_err("refus attendu");
        assert!(matches!(err, PluginError::NotApproved { .. }), "{err}");

        registre.approve("csv").expect("approbation");
        registre.save().expect("écriture");
        registre.check_approval("csv").expect("rien n'a changé");

        racine.plugin(
            "csv",
            &export_toml("csv", "\"a.example:443\", \"exfiltration.example:443\"")
                .replace("network = [", "connections = \"read_write\"\nnetwork = ["),
        );
        let mut relu = PluginRegistry::new(racine.path());
        relu.discover().expect("découverte");

        let err = relu.check_approval("csv").expect_err("refus attendu");
        let message = err.to_string();
        assert!(err.needs_user_decision());
        assert!(message.contains("exfiltration.example:443"), "{message}");
        assert!(message.contains("read_write"), "{message}");
        assert!(
            !message.contains("a.example:443"),
            "ce qui était déjà accordé n'est pas présenté comme nouveau : {message}"
        );
    }

    #[test]
    fn un_plugin_utilisable_est_approuve_et_supporte_par_cette_compilation() {
        let racine = TempDir::new("utilisable");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        // Non approuvé : inutilisable, quelle que soit la compilation.
        assert!(
            registre
                .require("revue")
                .expect("plugin")
                .check_usable()
                .is_err()
        );

        registre.approve("revue").expect("approbation");
        registre.approve("csv").expect("approbation");

        // Un agent déclaratif n'a jamais besoin de l'hôte WebAssembly.
        registre
            .require("revue")
            .expect("plugin")
            .check_usable()
            .expect("un agent approuvé est utilisable partout");

        // Un plugin qui exécute du code, lui, dépend de la compilation.
        let export = registre.require("csv").expect("plugin");
        #[cfg(feature = "wasm-host")]
        {
            export
                .check_usable()
                .expect("l'hôte WebAssembly est compilé");
        }
        #[cfg(not(feature = "wasm-host"))]
        {
            let err = export.check_usable().expect_err("refus attendu");
            assert!(
                matches!(err, PluginError::WasmHostUnavailable { .. }),
                "{err}"
            );
        }
    }

    #[test]
    fn un_plugin_casse_n_en_casse_aucun_autre() {
        let racine = TempDir::new("casse");
        racine.plugin("bon", &export_toml("bon", ""));
        racine.plugin("casse", "ceci n'est pas du TOML = = =");
        racine.plugin(
            "vide",
            "id = \"vide\"\nname = \"\"\nversion = \"1.0.0\"\n\
             api_version = \"0.1.0\"\nkind = \"export\"\nentrypoint = \"v.wasm\"\n",
        );

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        assert_eq!(registre.len(), 3);
        assert_eq!(
            *registre.require("bon").expect("plugin").state(),
            PluginState::Installed
        );
        for casse in ["casse", "vide"] {
            let plugin = registre.require(casse).expect("plugin découvert");
            assert!(
                matches!(plugin.state(), PluginState::Failed { .. }),
                "{casse} : {:?}",
                plugin.state()
            );
            assert!(plugin.manifest().is_none());
            assert_eq!(plugin.display_name(), casse, "le nom de repli est le slug");
        }
    }

    #[test]
    fn un_manifeste_qui_ment_sur_son_repertoire_est_refuse() {
        // Sinon l'approbation d'un plugin couvrirait un autre répertoire.
        let racine = TempDir::new("mensonge");
        racine.plugin("innocent", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        let plugin = registre.require("innocent").expect("plugin découvert");
        match plugin.state() {
            PluginState::Failed { reason } => {
                assert!(reason.contains("innocent"), "{reason}");
                assert!(reason.contains("approbation"), "{reason}");
            }
            autre => panic!("état inattendu : {autre:?}"),
        }
    }

    #[test]
    fn un_plugin_ne_peut_pas_demander_le_repertoire_des_plugins() {
        // Il y réécrirait sa propre approbation, et celle des autres.
        let racine = TempDir::new("autoapprobation");
        let manifeste = format!(
            "id = \"glouton\"\nname = \"Glouton\"\nversion = \"1.0.0\"\n\
             api_version = \"0.1.0\"\nkind = \"export\"\nentrypoint = \"g.wasm\"\n\
             \n[permissions]\nfilesystem = [\"{}\"]\n",
            racine.path().display()
        );
        racine.plugin("glouton", &manifeste);

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        let plugin = registre.require("glouton").expect("plugin découvert");
        match plugin.state() {
            PluginState::Failed { reason } => {
                assert!(reason.contains("approbations"), "{reason}");
            }
            autre => panic!("état inattendu : {autre:?}"),
        }
    }

    #[test]
    fn on_n_approuve_pas_ce_qui_ne_pourra_jamais_tourner() {
        let racine = TempDir::new("incompatible");
        let manifeste =
            export_toml("futur", "").replace("api_version = \"0.1.0\"", "api_version = \"9.9.9\"");
        racine.plugin("futur", &manifeste);

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        let err = registre.approve("futur").expect_err("refus attendu");
        assert!(
            matches!(err, PluginError::IncompatibleInterface { .. }),
            "{err}"
        );
    }

    #[test]
    fn un_repertoire_sans_manifeste_n_est_pas_un_plugin() {
        let racine = TempDir::new("intrus");
        fs::create_dir_all(racine.path().join("notes")).expect("répertoire");
        fs::write(racine.path().join("lisez-moi.txt"), "bonjour").expect("fichier");
        racine.plugin("csv", &export_toml("csv", ""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        assert_eq!(registre.len(), 1);
        assert!(registre.get("notes").is_none());
    }

    #[test]
    fn un_fichier_d_approbations_illisible_est_signale_pas_ignore() {
        // Le perdre en silence ferait retomber tous les plugins à `Installed`
        // sans que personne ne sache pourquoi.
        let racine = TempDir::new("approbations-cassees");
        racine.plugin("csv", &export_toml("csv", ""));
        fs::write(racine.path().join(APPROVALS_FILE), "= = =").expect("écriture");

        let mut registre = PluginRegistry::new(racine.path());
        let err = registre.discover().expect_err("refus attendu");
        assert!(matches!(err, PluginError::Approvals { .. }), "{err}");
    }

    #[test]
    fn les_protocoles_revendiques_ne_sortent_que_des_plugins_approuves() {
        let racine = TempDir::new("drivers");
        racine.plugin("duckdb", &driver_toml("duckdb"));
        racine.file("duckdb", "duckdb.wasm", b"\0asm\x0d\x00\x01\x00");

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        assert!(registre.driver_specs().is_empty());

        registre.approve("duckdb").expect("approbation");
        let specs = registre.driver_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id.as_str(), "duckdb");
    }

    #[test]
    fn l_ordre_est_alphabetique_donc_reproductible() {
        let racine = TempDir::new("ordre");
        for slug in ["zeta", "alpha", "mu"] {
            racine.plugin(slug, &export_toml(slug, ""));
        }
        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        let slugs: Vec<&str> = registre.iter().map(InstalledPlugin::slug).collect();
        assert_eq!(slugs, ["alpha", "mu", "zeta"]);
    }

    #[test]
    fn approuver_un_plugin_inconnu_donne_un_message() {
        let racine = TempDir::new("inconnu");
        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");
        let err = registre.approve("fantome").expect_err("refus attendu");
        assert!(matches!(err, PluginError::Unknown { .. }), "{err}");
    }

    #[test]
    fn le_resume_des_permissions_est_celui_qu_on_montre_avant_d_approuver() {
        let racine = TempDir::new("resume");
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        let plugin = registre.require("csv").expect("plugin");
        let manifeste = plugin.manifest().expect("manifeste");
        let resume = manifeste.permissions.summary();
        assert_eq!(resume.len(), 1);
        assert!(resume[0].contains("a.example:443"), "{resume:?}");

        // Et l'accord porte bien sur cet hôte, pas sur un sous-domaine.
        let accord = HostPort::new("a.example:443").expect("hôte");
        assert!(manifeste.permissions.network.contains(&accord));
        assert!(!manifeste.permissions.allows_host("evil.a.example", 443));
    }
}
