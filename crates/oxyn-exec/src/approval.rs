//! Commands awaiting approval, and their expiry.
//!
//! When the `PolicyGate` answers
//! [`RequireApproval`](oxyn_core::Decision::RequireApproval), **nothing
//! executes**. The command — already reclassified, as it will run if it is
//! accepted — is set aside here, and the interface receives an
//! [`Event::ApprovalRequested`](oxyn_core::Event) carrying its [`CommandId`].
//!
//! # Three properties, and what each prevents
//!
//! **What is set aside is what will be executed.** The stored command is the
//! one the gate saw, not the original text. Without that, the exact gap that
//! reclassification closes would reopen: approving a `SELECT` and executing a
//! `DELETE`.
//!
//! **An approval is used only once.** [`take`](ApprovalRegistry::take)
//! **removes** the entry. A replayable approval is an approval an agent can
//! replay.
//!
//! **An approval expires.** A request left on screen for a whole night no
//! longer concerns the same state of the world: the table changed, the
//! connection may have been re-marked production. Past the delay, the command
//! is not executed — it is re-emitted, and goes through the gate again.
//!
//! Expiry is checked **on removal**, not by a background task: a cleanup clock
//! that does not run would leave a stale entry usable, whereas a check on
//! removal cannot be forgotten. [`sweep`](ApprovalRegistry::sweep) only exists
//! to clear the display.
//!
//! A stale request **is still reported stale** once removed: the registry
//! keeps the last expired identifiers, and an approval arriving afterwards
//! says "expired", not "no command is waiting". Without that, the next request
//! — which purges stale ones so as not to count them in the bound — changed
//! the answer given to the user, who then read that their request had never
//! existed.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use oxyn_core::{Actor, Command, CommandId, OxynError, Preview};
use parking_lot::Mutex;

/// Delay after which an approval request stops being valid.
///
/// Five minutes: enough to read the statement, compare it with what was
/// expected and decide; too short for a forgotten request to be accepted by
/// reflex the next morning.
pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);

/// Maximum number of simultaneously pending commands.
///
/// A bound, because an agent in a loop would otherwise produce an endless
/// queue — and an endless queue is a memory leak *and* an unusable interface.
/// Beyond it, new requests are refused, old ones are not evicted: evicting
/// would make the user believe they answered a request that has disappeared.
pub const DEFAULT_CAPACITY: usize = 64;

/// A command set aside while waiting for an approval.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingCommand {
    /// The identifier under which the approval will be given. It is also the
    /// correlation key with the audit log.
    pub id: CommandId,
    /// Who asked. **Kept as is**: approving an agent's command does not make it
    /// a human command, and the log must keep saying who wrote it.
    pub actor: Actor,
    /// The **reclassified** command, as it will be executed.
    pub command: Command,
    /// What the user must decide on.
    pub reason: String,
    /// What is needed to judge without reading elsewhere.
    pub preview: Option<Preview>,
    /// When the approval was requested.
    pub requested_at: Instant,
    /// When the request stops being valid.
    pub expires_at: Instant,
}

impl PendingCommand {
    /// Is the request stale at instant `now`?
    #[must_use]
    pub fn is_expired_at(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    /// Is the request stale?
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Instant::now())
    }

    /// How long the request has been waiting.
    #[must_use]
    pub fn waiting_for(&self) -> Duration {
        self.requested_at.elapsed()
    }
}

