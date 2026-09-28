//! A plugin's manifest: `plugin.toml`.
//!
//! A manifest is written by a third party. It is therefore read as **hostile
//! data** ([`SECURITY` §input surface](../../../docs/SECURITY.md)): every field
//! is validated, and what is not understood is refused rather than ignored.
//!
//! # The rule that governs this module
//!
//! **By default, everything is refused.** A manifest without a `[permissions]`
//! section grants nothing: neither network, nor file, nor access to connections
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)). The default of
//! [`PluginPermissions`] carries it, and it is tested — because a permissive
//! default shows neither at compile time nor in review, but only the day a
//! plugin exfiltrates.
//!
//! A less obvious corollary: **a declared permission authorizes nothing by
//! itself**. `connections = "read_write"` does not give the right to write; it
//! gives the right to *ask*, and the request goes through the `PolicyGate` like
//! a human's ([ADR-0004](../../../docs/adr/0004-command-bus.md), I-01). The
//! manifest can only **restrict**.
//!
//! # File shape
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

/// Name of the manifest, in a plugin's directory.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// Version of the host contract this build of Oxyn provides.
///
/// It covers the manifest shape **and** the WIT interfaces. As long as the
/// major is `0`, every minor increment is a break: it is the usual convention
/// for a contract that has no third-party implementation yet, and
/// [`PluginVersion::is_compatible_with`] applies it.
pub const HOST_API_VERSION: PluginVersion = PluginVersion::new(0, 1, 0);

// ─────────────────────────────────────────────────────────────────────────────
// Version
// ─────────────────────────────────────────────────────────────────────────────

/// A `major.minor.patch` version number.
///
/// Deliberately narrower than full SemVer: no pre-release, no build metadata. A
/// manifest is a configuration file, not a package repository, and accepting
/// `1.0.0-rc.1+build.7` would force deciding its ordering — a decision that
/// brings nothing here and that would then have to be upheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl PluginVersion {
    /// Builds a version.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// The major.
    #[must_use]
    pub const fn major(&self) -> u32 {
        self.major
    }

    /// The minor.
    #[must_use]
    pub const fn minor(&self) -> u32 {
        self.minor
    }

    /// The patch.
    #[must_use]
    pub const fn patch(&self) -> u32 {
        self.patch
    }

    /// Can a plugin targeting `self` run on a host providing `host`?
    ///
    /// Two regimes, and the difference is deliberate:
    ///
    /// * **major `0`** — the contract is unstable: the minor must be
    ///   **identical**. A plugin built on `0.1` does not run on `0.2`;
    /// * **major ≥ 1** — the major must be identical and the plugin's minor
    ///   lower than or equal to the host's. A plugin built on `1.2` runs on
    ///   `1.4`; the reverse is refused, because the host does not provide the
    ///   interfaces the plugin expects.
    ///
    /// The patch never counts: by definition it neither adds nor removes an
    /// interface.
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
        // Only decimal digits are accepted: `u32::from_str` would accept `+1`,
        // which would let `+1.0.0` pass for a version.
        fn number(part: &str) -> std::result::Result<u32, IdParseError> {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(IdParseError::new(
                    "PluginVersion",
                    "expected: three decimal numbers separated by dots",
                ));
            }
            part.parse::<u32>().map_err(|_| {
                IdParseError::new("PluginVersion", "a component exceeds the capacity of a u32")
            })
        }

        let mut parts = s.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(IdParseError::new(
                "PluginVersion",
                "expected: `major.minor.patch`",
            ));
        };
        Ok(Self::new(number(major)?, number(minor)?, number(patch)?))
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
// Identifier
// ─────────────────────────────────────────────────────────────────────────────

/// Stable identifier of a plugin.
///
/// It names the plugin's **directory** and the key of its approval. Its
/// normalization is therefore not cosmetic: an identifier containing `/`, `..`
/// or a control byte would let a manifest designate a directory that is not its
/// own, or overwrite another plugin's approval.
///
/// Same grammar as [`DriverId`], longer: ASCII lowercase, digits, `-` and `_`,
/// first letter alphabetic, 64 characters at most.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(Arc<str>);

impl PluginId {
    /// Maximum length, in bytes.
    pub const MAX_LEN: usize = 64;

