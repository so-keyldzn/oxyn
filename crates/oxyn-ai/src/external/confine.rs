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
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::CurrentModeUpdate;

    const CLAUDE: &str = "@agentclientprotocol/claude-agent-acp@0.78.0";
    const CODEX: &str = "@agentclientprotocol/codex-acp@1.12.0";
    const URL: &str = "http://127.0.0.1:4711/mcp";

    fn declared(args: &[&str]) -> ExternalAgentConfig {
        let mut agent = ExternalAgentConfig::new(
            oxyn_core::ProviderId::new("agent").expect("a valid identifier"),
            "Agent",
            "npx",
        );
        agent.args = args.iter().map(|&arg| arg.to_owned()).collect();
        agent
    }

    fn none() -> Result<Vec<Layer>, Unconfinable> {
        Ok(Vec::new())
    }

    fn confined(
        agent: &ExternalAgentConfig,
        tool_url: Option<&str>,
        user: Option<&str>,
    ) -> Result<Confinement, Unconfinable> {
        confinement_for(agent, tool_url, || {
            Ok(user.map(Layer::user).into_iter().collect())
        })
        .map(|found| found.expect("a known adapter is confined"))
    }

    fn codex_config(confinement: &Confinement) -> Value {
        confinement
            .env
            .iter()
            .find(|(name, _)| *name == "CODEX_CONFIG")
            .map(|(_, value)| serde_json::from_str::<Value>(value).expect("valid JSON"))
            .expect("the configuration is set")
    }

    #[test]
    fn a_mode_left_after_arming_is_seen_once_and_kept() {
        let watch = ModeWatch::default();
        let to = |id: &str| SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(id.to_owned()));
        // Before arming, the starting mode is announced: not a departure.
        assert!(!watch.observe("default", &to("auto")));
        assert!(!watch.left());
        watch.arm();
        assert!(!watch.observe("default", &to("default")));
        assert!(
            watch.observe("default", &to("bypassPermissions")),
            "the departure"
        );
        assert!(watch.left());
        // Said once, and coming back does not undo it.
        assert!(!watch.observe("default", &to("default")));
        assert!(watch.left());
    }

    #[test]
    fn claude_agent_keeps_no_tool_of_its_own() {
        let confinement = confined(&declared(&["-y", CLAUDE]), Some(URL), None).expect("confined");
        let options = &confinement
            .session_meta
            .clone()
            .expect("options are passed")["claudeCode"]["options"];
        assert_eq!(options["tools"], json!([]));
        assert_eq!(options["allowedTools"], json!(["mcp__oxyn"]));
        assert_eq!(options["strictMcpConfig"], json!(true));
        assert_eq!(options["settingSources"], json!([]));
        assert_eq!(options["allowDangerouslySkipPermissions"], json!(false));
        assert_eq!(confinement.mode, "default");
        assert!(!confinement.tools_in_env, "declared over ACP");
    }

    /// Every tool of the registry is reachable by both confined agents, and
    /// neither confinement names a tool: a tool added to the registry is
    /// covered without touching this file. Named one by one, the forgotten
    /// tool would be refused in silence — the agent would just not use it.
    #[test]
    fn every_registry_tool_passes_both_confinements_without_being_named() {
        let tools = crate::tools::ToolRegistry::builtin().names();
        assert!(tools.contains(&crate::tools::DESCRIBE_SCHEMA));

        let claude = confined(&declared(&["-y", CLAUDE]), Some(URL), None).expect("confined");
        let rules: Vec<String> = claude.session_meta.clone().expect("options are passed")
            ["claudeCode"]["options"]["allowedTools"]
            .as_array()
            .expect("a list of rules")
            .iter()
            .filter_map(|rule| rule.as_str().map(str::to_owned))
            .collect();
        for tool in &tools {
            let name = mcp::claude_tool_name(tool);
            assert!(
                rules
                    .iter()
                    .any(|rule| *rule == name || name.starts_with(&format!("{rule}__"))),
                "{name} is not allowed by {rules:?}"
            );
            assert!(
                rules.iter().all(|rule| !rule.ends_with(tool)),
                "the rule names the tool {tool}: {rules:?}"
            );
        }

        let codex = confined(&declared(&["-y", CODEX]), Some(URL), None).expect("confined");
        let server = &codex_config(&codex)["mcp_servers"][mcp::SERVER_NAME];
        assert_eq!(server["default_tools_approval_mode"], json!("approve"));
        for per_tool in ["enabled_tools", "disabled_tools", "tools"] {
            assert_eq!(
                server[per_tool],
                Value::Null,
                "Codex approves the server as a whole, not a list of tools"
            );
        }
    }

    #[test]
    fn codex_runs_without_shell_nor_web_and_reaches_oxyns_tools_only_by_its_config() {
        let agent = declared(&["-y", CODEX]);
        let confinement = confined(&agent, Some(URL), None).expect("confined");
        let config = codex_config(&confinement);
        assert_eq!(config["features"]["shell_tool"], json!(false));
        assert_eq!(config["features"]["unified_exec"], json!(false));
        assert_eq!(config["features"]["hooks"], json!(false));
        assert_eq!(config["web_search"], json!("disabled"));
        assert!(
            confinement
                .env
                .contains(&("INITIAL_AGENT_MODE", "read-only".to_owned()))
        );
        assert_eq!(confinement.mode, "read-only");
        let server = &config["mcp_servers"]["oxyn"];
        assert_eq!(server["url"], json!(URL));
        assert_eq!(server["bearer_token_env_var"], json!(TOOL_TOKEN_VAR));
        assert!(confinement.tools_in_env);

        let without = confined(&agent, None, None).expect("confined without tools too");
        assert_eq!(codex_config(&without)["mcp_servers"], Value::Null);
        assert!(!without.tools_in_env);
    }

    #[test]
    fn the_users_own_codex_servers_and_plugins_are_turned_off_by_name_only() {
        let agent = declared(&["-y", CODEX]);
        let user = r#"
            model = "whatever"
            [mcp_servers.supabase]
            command = "npx"
            env = { SUPABASE_ACCESS_TOKEN = "sbp_SECRET-TOKEN" }
            [plugins."computer-use@openai-bundled"]
            enabled = true
        "#;
        let confinement = confined(&agent, Some(URL), Some(user)).expect("confined");
        let config = codex_config(&confinement);
        assert_eq!(
            config["mcp_servers"]["supabase"],
            json!({ "enabled": false })
        );
        assert_eq!(
            config["plugins"]["computer-use@openai-bundled"],
            json!({ "enabled": false })
        );
        // Names only: nothing of the user's values travels (I-03).
        assert!(
            !confinement
                .env
                .iter()
                .any(|(_, value)| value.contains("SECRET-TOKEN"))
        );
        // And the `Debug` shows no value at all.
        assert!(!format!("{confinement:?}").contains("features"));
    }

    #[test]
    fn a_codex_configuration_that_cannot_be_read_whole_stops_the_launch() {
        let agent = declared(&["-y", CODEX]);
        assert_eq!(
            confined(&agent, None, Some("[[not toml")),
            Err(Unconfinable::UnreadableConfig)
        );
        let crowded: String = (0..=codex_layers::MAX_ENTRIES)
            .map(|index| format!("[mcp_servers.s{index}]\ncommand = \"x\"\n"))
            .collect();
        assert_eq!(
            confined(&agent, None, Some(&crowded)),
            Err(Unconfinable::TooManyEntries)
        );
        assert_eq!(
            confinement_for(&agent, None, || Err(Unconfinable::UnreadableConfig)),
            Err(Unconfinable::UnreadableConfig)
        );
    }

    #[test]
    fn a_codex_configuration_nested_past_reason_is_refused_not_a_stack_overflow() {
        // `toml` 0.9 stops at 80 levels unless its `unbounded` feature is on;
        // this fails, by aborting, the day a dependency turns it on.
        let deep = format!("a = {}", "[".repeat(200_000));
        assert_eq!(
            confined(&declared(&["-y", CODEX]), None, Some(&deep)),
            Err(Unconfinable::UnreadableConfig)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_special_file_in_place_of_the_codex_configuration_is_refused_before_opening() {
        // A socket, because the standard library makes one: a FIFO — whose
        // opening would block forever — takes a spawn or `mknodat`, which
        // rustix lacks on macOS. Both are refused by the same `stat`.
        let home = std::env::temp_dir().join(format!("oxyn-codex-sock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("scratch CODEX_HOME");
        let _socket = std::os::unix::net::UnixListener::bind(home.join("config.toml"))
            .expect("a socket where the configuration goes");
        let mut agent = declared(&["-y", CODEX]);
        agent.env = vec![("CODEX_HOME".to_owned(), home.display().to_string())];
        assert_eq!(
            codex_config_layers(&agent).map(|layers| layers.len()),
            Err(Unconfinable::UnreadableConfig)
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_user_server_named_like_oxyns_is_refused_with_tools_and_turned_off_without() {
        let agent = declared(&["-y", CODEX]);
        let user = "[mcp_servers.oxyn]\ncommand = \"not-ours\"\n";
        assert_eq!(
            confined(&agent, Some(URL), Some(user)),
            Err(Unconfinable::NameTaken)
        );
        let without = confined(&agent, None, Some(user)).expect("confined");
        assert_eq!(
            codex_config(&without)["mcp_servers"]["oxyn"],
            json!({ "enabled": false })
        );
    }

    #[test]
    fn only_the_measured_version_with_its_own_arguments_is_confined() {
        for args in [
            vec!["my-agent", "--acp"],
            // Another version was not measured.
            vec!["-y", "@agentclientprotocol/codex-acp@2.0.0"],
            vec!["-y", "@agentclientprotocol/codex-acp"],
            // An argument more may undo the confinement.
            vec!["-y", CODEX, "-c", "sandbox_mode=danger-full-access"],
            // A name that only starts like a known one.
            vec!["-y", "@agentclientprotocol/codex-acp-fork@1.12.0"],
        ] {
            assert_eq!(
                confinement_for(&declared(&args), Some(URL), none),
                Ok(None),
                "{args:?}"
            );
        }
    }
}
