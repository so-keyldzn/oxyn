//! What Oxyn does to keep a known agent from acting on the machine by itself.
//!
//! # Why asking is not enough
//!
//! [`super::permission_for`] refuses every file, command and network access an
//! agent **asks** for. An agent asks only when its own mode says so. Measured on
//! 2026-09-23 with the pinned adapters: in its default mode, Claude Agent runs a
//! shell command without a single `session/request_permission`, and Codex does
//! the same for anything its sandbox allows, in every mode. A refusal nobody
//! asks for protects nothing; a table name shaped like an instruction was
//! enough to have a command run.
//!
//! So each known adapter is **confined at launch**, with the switches its own
//! documentation or source gives, and is kept in its most restrictive mode for
//! the whole session. What each switch does, and where it is written, is in
//! [RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md) (I-12).
//!
//! # Claude Agent: nothing but Oxyn's tools
//!
//! `_meta.claudeCode.options` is spread into the Agent SDK's options when the
//! session is created (`acp-agent.js` of 0.78.0). `tools: []` removes every
//! built-in tool — shell, files, web; `strictMcpConfig` keeps only the MCP
//! servers the client declares, which is Oxyn's; `settingSources: []` loads
//! neither the user's permission rules, hooks nor MCP servers; and
//! `allowDangerouslySkipPermissions: false` takes the bypass mode out of the
//! catalog. What remains is Oxyn's registry, each call a `Command` through the
//! `PolicyGate` (ADR-0030).
//!
//! # Codex: its shell and web search off, read-only
//!
//! `CODEX_CONFIG` is merged into the session's configuration and
//! `INITIAL_AGENT_MODE` picks the first mode (the adapter's README). Codex
//! keeps its file edits, which ask and are refused. The MCP servers and
//! plugins of the configuration layers Oxyn can read — system, user, managed
//! ([`codex_config_layers`]) — are turned off **by name**, since no single switch
//! does it: one enabled by default and named nowhere escapes (ADR-0032), and
//! so does what macOS MDM or the organization's cloud declares (ADR-0033),
//! rather than this module pretending.

use std::sync::atomic::{AtomicU8, Ordering};

use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOptionCategory, SessionUpdate,
};
use serde_json::{Map, Value, json};

use super::mcp;
use super::presets::{self, AgentPreset};
use oxyn_core::ExternalAgentConfig;

mod codex_layers;

pub use codex_layers::{Layer, codex_config_layers, managed_turns_on};

/// How one known adapter is confined.
#[derive(Clone, PartialEq)]
pub struct Confinement {
    /// Added to the process environment, after the declared variables: a
    /// declaration cannot loosen what is set here.
    pub env: Vec<(&'static str, String)>,
    /// The `_meta` of `session/new`.
    pub session_meta: Option<Map<String, Value>>,
    /// The mode set right after the session opens, before any prompt. The
    /// user is offered no other: the mode selector is withheld.
    pub mode: &'static str,
    /// Oxyn's tool server is declared in `env`, not in `session/new`; its
    /// token then travels in [`TOOL_TOKEN_VAR`], set by the caller.
    pub tools_in_env: bool,
}

/// Written by hand, names only: `env` holds values, and the next one added
/// may be a secret ([I-03](../../../../CLAUDE.md#i-03)).
impl std::fmt::Debug for Confinement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Confinement")
            .field(
                "env",
                &self.env.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            )
            .field("mode", &self.mode)
            .field("tools_in_env", &self.tools_in_env)
            .finish_non_exhaustive()
    }
}

/// The variable a confined agent reads the tool server's token from, when
/// the server is declared in its configuration. The token itself is never in
/// a [`Confinement`].
pub const TOOL_TOKEN_VAR: &str = "OXYN_TOOL_TOKEN";

/// Why a known agent is not started: Oxyn could not confine it as measured,
/// and a known agent is not started half-confined.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Unconfinable {
    /// The Codex configuration exists but could not be read whole.
    #[error(
        "Oxyn could not read Codex's configuration, so it cannot switch off the tools \
         installed there; Codex was not started"
    )]
    UnreadableConfig,
    /// More servers or plugins than Oxyn switches off.
    #[error(
        "Codex's configuration names more servers or plugins than Oxyn switches off; \
         Codex was not started"
    )]
    TooManyEntries,
    /// The user's configuration already has a server with Oxyn's name, which
    /// would be merged into Oxyn's own, approval included.
    #[error(
        "Codex's configuration already declares an MCP server named `oxyn`; rename it \
         to let Oxyn start Codex"
    )]
    NameTaken,
}

/// The confinement for this declaration, when it runs a known adapter at the
/// pinned version Oxyn measured ([`presets::pinned_preset_of`]).
///
/// `tool_url` is where Oxyn serves its tools to this session, if it does.
/// `codex_layers` returns the layers of Codex's configuration Oxyn reads; it
/// is called for Codex only — [`codex_config_layers`] in production.
///
/// `Ok(None)` for any other agent: Oxyn does not know its switches.
///
/// # Errors
/// [`Unconfinable`] when a known agent cannot be confined as measured.
pub fn confinement_for(
    agent: &ExternalAgentConfig,
    tool_url: Option<&str>,
    codex_layers: impl FnOnce() -> Result<Vec<Layer>, Unconfinable>,
) -> Result<Option<Confinement>, Unconfinable> {
    match presets::pinned_preset_of(agent) {
        Some(preset) => of_preset(preset, tool_url, codex_layers),
        None => Ok(None),
    }
}