/// What can keep an approval from going through.
///
/// None of these variants describes a failure: they are the three ways an
/// approval can be moot. They all translate into a denial — never into an
/// execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalError {
    /// No command is waiting under this identifier.
    ///
    /// Either the approval was already given — an approval is used only once
    /// —, or the request was rejected in the meantime.
    #[error("no command is awaiting approval under this identifier")]
    Unknown,

    /// The request expired. **Nothing was executed.**
    #[error("the approval request expired after {after:?}: nothing was executed")]
    Expired {
        /// Delay after which the request stopped being valid.
        after: Duration,
    },

    /// The waiting queue is full.
    #[error("too many commands are awaiting approval ({limit}): answer the pending requests")]
    QueueFull {
        /// The bound reached.
        limit: usize,
    },

    /// A request is already waiting under this identifier.
    ///
    /// Replacing it would change what an approval being read approves: the
    /// user would read one statement and validate another (ADR-0037 § 2).
    #[error("a command is already awaiting approval under this identifier")]
    AlreadyPending,
}

impl From<ApprovalError> for OxynError {
    /// A moot approval is a **denial**, not an internal error.
    ///
    /// The caller — the interface as well as the agent runtime — must conclude
    /// that the command did not take place and will not; that is exactly what
    /// [`PolicyDenied`](OxynError::PolicyDenied) says.
    fn from(err: ApprovalError) -> Self {
        Self::PolicyDenied {
            reason: err.to_string(),
        }
    }
}

/// The commands awaiting approval.
#[derive(Debug)]
pub struct ApprovalRegistry {
    ttl: Duration,
    capacity: usize,
    queue: Mutex<Queue>,
}

/// What is waiting, and what expired without an answer.
#[derive(Debug, Default)]
struct Queue {
    pending: HashMap<CommandId, PendingCommand>,
    /// The last identifiers removed because stale, oldest first. Bounded by the
    /// capacity: it is the memory of a full queue, not a history.
    expired: VecDeque<CommandId>,
}

impl Queue {
    fn remember_expired(&mut self, id: CommandId, capacity: usize) {
        if capacity == 0 {
            return;
        }
        while self.expired.len() >= capacity {
            self.expired.pop_front();
        }
        self.expired.push_back(id);
    }

    /// Removes the requests stale at `now`, remembering them, and returns them.
    fn purge(&mut self, now: Instant, capacity: usize) -> Vec<PendingCommand> {
        let stale: Vec<CommandId> = self
            .pending
            .values()
            .filter(|e| e.is_expired_at(now))
            .map(|e| e.id)
            .collect();
        let mut removed = Vec::with_capacity(stale.len());
        for id in stale {
            if let Some(entry) = self.pending.remove(&id) {
                self.remember_expired(id, capacity);
                removed.push(entry);
            }
        }
        removed
    }

    /// Has the identifier expired? Forgotten once read: the "expired" answer is
    /// given once, as an approval is used only once.
    fn forget_expired(&mut self, id: CommandId) -> bool {
        let before = self.expired.len();
        self.expired.retain(|expired| *expired != id);
        self.expired.len() != before
    }
}

impl ApprovalRegistry {
    /// Registry with default settings: [`DEFAULT_TTL`], [`DEFAULT_CAPACITY`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }

