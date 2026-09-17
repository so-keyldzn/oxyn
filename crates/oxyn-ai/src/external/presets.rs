//! Ready-made declarations for agents already installed on the machine.
//!
//! A preset is a **draft**: a command, arguments and environment the user reads
//! and confirms. Nothing is declared by this module, and nothing in it runs at
//! startup (ADR-0023). The user keeps their subscription and their sign-in with
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
    /// The program whose presence tells the agent is installed at all.
    pub agent_program: &'static str,
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
    agent_program: "claude",
};

/// OpenAI Codex CLI, through its ACP adapter.
pub const CODEX: AgentPreset = AgentPreset {
    id: "codex",
    label: "Codex",
    package: "@agentclientprotocol/codex-acp",
    version: "1.12.0",
    // OpenAI's Codex authentication page; the adapter reuses that sign-in.
    sign_in: "codex login",
    agent_program: "codex",
};

/// Every preset, in the order shown.
pub const PRESETS: [AgentPreset; 2] = [CLAUDE_CODE, CODEX];

/// The preset with this identifier.
#[must_use]
pub fn preset(id: &str) -> Option<AgentPreset> {
    PRESETS.into_iter().find(|preset| preset.id == id)
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