    /// Builds an identifier after validation.
    ///
    /// # Errors
    /// [`IdParseError`] if the string is empty, too long, does not start with
    /// an ASCII lowercase letter, or contains a character outside
    /// `[a-z0-9_-]`. The faulty value is never repeated in the error.
    pub fn new(name: impl AsRef<str>) -> std::result::Result<Self, IdParseError> {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(IdParseError::new("PluginId", "the string is empty"));
        }
        if name.len() > Self::MAX_LEN {
            return Err(IdParseError::new("PluginId", "more than 64 characters"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "PluginId",
                "must start with an ASCII lowercase letter",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "PluginId",
                "allowed characters: a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Borrowed view of the identifier.
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
// Entrypoint
// ─────────────────────────────────────────────────────────────────────────────

/// A plugin's WebAssembly component, relative to its directory.
///
/// **Concrete failure avoided:** a manifest declaring
/// `entrypoint = "../../../.ssh/id_rsa"` or `/usr/lib/libc.so`. The path is
/// therefore constrained to ordinary components — no root, no `..`, no `.`, no
/// volume prefix — and to the `.wasm` extension. The backslash is explicitly
/// refused: on Unix it separates nothing, and a manifest containing one targets
/// another system than the one reading it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Entrypoint(String);

impl Entrypoint {
    /// Builds an entrypoint after validation.
    ///
    /// # Errors
    /// [`IdParseError`] if the path is empty, absolute, contains `.`, `..`, a
    /// backslash or a control character, or if its extension is not `.wasm`.
    pub fn new(path: impl Into<String>) -> std::result::Result<Self, IdParseError> {
        let path = path.into();
        if path.is_empty() {
            return Err(IdParseError::new("Entrypoint", "the path is empty"));
        }
        if path.contains('\\') {
            return Err(IdParseError::new(
                "Entrypoint",
                "the backslash is not a separator here",
            ));
        }
        if path.chars().any(char::is_control) {
            return Err(IdParseError::new(
                "Entrypoint",
                "the path contains a control character",
            ));
        }
        let candidate = Path::new(&path);
        if candidate.is_absolute() || candidate.has_root() {
            return Err(IdParseError::new(
                "Entrypoint",
                "the path must be relative to the plugin directory",
            ));
        }
        if !candidate
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(IdParseError::new(
                "Entrypoint",
                "the path can contain neither `.` nor `..`",
            ));
        }
        if candidate.extension().and_then(std::ffi::OsStr::to_str) != Some("wasm") {
            return Err(IdParseError::new(
                "Entrypoint",
                "an entrypoint is a `.wasm` component",
            ));
        }
        Ok(Self(path))
    }

    /// The declared path, as is.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The absolute path of the component, in the plugin directory.
    ///
    /// Validation guarantees the result stays **lexically** under `plugin_dir`.
    /// It says nothing about symbolic links: it is up to the host to open the
    /// file only relative to a pre-opened directory.
    /// `// TODO(phase 4)`: relative opening, once wasmtime-wasi is wired.
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

/// The extension surface a plugin occupies.
///
/// The four values are exactly those of [ADR-0005](../../../docs/adr/0005-wasm-plugins.md).
/// The enumeration is **closed**, contrary to the repository's practice for
/// public enumerations: adding a surface must break the compilation of the
/// approval screen, which states to the user what a plugin can do. One more
/// surface silently approved by a `_ =>` is exactly the failure this type
/// exists to prevent — it is the same reasoning as for
/// [`Command`](oxyn_core::Command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// A database driver, behind the `oxyn:driver` WIT interface.
    Driver,
    /// A **declarative** AI agent: a prompt, tools, a context policy. No code,
    /// and therefore no sandbox to run.
    Agent,
    /// An export format.
    Export,
    /// A result visualization.
    Visualization,
}

impl PluginKind {
    /// All surfaces, in display order.
    pub const ALL: [Self; 4] = [Self::Driver, Self::Agent, Self::Export, Self::Visualization];

    /// Stable name, the one written in the manifest.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Driver => "driver",
            Self::Agent => "agent",
            Self::Export => "export",
            Self::Visualization => "visualization",
        }
    }

    /// Does this surface run code, and therefore require the WebAssembly host?
    ///
    /// It is `false` for [`Agent`](Self::Agent) — and that is the whole point:
    /// the common case of plugin-provided agents works today, without the
    /// `wasm-host` feature and without any third-party code running.
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

/// A network host and its port, granted together.
///
/// [ADR-0005](../../../docs/adr/0005-wasm-plugins.md): "network access granted
/// **host by host, port by port**". There is therefore no wildcard: `*` and
/// `0.0.0.0` are refused, and so is port `0`. A grant that cannot be stated to
/// the user is not an informed grant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HostPort {
    host: String,
    port: u16,
}

impl HostPort {
    /// Parses `host:port`, or `[v6-address]:port`.
    ///
    /// The host is normalized to lowercase: host name comparison is
    /// case-insensitive, and allowing two spellings of the same host would be a
    /// handy way around the list.
    ///
    /// # Errors
    /// [`IdParseError`] for a missing, zero or out-of-range port, an empty
    /// host, a wildcard, a non-ASCII character (internationalized names are
    /// declared in punycode), or anything betraying a URL rather than a host:
    /// `/`, `@`, a space.
    pub fn new(text: impl AsRef<str>) -> std::result::Result<Self, IdParseError> {
        let text = text.as_ref();
        let (host, port) = if let Some(rest) = text.strip_prefix('[') {
            let Some((host, port)) = rest.split_once("]:") else {
                return Err(IdParseError::new(
                    "HostPort",
                    "malformed IPv6 address: expected `[address]:port`",
                ));
            };
            (host, port)
        } else {
            let Some((host, port)) = text.rsplit_once(':') else {
                return Err(IdParseError::new(
                    "HostPort",
                    "expected `host:port` — the port is declared explicitly",
                ));
            };
            (host, port)
        };

        if host.is_empty() {
            return Err(IdParseError::new("HostPort", "the host is empty"));
        }
        if !host.is_ascii() {
            return Err(IdParseError::new(
                "HostPort",
                "an internationalized name is declared in punycode",
            ));
        }
        if host.contains(['*', '/', '@', '?', '#', ' ']) {
            return Err(IdParseError::new(
                "HostPort",
                "a host is declared bare, without wildcard, scheme or path",
            ));
        }
        if host == "0.0.0.0" {
            return Err(IdParseError::new(
                "HostPort",
                "`0.0.0.0` designates every interface: it is not a host",
            ));
        }
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(IdParseError::new("HostPort", "the port is not a number"));
        }
        let port: u16 = port
            .parse()
            .map_err(|_| IdParseError::new("HostPort", "the port exceeds 65535"))?;
        if port == 0 {
            return Err(IdParseError::new("HostPort", "port `0` does not exist"));
        }

        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    /// The host, in lowercase.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Does this grant cover this destination?
    ///
    /// Exact, case-insensitive comparison. No subdomain is implied: granting
    /// `example.com:443` does not grant `exfiltration.example.com:443`.
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

/// What a plugin can ask of the user's databases.
///
/// The order of the variants is that of increasing power, and it is
/// **significant**: [`PluginPermissions::is_subset_of`] uses it to decide
/// whether an update widens what had been approved.
///
/// None of these values authorizes anything by itself. A plugin never gets a
/// handle to a driver nor the list of open connections
/// ([`PLUGIN-CONTRACT` §1](../../../docs/PLUGIN-CONTRACT.md)): it emits
/// `Command`s, and the `PolicyGate` decides. `ReadWrite` therefore means "this
/// plugin can *ask* for a write", not "its writes go through".
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAccess {
    /// No access. **It is the default.**
    #[default]
    Denied,
    /// The plugin can ask for reads and introspection.
    ReadOnly,
    /// The plugin can ask for writes — which remain subject to the
    /// `PolicyGate`'s approval.
    ReadWrite,
}

impl ConnectionAccess {
    /// Stable name, the manifest's.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
        }
    }

