//! The agents a conversation can run, as they cross to the webview
//! ([ADR-0049](../../../../../docs/adr/0049-agents-declared-as-markdown-files.md) § 10).

use serde::Serialize;

/// One line of the agent picker.
///
/// Named `AgentRoleOption` here because `AgentOption` is already an external
/// agent's setting; the webview's type is `AgentOption`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRoleOption {
    /// The agent's id; for a user file that cannot be picked, its file name.
    pub id: String,
    pub name: String,
    pub description: String,
    /// `shipped` or `user`.
    pub origin: &'static str,
    /// Why it cannot be picked; `None` for a selectable agent.
    pub error: Option<String>,
    /// The declared destinations this agent is not written for: the panel
    /// shows them disabled while a conversation runs this agent (ADR-0049
    /// § 6). Empty for an entry in error, which runs nowhere.
    pub disabled_destinations: Vec<DestinationRef>,
}

/// A declared destination, by kind and id, as `DestinationChoice` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationRef {
    /// `provider` or `agent`.
    pub kind: &'static str,
    pub id: String,
}

/// The agent a conversation was recorded with, when it no longer exists and
/// the SQL agent answers in its place: the panel says so (ADR-0049 § 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingAgent {
    /// The recorded agent's id: the store keeps the id and not the name, and
    /// a file that is gone can no longer say what it was called.
    pub name: String,
}
