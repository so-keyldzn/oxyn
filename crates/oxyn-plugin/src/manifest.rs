//! Le manifeste d'un plugin : `plugin.toml`.
//!
//! Un manifeste est écrit par un tiers. Il est donc lu comme une **donnée
//! hostile** ([`SECURITY` §surface d'entrée](../../../docs/SECURITY.md)) :
//! chaque champ est validé, et ce qui n'est pas compris est refusé plutôt
//! qu'ignoré.
//!
//! # La règle qui gouverne ce module
//!
//! **Par défaut, tout est refusé.** Un manifeste sans section `[permissions]`
//! n'accorde rien : ni réseau, ni fichier, ni accès aux connexions
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)). C'est
//! le défaut de [`PluginPermissions`] qui le porte, et c'est testé — parce qu'un
//! défaut permissif ne se voit ni à la compilation, ni à la relecture, mais
//! seulement le jour où un plugin exfiltre.
//!
//! Corollaire moins évident : **une permission déclarée n'autorise rien par
//! elle-même**. `connections = "read_write"` ne donne pas le droit d'écrire ;
//! il donne le droit de *demander*, et la demande traverse le `PolicyGate`
//! comme celle d'un humain ([ADR-0004](../../../docs/adr/0004-command-bus.md),
//! I-01). Le manifeste ne sait que **restreindre**.
//!
//! # Forme du fichier
//!
//! ```toml
//! id          = "duckdb"
//! name        = "DuckDB"
//! version     = "0.3.1"
//! api_version = "0.1.0"
//! kind        = "driver"
//! entrypoint  = "duckdb.wasm"
//!
//! [permissions]
//! network     = ["catalog.example.com:443"]
//! connections = "read_only"
//!
//! [driver]
//! id           = "duckdb"
//! display_name = "DuckDB"
//! family       = "analytical"
//! ```

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use oxyn_core::{DriverId, IdParseError};
use oxyn_driver::DriverFamily;
use serde::{Deserialize, Serialize};

use crate::agent_plugin::PluginAgentSpec;
use crate::error::{PluginError, Result};

/// Nom du manifeste, dans le répertoire d'un plugin.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// Version du contrat d'hôte que cette compilation d'Oxyn fournit.
///
/// Elle couvre la forme du manifeste **et** les interfaces WIT. Tant que le
/// majeur vaut `0`, chaque incrément du mineur est une rupture : c'est la
/// convention usuelle pour un contrat qui n'a pas encore d'implémentation
/// tierce, et [`PluginVersion::is_compatible_with`] l'applique.
pub const HOST_API_VERSION: PluginVersion = PluginVersion::new(0, 1, 0);

// ─────────────────────────────────────────────────────────────────────────────
// Version
// ─────────────────────────────────────────────────────────────────────────────

/// Un numéro de version `majeur.mineur.correctif`.
///
/// Volontairement plus étroit que SemVer complet : ni pré-version, ni
/// métadonnée de compilation. Un manifeste est un fichier de configuration, pas
/// un dépôt de paquets, et accepter `1.0.0-rc.1+build.7` obligerait à décider
/// de son ordre — décision qui n'apporte rien ici et qu'il faudrait ensuite
/// tenir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl PluginVersion {
    /// Construit une version.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Le majeur.
    #[must_use]
    pub const fn major(&self) -> u32 {
        self.major
    }

    /// Le mineur.
    #[must_use]
    pub const fn minor(&self) -> u32 {
        self.minor
    }

    /// Le correctif.
    #[must_use]
    pub const fn patch(&self) -> u32 {
        self.patch
    }

    /// Un plugin visant `self` peut-il tourner sur un hôte fournissant `host` ?
    ///
    /// Deux régimes, et la différence est délibérée :
    ///
    /// * **majeur `0`** — le contrat est instable : le mineur doit être
    ///   **identique**. Un plugin bâti sur `0.1` ne tourne pas sur `0.2` ;
    /// * **majeur ≥ 1** — le majeur doit être identique et le mineur du plugin
    ///   inférieur ou égal à celui de l'hôte. Un plugin bâti sur `1.2` tourne
    ///   sur `1.4` ; l'inverse est refusé, parce que l'hôte ne fournit pas les
    ///   interfaces que le plugin attend.
    ///
    /// Le correctif n'entre jamais en compte : par définition il n'ajoute ni ne
    /// retire d'interface.
    #[must_use]
    pub const fn is_compatible_with(&self, host: &Self) -> bool {
        if self.major != host.major {
            return false;
        }
        if self.major == 0 {
            return self.minor == host.minor;
        }
        self.minor <= host.minor
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for PluginVersion {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // Seuls les chiffres décimaux sont acceptés : `u32::from_str`
        // accepterait `+1`, ce qui ferait passer `+1.0.0` pour une version.
        fn nombre(part: &str) -> std::result::Result<u32, IdParseError> {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(IdParseError::new(
                    "PluginVersion",
                    "attendu : trois nombres décimaux séparés par des points",
                ));
            }
            part.parse::<u32>().map_err(|_| {
                IdParseError::new("PluginVersion", "un composant dépasse la capacité d'un u32")
            })
        }

        let mut parts = s.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(IdParseError::new(
                "PluginVersion",
                "attendu : `majeur.mineur.correctif`",
            ));
        };
        Ok(Self::new(nombre(major)?, nombre(minor)?, nombre(patch)?))
    }
}

