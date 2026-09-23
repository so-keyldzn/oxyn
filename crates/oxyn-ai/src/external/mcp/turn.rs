//! The question a tool call answers.
//!
//! An agent's session outlives a question: it is kept for the next one. The
//! tool endpoint does too. What must **not** outlive a question is what a call
//! is attached to — where it shows in the panel, which « Stop » reaches it, and
//! how many calls it may make. Bound once at launch, all three belonged to the
//! first question for the life of the session: a stopped first question left
//! every later call cancelled before it ran, a « Stop » on a later one reached
//! nothing, and between two questions an agent could call the tools unseen and
//! without limit, piling approval requests under an answer the user had
//! stopped reading.
//!
//! So each question **opens** a turn and closing it takes the tools away:
//!
//! * **no question in progress, nothing runs.** The user is not looking, and a
//!   confirmation asked for when nobody watches is one clicked by reflex;
//! * **a ceiling of calls per question**, the one the internal assistant has;
//! * **nothing runs while a request of this agent waits for the user.** Ten
//!   requests in a row are how the tenth gets approved without being read
//!   (I-02 in its spirit). « Waits » is read from the executor, which holds the
//!   requests: a copy kept here missed a command the executor reclassified, and
//!   a request left undecided by the previous question;
//! * **one call at a time.** Each HTTP connection has its own task, and the
//!   check above is only true if no other call slips between it and the
//!   executor's answer: two writes sent together both passed it once;
//! * **closing a question cancels its calls**, and a result that lands after
//!   the question closed is discarded rather than shown to nobody.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use oxyn_core::{Actor, CancelToken, Command};

use crate::observer::AgentObserver;
use crate::runtime::{CommandSink, DispatchOutcome};
use crate::tools::SampleAsk;

/// Where an external agent's tool calls go, question by question.
///
/// Cheap to clone: every clone is the same slot.
#[derive(Clone)]
pub struct ToolTurns {
    slot: Arc<Mutex<Slot>>,
    max_calls: usize,
    /// Does a request of this agent wait for the user's decision?
    waiting: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Held from the check to the executor's answer: calls run one at a time.
    order: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for ToolTurns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolTurns")
            .field("max_calls", &self.max_calls)
            .field("open", &self.lock().current.is_some())
            .finish()
    }
}

#[derive(Default)]
struct Slot {
    /// Increments on every `open`, so a turn that ended cannot close its
    /// successor when its guard drops late.
    generation: u64,
    current: Option<Active>,
}

struct Active {
    generation: u64,
    sink: Arc<dyn CommandSink>,
    observer: Arc<dyn AgentObserver>,
    /// A child of the question's own token: cancelled when the question
    /// stops, and also when its turn closes.
    cancel: CancelToken,
    calls: usize,
}

