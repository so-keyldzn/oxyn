//! When an agent counts as gone silent, and how long a stop is waited for.
//!
//! Two waits had no bound, and each held the session for good:
//!
//! * **an answer that never comes.** An agent whose own model connection
//!   hangs sends nothing more, and the question waited forever. Any update
//!   from the agent proves it alive and restarts the count;
//! * **a stop never confirmed.** After `session/cancel`, the protocol has the
//!   agent close the prompt with `cancelled`. `serve` handles one request at a
//!   time: an agent that never did blocked every later question behind it.
//!
//! **What does not count as silence**: a tool call in flight. A call to Oxyn's
//! tools can wait on the user — an approval left open while they read it — or
//! on a long query, and the agent says nothing meanwhile. Those waits have
//! their own bounds, and the user is watching them; cutting the question there
//! would cut an approval in the middle of its reading.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use agent_client_protocol::schema::v1::{SessionUpdate, ToolCallStatus};

/// Longest silence accepted from an agent while it answers, outside a tool
/// call.
///
/// A product bound, not a measurement: the protocol says nothing of how often
/// an agent reports, and an agent may think a long time before its first
/// words. Five minutes, as for a provider's stream
/// ([AI-PROVIDERS](../../../../../docs/AI-PROVIDERS.md#a-silent-provider)) —
/// "Stop" stays the fast way out.
pub(super) const AGENT_IDLE_LIMIT: Duration = Duration::from_secs(300);

/// How long the agent's `cancelled` answer is awaited after a stop.
///
/// Past it the agent is stopped — its process group with it — so that the
/// next question starts a fresh one instead of waiting behind this one.
pub(super) const CANCEL_GRACE: Duration = Duration::from_secs(10);

/// Open tool calls tracked, at most. An agent that announces calls without
/// ever finishing them cannot grow the set without limit; past this, the
/// watch stays suspended until the question ends, which is the side that does
/// not cut a user's approval.
const MAX_OPEN_CALLS: usize = 256;

/// The two bounds of a question, replaced only by the tests.
#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    /// See [`AGENT_IDLE_LIMIT`].
    pub(super) idle: Duration,
    /// See [`CANCEL_GRACE`].
    pub(super) cancel_grace: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            idle: AGENT_IDLE_LIMIT,
            cancel_grace: CANCEL_GRACE,
        }
    }
}

/// What the agent did lately, as the protocol callbacks see it.
#[derive(Default)]
pub(super) struct Liveness {
    /// Woken by every update. One stored permit is enough: the watch only
    /// needs to know that something came since it last looked.
    activity: tokio::sync::Notify,
    /// Tool calls announced and not yet finished, in the question in progress.
    open: Mutex<HashSet<String>>,
}

impl Liveness {
    /// A question starts: what the previous one left open does not hold this
    /// one's watch.
    pub(super) fn begin(&self) {
        self.lock().clear();
    }

    /// The agent sent an update.
    pub(super) fn saw(&self, update: &SessionUpdate) {
        let (id, status) = match update {
            SessionUpdate::ToolCall(call) => (call.tool_call_id.to_string(), Some(call.status)),
            SessionUpdate::ToolCallUpdate(update) => {
                (update.tool_call_id.to_string(), update.fields.status)
            }
            _ => {
                self.activity.notify_one();
                return;
            }
        };
        {
            let mut open = self.lock();
            match status {
                Some(ToolCallStatus::Completed | ToolCallStatus::Failed) => {
                    open.remove(&id);
                }
                Some(_) if open.len() < MAX_OPEN_CALLS => {
                    open.insert(id);
                }
                // An update without a status leaves the call as it was; past
                // the ceiling, the set is not grown.
                _ => {}
            }
        }
        self.activity.notify_one();
    }

    /// The agent asked something of Oxyn: it is alive.
    pub(super) fn poke(&self) {
        self.activity.notify_one();
    }

    /// Resolves once the agent has said nothing for `limit`, no tool call
    /// being in flight. Never resolves while it keeps talking.
    pub(super) async fn silent_for(&self, limit: Duration) {
        loop {
            match tokio::time::timeout(limit, self.activity.notified()).await {
                Ok(()) => {}
                Err(_) if !self.lock().is_empty() => {}
                Err(_) => return,
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
