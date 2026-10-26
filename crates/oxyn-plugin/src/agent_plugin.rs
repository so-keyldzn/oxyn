//! Agents provided by plugins — **declarative, and without WebAssembly**.
//!
//! It is the common case, and it works today: an agent is "a configuration,
//! not a separate implementation"
//! ([ARCHITECTURE §7.3](../../../docs/ARCHITECTURE.md)) — a system prompt, a
//! subset of tools, an output schema. None of this requires running
//! third-party code, so none of it waits for phase 4 or the `wasm-host`
//! feature.
//!
//! # What a declaration cannot contain
//!
//! The list matters as much as the structure, because it is **enforced**:
//! [`PluginAgentSpec`] refuses any key it does not know, rather than ignoring
//! it. A manifest that wrote `privacy = "sampled"` or `api_key = "…"` is
//! therefore rejected, not silently stripped.
//!
//! * **no connection, no session** — they come from what the user opened,
//!   never from the manifest;
//! * **no privacy tier** — it is attached to the connection and to nothing
//!   else ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md), I-04). An
//!   agent declaring its own would make the connection's setting inoperative;
//! * **no endpoint, no key** — a plugin does not get a network channel by way
//!   of an agent (I-03);
//! * **no agent identifier** — it is assigned by the host. An
//!   [`AgentId`](oxyn_core::AgentId) chosen by a third party could be confused
//!   with that of a built-in agent, and it is this pair that identifies the
//!   author of a command in the audit journal.
//!
//! # What a declaration cannot grant
//!
//! `allowed_tools` **restricts**, it never extends: the tool registry of
//! `oxyn-ai` stays the authority, and a tool it does not know fails validation
//! there. Here, the **form** is checked — that is, what can be checked without
//! knowing the registry.
//!
//! # Why this type is not `oxyn_ai::AgentSpec`
//!
//! `oxyn-plugin` does not depend on `oxyn-ai`, and must not: the plugin host
//! has no need to know an agent runtime exists. [`PluginAgentSpec`] is
//! therefore the **file form**; the conversion to `oxyn_ai::AgentSpec`, with
//! the assignment of the identifier and validation against the real tool
//! registry, belongs to `oxyn-desktop`, which knows both.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{PluginError, Result};
use crate::manifest::{ConnectionAccess, MANIFEST_FILE, PluginId, PluginKind, PluginManifest};

/// The agent declaration a `plugin.toml` carries, `[agent]` section.
///
/// `deny_unknown_fields` is what makes the module's list hold: an unknown key
/// is refused, so there is no field a manifest can slip in hoping a later
/// version will honor it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAgentSpec {
    /// Displayable name of the agent, in the conversation picker.
    pub name: String,

    /// What the agent does, for the user who picks it.
    ///
    /// **Not sent to the model**: [`system_prompt`](Self::system_prompt) is
    /// what addresses it.
    #[serde(default)]
    pub description: String,

    /// The system prompt. In English: it is code text.
    pub system_prompt: String,

    /// The requested tools, by their name in the `oxyn-ai` registry.
    ///
    /// An empty list is legitimate and means **no tool**: an agent that only
    /// comments on a schema has nothing to run.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Maximum number of model → tools → model round trips.
    ///
    /// Absent, the default of `oxyn-ai` applies. The **ceiling** lives there
    /// too and is not copied here: two copies of the same limit diverge, and
    /// the more permissive then wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<usize>,

    /// JSON schema of the expected response, when the agent must produce a
    /// structure rather than prose.
    ///
    /// Carried **opaque**: `oxyn-plugin` does not depend on a JSON parser, and
    /// validating it here would force adding one to decide nothing. It is
    /// `oxyn-ai` that parses it, at the moment it also knows what to do with
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
}

