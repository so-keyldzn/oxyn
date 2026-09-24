//! An agent's calls to Oxyn's own tools, told apart in what it streams.
//!
//! When an agent calls a tool of Oxyn's MCP server, its adapter also reports
//! the call as one of its own steps. The panel already shows the call as an
//! Oxyn card — the one that went through the bus and the `PolicyGate`, and the
//! one that says what ran. Relaying the step too would draw the same call
//! twice, once as an anonymous « Agent step ».
//!
//! **What recognises a call**, per adapter, read in their sources (see
//! [RESEARCH-NOTES](../../../../../docs/RESEARCH-NOTES.md#ce-que-les-adaptateurs-acp-disent-dun-appel-mcp--relu-le-2026-09-24)):
//!
//! * Claude's adapter puts the programmatic tool name in
//!   `_meta.claudeCode.toolName` — `mcp__oxyn__<tool>` for one of ours;
//! * Codex's adapter marks an MCP call `_meta.is_mcp_tool_call: true` and puts
//!   the server and tool, as Codex knows them, in `rawInput.server` and
//!   `rawInput.tool`.
//!
//! The title is **not** a signal: it is display text, and for some tools the
//! agent composes it from what it runs — a shell command can be spelled like
//! one of Oxyn's tools. `rawInput` alone is not one either: for any other tool
//! it is the tool's arguments, which the model writes.
//!
//! Only an exact match against the tools the server announced counts. A call
//! nothing recognises is relayed as it was: hiding an agent step Oxyn has no
//! card for would hide what the agent did.

use std::collections::HashSet;

use agent_client_protocol::schema::v1::{Meta, SessionUpdate, ToolCallStatus};
use serde_json::Value;

use crate::external::mcp;

/// The calls to Oxyn's tools seen in one session, and the tools that make one.
pub(super) struct OxynCalls {
    /// What Oxyn's server announced to this agent. Empty when it serves none:
    /// then nothing is Oxyn's, whatever its name.
    served: Vec<String>,
    /// Calls hidden and not yet finished, so their updates are hidden too:
    /// Codex's completion carries no mark of its own.
    open: HashSet<String>,
}

impl OxynCalls {
    pub(super) fn new(served: Vec<String>) -> Self {
        Self {
            served,
            open: HashSet::new(),
        }
    }

    /// Whether `update` is about a call to one of Oxyn's tools, and so is not
    /// to be relayed. Remembers the call until it finishes.
    pub(super) fn hides(&mut self, update: &SessionUpdate) -> bool {
        let (id, status, ours) = match update {
            SessionUpdate::ToolCall(call) => (
                call.tool_call_id.to_string(),
                Some(call.status),
                self.names_ours(call.meta.as_ref(), call.raw_input.as_ref()),
            ),
            SessionUpdate::ToolCallUpdate(update) => (
                update.tool_call_id.to_string(),
                update.fields.status,
                self.names_ours(update.meta.as_ref(), update.fields.raw_input.as_ref()),
            ),
            _ => return false,
        };
        if !ours && !self.open.contains(&id) {
            return false;
        }
        // Forgotten once finished, so a long session does not keep every call.
        if matches!(
            status,
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
        ) {
            self.open.remove(&id);
        } else {
            self.open.insert(id);
        }
        true
    }

    fn names_ours(&self, meta: Option<&Meta>, raw_input: Option<&Value>) -> bool {
        if self.served.is_empty() {
            return false;
        }
        let Some(meta) = meta else {
            return false;
        };
        // Claude's adapter.
        let claude = meta
            .get("claudeCode")
            .and_then(|claude| claude.get("toolName"))
            .and_then(Value::as_str)
            .is_some_and(|name| {
                self.served
                    .iter()
                    .any(|tool| name == mcp::claude_tool_name(tool))
            });
        // Codex's adapter: the mark says `rawInput` is the server's and the
        // tool's, not the model's arguments.
        let codex = meta.get("is_mcp_tool_call") == Some(&Value::Bool(true))
            && raw_input.is_some_and(|input| {
                input.get("server").and_then(Value::as_str) == Some(mcp::SERVER_NAME)
                    && input
                        .get("tool")
                        .and_then(Value::as_str)
                        .is_some_and(|tool| self.served.iter().any(|served| served == tool))
            });
        claude || codex
    }
}

#[cfg(test)]
mod tests;
