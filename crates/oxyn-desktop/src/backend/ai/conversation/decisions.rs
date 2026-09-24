//! An agent's write waits for the user, inside its tool call.
//!
//! When the `PolicyGate` holds an agent's command back, the call does **not**
//! come back with « awaiting approval ». It waits until the request ends, and
//! answers the model with what really happened:
//!
//! * **the user decided**, through `decide` — the outcome of the approved
//!   command, or « rejected »;
//! * **nobody decided**, and nothing ran — the request expired, the question
//!   was stopped, the agent was released or stopped waiting. The card says so
//!   itself: the panel must stop offering « Review… » at that moment, not when
//!   the user clicks.
//!
//! Coming back at once was the defect this module closes. The model went on
//! writing as if nothing were pending, the answer ended, and whatever then
//! withdrew the request — an agent released after an exchange that carried a
//! sample, a new agent launched, a connection edited — did it silently. The
//! card still offered « Review… », and approving said « no command is
//! awaiting approval under this identifier ».
//!
//! # One bound, said
//!
//! The wait ends at the request's own expiry, the approval registry's lifetime
//! ([`oxyn_exec::ApprovalRegistry::ttl`]), sent with the request
//! (`expiresAtMs`). Past it, the call withdraws the request **as expired** — a
//! late approval says « expired » — the card says « Expired », and nothing
//! runs. There is no other clock here: an external agent's own bound on the
//! call is its adapter's, recorded in RESEARCH-NOTES.
//!
//! # Never a silent loss (I-13)
//!
//! Once `decide` took the request, the command may run: the call then waits for
//! its outcome, whatever else happens — a stop, the expiry. And if `decide`
//! ends without saying how it went, the call says « it may have been applied »,
//! never « nothing ran ».

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{CancelToken, CommandId, ErrorClass};
use oxyn_exec::{DispatchReport, Executor};
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::ipc::ai::ToolStatus;

/// How a request the call waits on was settled from outside the call.
pub(crate) enum Settled {
    /// The user answered through `decide`; the panel shows the outcome itself.
    Answered(DispatchReport),
    /// Withdrawn without an answer, because the agent was released. Nothing
    /// ran.
    Released,
}

/// The calls waiting on a request, by request.
#[derive(Default)]
pub(crate) struct Decisions {
    waiting: Mutex<HashMap<CommandId, oneshot::Sender<Settled>>>,
}

impl std::fmt::Debug for Decisions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decisions")
            .field("waiting", &self.waiting.lock().len())
            .finish()
    }
}

impl Decisions {
    /// Registers a call's wait on `command`, before the request is shown: an
    /// answer given the instant it appears must find someone to tell.
    pub(crate) fn expect(self: &Arc<Self>, command: CommandId) -> Awaited {
        let (sender, receiver) = oneshot::channel();
        self.waiting.lock().insert(command, sender);
        Awaited {
            decisions: Arc::clone(self),
            command,
            receiver,
        }
    }

    /// Tells the call waiting on `command`, if one still does.
    fn settle(&self, command: CommandId, settled: Settled) {
        if let Some(waiting) = self.waiting.lock().remove(&command) {
            // A call that stopped waiting meanwhile has nothing left to tell.
            let _gone = waiting.send(settled);
        }
    }

    /// Withdraws `command` at the executor, because the agent that proposed it
    /// is released, and tells its call. `false` if it no longer waited.
    pub(crate) fn release(&self, executor: &Executor, command: CommandId) -> bool {
        let withdrawn = executor.reject(command).is_some();
        if withdrawn {
            self.settle(command, Settled::Released);
        }
        withdrawn
    }

    /// The answer `decide` owes the call waiting on `command`, from the moment
    /// it starts deciding. Dropped unanswered, it says the command may have
    /// run: `decide` may have been cut off after the request was taken.
    pub(crate) fn answer(self: &Arc<Self>, command: CommandId) -> Answer {
        Answer {
            decisions: Arc::clone(self),
            command,
            sent: false,
        }
    }
}

/// A call's registration on a request. Dropping it forgets the call.
pub(crate) struct Awaited {
    decisions: Arc<Decisions>,
    command: CommandId,
    receiver: oneshot::Receiver<Settled>,
}

impl Drop for Awaited {
    fn drop(&mut self) {
        self.decisions.waiting.lock().remove(&self.command);
    }
}

/// See [`Decisions::answer`].
pub(crate) struct Answer {
    decisions: Arc<Decisions>,
    command: CommandId,
    sent: bool,
}