impl PluginAgentSpec {
    /// Declares a minimal agent: a name, a prompt, no tool.
    #[must_use]
    pub fn new(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            system_prompt: system_prompt.into(),
            allowed_tools: Vec::new(),
            max_turns: None,
            output_schema: None,
        }
    }

    /// Gives the description shown to the user.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Requests tools, by name.
    #[must_use]
    pub fn with_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Is this tool requested by this declaration?
    ///
    /// Answering `true` does not mean the tool will be granted: the `oxyn-ai`
    /// registry has the last word.
    #[must_use]
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|name| name == tool)
    }

    /// Checks what can be checked without knowing the tool registry.
    ///
    /// Four refusals:
    ///
    /// 1. an empty name or prompt — the agent would be unusable and
    ///    undisplayable;
    /// 2. a tool name outside the `[a-z][a-z0-9_]*` grammar — a tool name is a
    ///    technical key, and accepting any byte would invite using it as a
    ///    channel;
    /// 3. a tool declared twice — either a mistake, or an attempt to make the
    ///    list unreadable in the approval screen;
    /// 4. `max_turns = 0` — the agent could never answer.
    ///
    /// # Errors
    /// [`PluginError::InvalidManifest`], naming the broken rule.
    pub fn validate(&self, plugin: &str) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(PluginError::invalid_manifest(
                plugin,
                "`agent.name` is empty",
            ));
        }
        if self.system_prompt.trim().is_empty() {
            return Err(PluginError::invalid_manifest(
                plugin,
                "`agent.system_prompt` is empty",
            ));
        }
        if self.max_turns == Some(0) {
            return Err(PluginError::invalid_manifest(
                plugin,
                "`agent.max_turns` is zero: the agent could never answer",
            ));
        }
        for (rank, tool_decl) in self.allowed_tools.iter().enumerate() {
            if !is_tool_name(tool_decl) {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    "a tool name follows the `[a-z][a-z0-9_]*` grammar",
                ));
            }
            if self
                .allowed_tools
                .iter()
                .take(rank)
                .any(|seen| seen == tool_decl)
            {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    format!("the tool `{tool_decl}` is declared twice"),
                ));
            }
        }
        Ok(())
    }
}

/// A tool name is a technical key, not free text.
fn is_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A declarative agent ready to be handed to the runtime, with the identity of
/// the plugin that provides it.
///
/// The identity travels with the declaration because the audit journal needs
/// it: "this `UPDATE` comes from the *Schema* agent provided by the
/// *revue-schema* plugin" is a sentence the user must be able to read
/// afterwards. A bare `AgentSpec` would not allow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarativeAgent {
    plugin: PluginId,
    plugin_name: String,
    connections: ConnectionAccess,
    spec: PluginAgentSpec,
}

impl DeclarativeAgent {
    /// Extracts the agent from an already validated manifest.
    ///
    /// # Errors
    /// [`PluginError::InvalidManifest`] if the manifest is not of type
    /// [`Agent`](PluginKind::Agent), or if it carries no `[agent]` section.
    /// Both cases are already refused by [`PluginManifest::validate`]; the
    /// check is repeated here because nothing guarantees a caller went through
    /// it.
    pub fn from_manifest(manifest: &PluginManifest) -> Result<Self> {
        if manifest.kind != PluginKind::Agent {
            return Err(PluginError::invalid_manifest(
                manifest.id.as_str(),
                format!(
                    "this plugin is of type `{}`: it does not provide an agent",
                    manifest.kind
                ),
            ));
        }
        let Some(spec) = &manifest.agent else {
            return Err(PluginError::invalid_manifest(
                manifest.id.as_str(),
                "an `agent` plugin must carry an `[agent]` section",
            ));
        };
        spec.validate(manifest.id.as_str())?;

        Ok(Self {
            plugin: manifest.id.clone(),
            plugin_name: manifest.name.clone(),
            connections: manifest.permissions.connections,
            spec: spec.clone(),
        })
    }

    /// The plugin that provides this agent.
    #[must_use]
    pub const fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// The name of the plugin, displayable.
    #[must_use]
    pub fn plugin_name(&self) -> &str {
        &self.plugin_name
    }

    /// The declaration itself.
    #[must_use]
    pub const fn spec(&self) -> &PluginAgentSpec {
        &self.spec
    }

    /// What the plugin has the right to **ask** of databases.
    ///
    /// The `PolicyGate` alone decides what goes through: this value only bounds
    /// what the agent can propose (I-01).
    #[must_use]
    pub const fn connections(&self) -> ConnectionAccess {
        self.connections
    }
}

/// Reads a declarative agent from a directory's `plugin.toml`.
///
/// No code runs and the WebAssembly host is not called on: it is exactly what
/// phase 0 already knows how to do of the plugin → agent path.
///
/// **Synchronous, and does I/O.** Do not call it from the interface thread
/// (I-05).
///
/// # Errors
/// [`PluginError::Directory`] if the file is missing or unreadable;
/// [`PluginError::UnreadableManifest`] or [`PluginError::InvalidManifest`]
/// depending on what the manifest breaks.
pub fn load_from_dir(plugin_dir: &Path) -> Result<DeclarativeAgent> {
    let item_path = plugin_dir.join(MANIFEST_FILE);
    let text = match fs::read_to_string(&item_path) {
        Ok(text) => text,
        Err(source) => {
            return Err(PluginError::Directory {
                path: item_path,
                source,
            });
        }
    };
    let manifest = PluginManifest::from_toml(&text)?;
    DeclarativeAgent::from_manifest(&manifest)
}

#[cfg(test)]
mod tests;