impl ToolTurns {
    /// No question open yet. `max_calls` bounds each question; `waiting` says
    /// whether a request of this agent is still before the user — asked of the
    /// executor, never remembered here.
    #[must_use]
    pub fn new(max_calls: usize, waiting: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            slot: Arc::new(Mutex::new(Slot::default())),
            max_calls,
            waiting,
            order: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Ends whatever question is open and cancels its calls, then gives a call
    /// already at the executor up to `grace` to wind down.
    ///
    /// Waiting matters: an aborted future cancels nothing on the server, so a
    /// long `SELECT` would go on running there. Cancelling first, then letting
    /// the call observe it, is what reaches the server.
    pub(crate) async fn close(&self, grace: Duration) {
        self.close_now();
        // The order lock is free exactly when no call is at the executor.
        let _settled = tokio::time::timeout(grace, self.order.lock()).await;
    }

    /// The question the user is now waiting on: its sink, its panel, its stop.
    ///
    /// Replaces any turn still open. The tools stay available until the
    /// returned guard drops.
    #[must_use]
    pub fn open(
        &self,
        sink: Arc<dyn CommandSink>,
        observer: Arc<dyn AgentObserver>,
        cancel: CancelToken,
    ) -> OpenTurn {
        let mut slot = self.lock();
        slot.generation = slot.generation.wrapping_add(1);
        let generation = slot.generation;
        if let Some(previous) = slot.current.take() {
            previous.cancel.cancel();
        }
        slot.current = Some(Active {
            generation,
            sink,
            observer,
            cancel: cancel.child(),
            calls: 0,
        });
        OpenTurn {
            turns: self.clone(),
            generation,
        }
    }

    /// Ends the open question and cancels its calls, without waiting — for a
    /// `Drop`, where nothing can be awaited.
    pub(crate) fn close_now(&self) {
        if let Some(active) = self.lock().current.take() {
            active.cancel.cancel();
        }
    }

    /// Lets one call through, or says why not. Counts it when it passes.
    pub(crate) fn admit(&self) -> Admission {
        let mut slot = self.lock();
        let Some(active) = slot.current.as_mut() else {
            return Admission::NoQuestion;
        };
        if active.calls >= self.max_calls {
            return Admission::LimitReached {
                max: self.max_calls,
            };
        }
        active.calls += 1;
        Admission::Admitted(Admitted {
            sink: WriteGate {
                turns: self.clone(),
                generation: active.generation,
                inner: Arc::clone(&active.sink),
            },
            observer: Arc::clone(&active.observer),
            cancel: active.cancel.clone(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Keeps a question's turn open. Dropping it takes the tools away.
pub struct OpenTurn {
    turns: ToolTurns,
    generation: u64,
}

impl std::fmt::Debug for OpenTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenTurn")
            .field("generation", &self.generation)
            .finish()
    }
}

impl Drop for OpenTurn {
    fn drop(&mut self) {
        let mut slot = self.turns.lock();
        if slot
            .current
            .as_ref()
            .is_some_and(|active| active.generation == self.generation)
            && let Some(closed) = slot.current.take()
        {
            // A call still running for this question is cancelled with it,
            // not left to report into an answer that has ended.
            closed.cancel.cancel();
        }
    }
}

/// What [`ToolTurns::admit`] decided.
pub(crate) enum Admission {
    Admitted(Admitted),
    NoQuestion,
    LimitReached { max: usize },
}

/// A call let through: what it runs with.
pub(crate) struct Admitted {
    pub(crate) sink: WriteGate,
    pub(crate) observer: Arc<dyn AgentObserver>,
    pub(crate) cancel: CancelToken,
}

/// The question's sink: one call at a time, none while a request waits.
///
/// A sink and not a check before the call: it is the one place that holds the
/// `Command` up to the executor's answer.
pub(crate) struct WriteGate {
    turns: ToolTurns,
    generation: u64,
    inner: Arc<dyn CommandSink>,
}

/// Said to the agent, and shown, when a request already waits for the user.
pub(crate) const ONE_REQUEST_AWAITING: &str = "a request from this agent is already waiting for \
     the user's approval; nothing ran. Wait for the user's decision instead of submitting another";

/// Said when the question closed before or while the call ran.
pub(crate) const QUESTION_CLOSED: &str = "the question this call answered has ended; its \
     result was discarded. If it was a write, it may or may not have been applied: do not retry \
     it, tell the user";

#[async_trait]
impl CommandSink for WriteGate {
    async fn dispatch(
        &self,
        actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        let _in_order = self.turns.order.lock().await;
        if !self.still_open() {
            return DispatchOutcome::Denied {
                reason: QUESTION_CLOSED.to_owned(),
            };
        }
        // Every command, not only those this side calls mutating: the
        // executor may reclassify a « read » into a request for approval.
        if (self.turns.waiting)() {
            return DispatchOutcome::Denied {
                reason: ONE_REQUEST_AWAITING.to_owned(),
            };
        }
        let outcome = self.inner.dispatch(actor, command, cancel).await;
        if self.still_open() {
            outcome
        } else {
            // Ambiguous, never « failed »: the command reached the executor,
            // and nobody can say whether it took effect (I-13).
            DispatchOutcome::Failed {
                class: oxyn_core::ErrorClass::Ambiguous,
                message: QUESTION_CLOSED.to_owned(),
            }
        }
    }

    /// The same gate as a command: in order, never while a request of this
    /// agent waits, and only for the question still open. The order lock is
    /// held while the user decides — the agent waits for the answer anyway, and
    /// a second call slipped in meanwhile would be a second screen.
    async fn request_sample(
        &self,
        actor: Actor,
        ask: SampleAsk,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        let _in_order = self.turns.order.lock().await;
        if !self.still_open() {
            return DispatchOutcome::Denied {
                reason: QUESTION_CLOSED.to_owned(),
            };
        }
        if (self.turns.waiting)() {
            return DispatchOutcome::Denied {
                reason: ONE_REQUEST_AWAITING.to_owned(),
            };
        }
        let outcome = self.inner.request_sample(actor, ask, cancel).await;
        if self.still_open() {
            outcome
        } else {
            // A read, so nothing to call ambiguous: the rows are dropped here,
            // and the answer nobody reads does not carry them.
            DispatchOutcome::Denied {
                reason: QUESTION_CLOSED.to_owned(),
            }
        }
    }
}

impl WriteGate {
    fn still_open(&self) -> bool {
        self.turns
            .lock()
            .current
            .as_ref()
            .is_some_and(|active| active.generation == self.generation)
    }
}

#[cfg(test)]
mod tests;