    /// No access is granted.
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

/// What a manifest asks for, and nothing else.
///
/// `deny_unknown_fields` is deliberate: an unknown key in `[permissions]` is
/// either a typo — which would make the plugin fail much later and without
/// explanation — or a permission a later version of Oxyn knows and this one
/// cannot grant. In both cases, refusing the manifest is the right answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPermissions {
    /// The granted network destinations, host by host and port by port.
    #[serde(default)]
    pub network: Vec<HostPort>,
    /// The granted file system roots, as absolute paths.
    #[serde(default)]
    pub filesystem: Vec<PathBuf>,
    /// What the plugin can ask of the user's databases.
    #[serde(default)]
    pub connections: ConnectionAccess,
}

impl PluginPermissions {
    /// This manifest asks for nothing at all.
    #[must_use]
    pub fn grants_nothing(&self) -> bool {
        self.network.is_empty() && self.filesystem.is_empty() && self.connections.is_denied()
    }

    /// Checks the consistency of the declared roots.
    ///
    /// A relative root makes no sense in a manifest — relative to what? — and a
    /// root containing `..` designates something other than what the user
    /// reads in the approval screen. Both are refused.
    ///
    /// # Errors
    /// [`PluginError::InvalidManifest`] naming the faulty root.
    pub fn validate(&self, plugin: &str) -> Result<()> {
        for root_dir in &self.filesystem {
            if !root_dir.is_absolute() {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    format!(
                        "the file root `{}` must be absolute: \
                         a relative root cannot be shown to the user",
                        root_dir.display()
                    ),
                ));
            }
            if root_dir
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    format!(
                        "the file root `{}` contains `.` or `..`: \
                         it does not designate what it shows",
                        root_dir.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Is this network destination granted?
    #[must_use]
    pub fn allows_host(&self, host: &str, port: u16) -> bool {
        self.network
            .iter()
            .any(|allowed| allowed.matches(host, port))
    }

    /// Is this path under a granted root?
    ///
    /// The requested path is refused if it contains `.` or `..`: containment is
    /// **lexical**, and `/data/../../etc/passwd` does start with `/data`. It is
    /// the check `starts_with` alone does not make.
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
            .any(|root_dir| path.starts_with(root_dir))
    }

