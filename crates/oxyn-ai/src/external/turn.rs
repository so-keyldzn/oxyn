//! One conversation turn with an external agent, end to end.
//!
//! Authority: [ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md).
//!
//! # What this module does not do
//!
//! It does not watch the child process. The crate's `ConnectTo` path already
//! installs a guard that terminates the process **group** — not only the
//! immediate child, because an agent distributed behind `npx` or `uvx` would
//! re-attach to pid 1 and would not stop reliably. Rewriting that would have
//! meant rewriting it worse.
//!
//! **Checked in the source, on 2026-09-15** — this sentence was a claim taken
//! over without proof, and all of this module's cancellation depends on it.
//! `agent-client-protocol 2.1.0`, `src/acp_agent.rs`: `spawn_process` sets
//! `process_group(0)`, so the child leads its own group and killing it does not
//! reach Oxyn; `ChildGuard::terminate` sends `SIGKILL` to the group then
//! `kill()` as a fallback; and the guard is built **before** the first `poll`,
//! with this upstream comment: "Create the guard eagerly so cancelling this
//! connection before the monitor is first polled still terminates the whole
//! process group." Dropping the future is therefore enough, even immediately.
//!
//! Nor does it decide permissions: that is [`super::permission_for`] and
//! [`super::option_for`], tested separately. Here, we wire them.

use std::pin::pin;
use std::sync::Arc;

use futures::future::{Either, select};
use oxyn_core::{CancelToken, ExternalAgentConfig, Result};

use super::prompt::AgentPrompt;
use super::session::ExternalSession;
use crate::observer::AgentObserver;
use crate::privacy::PrivacyTier;

/// How an external agent turn ended.
///
/// Oxyn's vocabulary, **not the protocol's**: `StopReason` does not cross this
/// boundary. It is the same stance as `oxyn-llm`, which does not let its
/// transport leak — otherwise `oxyn-desktop` would have to depend on the
/// protocol's crate to read the end of a conversation, and the choice of that
/// protocol would stop being reversible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnEnd {
    /// The agent answered.
    Answered {
        /// Was the answer **cut**? A cut answer that does not say so looks
        /// like a wrong answer.
        truncated: bool,
    },
    /// The turn was cancelled.
    Cancelled,
    /// The agent refused to continue. The protocol specifies that the refused
    /// question will not reach the next prompt: the interface must say so.
    Refused,
    /// The agent reached its own request ceiling for this turn.
    TurnLimit,
}

/// Launches the agent, sends a prompt, and returns how the turn ended.
///
/// # The tier is checked **before** launching anything
///
/// An external agent counts as `Reach::Unresolved`: under
/// [`PrivacyTier::Local`], the answer is a refusal, and it falls **before** the
/// process is launched. Starting the agent and then refusing to talk to it
/// would already be too late: merely launching it can be enough to make it
/// contact its service.
///
/// # Errors
///
/// [`oxyn_core::OxynError::Config`] if the tier forbids it or if the
/// declaration is invalid; [`oxyn_core::OxynError::Internal`] if the protocol
/// fails. No message copies the content of the prompt.
///
/// # Cancellation
///
/// `cancel` abandons the conversation, which destroys the transport — and the
/// `agent-client-protocol` guard then terminates the process **group**, as the
/// header of this module explains. It is the only way to take back control in
/// this mode: the destination not being verifiable, a stop button that only
/// changed the display would lie precisely where it matters.
pub async fn run_turn(
    agent: &ExternalAgentConfig,
    tier: PrivacyTier,
    prompt: &AgentPrompt,
    cancel: &CancelToken,
    observer: Arc<dyn AgentObserver>,
) -> Result<TurnEnd> {
    // Before the tier: an already cancelled turn does not launch a process only
    // to find out afterwards that it had to stop it.
    if cancel.is_cancelled() {
        return Ok(TurnEnd::Cancelled);
    }
    let (session, driver) = ExternalSession::launch(agent, tier)?;
    let driver = pin!(driver);
    let turn = pin!(session.prompt(prompt, observer, cancel));
    // The turn and the transport move forward together; the session dropped on
    // the way out terminates the process group, cancellation included.
    match select(turn, driver).await {
        Either::Left((fin, _)) => Ok(fin?),
        Either::Right(((), _)) => Err(super::session::ExternalError::Exited.into()),
    }
}
