//! The plugin host's error.
//!
//! A single rule governs this module, and it comes from
//! [`PLUGIN-CONTRACT`](../../../docs/PLUGIN-CONTRACT.md): **a refusal is a
//! refusal**. An unreadable manifest, an older interface version, an
//! unapproved permission never produce a degraded load "to see"; they produce
//! a [`PluginError`] variant that the interface displays.
//!
//! The content of a `plugin.toml` is written by a third party
//! ([`SECURITY` §input surface](../../../docs/SECURITY.md)). Messages therefore
//! repeat the **diagnostic** — the faulty line as the TOML parser describes it
//! — but never a connection identifier, a keychain path or a secret value:
//! those have no business in a manifest anyway, and
//! [`crate::manifest::PluginPermissions`] refuses to carry them.

use std::path::PathBuf;

use oxyn_core::{IdParseError, OxynError};

use crate::manifest::PluginVersion;

/// Result of this crate's operations.
pub type Result<T> = std::result::Result<T, PluginError>;

/// What can fail between a plugin directory and a loaded component.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// The manifest text is not TOML, or does not match the expected shape.
    ///
    /// The detail comes from the parser: it names the faulty line and key,
    /// which is exactly what the plugin author needs.
    #[error("unreadable manifest: {detail}")]
    UnreadableManifest {
        /// TOML parser diagnostic.
        detail: String,
    },

    /// The manifest parses but violates a rule of the plugin contract.
    ///
    /// It is the case of a declarative agent carrying an entrypoint, of a
    /// driver carrying none, or of a plugin whose directory does not carry its
    /// identifier.
    #[error("invalid manifest for `{plugin}`: {detail}")]
    InvalidManifest {
        /// Plugin identifier, or the name of its directory if it could not be
        /// read.
        plugin: String,
        /// What is refused, and why.
        detail: String,
    },

    /// An identifier of the manifest is malformed.
    ///
    /// The faulty value is **never** repeated in the message: it is the
    /// guarantee [`IdParseError`] carries, and it holds here too.
    #[error("invalid identifier: {0}")]
    InvalidId(#[from] IdParseError),

    /// The plugin directory, or one of its subdirectories, is not readable.
    #[error("`{}` unreadable: {source}", path.display())]
    Directory {
        /// The path at fault.
        path: PathBuf,
        /// The file system error.
        #[source]
        source: std::io::Error,
    },

    /// The approvals file could not be read or written.
    ///
    /// It is not a benign error: without it, every plugin falls back to the
    /// [`Installed`](crate::registry::PluginState::Installed) state and nothing
    /// loads. That is the point of the refusal — losing the approvals must
    /// show.
    #[error("plugin approvals unreadable or not written: {detail}")]
    Approvals {
        /// What failed.
        detail: String,
    },

    /// The plugin was not approved by the user.
    ///
    /// An installed plugin is not an authorized plugin: its permissions must be
    /// presented to the user, and accepted, before any load (ADR-0005).
    #[error(
        "the plugin `{plugin}` is not approved: its permissions must be \
         presented to the user before any load"
    )]
    NotApproved {
        /// The refused plugin.
        plugin: String,
    },

    /// The plugin asks for more than what the user had approved.
    ///
    /// It is the case of an update that adds a network host or write access to
    /// connections. The approval is **void**: it is not silently extended.
    #[error("the permissions of `{plugin}` changed since the approval: {detail}")]
    ApprovalStale {
        /// The plugin whose approval is void.
        plugin: String,
        /// What was added compared with the approval.
        detail: String,
    },

    /// The plugin targets an interface version this host does not provide.
    ///
    /// [`PLUGIN-CONTRACT` §3](../../../docs/PLUGIN-CONTRACT.md): a plugin built
    /// against another version is refused with a clear message, never loaded
    /// "to see".
    #[error(
        "the plugin `{plugin}` targets interface {declared}, this version of Oxyn \
         provides {host}"
    )]
    IncompatibleInterface {
        /// The refused plugin.
        plugin: String,
        /// What the manifest declares.
        declared: PluginVersion,
        /// What the host provides.
        host: PluginVersion,
    },

    /// The component asks at runtime for an access its manifest did not
    /// declare.
    ///
    /// The sandbox makes the attempt harmless; this error makes it **visible**,
    /// which the sandbox alone does not.
    #[error("permission denied to `{plugin}`: {detail}")]
    PermissionDenied {
        /// The plugin at fault.
        plugin: String,
        /// The denied access.
        detail: String,
    },

    /// No plugin of this name is installed.
    #[error("no plugin `{plugin}` is installed")]
    Unknown {
        /// The searched name.
        plugin: String,
    },

    /// The plugin needs the WebAssembly host, missing from this build.
    ///
    /// A declarative agent never needs it — it is the common case, and it works
    /// without the `wasm-host` feature.
    #[error(
        "the plugin `{plugin}` requires the WebAssembly host, missing from this \
         build of Oxyn (feature `wasm-host`)"
    )]
    WasmHostUnavailable {
        /// The plugin that cannot be loaded.
        plugin: String,
    },

    /// The WebAssembly host failed: engine, component compilation, store.
    #[cfg(feature = "wasm-host")]
    #[error("WebAssembly host: {detail}")]
    WasmHost {
        /// The wasmtime diagnostic.
        detail: String,
    },
}