    /// What `self` asks for that `approved` did not grant, one line per
    /// addition.
    ///
    /// It is what the interface shows when an update voids an approval: "it
    /// changed" helps nobody decide, "this update also wants to reach
    /// `exfiltration.example:443`" does.
    #[must_use]
    pub fn additions_over(&self, approved: &Self) -> Vec<String> {
        let mut additions = Vec::new();
        for allowed in &self.network {
            if !approved.network.contains(allowed) {
                additions.push(format!("the host `{allowed}`"));
            }
        }
        for root_dir in &self.filesystem {
            if !approved
                .filesystem
                .iter()
                .any(|granted_perms| root_dir.starts_with(granted_perms))
            {
                additions.push(format!("the root `{}`", root_dir.display()));
            }
        }
        if self.connections > approved.connections {
            additions.push(format!("the connection access `{}`", self.connections));
        }
        additions
    }

    /// Does `self` ask for nothing more than `other`?
    ///
    /// It is the question a plugin update raises: if the answer is yes, the
    /// existing approval still covers the manifest and the user is not
    /// disturbed. Otherwise, it is void.
    ///
    /// Defined by [`additions_over`](Self::additions_over), and not alongside
    /// it: two implementations of the same rule diverge, and the laxer one then
    /// decides. It allocates, then — it is a discovery path, not a per-row
    /// path.
    #[must_use]
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.additions_over(other).is_empty()
    }

    /// What the approval screen states to the user, one line per grant.
    ///
    /// Allocates on every call: it is a dialog-opening path, not a per-row
    /// path.
    #[must_use]
    pub fn summary(&self) -> Vec<String> {
        if self.grants_nothing() {
            return vec!["asks for no permission".to_owned()];
        }
        let mut lines = Vec::new();
        for allowed in &self.network {
            lines.push(format!("reach the network: {allowed}"));
        }
        for root_dir in &self.filesystem {
            lines.push(format!("read and write under: {}", root_dir.display()));
        }
        match self.connections {
            ConnectionAccess::Denied => {}
            ConnectionAccess::ReadOnly => {
                lines.push("ask for reads on your connections".to_owned());
            }
            ConnectionAccess::ReadWrite => {
                lines.push(
                    "ask for reads and writes on your connections \
                     (each write remains subject to your approval)"
                        .to_owned(),
                );
            }
        }
        lines
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The manifest
// ─────────────────────────────────────────────────────────────────────────────

/// What a [`Driver`](PluginKind::Driver) plugin declares before even being
/// instantiated.
///
/// The benefit is concrete: `oxyn-desktop` can refuse a plugin claiming an
/// already taken driver identifier **before** running a single instruction of
/// the component — same refusal as
/// [`DriverRegistry::register`](oxyn_driver::DriverRegistry::register), for the
/// same reason: a plugin replacing `postgres` would receive the production
/// credentials the user believes they give to the original driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDriverSpec {
    /// The claimed protocol identifier.
    pub id: DriverId,
    /// Showable name in the connection picker.
    pub display_name: String,
    /// Family, for grouping the list.
    pub family: DriverFamily,
}