    /// Registry with a chosen validity duration.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            capacity: DEFAULT_CAPACITY,
            queue: Mutex::new(Queue::default()),
        }
    }

    /// Sets the maximum number of simultaneous requests.
    #[must_use]
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// The validity duration of a request.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Sets a command aside and returns the created request.
    ///
    /// `command` must be the **reclassified** command: it is the one that will
    /// be executed if the approval is given.
    ///
    /// # Errors
    /// [`ApprovalError::QueueFull`] when the bound of simultaneous requests is
    /// reached, [`ApprovalError::AlreadyPending`] when a request is already
    /// waiting under `id`. The command is then not set aside, and the caller
    /// must treat it as denied.
    pub fn submit(
        &self,
        id: CommandId,
        actor: Actor,
        command: Command,
        reason: impl Into<String>,
        preview: Option<Preview>,
    ) -> Result<PendingCommand, ApprovalError> {
        let now = Instant::now();
        let entry = PendingCommand {
            id,
            actor,
            command,
            reason: reason.into(),
            preview,
            requested_at: now,
            expires_at: now + self.ttl,
        };

        let mut guard = self.queue.lock();
        // Stale ones do not count in the bound: otherwise a queue filled with
        // dead requests would block the product until restart.
        guard.purge(now, self.capacity);
        if guard.pending.len() >= self.capacity {
            return Err(ApprovalError::QueueFull {
                limit: self.capacity,
            });
        }
        if guard.pending.contains_key(&id) {
            return Err(ApprovalError::AlreadyPending);
        }
        guard.pending.insert(id, entry.clone());
        Ok(entry)
    }

    /// Removes the approved command, if it is still valid.
    ///
    /// The entry is removed **in every case** — approval given or request
    /// stale: what was presented once must not be approvable a second time.
    ///
    /// # Errors
    /// [`ApprovalError::Expired`] if the request expired, whether it is still
    /// there or already removed for that reason; [`ApprovalError::Unknown`] if
    /// nothing is waiting under this identifier — nothing is executed in
    /// either case.
    pub fn take(&self, id: CommandId) -> Result<PendingCommand, ApprovalError> {
        self.take_at(id, Instant::now())
    }

    /// [`take`](Self::take), at a given instant. Reserved for tests.
    #[doc(hidden)]
    pub fn take_at(&self, id: CommandId, now: Instant) -> Result<PendingCommand, ApprovalError> {
        let mut guard = self.queue.lock();
        let Some(entry) = guard.pending.remove(&id) else {
            return Err(if guard.forget_expired(id) {
                ApprovalError::Expired { after: self.ttl }
            } else {
                ApprovalError::Unknown
            });
        };
        if entry.is_expired_at(now) {
            return Err(ApprovalError::Expired { after: self.ttl });
        }
        Ok(entry)
    }

    /// Removes a request the user answered "no" to.
    ///
    /// An explicit denial is not an error: there is nothing to report beyond
    /// the fact that the command will not take place.
    pub fn reject(&self, id: CommandId) -> Option<PendingCommand> {
        self.queue.lock().pending.remove(&id)
    }

    /// Removes a request **as stale**, whether or not its delay has passed on
    /// this registry's clock.
    ///
    /// For whoever holds a request's deadline themselves — the call of an agent
    /// waiting for it: it removes it when its wait ends, and an approval
    /// arriving afterwards must say "expired". `None` if it was no longer
    /// waiting: already decided, or removed.
    pub fn expire(&self, id: CommandId) -> Option<PendingCommand> {
        let mut guard = self.queue.lock();
        let entry = guard.pending.remove(&id)?;
        guard.remember_expired(id, self.capacity);
        Some(entry)
    }

    /// What is waiting for an answer, without removing anything.
    ///
    /// The order is not guaranteed: it is up to the interface to sort what it
    /// displays, on [`PendingCommand::requested_at`].
    #[must_use]
    pub fn pending(&self) -> Vec<PendingCommand> {
        self.queue.lock().pending.values().cloned().collect()
    }

    /// The request waiting under `id`, without removing it.
    ///
    /// For whoever must know what an approval is about **before** giving it —
    /// ADR-0037's native dialog; [`take`](Self::take) remains the only
    /// removal, and the only one that checks expiry.
    #[must_use]
    pub fn peek(&self, id: CommandId) -> Option<PendingCommand> {
        self.queue.lock().pending.get(&id).cloned()
    }

    /// Number of pending requests, stale ones included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.lock().pending.len()
    }

    /// No pending request?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.lock().pending.is_empty()
    }

    /// Removes stale requests and returns them.
    ///
    /// Purely cosmetic: expiry is already checked by [`take`](Self::take),
    /// which cannot be forgotten. This serves to remove from the screen
    /// requests it is no longer any use answering.
    pub fn sweep(&self) -> Vec<PendingCommand> {
        self.queue.lock().purge(Instant::now(), self.capacity)
    }
}