fn of_preset(
    preset: AgentPreset,
    tool_url: Option<&str>,
    codex_layers: impl FnOnce() -> Result<Vec<Layer>, Unconfinable>,
) -> Result<Option<Confinement>, Unconfinable> {
    if preset == presets::CLAUDE_CODE {
        let meta = json!({
            "claudeCode": {
                "options": {
                    "tools": [],
                    // Oxyn's own server, named in `session/new`: its calls
                    // are reviewed by the `PolicyGate`, not by this agent's
                    // prompt, which would ask Oxyn and be refused.
                    // Derived from the server's name, never from a list of
                    // tools: a tool added to the registry is covered.
                    "allowedTools": [mcp::claude_permission_rule()],
                    "strictMcpConfig": true,
                    "settingSources": [],
                    "allowDangerouslySkipPermissions": false,
                }
            }
        });
        return Ok(Some(Confinement {
            env: Vec::new(),
            session_meta: meta.as_object().cloned(),
            // « Manual — always ask before making changes ».
            mode: "default",
            tools_in_env: false,
        }));
    }
    if preset == presets::CODEX {
        let mut config = json!({
            "features": {
                "shell_tool": false,
                "unified_exec": false,
                "hooks": false,
                "apps": false,
            },
            "web_search": "disabled",
        });
        // No key turns the user's own servers and plugins off at once: each is
        // turned off by name, across the layers Oxyn can read. A plugin
        // enabled by default and named nowhere escapes this — the gap ADR-0032
        // writes down —, and so does what a layer Oxyn cannot read declares
        // (ADR-0033). A file that cannot be read whole stops the launch rather
        // than switching nothing.
        let codex_layers::Entries {
            servers, plugins, ..
        } = codex_layers::entries(&codex_layers()?)?;
        // `CODEX_CONFIG` is merged: an entry of the user's named like Oxyn's
        // would lend its command to Oxyn's approval.
        if tool_url.is_some() && servers.iter().any(|name| name == mcp::SERVER_NAME) {
            return Err(Unconfinable::NameTaken);
        }
        let mut mcp_servers: Map<String, Value> = servers
            .into_iter()
            .map(|name| (name, json!({ "enabled": false })))
            .collect();
        // Read-only asks before every MCP call, and the approval setting is
        // not applied to a server declared over ACP (measured 2026-09-23):
        // declared here, Oxyn's server is approved up front, its calls being
        // reviewed by the `PolicyGate` rather than by a prompt Oxyn would
        // have to refuse.
        if let Some(url) = tool_url {
            // Approval is set for the whole server, never tool by tool: a tool
            // added to the registry is approved with the others.
            mcp_servers.insert(
                mcp::SERVER_NAME.to_owned(),
                json!({
                    "url": url,
                    "bearer_token_env_var": TOOL_TOKEN_VAR,
                    "default_tools_approval_mode": "approve",
                }),
            );
        }
        if let Some(object) = config.as_object_mut() {
            if !mcp_servers.is_empty() {
                object.insert("mcp_servers".to_owned(), Value::Object(mcp_servers));
            }
            if !plugins.is_empty() {
                let plugins: Map<String, Value> = plugins
                    .into_iter()
                    .map(|name| (name, json!({ "enabled": false })))
                    .collect();
                object.insert("plugins".to_owned(), Value::Object(plugins));
            }
        }
        return Ok(Some(Confinement {
            env: vec![
                ("CODEX_CONFIG", config.to_string()),
                ("INITIAL_AGENT_MODE", "read-only".to_owned()),
            ],
            session_meta: None,
            mode: "read-only",
            tools_in_env: tool_url.is_some(),
        }));
    }
    Ok(None)
}

/// Watches a confined session for the agent leaving the mode it was put in.
///
/// Armed once the mode is set: before that, what the agent announces is the
/// mode it started in, which is exactly what is being replaced.
#[derive(Debug, Default)]
pub(crate) struct ModeWatch {
    state: AtomicU8,
}

const IDLE: u8 = 0;
const ARMED: u8 = 1;
const LEFT: u8 = 2;

impl ModeWatch {
    /// The mode is set: from now on, any other one is a departure.
    pub(crate) fn arm(&self) {
        let _ = self
            .state
            .compare_exchange(IDLE, ARMED, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Whether the agent left its mode since it was armed. Never reset: an
    /// agent that switched once is not trusted to stay switched back.
    pub(crate) fn left(&self) -> bool {
        self.state.load(Ordering::SeqCst) == LEFT
    }

    /// Reads one update from the agent against `mode`. Says whether this is
    /// the update that left it, so the turn in progress can be stopped at once.
    pub(crate) fn observe(&self, mode: &str, update: &SessionUpdate) -> bool {
        let elsewhere = match update {
            SessionUpdate::CurrentModeUpdate(current) => {
                current.current_mode_id.to_string() != mode
            }
            SessionUpdate::ConfigOptionUpdate(options) => {
                options.config_options.iter().any(|option| {
                    option.category == Some(SessionConfigOptionCategory::Mode)
                        && match &option.kind {
                            SessionConfigKind::Select(select) => {
                                select.current_value.to_string() != mode
                            }
                            // A mode that is not a list is not one Oxyn set.
                            _ => true,
                        }
                })
            }
            _ => false,
        };
        elsewhere
            && self
                .state
                .compare_exchange(ARMED, LEFT, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
    }
}

#[cfg(test)]
mod tests;