/// A `plugin.toml`, parsed and validated.
///
/// `deny_unknown_fields` is **not** set here, contrary to
/// [`PluginPermissions`]: a manifest written for a later version of Oxyn must
/// parse far enough for [`api_version`](Self::api_version) to be read and for
/// the refusal to say "interface {x} versus {y}" rather than "unknown key line
/// 12". An exact message is better than an early refusal when both refuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin identifier. Must be the name of its directory.
    pub id: PluginId,
    /// Showable name.
    pub name: String,
    /// Version of the plugin itself.
    pub version: PluginVersion,
    /// Targeted host contract version. **Mandatory**: a manifest that does not
    /// say what it was built against is not assumed compatible, it is refused.
    pub api_version: PluginVersion,
    /// The surface occupied.
    pub kind: PluginKind,
    /// What the plugin does, for the user approving it.
    #[serde(default)]
    pub description: String,
    /// The WebAssembly component. Absent for a declarative agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Entrypoint>,
    /// What the plugin asks for. Absent, the section grants **nothing**.
    #[serde(default)]
    pub permissions: PluginPermissions,
    /// The agent declaration, for an [`Agent`](PluginKind::Agent) plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<PluginAgentSpec>,
    /// The driver declaration, for a [`Driver`](PluginKind::Driver) plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<PluginDriverSpec>,
}

impl PluginManifest {
    /// Parses and validates the text of a `plugin.toml`.
    ///
    /// # Errors
    /// [`PluginError::UnreadableManifest`] if the text is not TOML matching the
    /// expected shape; the variants of [`Self::validate`] afterwards.
    pub fn from_toml(text: &str) -> Result<Self> {
        let manifest: Self =
            toml::from_str(text).map_err(|err| PluginError::UnreadableManifest {
                detail: err.to_string(),
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Renders the manifest as TOML.
    ///
    /// Used by tests and to write an example manifest. The format stays open
    /// and readable without Oxyn (I-11).
    ///
    /// # Errors
    /// [`PluginError::UnreadableManifest`] if a file path is not UTF-8 — the
    /// only case where rendering can fail.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|err| PluginError::UnreadableManifest {
            detail: err.to_string(),
        })
    }

    /// Checks the rules the file shape alone does not guarantee.
    ///
    /// Five refusals, all meant to fail at read time what would otherwise fail
    /// at load time, or worse, silently:
    ///
    /// 1. the plugin name is not empty;
    /// 2. a surface that runs code **carries** an entrypoint;
    /// 3. a declarative agent does **not** carry one — otherwise
    ///    `kind = "agent"` would become a way to run code without the approval
    ///    screen saying so;
    /// 4. the specific section matches the `kind`: no `[driver]` on an agent,
    ///    no `[agent]` on a driver;
    /// 5. an agent that declares tools also declares a connection access,
    ///    otherwise its tools are inert and the user reads a failure where
    ///    there is an inconsistent declaration.
    ///
    /// The permissions are validated at the same time.
    ///
    /// # Errors
    /// [`PluginError::InvalidManifest`], naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        let plugin = self.id.as_str();

        if self.name.trim().is_empty() {
            return Err(PluginError::invalid_manifest(plugin, "`name` is empty"));
        }

        if self.kind.runs_code() && self.entrypoint.is_none() {
            return Err(PluginError::invalid_manifest(
                plugin,
                format!("a `{}` plugin must declare an `entrypoint`", self.kind),
            ));
        }
        if !self.kind.runs_code() && self.entrypoint.is_some() {
            return Err(PluginError::invalid_manifest(
                plugin,
                "an agent is declarative: it cannot carry an `entrypoint`, \
                 otherwise `kind = \"agent\"` would run code without saying so",
            ));
        }

        match self.kind {
            PluginKind::Agent => {
                let Some(agent) = &self.agent else {
                    return Err(PluginError::invalid_manifest(
                        plugin,
                        "an `agent` plugin must carry an `[agent]` section",
                    ));
                };
                agent.validate(plugin)?;
                if !agent.allowed_tools.is_empty() && self.permissions.connections.is_denied() {
                    return Err(PluginError::invalid_manifest(
                        plugin,
                        "this agent declares tools but no connection access: \
                         its tools would be inert, which reads as a failure",
                    ));
                }
            }
            PluginKind::Driver => {
                if self.driver.is_none() {
                    return Err(PluginError::invalid_manifest(
                        plugin,
                        "a `driver` plugin must carry a `[driver]` section",
                    ));
                }
            }
            PluginKind::Export | PluginKind::Visualization => {}
        }