impl Default for ApprovalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        AgentId, AgentSessionId, ConnectionId, ExecRequest, QueryLanguage, SessionId,
        StatementIntent,
    };

    fn sample_command() -> Command {
        Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "DELETE FROM commandes")
                    .with_intent(StatementIntent::Write),
            ),
        }
    }

    fn agent() -> Actor {
        Actor::agent(AgentId::new(), AgentSessionId::new())
    }

    #[test]
    fn an_approval_is_used_only_once() {
        // A replayable approval is an approval an agent can replay.
        let registry = ApprovalRegistry::new();
        let id = CommandId::new();
        registry
            .submit(id, agent(), sample_command(), "write by an agent", None)
            .expect("the queue is empty");

        assert!(registry.take(id).is_ok());
        assert_eq!(registry.take(id), Err(ApprovalError::Unknown));
        assert!(registry.is_empty());
    }

    #[test]
    fn a_stale_request_executes_nothing() {
        let registry = ApprovalRegistry::with_ttl(Duration::ZERO);
        let id = CommandId::new();
        registry
            .submit(id, agent(), sample_command(), "write by an agent", None)
            .expect("the queue is empty");

        let outcome = registry.take(id);
        assert!(
            matches!(outcome, Err(ApprovalError::Expired { .. })),
            "{outcome:?}"
        );
        // And it was removed: it cannot be "caught up".
        assert!(registry.is_empty());
    }

    #[test]
    fn a_stale_approval_becomes_a_denial_not_a_failure() {
        let error: OxynError = ApprovalError::Expired {
            after: Duration::from_secs(300),
        }
        .into();
        assert!(matches!(error, OxynError::PolicyDenied { .. }));
        assert!(error.is_user_error(), "{error:?}");
        assert!(!error.is_retryable(), "a stale approval is not replayed");
    }

    #[test]
    fn the_command_set_aside_is_the_one_that_will_run() {
        // It is stored as is: approving one text and executing another would
        // reopen the gap that reclassification closes.
        let registry = ApprovalRegistry::new();
        let id = CommandId::new();
        let cmd = sample_command();
        registry
            .submit(id, Actor::Human, cmd.clone(), "production", None)
            .expect("the queue is empty");

        let resumption = registry.take(id).expect("approval given");
        assert_eq!(resumption.command, cmd);
        assert_eq!(resumption.actor, Actor::Human);
    }

    #[test]
    fn a_pending_request_is_not_replaced() {
        // Replaced while a dialog shows it, the approval given to what was read
        // would execute something else (ADR-0037).
        let registry = ApprovalRegistry::new();
        let id = CommandId::new();
        let read_back = sample_command();
        registry
            .submit(id, Actor::Human, read_back.clone(), "production", None)
            .expect("the queue is empty");
        let other = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(ExecRequest::new(QueryLanguage::SQL, "DROP TABLE audit")),
        };
        assert_eq!(
            registry
                .submit(id, Actor::Human, other, "production", None)
                .err(),
            Some(ApprovalError::AlreadyPending)
        );
        assert_eq!(
            registry.take(id).expect("approval given").command,
            read_back
        );
    }

    #[test]
    fn approving_does_not_change_the_actor() {
        // An approved agent command stays an agent command: the log must keep
        // saying who wrote it.
        let registry = ApprovalRegistry::new();
        let id = CommandId::new();
        let who = agent();
        registry
            .submit(id, who, sample_command(), "write by an agent", None)
            .expect("the queue is empty");

        let resumption = registry.take(id).expect("approval given");
        assert!(resumption.actor.is_agent());
        assert_eq!(resumption.actor, who);
    }

    #[test]
    fn the_queue_is_bounded() {
        let registry = ApprovalRegistry::new().with_capacity(2);
        for _ in 0..2 {
            registry
                .submit(CommandId::new(), agent(), sample_command(), "motif", None)
                .expect("under the bound");
        }
        let outcome = registry.submit(CommandId::new(), agent(), sample_command(), "motif", None);
        assert_eq!(
            outcome.err(),
            Some(ApprovalError::QueueFull { limit: 2 }),
            "the bound must refuse the new request"
        );
        assert_eq!(registry.len(), 2, "no old one was evicted");
    }

    #[test]
    fn stale_requests_do_not_block_the_queue() {
        let registry = ApprovalRegistry::with_ttl(Duration::ZERO).with_capacity(1);
        registry
            .submit(CommandId::new(), agent(), sample_command(), "motif", None)
            .expect("queue not empty");
        // The previous one is stale: it no longer counts in the bound.
        registry
            .submit(CommandId::new(), agent(), sample_command(), "motif", None)
            .expect("the stale one made room");
    }

    #[test]
    fn the_sweep_removes_only_stale_ones() {
        let live = ApprovalRegistry::new();
        let id = CommandId::new();
        live.submit(id, Actor::Human, sample_command(), "motif", None)
            .expect("queue not empty");
        assert!(live.sweep().is_empty());
        assert_eq!(live.len(), 1);

        let dead = ApprovalRegistry::with_ttl(Duration::ZERO);
        dead.submit(
            CommandId::new(),
            Actor::Human,
            sample_command(),
            "motif",
            None,
        )
        .expect("queue not empty");
        assert_eq!(dead.sweep().len(), 1);
        assert!(dead.is_empty());
    }

    #[test]
    fn a_request_purged_by_the_next_is_still_reported_stale() {
        // The regression: the next request purged the stale one, and the late
        // approval read "no command is awaiting approval" — a request that
        // would never have existed.
        let registry = ApprovalRegistry::with_ttl(Duration::ZERO);
        let stale = CommandId::new();
        registry
            .submit(stale, agent(), sample_command(), "motif", None)
            .expect("queue not empty");
        registry
            .submit(CommandId::new(), agent(), sample_command(), "motif", None)
            .expect("the stale one made room");

        let outcome = registry.take(stale);
        assert!(
            matches!(outcome, Err(ApprovalError::Expired { .. })),
            "{outcome:?}"
        );
        // Said once: afterwards, nothing waits under this identifier any more.
        assert_eq!(registry.take(stale), Err(ApprovalError::Unknown));
    }

    #[test]
    fn expiring_removes_the_request_and_the_late_approval_says_so() {
        let registry = ApprovalRegistry::new();
        let id = CommandId::new();
        registry
            .submit(id, agent(), sample_command(), "motif", None)
            .expect("queue not empty");

        assert!(registry.expire(id).is_some());
        assert!(registry.is_empty());
        assert!(registry.expire(id).is_none(), "already removed");
        let outcome = registry.take(id);
        assert!(
            matches!(outcome, Err(ApprovalError::Expired { .. })),
            "{outcome:?}"
        );
    }

    #[test]
    fn the_memory_of_stale_ones_is_bounded() {
        let registry = ApprovalRegistry::new().with_capacity(2);
        let ids: Vec<CommandId> = (0..3).map(|_| CommandId::new()).collect();
        for id in &ids {
            registry
                .submit(*id, agent(), sample_command(), "motif", None)
                .expect("under the bound");
            registry.expire(*id);
        }
        // The oldest is forgotten: the bound also applies to memory.
        assert_eq!(registry.take(ids[0]), Err(ApprovalError::Unknown));
        assert!(matches!(
            registry.take(ids[2]),
            Err(ApprovalError::Expired { .. })
        ));
    }

    #[test]
    fn an_explicit_denial_removes_the_request() {
        let registry = ApprovalRegistry::new();
        let id = CommandId::new();
        registry
            .submit(id, agent(), sample_command(), "motif", None)
            .expect("queue not empty");
        assert!(registry.reject(id).is_some());
        assert_eq!(registry.take(id), Err(ApprovalError::Unknown));
    }
}
