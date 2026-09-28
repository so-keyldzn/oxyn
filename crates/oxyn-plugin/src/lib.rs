//! Oxyn's plugin host: manifests, permissions, approvals.
//!
//! Three extension surfaces ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)):
//! **drivers**, **agents** and **export formats or visualizations**. Two of
//! them run third-party code and wait for phase 4; the third — agents — is
//! **declarative** and works today, without WebAssembly and without any foreign
//! instruction running.
//!
//! It is the reason for this crate's split: the common case does not pay the
//! price of the rare one. `wasmtime` is behind the `wasm-host` feature, off by
//! default, and nothing a plugin agent does depends on it.
//!
//! # What it contains
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`manifest`] | `plugin.toml`: identity, surface, permissions | ADR-0005, PLUGIN-CONTRACT |
//! | [`registry`] | discovery, states, persisted approvals | ADR-0005 |
//! | [`agent_plugin`] | declarative agents, **without WASM** | ARCHITECTURE §7.3 |
//! | [`error`] | [`PluginError`], and its projection onto the domain | — |
//! | `host` *(feature `wasm-host`)* | wasmtime engine, limits, store | PLUGIN-CONTRACT §2 |
//!
//! # The three choices that govern this crate
//!
//! **By default, nothing is granted.** A manifest without a `[permissions]`
//! section gives neither network, nor file, nor access to connections: that is
//! what the default of [`PluginPermissions`] carries. Dropping a directory
//! grants nothing either: the plugin stays [`Installed`](PluginState::Installed)
//! until the user has seen what it asks for.
//!
//! **What is not understood is refused.** A `plugin.toml` is written by a third
//! party ([SECURITY](../../../docs/SECURITY.md)). A misspelled permission, an
//! `entrypoint` that climbs up one level, an identifier that does not match the
//! directory, an older interface version: each produces a named refusal, never
//! a degraded load.
//!
//! **A plugin receives nothing it could divert.** No driver handle, no list of
//! connections, no keychain
//! ([`PLUGIN-CONTRACT` §1](../../../docs/PLUGIN-CONTRACT.md)). What a plugin
//! gets at best is the right to **emit `Command`s** — which go through the
//! `PolicyGate` like a human's (I-01,
//! [ADR-0004](../../../docs/adr/0004-command-bus.md)). `connections =
//! "read_write"` therefore does not mean its writes go through; it means it has
//! the right to propose them.
//!
//! # Example: an agent provided by a plugin, without a line of WebAssembly
//!
//! ```
//! use oxyn_plugin::{ConnectionAccess, DeclarativeAgent, PluginKind, PluginManifest};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let text = r#"
//! id          = "revue-schema"
//! name        = "Schema review"
//! version     = "1.0.0"
//! api_version = "0.1.0"
//! kind        = "agent"
//!
//! [permissions]
//! connections = "read_only"
//!
//! [agent]
//! name          = "Schema"
//! system_prompt = "You review database schemas."
//! allowed_tools = ["refresh_catalog"]
//! "#;
//!
//! let manifest_toml = PluginManifest::from_toml(text)?;
//! assert_eq!(manifest_toml.kind, PluginKind::Agent);
//!
//! // No third-party code will run: the `wasm-host` feature is not involved.
//! assert!(!manifest_toml.requires_wasm());
//!
//! let agent = DeclarativeAgent::from_manifest(&manifest_toml)?;
//! assert_eq!(agent.connections(), ConnectionAccess::ReadOnly);
//! assert!(agent.spec().allows("refresh_catalog"));
//! assert!(!agent.spec().allows("execute_query"));
//! # Ok(())
//! # }
//! ```
//!
//! To read an agent directly from a directory:
//! [`agent_plugin::load_from_dir`].

pub mod agent_plugin;
pub mod error;
pub mod manifest;
pub mod registry;

#[cfg(feature = "wasm-host")]
pub mod host;

#[cfg(test)]
mod fixtures;

pub use agent_plugin::{DeclarativeAgent, PluginAgentSpec};
pub use error::{PluginError, Result};
pub use manifest::{
    ConnectionAccess, Entrypoint, HOST_API_VERSION, HostPort, MANIFEST_FILE, PluginDriverSpec,
    PluginId, PluginKind, PluginManifest, PluginPermissions, PluginVersion,
};
pub use registry::{
    APPROVALS_FILE, ApprovalDecision, ApprovalRecord, InstalledPlugin, PluginRegistry, PluginState,
};

#[cfg(feature = "wasm-host")]
pub use host::{HostLimits, HostState, PreparedPlugin, WasmHost};

#[cfg(test)]
mod tests {
    use crate::fixtures::{TempDir, agent_toml, export_toml};

    use super::*;

    /// The full path of the crate, on the scenario that really brings it into
    /// play: an agent provided by a plugin becomes usable, and nothing else
    /// does on the way.
    #[test]
    fn the_path_of_a_plugin_provided_agent() {
        let root = TempDir::new("trajet");
        root.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));
        root.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut plugin_registry = PluginRegistry::new(root.path());
        plugin_registry.discover().expect("discovery");

        // 1. Two plugins are installed, none is authorized.
        assert_eq!(plugin_registry.len(), 2);
        assert_eq!(plugin_registry.approved().count(), 0);
        assert!(plugin_registry.declarative_agents().is_empty());

        // 2. What is shown to the user before they approve.
        let summary_lines = plugin_registry
            .require("csv")
            .expect("discovered plugin")
            .manifest()
            .expect("readable manifest")
            .permissions
            .summary();
        assert!(
            summary_lines.iter().any(|l| l.contains("a.example:443")),
            "{summary_lines:?}"
        );

        // 3. The user approves the agent only.
        plugin_registry.approve("revue").expect("approval");
        plugin_registry.save().expect("writing the approvals");

        let agents = plugin_registry.declarative_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].plugin().as_str(), "revue");
        assert_eq!(agents[0].connections(), ConnectionAccess::ReadOnly);

        // 4. The approved agent runs no code: phase 4 is not a condition for
        //    using it.
        let revue = plugin_registry.require("revue").expect("discovered plugin");
        assert!(!revue.manifest().expect("manifest").requires_wasm());

        // 5. And the other plugin got nothing.
        let csv = plugin_registry.require("csv").expect("discovered plugin");
        assert!(csv.effective_permissions().is_err());
        assert_eq!(*csv.state(), PluginState::Installed);
    }

    /// A plugin error must stay readable after its projection onto the
    /// domain: it is what the interface displays.
    #[test]
    fn a_plugin_error_keeps_its_meaning_in_the_domain() {
        let root = TempDir::new("errors");
        root.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut plugin_registry = PluginRegistry::new(root.path());
        plugin_registry.discover().expect("discovery");

        let plugin = plugin_registry.require("revue").expect("discovered plugin");
        let err = plugin
            .effective_permissions()
            .expect_err("the plugin is not approved");
        let domain = oxyn_core::OxynError::from(err);
        assert!(domain.is_user_error());
        assert!(!domain.is_retryable());
    }
}