impl PluginError {
    /// Builds a manifest refusal.
    #[must_use]
    pub fn invalid_manifest(plugin: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::InvalidManifest {
            plugin: plugin.into(),
            detail: detail.into(),
        }
    }

    /// Does the error call for a user decision — approve, reapprove — rather
    /// than stem from a defect of the plugin or the host?
    ///
    /// The interface uses it to choose between "this plugin awaits your
    /// approval" and "this plugin is broken".
    #[must_use]
    pub const fn needs_user_decision(&self) -> bool {
        matches!(self, Self::NotApproved { .. } | Self::ApprovalStale { .. })
    }
}

impl From<PluginError> for OxynError {
    /// Projects the error onto the domain vocabulary, keeping its **meaning**,
    /// not its wording.
    ///
    /// What awaits a user decision becomes
    /// [`ApprovalRequired`](OxynError::ApprovalRequired) and not `Config`: a
    /// plugin awaiting approval must not display as an incident. A permission
    /// refusal stays a [`PolicyDenied`](OxynError::PolicyDenied) — the product
    /// is doing its job.
    fn from(err: PluginError) -> Self {
        let message = err.to_string();
        match err {
            PluginError::NotApproved { .. } | PluginError::ApprovalStale { .. } => {
                Self::ApprovalRequired { reason: message }
            }
            PluginError::PermissionDenied { .. } => Self::PolicyDenied { reason: message },
            PluginError::WasmHostUnavailable { .. } | PluginError::IncompatibleInterface { .. } => {
                Self::NotSupported {
                    capability: message,
                }
            }
            PluginError::Directory { source, .. } => Self::Io(source),
            PluginError::UnreadableManifest { .. } | PluginError::InvalidId(_) => {
                Self::Serialization(message)
            }
            _ => Self::Config(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_approval_is_not_an_incident() {
        // A plugin awaiting an approval must display as a possible action, not
        // as an Oxyn failure.
        let err = PluginError::NotApproved {
            plugin: "hello".to_owned(),
        };
        assert!(err.needs_user_decision());

        let domaine: OxynError = err.into();
        assert!(matches!(domaine, OxynError::ApprovalRequired { .. }));
        assert!(domaine.is_user_error());
        assert!(!domaine.is_retryable());
    }

    #[test]
    fn a_stale_approval_asks_for_a_decision() {
        let err = PluginError::ApprovalStale {
            plugin: "hello".to_owned(),
            detail: "a network host was added".to_owned(),
        };
        assert!(err.needs_user_decision());
        assert!(matches!(
            OxynError::from(err),
            OxynError::ApprovalRequired { .. }
        ));
    }

    #[test]
    fn a_permission_refusal_stays_a_refusal() {
        let err = PluginError::PermissionDenied {
            plugin: "hello".to_owned(),
            detail: "host `exfiltration.example:443` not granted".to_owned(),
        };
        assert!(!err.needs_user_decision());

        let domaine: OxynError = err.into();
        assert!(
            matches!(domaine, OxynError::PolicyDenied { .. }),
            "a refusal displayed as an internal incident reads as an Oxyn bug"
        );
    }

    #[test]
    fn an_incompatible_interface_is_a_missing_capability() {
        let err = PluginError::IncompatibleInterface {
            plugin: "hello".to_owned(),
            declared: PluginVersion::new(0, 9, 0),
            host: PluginVersion::new(0, 1, 0),
        };
        let rendu = err.to_string();
        assert!(rendu.contains("0.9.0"), "{rendu}");
        assert!(rendu.contains("0.1.0"), "{rendu}");
        assert!(matches!(
            OxynError::from(err),
            OxynError::NotSupported { .. }
        ));
    }

    #[test]
    fn a_faulty_identifier_is_not_copied_into_the_message() {
        // The guarantee of `IdParseError` goes through the conversion: a third
        // party's manifest does not dictate the content of a log.
        let err = PluginError::InvalidId(IdParseError::new(
            "PluginId",
            "allowed characters: a-z, 0-9, `-`, `_`",
        ));
        let rendu = err.to_string();
        assert!(rendu.contains("PluginId"), "{rendu}");
        assert!(matches!(OxynError::from(err), OxynError::Serialization(_)));
    }
}