impl TryFrom<String> for PluginVersion {
    type Error = IdParseError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<PluginVersion> for String {
    fn from(version: PluginVersion) -> Self {
        version.to_string()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Identifiant
// ─────────────────────────────────────────────────────────────────────────────

/// Identifiant stable d'un plugin.
///
/// Il nomme le **répertoire** du plugin et la clé de son approbation. Sa
/// normalisation n'est donc pas cosmétique : un identifiant contenant `/`,
/// `..` ou un octet de contrôle permettrait à un manifeste de désigner un
/// répertoire qui n'est pas le sien, ou d'écraser l'approbation d'un autre
/// plugin.
///
/// Même grammaire que [`DriverId`], en plus long : minuscules ASCII, chiffres,
/// `-` et `_`, première lettre alphabétique, 64 caractères au plus.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(Arc<str>);

impl PluginId {
    /// Longueur maximale, en octets.
    pub const MAX_LEN: usize = 64;

    /// Construit un identifiant après validation.
    ///
    /// # Erreurs
    /// [`IdParseError`] si la chaîne est vide, trop longue, ne commence pas par
    /// une lettre minuscule ASCII, ou contient un caractère hors `[a-z0-9_-]`.
    /// La valeur fautive n'est jamais reprise dans l'erreur.
    pub fn new(name: impl AsRef<str>) -> std::result::Result<Self, IdParseError> {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(IdParseError::new("PluginId", "la chaîne est vide"));
        }
        if name.len() > Self::MAX_LEN {
            return Err(IdParseError::new("PluginId", "plus de 64 caractères"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "PluginId",
                "doit commencer par une lettre minuscule ASCII",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "PluginId",
                "caractères autorisés : a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Vue empruntée de l'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PluginId({:?})", self.as_str())
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for PluginId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for PluginId {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for PluginId {
    type Error = IdParseError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.as_str().to_owned()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Point d'entrée
// ─────────────────────────────────────────────────────────────────────────────

/// Le composant WebAssembly d'un plugin, relatif à son répertoire.
///
/// **Panne concrète évitée :** un manifeste déclarant
/// `entrypoint = "../../../.ssh/id_rsa"` ou `/usr/lib/libc.so`. Le chemin est
/// donc contraint à des composants ordinaires — ni racine, ni `..`, ni `.`, ni
/// préfixe de volume — et à l'extension `.wasm`. La barre oblique inverse est
/// refusée explicitement : sur Unix elle ne sépare rien, et un manifeste qui en
/// contient une vise un autre système que celui qui le lit.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Entrypoint(String);

impl Entrypoint {
    /// Construit un point d'entrée après validation.
    ///
    /// # Erreurs
    /// [`IdParseError`] si le chemin est vide, absolu, contient `.`, `..`, une
    /// barre oblique inverse ou un caractère de contrôle, ou si son extension
    /// n'est pas `.wasm`.
    pub fn new(path: impl Into<String>) -> std::result::Result<Self, IdParseError> {
        let path = path.into();
        if path.is_empty() {
            return Err(IdParseError::new("Entrypoint", "le chemin est vide"));
        }
        if path.contains('\\') {
            return Err(IdParseError::new(
                "Entrypoint",
                "la barre oblique inverse n'est pas un séparateur ici",
            ));
        }
        if path.chars().any(char::is_control) {
            return Err(IdParseError::new(
                "Entrypoint",
                "le chemin contient un caractère de contrôle",
            ));
        }
        let candidat = Path::new(&path);
        if candidat.is_absolute() || candidat.has_root() {
            return Err(IdParseError::new(
                "Entrypoint",
                "le chemin doit être relatif au répertoire du plugin",
            ));
        }
        if !candidat
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(IdParseError::new(
                "Entrypoint",
                "le chemin ne peut contenir ni `.` ni `..`",
            ));
        }
        if candidat.extension().and_then(std::ffi::OsStr::to_str) != Some("wasm") {
            return Err(IdParseError::new(
                "Entrypoint",
                "un point d'entrée est un composant `.wasm`",
            ));
        }
        Ok(Self(path))
    }

    /// Le chemin déclaré, tel quel.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Le chemin absolu du composant, dans le répertoire du plugin.
    ///
    /// La validation garantit que le résultat reste **lexicalement** sous
    /// `plugin_dir`. Elle ne dit rien des liens symboliques : c'est à l'hôte de
    /// n'ouvrir le fichier que relativement à un répertoire pré-ouvert.
    /// `// TODO(phase 4)` : ouverture relative, une fois wasmtime-wasi câblé.
    #[must_use]
    pub fn resolve(&self, plugin_dir: &Path) -> PathBuf {
        plugin_dir.join(&self.0)
    }
}

impl fmt::Debug for Entrypoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entrypoint({:?})", self.as_str())
    }
}

impl fmt::Display for Entrypoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Entrypoint {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for Entrypoint {
    type Error = IdParseError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Entrypoint> for String {
    fn from(entrypoint: Entrypoint) -> Self {
        entrypoint.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Surfaces
// ─────────────────────────────────────────────────────────────────────────────

/// La surface d'extension qu'un plugin occupe.
///
/// Les quatre valeurs sont exactement celles d'[ADR-0005](../../../docs/adr/0005-wasm-plugins.md).
/// L'énumération est **fermée**, contrairement à l'usage du dépôt pour les
/// énumérations publiques : ajouter une surface doit casser la compilation de
/// l'écran d'approbation, qui énonce à l'utilisateur ce qu'un plugin peut
/// faire. Une surface de plus silencieusement approuvée par un `_ =>` est
/// exactement la panne que ce type existe pour éviter — c'est le même
/// raisonnement que pour [`Command`](oxyn_core::Command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// Un driver de base de données, derrière l'interface WIT `oxyn:driver`.
    Driver,
    /// Un agent IA **déclaratif** : une invite, des outils, une politique de
    /// contexte. Aucun code, et donc aucun bac à sable à faire tourner.
    Agent,
    /// Un format d'export.
    Export,
    /// Une visualisation de résultats.
    Visualization,
}

impl PluginKind {
    /// Toutes les surfaces, dans l'ordre d'affichage.
    pub const ALL: [Self; 4] = [Self::Driver, Self::Agent, Self::Export, Self::Visualization];

    /// Nom stable, celui qui est écrit dans le manifeste.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Driver => "driver",
            Self::Agent => "agent",
            Self::Export => "export",
            Self::Visualization => "visualization",
        }
    }

    /// Cette surface exécute-t-elle du code, et donc exige-t-elle l'hôte
    /// WebAssembly ?
    ///
    /// C'est `false` pour [`Agent`](Self::Agent) — et c'est tout l'intérêt :
    /// le cas courant des agents fournis par plugin fonctionne dès aujourd'hui,
    /// sans la feature `wasm-host` et sans qu'aucun code tiers ne s'exécute.
    #[must_use]
    pub const fn runs_code(&self) -> bool {
        !matches!(self, Self::Agent)
    }
}

impl fmt::Display for PluginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Permissions
// ─────────────────────────────────────────────────────────────────────────────

/// Un hôte réseau et son port, accordés ensemble.
///
/// [ADR-0005](../../../docs/adr/0005-wasm-plugins.md) : « accès réseau accordé
/// **hôte par hôte, port par port** ». Il n'y a donc pas de joker : `*` et
/// `0.0.0.0` sont refusés, et un port `0` aussi. Un accord qu'on ne sait pas
/// énoncer à l'utilisateur n'est pas un accord éclairé.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HostPort {
    host: String,
    port: u16,
}

impl HostPort {
    /// Analyse `hôte:port`, ou `[adresse-v6]:port`.
    ///
    /// L'hôte est normalisé en minuscules : une comparaison de noms d'hôtes est
    /// insensible à la casse, et laisser deux écritures du même hôte serait un
    /// moyen commode de contourner la liste.
    ///
    /// # Erreurs
    /// [`IdParseError`] pour un port absent, nul ou hors bornes, un hôte vide,
    /// un joker, un caractère non ASCII (les noms internationalisés se
    /// déclarent en punycode), ou tout ce qui trahit une URL plutôt qu'un hôte :
    /// `/`, `@`, un espace.
    pub fn new(text: impl AsRef<str>) -> std::result::Result<Self, IdParseError> {
        let text = text.as_ref();
        let (host, port) = if let Some(reste) = text.strip_prefix('[') {
            let Some((host, port)) = reste.split_once("]:") else {
                return Err(IdParseError::new(
                    "HostPort",
                    "adresse IPv6 mal formée : attendu `[adresse]:port`",
                ));
            };
            (host, port)
        } else {
            let Some((host, port)) = text.rsplit_once(':') else {
                return Err(IdParseError::new(
                    "HostPort",
                    "attendu `hôte:port` — le port se déclare explicitement",
                ));
            };
            (host, port)
        };

        if host.is_empty() {
            return Err(IdParseError::new("HostPort", "l'hôte est vide"));
        }
        if !host.is_ascii() {
            return Err(IdParseError::new(
                "HostPort",
                "un nom internationalisé se déclare en punycode",
            ));
        }
        if host.contains(['*', '/', '@', '?', '#', ' ']) {
            return Err(IdParseError::new(
                "HostPort",
                "un hôte se déclare nu, sans joker, sans schéma et sans chemin",
            ));
        }
        if host == "0.0.0.0" {
            return Err(IdParseError::new(
                "HostPort",
                "`0.0.0.0` désigne toutes les interfaces : ce n'est pas un hôte",
            ));
        }
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(IdParseError::new("HostPort", "le port n'est pas un nombre"));
        }
        let port: u16 = port
            .parse()
            .map_err(|_| IdParseError::new("HostPort", "le port dépasse 65535"))?;
        if port == 0 {
            return Err(IdParseError::new("HostPort", "le port `0` n'existe pas"));
        }

        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    /// L'hôte, en minuscules.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Le port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Cet accord couvre-t-il cette destination ?
    ///
    /// Comparaison exacte, insensible à la casse. Aucun sous-domaine n'est
    /// impliqué : accorder `example.com:443` n'accorde pas
    /// `exfiltration.example.com:443`.
    #[must_use]
    pub fn matches(&self, host: &str, port: u16) -> bool {
        self.port == port && self.host.eq_ignore_ascii_case(host)
    }
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

impl FromStr for HostPort {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for HostPort {
    type Error = IdParseError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<HostPort> for String {
    fn from(hp: HostPort) -> Self {
        hp.to_string()
    }
}

/// Ce qu'un plugin peut demander aux bases de données de l'utilisateur.
///
/// L'ordre des variantes est celui du pouvoir croissant, et il est
/// **signifiant** : [`PluginPermissions::is_subset_of`] s'en sert pour décider
/// si une mise à jour élargit ce qui avait été approuvé.
///
/// Aucune de ces valeurs n'autorise quoi que ce soit par elle-même. Un plugin
/// n'obtient jamais de poignée vers un driver ni la liste des connexions
/// ouvertes ([`PLUGIN-CONTRACT` §1](../../../docs/PLUGIN-CONTRACT.md)) : il
/// émet des `Command`, et le `PolicyGate` décide. `ReadWrite` signifie donc
/// « ce plugin peut *demander* une écriture », pas « ses écritures passent ».
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAccess {
    /// Aucun accès. **C'est le défaut.**
    #[default]
    Denied,
    /// Le plugin peut demander des lectures et de l'introspection.
    ReadOnly,
    /// Le plugin peut demander des écritures — qui restent soumises à
    /// l'approbation du `PolicyGate`.
    ReadWrite,
}

impl ConnectionAccess {
    /// Nom stable, celui du manifeste.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
        }
    }

    /// Aucun accès n'est accordé.
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied)
    }
}

impl fmt::Display for ConnectionAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ce qu'un manifeste demande, et rien d'autre.
///
/// `deny_unknown_fields` est délibéré : une clé inconnue dans `[permissions]`
/// est soit une faute de frappe — qui ferait échouer le plugin bien plus tard
/// et sans explication —, soit une permission qu'une version ultérieure d'Oxyn
/// connaît et que celle-ci ne sait pas accorder. Dans les deux cas, refuser le
/// manifeste est la bonne réponse.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPermissions {
    /// Les destinations réseau accordées, hôte par hôte et port par port.
    #[serde(default)]
    pub network: Vec<HostPort>,
    /// Les racines du système de fichiers accordées, en chemins absolus.
    #[serde(default)]
    pub filesystem: Vec<PathBuf>,
    /// Ce que le plugin peut demander aux bases de données de l'utilisateur.
    #[serde(default)]
    pub connections: ConnectionAccess,
}

impl PluginPermissions {
    /// Ce manifeste ne demande rien du tout.
    #[must_use]
    pub fn grants_nothing(&self) -> bool {
        self.network.is_empty() && self.filesystem.is_empty() && self.connections.is_denied()
    }

    /// Vérifie la cohérence des racines déclarées.
    ///
    /// Une racine relative n'a pas de sens dans un manifeste — relative à quoi ?
    /// — et une racine contenant `..` désigne autre chose que ce que
    /// l'utilisateur lit dans l'écran d'approbation. Les deux sont refusées.
    ///
    /// # Erreurs
    /// [`PluginError::InvalidManifest`] en nommant la racine fautive.
    pub fn validate(&self, plugin: &str) -> Result<()> {
        for racine in &self.filesystem {
            if !racine.is_absolute() {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    format!(
                        "la racine de fichiers `{}` doit être absolue : \
                         une racine relative n'est pas montrable à l'utilisateur",
                        racine.display()
                    ),
                ));
            }
            if racine
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    format!(
                        "la racine de fichiers `{}` contient `.` ou `..` : \
                         elle ne désigne pas ce qu'elle donne à lire",
                        racine.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Cette destination réseau est-elle accordée ?
    #[must_use]
    pub fn allows_host(&self, host: &str, port: u16) -> bool {
        self.network.iter().any(|accord| accord.matches(host, port))
    }

    /// Ce chemin est-il sous une racine accordée ?
    ///
    /// Le chemin demandé est refusé s'il contient `.` ou `..` : la containment
    /// est **lexicale**, et `/data/../../etc/passwd` commence bien par `/data`.
    /// C'est le contrôle que `starts_with` seul ne fait pas.
    #[must_use]
    pub fn allows_path(&self, path: &Path) -> bool {
        if path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return false;
        }
        self.filesystem
            .iter()
            .any(|racine| path.starts_with(racine))
    }

    /// Ce que `self` demande et que `approved` n'accordait pas, une ligne par
    /// ajout.
    ///
    /// C'est ce que l'interface montre quand une mise à jour rend une
    /// approbation caduque : « ça a changé » n'aide personne à décider, « cette
    /// mise à jour veut aussi joindre `exfiltration.example:443` » si.
    #[must_use]
    pub fn additions_over(&self, approved: &Self) -> Vec<String> {
        let mut ajouts = Vec::new();
        for accord in &self.network {
            if !approved.network.contains(accord) {
                ajouts.push(format!("l'hôte `{accord}`"));
            }
        }
        for racine in &self.filesystem {
            if !approved
                .filesystem
                .iter()
                .any(|accorde| racine.starts_with(accorde))
            {
                ajouts.push(format!("la racine `{}`", racine.display()));
            }
        }
        if self.connections > approved.connections {
            ajouts.push(format!("l'accès aux connexions `{}`", self.connections));
        }
        ajouts
    }

    /// `self` ne demande-t-il rien de plus que `other` ?
    ///
    /// C'est la question que pose une mise à jour de plugin : si la réponse est
    /// oui, l'approbation existante couvre encore le manifeste et l'utilisateur
    /// n'est pas dérangé. Sinon, elle est caduque.
    ///
    /// Défini par [`additions_over`](Self::additions_over), et non en parallèle
    /// de lui : deux implémentations de la même règle divergent, et c'est alors
    /// la plus laxiste qui décide. Alloue, donc — c'est un chemin de découverte,
    /// pas un chemin par ligne.
    #[must_use]
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.additions_over(other).is_empty()
    }

    /// Ce que l'écran d'approbation énonce à l'utilisateur, une ligne par
    /// accord.
    ///
    /// Alloue à chaque appel : c'est un chemin d'ouverture de dialogue, pas un
    /// chemin par ligne.
    #[must_use]
    pub fn summary(&self) -> Vec<String> {
        if self.grants_nothing() {
            return vec!["ne demande aucune permission".to_owned()];
        }
        let mut lignes = Vec::new();
        for accord in &self.network {
            lignes.push(format!("joindre le réseau : {accord}"));
        }
        for racine in &self.filesystem {
            lignes.push(format!("lire et écrire sous : {}", racine.display()));
        }
        match self.connections {
            ConnectionAccess::Denied => {}
            ConnectionAccess::ReadOnly => {
                lignes.push("demander des lectures sur vos connexions".to_owned());
            }
            ConnectionAccess::ReadWrite => {
                lignes.push(
                    "demander des lectures et des écritures sur vos connexions \
                     (chaque écriture reste soumise à votre approbation)"
                        .to_owned(),
                );
            }
        }
        lignes
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Le manifeste
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'un plugin de type [`Driver`](PluginKind::Driver) déclare avant même
/// d'être instancié.
///
/// L'intérêt est concret : `oxyn-desktop` peut refuser un plugin qui revendique un
/// identifiant de driver déjà pris **avant** de faire tourner la moindre
/// instruction du composant — même refus que
/// [`DriverRegistry::register`](oxyn_driver::DriverRegistry::register), pour la
/// même raison : un plugin qui remplacerait `postgres` recevrait les
/// identifiants de production que l'utilisateur croit donner au driver
/// d'origine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDriverSpec {
    /// L'identifiant de protocole revendiqué.
    pub id: DriverId,
    /// Nom montrable dans le sélecteur de connexion.
    pub display_name: String,
    /// Famille, pour le regroupement de la liste.
    pub family: DriverFamily,
}

/// Un `plugin.toml`, analysé et validé.
///
/// `deny_unknown_fields` n'est **pas** posé ici, contrairement à
/// [`PluginPermissions`] : un manifeste écrit pour une version ultérieure
/// d'Oxyn doit pouvoir être analysé assez loin pour que
/// [`api_version`](Self::api_version) soit lu et que le refus dise
/// « interface {x} contre {y} » plutôt que « clé inconnue ligne 12 ». Un message
/// exact vaut mieux qu'un refus précoce quand les deux refusent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Identifiant du plugin. Doit être le nom de son répertoire.
    pub id: PluginId,
    /// Nom montrable.
    pub name: String,
    /// Version du plugin lui-même.
    pub version: PluginVersion,
    /// Version du contrat d'hôte visée. **Obligatoire** : un manifeste qui ne
    /// dit pas contre quoi il a été bâti n'est pas supposé compatible, il est
    /// refusé.
    pub api_version: PluginVersion,
    /// La surface occupée.
    pub kind: PluginKind,
    /// Ce que fait le plugin, pour l'utilisateur qui l'approuve.
    #[serde(default)]
    pub description: String,
    /// Le composant WebAssembly. Absent pour un agent déclaratif.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Entrypoint>,
    /// Ce que le plugin demande. Absente, la section n'accorde **rien**.
    #[serde(default)]
    pub permissions: PluginPermissions,
    /// La déclaration d'agent, pour un plugin de type
    /// [`Agent`](PluginKind::Agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<PluginAgentSpec>,
    /// La déclaration de driver, pour un plugin de type
    /// [`Driver`](PluginKind::Driver).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<PluginDriverSpec>,
}

impl PluginManifest {
    /// Analyse et valide le texte d'un `plugin.toml`.
    ///
    /// # Erreurs
    /// [`PluginError::UnreadableManifest`] si le texte n'est pas du TOML
    /// conforme à la forme attendue ; les variantes de [`Self::validate`]
    /// ensuite.
    pub fn from_toml(text: &str) -> Result<Self> {
        let manifest: Self =
            toml::from_str(text).map_err(|err| PluginError::UnreadableManifest {
                detail: err.to_string(),
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Rend le manifeste en TOML.
    ///
    /// Sert aux tests et à l'écriture d'un manifeste d'exemple. Le format reste
    /// ouvert et relisible sans Oxyn (I-11).
    ///
    /// # Erreurs
    /// [`PluginError::UnreadableManifest`] si un chemin de fichier n'est pas de
    /// l'UTF-8 — seul cas où le rendu peut échouer.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|err| PluginError::UnreadableManifest {
            detail: err.to_string(),
        })
    }

    /// Vérifie les règles que la seule forme du fichier ne garantit pas.
    ///
    /// Cinq refus, tous destinés à faire échouer à la lecture ce qui échouerait
    /// autrement au chargement, ou pire, silencieusement :
    ///
    /// 1. le nom du plugin n'est pas vide ;
    /// 2. une surface qui exécute du code **porte** un point d'entrée ;
    /// 3. un agent déclaratif n'en porte **pas** — sinon `kind = "agent"`
    ///    deviendrait un moyen de faire tourner du code sans que l'écran
    ///    d'approbation ne le dise ;
    /// 4. la section spécifique correspond au `kind` : pas de `[driver]` sur un
    ///    agent, pas de `[agent]` sur un driver ;
    /// 5. un agent qui déclare des outils déclare aussi un accès aux
    ///    connexions, sans quoi ses outils sont inertes et l'utilisateur lit
    ///    une panne là où il y a une déclaration incohérente.
    ///
    /// Les permissions sont validées par la même occasion.
    ///
    /// # Erreurs
    /// [`PluginError::InvalidManifest`], en nommant la règle enfreinte.
    pub fn validate(&self) -> Result<()> {
        let plugin = self.id.as_str();

        if self.name.trim().is_empty() {
            return Err(PluginError::invalid_manifest(plugin, "`name` est vide"));
        }

        if self.kind.runs_code() && self.entrypoint.is_none() {
            return Err(PluginError::invalid_manifest(
                plugin,
                format!("un plugin `{}` doit déclarer un `entrypoint`", self.kind),
            ));
        }
        if !self.kind.runs_code() && self.entrypoint.is_some() {
            return Err(PluginError::invalid_manifest(
                plugin,
                "un agent est déclaratif : il ne peut pas porter d'`entrypoint`, \
                 sinon `kind = \"agent\"` ferait tourner du code sans le dire",
            ));
        }

        match self.kind {
            PluginKind::Agent => {
                let Some(agent) = &self.agent else {
                    return Err(PluginError::invalid_manifest(
                        plugin,
                        "un plugin `agent` doit porter une section `[agent]`",
                    ));
                };
                agent.validate(plugin)?;
                if !agent.allowed_tools.is_empty() && self.permissions.connections.is_denied() {
                    return Err(PluginError::invalid_manifest(
                        plugin,
                        "cet agent déclare des outils mais aucun accès aux connexions : \
                         ses outils seraient inertes, ce qui se lit comme une panne",
                    ));
                }
            }
            PluginKind::Driver => {
                if self.driver.is_none() {
                    return Err(PluginError::invalid_manifest(
                        plugin,
                        "un plugin `driver` doit porter une section `[driver]`",
                    ));
                }
            }
            PluginKind::Export | PluginKind::Visualization => {}
        }

        if self.agent.is_some() && self.kind != PluginKind::Agent {
            return Err(PluginError::invalid_manifest(
                plugin,
                "une section `[agent]` sur un plugin qui n'est pas un agent",
            ));
        }
        if self.driver.is_some() && self.kind != PluginKind::Driver {
            return Err(PluginError::invalid_manifest(
                plugin,
                "une section `[driver]` sur un plugin qui n'est pas un driver",
            ));
        }

        self.permissions.validate(plugin)
    }

    /// Ce plugin exige-t-il l'hôte WebAssembly ?
    #[must_use]
    pub const fn requires_wasm(&self) -> bool {
        self.kind.runs_code()
    }

    /// Le plugin vise-t-il une interface que cet hôte fournit ?
    #[must_use]
    pub fn is_api_compatible(&self) -> bool {
        self.api_version.is_compatible_with(&HOST_API_VERSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un manifeste d'agent déclaratif complet — le cas courant, celui qui doit
    /// marcher sans la feature `wasm-host`.
    const AGENT_TOML: &str = r#"
id          = "revue-schema"
name        = "Revue de schéma"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "agent"
description = "Relit un schéma et propose des index."

[permissions]
connections = "read_only"

[agent]
name          = "Schema"
system_prompt = "You review database schemas."
allowed_tools = ["refresh_catalog"]
"#;

    const DRIVER_TOML: &str = r#"
id          = "duckdb"
name        = "DuckDB"
version     = "0.3.1"
api_version = "0.1.0"
kind        = "driver"
entrypoint  = "duckdb.wasm"

[permissions]
network = ["catalog.example.com:443"]

[driver]
id           = "duckdb"
display_name = "DuckDB"
family       = "analytical"
"#;

    // ── Versions ────────────────────────────────────────────────────────────

    #[test]
    fn aller_retour_des_versions() {
        for texte in ["0.0.0", "0.1.0", "1.2.3", "10.20.30"] {
            let version: PluginVersion = texte.parse().expect("version valide");
            assert_eq!(version.to_string(), texte);
        }
    }

    #[test]
    fn une_version_hors_gabarit_est_refusee() {
        for texte in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "v1.2.3",
            "+1.2.3",
            "1..3",
            "1.2.3-rc.1",
            "1.2.3+build",
        ] {
            assert!(
                texte.parse::<PluginVersion>().is_err(),
                "{texte:?} devrait être refusé"
            );
        }
    }

    #[test]
    fn en_zero_point_x_chaque_mineur_est_une_rupture() {
        // Le contrat n'a pas d'implémentation tierce : un plugin bâti sur 0.1
        // ne peut rien supposer de 0.2.
        let hote = PluginVersion::new(0, 1, 0);
        assert!(PluginVersion::new(0, 1, 0).is_compatible_with(&hote));
        assert!(PluginVersion::new(0, 1, 7).is_compatible_with(&hote));
        assert!(!PluginVersion::new(0, 2, 0).is_compatible_with(&hote));
        assert!(!PluginVersion::new(0, 0, 9).is_compatible_with(&hote));
        assert!(!PluginVersion::new(1, 1, 0).is_compatible_with(&hote));
    }

    #[test]
    fn apres_le_premier_majeur_l_hote_peut_etre_en_avance_mais_pas_en_retard() {
        let hote = PluginVersion::new(1, 4, 2);
        assert!(PluginVersion::new(1, 2, 0).is_compatible_with(&hote));
        assert!(PluginVersion::new(1, 4, 9).is_compatible_with(&hote));
        assert!(
            !PluginVersion::new(1, 5, 0).is_compatible_with(&hote),
            "l'hôte ne fournit pas les interfaces que ce plugin attend"
        );
        assert!(!PluginVersion::new(2, 0, 0).is_compatible_with(&hote));
    }

    // ── Identifiants et chemins ─────────────────────────────────────────────

    #[test]
    fn un_identifiant_de_plugin_ne_peut_pas_designer_un_autre_repertoire() {
        // C'est la règle qui compte : l'identifiant nomme un répertoire et une
        // clé d'approbation.
        for nom in [
            "",
            "..",
            "../voisin",
            "a/b",
            "a\\b",
            "Plugin",
            "2fast",
            "plugin.toml",
            "plug in",
            "plugin\0",
        ] {
            assert!(PluginId::new(nom).is_err(), "{nom:?} devrait être refusé");
        }
        assert!(PluginId::new("a".repeat(65)).is_err());

        for nom in ["duckdb", "revue-schema", "export_parquet", "mongo2"] {
            assert!(PluginId::new(nom).is_ok(), "{nom} devrait être accepté");
        }
    }

    #[test]
    fn un_point_d_entree_ne_sort_pas_du_repertoire_du_plugin() {
        for chemin in [
            "",
            "/usr/lib/evil.wasm",
            "../voisin/evil.wasm",
            "./evil.wasm",
            "sous/../../evil.wasm",
            "..\\evil.wasm",
            "evil.so",
            "evil",
            "evil.wasm\n",
        ] {
            assert!(
                Entrypoint::new(chemin).is_err(),
                "{chemin:?} devrait être refusé"
            );
        }

        let entree = Entrypoint::new("build/plugin.wasm").expect("chemin relatif simple");
        assert_eq!(
            entree.resolve(Path::new("/plugins/duckdb")),
            Path::new("/plugins/duckdb/build/plugin.wasm")
        );
    }

    // ── Permissions ─────────────────────────────────────────────────────────

    #[test]
    fn un_manifeste_sans_section_permissions_n_accorde_rien() {
        // ADR-0005. C'est le test qui protège le défaut : un défaut permissif
        // ne se voit ni à la compilation, ni à la relecture.
        let toml = r#"
id          = "vide"
name        = "Sans permissions"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "export"
entrypoint  = "vide.wasm"
"#;
        let manifeste = PluginManifest::from_toml(toml).expect("manifeste valide");
        assert!(manifeste.permissions.grants_nothing());
        assert_eq!(manifeste.permissions.connections, ConnectionAccess::Denied);
        assert!(!manifeste.permissions.allows_host("example.com", 443));
        assert!(!manifeste.permissions.allows_path(Path::new("/etc/passwd")));
        assert_eq!(
            manifeste.permissions.summary(),
            ["ne demande aucune permission"]
        );
    }

    #[test]
    fn le_reseau_s_accorde_hote_par_hote_et_port_par_port() {
        let accord = HostPort::new("Db.Example.COM:5432").expect("hôte valide");
        assert_eq!(accord.host(), "db.example.com");
        assert_eq!(accord.port(), 5432);
        assert!(accord.matches("DB.EXAMPLE.com", 5432));
        assert!(!accord.matches("db.example.com", 5433));
        assert!(
            !accord.matches("evil.db.example.com", 5432),
            "un accord ne couvre pas les sous-domaines"
        );
    }

    #[test]
    fn un_joker_reseau_est_refuse() {
        // « accès réseau accordé hôte par hôte, port par port » (ADR-0005).
        for texte in [
            "*:443",
            "*.example.com:443",
            "0.0.0.0:443",
            "example.com",
            "example.com:0",
            "example.com:99999",
            "example.com:https",
            ":443",
            "http://example.com:443",
            "user@example.com:443",
            "example.com/path:443",
            "exemplé.fr:443",
            "exemple.fr:44\u{200b}3",
        ] {
            assert!(
                HostPort::new(texte).is_err(),
                "{texte:?} devrait être refusé"
            );
        }
        assert!(HostPort::new("[::1]:6379").is_ok());
        assert_eq!(
            HostPort::new("[::1]:6379").expect("adresse v6").to_string(),
            "[::1]:6379"
        );
    }

    #[test]
    fn une_permission_de_fichiers_relative_ou_avec_deux_points_est_refusee() {
        let mut permissions = PluginPermissions {
            filesystem: vec![PathBuf::from("donnees")],
            ..PluginPermissions::default()
        };
        assert!(permissions.validate("x").is_err(), "racine relative");

        permissions.filesystem = vec![PathBuf::from("/home/x/../../etc")];
        assert!(permissions.validate("x").is_err(), "racine avec `..`");

        permissions.filesystem = vec![PathBuf::from("/home/x/donnees")];
        permissions.validate("x").expect("racine absolue et nette");
    }

    #[test]
    fn un_chemin_qui_remonte_n_est_jamais_sous_une_racine_accordee() {
        // `starts_with` seul dirait oui : `/data/../../etc/passwd` commence bien
        // par `/data`.
        let permissions = PluginPermissions {
            filesystem: vec![PathBuf::from("/data")],
            ..PluginPermissions::default()
        };
        assert!(permissions.allows_path(Path::new("/data/rapport.csv")));
        assert!(!permissions.allows_path(Path::new("/data/../../etc/passwd")));
        assert!(!permissions.allows_path(Path::new("/etc/passwd")));
        assert!(!permissions.allows_path(Path::new("/database/secret")));
    }

    #[test]
    fn une_mise_a_jour_qui_elargit_rend_l_approbation_caduque() {
        let approuve = PluginPermissions {
            network: vec![HostPort::new("a.example:443").expect("hôte")],
            filesystem: vec![PathBuf::from("/data")],
            connections: ConnectionAccess::ReadOnly,
        };

        // Identique, ou plus étroit : l'approbation tient.
        assert!(approuve.is_subset_of(&approuve));
        let plus_etroit = PluginPermissions {
            network: Vec::new(),
            filesystem: vec![PathBuf::from("/data/sous-dossier")],
            connections: ConnectionAccess::Denied,
        };
        assert!(plus_etroit.is_subset_of(&approuve));

        // Un hôte de plus, une racine de plus, l'écriture en plus : caduque.
        for elargi in [
            PluginPermissions {
                network: vec![
                    HostPort::new("a.example:443").expect("hôte"),
                    HostPort::new("exfiltration.example:443").expect("hôte"),
                ],
                ..approuve.clone()
            },
            PluginPermissions {
                filesystem: vec![PathBuf::from("/data"), PathBuf::from("/home")],
                ..approuve.clone()
            },
            PluginPermissions {
                connections: ConnectionAccess::ReadWrite,
                ..approuve.clone()
            },
            PluginPermissions {
                network: vec![HostPort::new("a.example:8443").expect("hôte")],
                ..approuve.clone()
            },
        ] {
            assert!(
                !elargi.is_subset_of(&approuve),
                "{elargi:?} élargit l'approbation"
            );
        }
    }

    #[test]
    fn une_permission_inconnue_est_refusee_pas_ignoree() {
        // Faute de frappe, ou permission d'une version ultérieure : dans les
        // deux cas, l'ignorer ferait échouer le plugin plus tard et sans
        // explication.
        let toml = r#"
id          = "x"
name        = "X"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "export"
entrypoint  = "x.wasm"

[permissions]
netwrok = ["example.com:443"]
"#;
        let err = PluginManifest::from_toml(toml).expect_err("refus attendu");
        assert!(
            matches!(err, PluginError::UnreadableManifest { .. }),
            "{err}"
        );
    }

    #[test]
    fn le_resume_enonce_chaque_accord() {
        let permissions = PluginPermissions {
            network: vec![HostPort::new("a.example:443").expect("hôte")],
            filesystem: vec![PathBuf::from("/data")],
            connections: ConnectionAccess::ReadWrite,
        };
        let resume = permissions.summary();
        assert_eq!(resume.len(), 3);
        assert!(resume[0].contains("a.example:443"), "{resume:?}");
        assert!(resume[1].contains("/data"), "{resume:?}");
        assert!(resume[2].contains("approbation"), "{resume:?}");
    }

    // ── Manifeste ───────────────────────────────────────────────────────────

    #[test]
    fn un_agent_declaratif_s_analyse_sans_hote_wasm() {
        let manifeste = PluginManifest::from_toml(AGENT_TOML).expect("manifeste valide");
        assert_eq!(manifeste.id.as_str(), "revue-schema");
        assert_eq!(manifeste.kind, PluginKind::Agent);
        assert!(!manifeste.requires_wasm());
        assert!(manifeste.is_api_compatible());
        assert!(manifeste.entrypoint.is_none());

        let agent = manifeste.agent.as_ref().expect("section agent");
        assert_eq!(agent.name, "Schema");
        assert!(agent.allows("refresh_catalog"));
        assert!(!agent.allows("execute_query"));
    }

    #[test]
    fn un_agent_ne_peut_pas_porter_de_point_d_entree() {
        // Sinon `kind = "agent"` deviendrait le moyen de faire tourner du code
        // en se présentant comme déclaratif.
        let toml = AGENT_TOML.replace(
            "kind        = \"agent\"",
            "kind        = \"agent\"\nentrypoint  = \"agent.wasm\"",
        );
        let err = PluginManifest::from_toml(&toml).expect_err("refus attendu");
        assert!(err.to_string().contains("déclaratif"), "{err}");
    }

    #[test]
    fn un_agent_avec_des_outils_mais_sans_acces_aux_connexions_est_refuse() {
        let toml = AGENT_TOML.replace("connections = \"read_only\"", "");
        let err = PluginManifest::from_toml(&toml).expect_err("refus attendu");
        assert!(err.to_string().contains("inertes"), "{err}");
    }

    #[test]
    fn une_surface_qui_execute_du_code_doit_declarer_son_point_d_entree() {
        for kind in ["driver", "export", "visualization"] {
            let toml = format!(
                "id = \"x\"\nname = \"X\"\nversion = \"1.0.0\"\n\
                 api_version = \"0.1.0\"\nkind = \"{kind}\"\n"
            );
            let err = PluginManifest::from_toml(&toml).expect_err("refus attendu");
            assert!(err.to_string().contains("entrypoint"), "{kind} : {err}");
        }
    }

    #[test]
    fn une_section_specifique_doit_correspondre_a_la_surface() {
        let toml = DRIVER_TOML.replace("kind        = \"driver\"", "kind        = \"export\"");
        let err = PluginManifest::from_toml(&toml).expect_err("refus attendu");
        assert!(err.to_string().contains("`[driver]`"), "{err}");
    }

    #[test]
    fn un_driver_declare_son_protocole_avant_toute_execution() {
        let manifeste = PluginManifest::from_toml(DRIVER_TOML).expect("manifeste valide");
        assert!(manifeste.requires_wasm());
        let driver = manifeste.driver.as_ref().expect("section driver");
        assert_eq!(driver.id, DriverId::new("duckdb").expect("identifiant"));
        assert_eq!(driver.family, DriverFamily::Analytical);
        assert!(
            manifeste
                .permissions
                .allows_host("catalog.example.com", 443)
        );
    }

    #[test]
    fn un_manifeste_sans_version_d_interface_est_refuse() {
        // Ne pas dire contre quoi on a été bâti ne vaut pas « compatible ».
        let toml = DRIVER_TOML.replace("api_version = \"0.1.0\"\n", "");
        let err = PluginManifest::from_toml(&toml).expect_err("refus attendu");
        assert!(
            matches!(err, PluginError::UnreadableManifest { .. }),
            "{err}"
        );
    }

    #[test]
    fn aller_retour_toml() {
        let manifeste = PluginManifest::from_toml(DRIVER_TOML).expect("manifeste valide");
        let rendu = manifeste.to_toml().expect("rendu");
        let relu = PluginManifest::from_toml(&rendu).expect("relecture");
        assert_eq!(relu, manifeste);
    }
}
