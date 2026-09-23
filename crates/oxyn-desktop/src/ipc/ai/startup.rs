//! What crosses for an external agent started before its first question.
//!
//! An agent declares its models, efforts and options only once its session is
//! open. Opening it when the panel shows the agent — not at the first question
//! — is what lets the panel offer them before anything is asked. The start is
//! the question's own launch done earlier: same refusal under `Local`, same
//! tools, same confinement ([ADR-0026](../../../../../docs/adr/0026-agents-externes-acp.md)).

use oxyn_ai::external::spawn::ExitReport;
use serde::{Deserialize, Serialize};

use super::{AgentSettingsView, FailureCategory, SignInHelp};

/// Starts the agent the next question to `agent` would use.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStartRequest {
    pub connection: String,
    /// The session the agent's tools will run in: the panel's.
    pub session: String,
    /// The conversation shown, if any. Its own agent, when it is the one that
    /// would answer after `parent`, is used rather than a new one.
    pub thread: Option<String>,
    pub parent: Option<u32>,
    /// The declared agent's identifier.
    pub agent: String,
}

/// How a start ended.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AgentStart {
    /// Its session is open: what it said it is, and its settings as it
    /// declared them.
    #[serde(rename_all = "camelCase")]
    Ready {
        version: Option<String>,
        settings: AgentSettingsView,
    },
    /// Stopped before it was ready: by the user, by another start that
    /// replaced it, or because what it was launched under changed.
    Cancelled,
    /// It could not start. Never retried by Oxyn: a new start is a click.
    #[serde(rename_all = "camelCase")]
    Failed {
        message: String,
        category: FailureCategory,
        sign_in: Option<SignInHelp>,
        found_elsewhere: Option<String>,
        exit: Option<AgentExit>,
    },
}

/// What the agent's process said on its way out.
///
/// `output` is the end of its error output, with every value Oxyn handed the
/// process — declared variables, the tool token — already replaced by a
/// marker when it was read (`oxyn_ai::external::spawn`). It can still quote
/// what the agent was asked, so it is shown and never logged.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExit {
    /// `None` when the process was stopped by a signal.
    pub code: Option<i32>,
    pub output: String,
}

impl AgentExit {
    pub fn of(report: &ExitReport) -> Self {
        Self {
            code: report.code,
            output: report.stderr.clone(),
        }
    }
}

/// The code only: the output is shown, never written to a journal (I-03).
impl std::fmt::Debug for AgentExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentExit")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}