impl Answer {
    /// What the decision did, for the call that waited on it.
    pub(crate) fn report(mut self, report: DispatchReport) {
        self.sent = true;
        self.decisions
            .settle(self.command, Settled::Answered(report));
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        if !self.sent {
            self.decisions.settle(
                self.command,
                Settled::Answered(DispatchReport::Failed {
                    command: self.command,
                    class: ErrorClass::Ambiguous,
                    error: INTERRUPTED.to_owned(),
                }),
            );
        }
    }
}

/// Said to the model when the decision ended without an outcome.
const INTERRUPTED: &str = "the user approved this statement, then Oxyn lost track of its \
     execution: it may or may not have been applied. Do not retry it; tell the user";

/// How a request ended for the call that waited on it.
pub(crate) enum RequestEnd {
    /// The user decided: what the command did, or « rejected ».
    Answered(DispatchReport),
    /// Nobody decided, and nothing ran.
    Withdrawn(Withdrawn),
}

/// A request that ended without the user's decision.
pub(crate) struct Withdrawn {
    /// What the card shows.
    pub(crate) status: ToolStatus,
    /// The card's words.
    pub(crate) detail: String,
    /// The model's words.
    pub(crate) reason: String,
}

impl Withdrawn {
    fn expired(after: Duration) -> Self {
        let after = spoken(after);
        Self {
            // Nothing ran and nothing will: a refusal, whose words say why.
            status: ToolStatus::Denied,
            detail: format!("Expired: no answer within {after}. Nothing ran."),
            reason: format!(
                "the user did not answer within {after}, so the request expired. Nothing ran"
            ),
        }
    }

    fn stopped() -> Self {
        Self {
            status: ToolStatus::Cancelled,
            detail: "Stopped before you answered. Nothing ran.".to_owned(),
            reason: "the question was stopped before the user answered. Nothing ran".to_owned(),
        }
    }

    fn released() -> Self {
        Self {
            status: ToolStatus::Cancelled,
            detail: "Withdrawn: the agent session ended before you answered. Nothing ran."
                .to_owned(),
            reason: "the agent session ended before the user answered, so the request was \
                     withdrawn. Nothing ran"
                .to_owned(),
        }
    }

    /// The agent's call went away while the request waited: an external agent
    /// that hung up.
    pub(crate) fn hung_up() -> Self {
        Self {
            status: ToolStatus::Cancelled,
            detail: "Withdrawn: the agent stopped waiting for your answer. Nothing ran.".to_owned(),
            reason: "the call stopped waiting before the user answered. Nothing ran".to_owned(),
        }
    }
}

/// Waits for the end of the request `awaited` names: the user's decision,
/// the `deadline`, the question's stop, or the agent's release.
///
/// Past the deadline or on a stop, the request is withdrawn — as expired for
/// the former — unless `decide` already took it: its outcome is then awaited,
/// since the command may be running.
pub(crate) async fn wait(
    executor: &Executor,
    mut awaited: Awaited,
    deadline: tokio::time::Instant,
    cancel: &CancelToken,
) -> RequestEnd {
    let command = awaited.command;
    let expiry = tokio::time::sleep_until(deadline);
    tokio::pin!(expiry);
    tokio::select! {
        settled = &mut awaited.receiver => return ending(executor, command, settled.ok()),
        () = cancel.cancelled() => {
            if executor.reject(command).is_some() {
                return RequestEnd::Withdrawn(Withdrawn::stopped());
            }
        }
        () = &mut expiry => {
            if executor.approvals().expire(command).is_some() {
                return RequestEnd::Withdrawn(Withdrawn::expired(executor.approvals().ttl()));
            }
        }
    }
    // Taken by `decide` at the same moment: it runs, and says how it went.
    let settled = (&mut awaited.receiver).await.ok();
    ending(executor, command, settled)
}

fn ending(executor: &Executor, command: CommandId, settled: Option<Settled>) -> RequestEnd {
    match settled {
        Some(Settled::Answered(report)) => RequestEnd::Answered(report),
        Some(Settled::Released) => RequestEnd::Withdrawn(Withdrawn::released()),
        // Nobody left to say: the backend is going away. Withdrawn if it still
        // waits; otherwise it was taken, and may have run.
        None if executor.reject(command).is_some() => RequestEnd::Withdrawn(Withdrawn::released()),
        None => RequestEnd::Answered(DispatchReport::Failed {
            command,
            class: ErrorClass::Ambiguous,
            error: INTERRUPTED.to_owned(),
        }),
    }
}

/// A duration as the card and the model read it: « 5 minutes », « 30 seconds ».
fn spoken(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match (seconds / 60, seconds % 60) {
        (1, 0) => "1 minute".to_owned(),
        (minutes, 0) if minutes > 0 => format!("{minutes} minutes"),
        _ if seconds == 1 => "1 second".to_owned(),
        _ => format!("{seconds} seconds"),
    }
}
