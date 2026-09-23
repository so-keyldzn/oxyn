//! Ready-made declarations for agents already installed on the machine.
//!
//! A preset is a **draft**: a command, arguments and environment the user reads
//! and confirms. Nothing is declared by this module: detection only reads the
//! file system, when the AI settings screen opens. The user keeps their subscription and their sign-in with
//! the agent; Oxyn holds no key and no token for it.
//!
//! # Where each value comes from
//!
//! Package names and versions were read at the npm registry and in the ACP
//! registry on 2026-09-15, and are recorded with their sources in
//! [RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md) (I-12). The versions
//! are **pinned**: `npx -y <package>` without a version would run whatever was
//! published a minute ago, and both adapters publish several times a week.

use std::path::Path;

use super::locate::SearchPath;

/// A known agent adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentPreset {
    /// Stable identifier.
    pub id: &'static str,
    /// The name shown and proposed as the declaration's label.
    pub label: &'static str,
    /// The npm package that speaks ACP for this agent.
    pub package: &'static str,
    /// Its pinned version.
    pub version: &'static str,
    /// The program the user signs in with, and how, in a terminal.
    pub sign_in: &'static str,
    /// The arguments that sign in through the CLI the adapter bundles,
    /// appended to the command that runs it. Empty when it bundles none: the
    /// agent's own program is then the only way.
    pub adapter_sign_in: &'static [&'static str],
    /// The program whose presence tells the agent is installed at all.
    pub agent_program: &'static str,
    /// The oldest Node major its adapter runs on (RESEARCH-NOTES).
    pub node_major: u64,
}

/// Claude Code, through Anthropic's ACP adapter.
pub const CLAUDE_CODE: AgentPreset = AgentPreset {
    id: "claude-code",
    label: "Claude Code",
    package: "@agentclientprotocol/claude-agent-acp",
    version: "0.78.0",
    // Claude Code's CLI reference: `claude auth login`; the adapter reads the
    // same configuration directory.
    sign_in: "claude auth login",
    // What the adapter 0.78.0 itself advertises as its terminal sign-in
    // (`dist/acp-agent.js`): `--cli` hands the rest to the Claude Code CLI its
    // SDK bundles (`dist/index.js`), so it works with no `claude` installed,
    // and signs in exactly the CLI the adapter reads (RESEARCH-NOTES).
    adapter_sign_in: &["--cli", "auth", "login", "--claudeai"],
    agent_program: "claude",
    // `engines` of the adapter 0.78.0: `node >=22`.
    node_major: 22,
};

/// OpenAI Codex CLI, through its ACP adapter.
pub const CODEX: AgentPreset = AgentPreset {
    id: "codex",
    label: "Codex",
    package: "@agentclientprotocol/codex-acp",
    version: "1.12.0",
    // OpenAI's Codex authentication page; the adapter reuses that sign-in.
    sign_in: "codex login",
    // Not verified for codex-acp: `codex login` stays the only proposal.
    adapter_sign_in: &[],
    agent_program: "codex",
    // The adapter 1.12.0 declares no `engines`; its dependency `open@^11`
    // declares `node >=20`, the highest among them.
    node_major: 20,
};

impl AgentPreset {
    /// The words that sign in through the adapter's bundled CLI, when the
    /// adapter is run by `command` and `args`; `None` when it bundles none.
    ///
    /// The words are raw: whoever shows them quotes them for a shell.
    #[must_use]
    pub fn adapter_sign_in(self, command: &str, args: &[String]) -> Option<Vec<String>> {
        (!self.adapter_sign_in.is_empty()).then(|| {
            std::iter::once(command.to_owned())
                .chain(args.iter().cloned())
                .chain(self.adapter_sign_in.iter().map(|&word| word.to_owned()))
                .collect()
        })
    }
}

/// Every preset, in the order shown.
pub const PRESETS: [AgentPreset; 2] = [CLAUDE_CODE, CODEX];

/// The preset with this identifier.
#[must_use]
pub fn preset(id: &str) -> Option<AgentPreset> {
    PRESETS.into_iter().find(|preset| preset.id == id)
}

/// The preset whose package this declaration runs, if any: recognized by an
/// argument naming the package, pinned or not.
#[must_use]
pub fn preset_of(config: &oxyn_core::ExternalAgentConfig) -> Option<AgentPreset> {
    PRESETS.into_iter().find(|preset| {
        config.args.iter().any(|arg| {
            arg == preset.package
                || arg
                    .strip_prefix(preset.package)
                    .is_some_and(|rest| rest.starts_with('@'))
        })
    })
}

/// The preset this declaration runs **exactly as Oxyn proposes it**: its
/// pinned version, and no other argument.
///
/// The confinement (ADR-0032) was measured on that version; another may ignore
/// its switches, and an argument more may override them. Anything else is
/// treated as an agent Oxyn does not know.
#[must_use]
pub fn pinned_preset_of(config: &oxyn_core::ExternalAgentConfig) -> Option<AgentPreset> {
    PRESETS
        .into_iter()
        .find(|&preset| config.args == arguments(preset))
}