        if self.agent.is_some() && self.kind != PluginKind::Agent {
            return Err(PluginError::invalid_manifest(
                plugin,
                "an `[agent]` section on a plugin that is not an agent",
            ));
        }
        if self.driver.is_some() && self.kind != PluginKind::Driver {
            return Err(PluginError::invalid_manifest(
                plugin,
                "a `[driver]` section on a plugin that is not a driver",
            ));
        }

        self.permissions.validate(plugin)
    }

    /// Does this plugin require the WebAssembly host?
    #[must_use]
    pub const fn requires_wasm(&self) -> bool {
        self.kind.runs_code()
    }

    /// Does the plugin target an interface this host provides?
    #[must_use]
    pub fn is_api_compatible(&self) -> bool {
        self.api_version.is_compatible_with(&HOST_API_VERSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete declarative agent manifest — the common case, the one that
    /// must work without the `wasm-host` feature.
    const AGENT_TOML: &str = r#"
id          = "revue-schema"
name        = "Schema review"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "agent"
description = "Reviews a schema and suggests indexes."

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
    fn versions_round_trip() {
        for raw_text in ["0.0.0", "0.1.0", "1.2.3", "10.20.30"] {
            let version: PluginVersion = raw_text.parse().expect("valid version");
            assert_eq!(version.to_string(), raw_text);
        }
    }

    #[test]
    fn an_off_template_version_is_refused() {
        for raw_text in [
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
                raw_text.parse::<PluginVersion>().is_err(),
                "{raw_text:?} should be refused"
            );
        }
    }

    #[test]
    fn at_zero_point_x_every_minor_is_a_break() {
        // The contract has no third-party implementation: a plugin built on 0.1
        // can assume nothing of 0.2.
        let wasm_host = PluginVersion::new(0, 1, 0);
        assert!(PluginVersion::new(0, 1, 0).is_compatible_with(&wasm_host));
        assert!(PluginVersion::new(0, 1, 7).is_compatible_with(&wasm_host));
        assert!(!PluginVersion::new(0, 2, 0).is_compatible_with(&wasm_host));
        assert!(!PluginVersion::new(0, 0, 9).is_compatible_with(&wasm_host));
        assert!(!PluginVersion::new(1, 1, 0).is_compatible_with(&wasm_host));
    }

    #[test]
    fn after_the_first_major_the_host_can_be_ahead_but_not_behind() {
        let wasm_host = PluginVersion::new(1, 4, 2);
        assert!(PluginVersion::new(1, 2, 0).is_compatible_with(&wasm_host));
        assert!(PluginVersion::new(1, 4, 9).is_compatible_with(&wasm_host));
        assert!(
            !PluginVersion::new(1, 5, 0).is_compatible_with(&wasm_host),
            "the host does not provide the interfaces this plugin expects"
        );
        assert!(!PluginVersion::new(2, 0, 0).is_compatible_with(&wasm_host));
    }

    // ── Identifiers and paths ───────────────────────────────────────────────

    #[test]
    fn a_plugin_identifier_cannot_designate_another_directory() {
        // This is the rule that matters: the identifier names a directory and
        // an approval key.
        for ident in [
            "",
            "..",
            "../neighbor",
            "a/b",
            "a\\b",
            "Plugin",
            "2fast",
            "plugin.toml",
            "plug in",
            "plugin\0",
        ] {
            assert!(PluginId::new(ident).is_err(), "{ident:?} should be refused");
        }
        assert!(PluginId::new("a".repeat(65)).is_err());

        for ident in ["duckdb", "revue-schema", "export_parquet", "mongo2"] {
            assert!(PluginId::new(ident).is_ok(), "{ident} should be accepted");
        }
    }

    #[test]
    fn an_entrypoint_does_not_leave_the_plugin_directory() {
        for item_path in [
            "",
            "/usr/lib/evil.wasm",
            "../neighbor/evil.wasm",
            "./evil.wasm",
            "sous/../../evil.wasm",
            "..\\evil.wasm",
            "evil.so",
            "evil",
            "evil.wasm\n",
        ] {
            assert!(
                Entrypoint::new(item_path).is_err(),
                "{item_path:?} should be refused"
            );
        }

        let entree = Entrypoint::new("build/plugin.wasm").expect("simple relative path");
        assert_eq!(
            entree.resolve(Path::new("/plugins/duckdb")),
            Path::new("/plugins/duckdb/build/plugin.wasm")
        );
    }

    // ── Permissions ─────────────────────────────────────────────────────────

    #[test]
    fn a_manifest_without_permissions_section_grants_nothing() {
        // ADR-0005. It is the test that protects the default: a permissive
        // default shows neither at compile time nor in review.
        let toml = r#"
id          = "empty"
name        = "Sans permissions"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "export"
entrypoint  = "empty.wasm"
"#;
        let manifest_toml = PluginManifest::from_toml(toml).expect("valid manifest");
        assert!(manifest_toml.permissions.grants_nothing());
        assert_eq!(
            manifest_toml.permissions.connections,
            ConnectionAccess::Denied
        );
        assert!(!manifest_toml.permissions.allows_host("example.com", 443));
        assert!(
            !manifest_toml
                .permissions
                .allows_path(Path::new("/etc/passwd"))
        );
        assert_eq!(
            manifest_toml.permissions.summary(),
            ["asks for no permission"]
        );
    }

    #[test]
    fn the_network_is_granted_host_by_host_and_port_by_port() {
        let allowed = HostPort::new("Db.Example.COM:5432").expect("valid host");
        assert_eq!(allowed.host(), "db.example.com");
        assert_eq!(allowed.port(), 5432);
        assert!(allowed.matches("DB.EXAMPLE.com", 5432));
        assert!(!allowed.matches("db.example.com", 5433));
        assert!(
            !allowed.matches("evil.db.example.com", 5432),
            "a grant does not cover subdomains"
        );
    }

    #[test]
    fn a_network_wildcard_is_refused() {
        // "network access granted host by host, port by port" (ADR-0005).
        for raw_text in [
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
            "exämple.com:443",
            "example.com:44\u{200b}3",
        ] {
            assert!(
                HostPort::new(raw_text).is_err(),
                "{raw_text:?} should be refused"
            );
        }
        assert!(HostPort::new("[::1]:6379").is_ok());
        assert_eq!(
            HostPort::new("[::1]:6379").expect("v6 address").to_string(),
            "[::1]:6379"
        );
    }

    #[test]
    fn a_relative_or_dotted_file_permission_is_refused() {
        let mut permissions = PluginPermissions {
            filesystem: vec![PathBuf::from("data")],
            ..PluginPermissions::default()
        };
        assert!(permissions.validate("x").is_err(), "relative root");

        permissions.filesystem = vec![PathBuf::from("/home/x/../../etc")];
        assert!(permissions.validate("x").is_err(), "root with `..`");

        permissions.filesystem = vec![PathBuf::from("/home/x/data")];
        permissions.validate("x").expect("absolute and clean root");
    }

    #[test]
    fn a_climbing_path_is_never_under_a_granted_root() {
        // `starts_with` alone would say yes: `/data/../../etc/passwd` does start
        // with `/data`.
        let permissions = PluginPermissions {
            filesystem: vec![PathBuf::from("/data")],
            ..PluginPermissions::default()
        };
        assert!(permissions.allows_path(Path::new("/data/report.csv")));
        assert!(!permissions.allows_path(Path::new("/data/../../etc/passwd")));
        assert!(!permissions.allows_path(Path::new("/etc/passwd")));
        assert!(!permissions.allows_path(Path::new("/database/secret")));
    }

    #[test]
    fn widened_permissions_void_the_approval() {
        let approved_perms = PluginPermissions {
            network: vec![HostPort::new("a.example:443").expect("host")],
            filesystem: vec![PathBuf::from("/data")],
            connections: ConnectionAccess::ReadOnly,
        };

        // Identical, or narrower: the approval holds.
        assert!(approved_perms.is_subset_of(&approved_perms));
        let narrower = PluginPermissions {
            network: Vec::new(),
            filesystem: vec![PathBuf::from("/data/sous-dossier")],
            connections: ConnectionAccess::Denied,
        };
        assert!(narrower.is_subset_of(&approved_perms));

        // One more host, one more root, write on top: void.
        for widened in [
            PluginPermissions {
                network: vec![
                    HostPort::new("a.example:443").expect("host"),
                    HostPort::new("exfiltration.example:443").expect("host"),
                ],
                ..approved_perms.clone()
            },
            PluginPermissions {
                filesystem: vec![PathBuf::from("/data"), PathBuf::from("/home")],
                ..approved_perms.clone()
            },
            PluginPermissions {
                connections: ConnectionAccess::ReadWrite,
                ..approved_perms.clone()
            },
            PluginPermissions {
                network: vec![HostPort::new("a.example:8443").expect("host")],
                ..approved_perms.clone()
            },
        ] {
            assert!(
                !widened.is_subset_of(&approved_perms),
                "{widened:?} widens the approval"
            );
        }
    }

    #[test]
    fn an_unknown_permission_is_refused_not_ignored() {
        // A typo, or a permission from a later version: in both cases, ignoring
        // it would make the plugin fail later and without explanation.
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
        let err = PluginManifest::from_toml(toml).expect_err("refusal expected");
        assert!(
            matches!(err, PluginError::UnreadableManifest { .. }),
            "{err}"
        );
    }

    #[test]
    fn the_summary_states_every_grant() {
        let permissions = PluginPermissions {
            network: vec![HostPort::new("a.example:443").expect("host")],
            filesystem: vec![PathBuf::from("/data")],
            connections: ConnectionAccess::ReadWrite,
        };
        let summary_lines = permissions.summary();
        assert_eq!(summary_lines.len(), 3);
        assert!(
            summary_lines[0].contains("a.example:443"),
            "{summary_lines:?}"
        );
        assert!(summary_lines[1].contains("/data"), "{summary_lines:?}");
        assert!(summary_lines[2].contains("approval"), "{summary_lines:?}");
    }

    // ── Manifest ────────────────────────────────────────────────────────────

    #[test]
    fn a_declarative_agent_parses_without_wasm_host() {
        let manifest_toml = PluginManifest::from_toml(AGENT_TOML).expect("valid manifest");
        assert_eq!(manifest_toml.id.as_str(), "revue-schema");
        assert_eq!(manifest_toml.kind, PluginKind::Agent);
        assert!(!manifest_toml.requires_wasm());
        assert!(manifest_toml.is_api_compatible());
        assert!(manifest_toml.entrypoint.is_none());

        let agent = manifest_toml.agent.as_ref().expect("agent section");
        assert_eq!(agent.name, "Schema");
        assert!(agent.allows("refresh_catalog"));
        assert!(!agent.allows("execute_query"));
    }

    #[test]
    fn an_agent_cannot_carry_an_entrypoint() {
        // Otherwise `kind = "agent"` would become the way to run code while
        // presenting itself as declarative.
        let toml = AGENT_TOML.replace(
            "kind        = \"agent\"",
            "kind        = \"agent\"\nentrypoint  = \"agent.wasm\"",
        );
        let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
        assert!(err.to_string().contains("declarative"), "{err}");
    }

    #[test]
    fn an_agent_with_tools_but_no_connection_access_is_refused() {
        let toml = AGENT_TOML.replace("connections = \"read_only\"", "");
        let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
        assert!(err.to_string().contains("inert"), "{err}");
    }

    #[test]
    fn a_surface_that_runs_code_must_declare_its_entrypoint() {
        for kind in ["driver", "export", "visualization"] {
            let toml = format!(
                "id = \"x\"\nname = \"X\"\nversion = \"1.0.0\"\n\
                 api_version = \"0.1.0\"\nkind = \"{kind}\"\n"
            );
            let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
            assert!(err.to_string().contains("entrypoint"), "{kind}: {err}");
        }
    }

    #[test]
    fn a_specific_section_must_match_the_surface() {
        let toml = DRIVER_TOML.replace("kind        = \"driver\"", "kind        = \"export\"");
        let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
        assert!(err.to_string().contains("`[driver]`"), "{err}");
    }

    #[test]
    fn a_driver_declares_its_protocol_before_any_execution() {
        let manifest_toml = PluginManifest::from_toml(DRIVER_TOML).expect("valid manifest");
        assert!(manifest_toml.requires_wasm());
        let driver = manifest_toml.driver.as_ref().expect("driver section");
        assert_eq!(driver.id, DriverId::new("duckdb").expect("identifier"));
        assert_eq!(driver.family, DriverFamily::Analytical);
        assert!(
            manifest_toml
                .permissions
                .allows_host("catalog.example.com", 443)
        );
    }

    #[test]
    fn a_manifest_without_interface_version_is_refused() {
        // Not saying what it was built against is not "compatible".
        let toml = DRIVER_TOML.replace("api_version = \"0.1.0\"\n", "");
        let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
        assert!(
            matches!(err, PluginError::UnreadableManifest { .. }),
            "{err}"
        );
    }

    #[test]
    fn toml_round_trip() {
        let manifest_toml = PluginManifest::from_toml(DRIVER_TOML).expect("valid manifest");
        let rendered = manifest_toml.to_toml().expect("render");
        let reread = PluginManifest::from_toml(&rendered).expect("reread");
        assert_eq!(reread, manifest_toml);
    }
}