/// The launcher both adapters are distributed through.
const LAUNCHER: &str = "npx";

/// The system directories kept after the detected ones, so that the adapter's
/// own `#!/usr/bin/env node` and its child processes still find the basics.
#[cfg(unix)]
const SYSTEM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
#[cfg(not(unix))]
const SYSTEM_DIRS: &[&str] = &[];

/// What detection found, to show before anything is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetDraft {
    /// The preset this draft comes from.
    pub preset: AgentPreset,
    /// The proposed command: the launcher's absolute path when found, its bare
    /// name otherwise.
    pub command: String,
    /// The proposed arguments.
    pub args: Vec<String>,
    /// The proposed environment. Only `PATH`, and only when detection found
    /// something to put in it; never a token.
    pub env: Vec<(String, String)>,
    /// The launcher, if found.
    pub launcher: Option<String>,
    /// The agent's own program, if found. Its absence is not an error: the
    /// adapter ships one; but the sign-in command needs it.
    pub agent_program: Option<String>,
}

impl PresetDraft {
    /// A draft without looking at the machine.
    #[must_use]
    pub fn blank(preset: AgentPreset) -> Self {
        Self {
            preset,
            command: LAUNCHER.to_owned(),
            args: arguments(preset),
            env: Vec::new(),
            launcher: None,
            agent_program: None,
        }
    }

    /// A draft completed with what `search` finds. Touches the file system.
    #[must_use]
    pub fn detect(preset: AgentPreset, search: &SearchPath) -> Self {
        let launcher = search.find(LAUNCHER);
        let node = search.find("node");
        let agent_program = search.find(preset.agent_program);
        let mut dirs: Vec<&Path> = Vec::new();
        for found in [&launcher, &node].into_iter().flatten() {
            if let Some(dir) = found.parent()
                && !dirs.contains(&dir)
            {
                dirs.push(dir);
            }
        }
        let env = if dirs.is_empty() {
            Vec::new()
        } else {
            let all = dirs
                .into_iter()
                .map(Path::to_path_buf)
                .chain(SYSTEM_DIRS.iter().map(Into::into));
            std::env::join_paths(all)
                .ok()
                .and_then(|joined| joined.into_string().ok())
                .map(|value| vec![("PATH".to_owned(), value)])
                .unwrap_or_default()
        };
        let text = |path: Option<std::path::PathBuf>| {
            path.and_then(|p| p.into_os_string().into_string().ok())
        };
        let launcher = text(launcher);
        Self {
            preset,
            command: launcher.clone().unwrap_or_else(|| LAUNCHER.to_owned()),
            args: arguments(preset),
            env,
            launcher,
            agent_program: text(agent_program),
        }
    }
}

fn arguments(preset: AgentPreset) -> Vec<String> {
    vec![
        "-y".to_owned(),
        format!("{}@{}", preset.package, preset.version),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_draft_pins_the_version_and_carries_no_environment() {
        let draft = PresetDraft::blank(CLAUDE_CODE);
        assert_eq!(draft.command, "npx");
        assert_eq!(
            draft.args,
            vec!["-y", "@agentclientprotocol/claude-agent-acp@0.78.0"]
        );
        assert!(draft.env.is_empty());
    }

    #[test]
    fn nothing_found_is_said_as_such_not_invented() {
        let draft = PresetDraft::detect(CODEX, &SearchPath::of(Vec::new()));
        assert_eq!(draft.launcher, None);
        assert_eq!(draft.agent_program, None);
        assert_eq!(draft.command, "npx");
        assert!(draft.env.is_empty());
    }

    #[test]
    fn claude_signs_in_through_its_adapter_and_codex_does_not() {
        let draft = PresetDraft::blank(CLAUDE_CODE);
        assert_eq!(
            CLAUDE_CODE.adapter_sign_in(&draft.command, &draft.args),
            Some(
                [
                    "npx",
                    "-y",
                    "@agentclientprotocol/claude-agent-acp@0.78.0",
                    "--cli",
                    "auth",
                    "login",
                    "--claudeai",
                ]
                .map(str::to_owned)
                .to_vec()
            )
        );
        assert_eq!(CODEX.adapter_sign_in("npx", &arguments(CODEX)), None);
    }

    #[test]
    fn presets_are_found_by_id() {
        assert_eq!(preset("codex"), Some(CODEX));
        assert_eq!(preset("cursor"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_found_launcher_is_used_by_path_and_its_directory_reaches_path() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("oxyn-preset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        for name in ["npx", "node"] {
            let path = dir.join(name);
            std::fs::write(&path, "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let draft = PresetDraft::detect(CLAUDE_CODE, &SearchPath::of(vec![dir.clone()]));
        let npx = dir.join("npx").to_string_lossy().into_owned();
        assert_eq!(draft.command, npx);
        let path = &draft.env.first().expect("PATH").1;
        assert!(path.starts_with(&*dir.to_string_lossy()), "{path}");
        assert_eq!(path.matches(&*dir.to_string_lossy()).count(), 1, "{path}");
    }
}
