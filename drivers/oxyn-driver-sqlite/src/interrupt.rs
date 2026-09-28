//! Targeted interruption: stop only the targeted work, never the next one.
//!
//! # Why `sqlite3_interrupt` alone is not enough
//!
//! `sqlite3_interrupt` targets the **connection**, not a statement. The engine
//! sets a flag that every running statement reads, and resets it only when a
//! statement starts while no other is active (`sqlite3_step` and
//! `sqlite3RunParser`, SQLite 3.50.2). Two consequences:
//!
//! * **a late interruption hits the neighbor.** A tab closed at the exact moment
//!   its query finishes: the worker thread has already moved on to the next
//!   query, and that one dies, without anyone asking for it;
//! * **an early interruption is lost.** Set during a preparation or between two
//!   statements of a batch, it is cleared when the next statement starts, which
//!   then runs to the end.
//!
//! # What this module guarantees
//!
//! Each task handed to the worker thread receives a [`WorkId`]. The thread
//! declares, **under a lock**, which one it is running; an interruption for `id`
//! goes to the engine only if `id` is that one, checked under the **same** lock.
//! The thread therefore cannot move on to the next work while an interruption
//! for the previous one is in flight.
//!
//! An interruption for a task still **queued** marks it abandoned: the thread
//! skips it instead of running it for nobody. An interruption for a **finished**
//! task does nothing.
//!
//! The early interruption is caught by a flag specific to the task,
//! [`Interrupter::checkpoint`], which the stream consults just before launching
//! each statement. A window of a few machine instructions remains, between this
//! check and the reset done by `sqlite3_step`: an interruption landing there lets
//! the statement run to its end, and the result goes to a caller that no longer
//! listens. It never hits the next task.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use oxyn_core::Result;
use parking_lot::Mutex;
use rusqlite::InterruptHandle;

use crate::error::{self, Bound, Effect};

/// The identity of a task handed to the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorkId(u64);

/// What the worker thread is doing, seen under the lock.
#[derive(Default)]
struct Turn {
    /// The task being executed, if there is one.
    running: Option<WorkId>,
    /// The queued tasks, and for each: has it been abandoned?
    queued: HashMap<WorkId, bool>,
}

/// A connection's interrupter, shared between the worker thread and those who
/// hand it work.
pub(crate) struct Interrupter {
    engine: InterruptHandle,
    turn: Mutex<Turn>,
    /// Set by an interruption targeting the current task; reset when a task
    /// starts. Survives the reset done by the engine.
    tripped: AtomicBool,
    next: AtomicU64,
}

impl Interrupter {
    /// The interrupter of the connection whose handle is `engine`.
    pub(crate) fn new(engine: InterruptHandle) -> Self {
        Self {
            engine,
            turn: Mutex::new(Turn::default()),
            tripped: AtomicBool::new(false),
            next: AtomicU64::new(0),
        }
    }

    /// Reserves a task's identity **before** it goes into the queue.
    ///
    /// Before, and not after: an interruption arriving between the send and the
    /// registration would find the task neither running nor queued, and would
    /// believe it finished.
    pub(crate) fn enqueue(&self) -> WorkId {
        let id = WorkId(self.next.fetch_add(1, Ordering::Relaxed));
        self.turn.lock().queued.insert(id, false);
        id
    }

    /// Removes a task that could not go into the queue.
    pub(crate) fn withdraw(&self, id: WorkId) {
        self.turn.lock().queued.remove(&id);
    }

    /// The worker thread takes task `id`.
    ///
    /// Returns `false` if it was abandoned while it waited: it must then be
    /// dropped without being executed.
    pub(crate) fn begin(&self, id: WorkId) -> bool {
        let mut turn = self.turn.lock();
        if turn.queued.remove(&id).unwrap_or(false) {
            return false;
        }
        turn.running = Some(id);
        self.tripped.store(false, Ordering::SeqCst);
        true
    }

    /// The worker thread has finished the current task, **statements included**:
    /// none is active on the connection anymore.
    pub(crate) fn end(&self) {
        self.turn.lock().running = None;
    }

    /// Interrupts task `id`, and it alone.
    ///
    /// Running: the engine is interrupted. Queued: it will not be executed.
    /// Finished: nothing.
    pub(crate) fn interrupt(&self, id: WorkId) {
        let mut turn = self.turn.lock();
        if turn.running == Some(id) {
            self.tripped.store(true, Ordering::SeqCst);
            // Under the lock: that is what prevents `end` then `begin` from
            // starting the next task before the engine flag is set. Once set
            // during task `id`, it is cleared by the engine when the next one
            // starts.
            self.engine.interrupt();
        } else if let Some(abandoned) = turn.queued.get_mut(&id) {
            *abandoned = true;
        }
    }

    /// Refuses to launch a statement if the current task was interrupted.
    ///
    /// To be called **just before** starting a statement. The error is the one the
    /// interrupted engine would have returned during its first step, classified
    /// according to what the statement could do.
    ///
    /// # Errors
    /// The interruption, translated as [`error::engine_bound`] translates it.
    pub(crate) fn checkpoint(&self, effect: Effect, bound: Bound) -> Result<()> {
        if !self.tripped.load(Ordering::SeqCst) {
            return Ok(());
        }
        let interrupted = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
            None,
        );
        Err(error::engine_bound(interrupted, effect, bound))
    }

    /// The current task, for tests waiting for the thread to pick it up.
    #[cfg(test)]
    pub(crate) fn running(&self) -> Option<WorkId> {
        self.turn.lock().running
    }
}

/// Interrupts a task if it is abandoned before its reply.
///
/// The future of a wait is what carries this guard: a destroyed future cannot
/// `await`, but it can set a flag. Without it, closing a tab during `execute`
/// would leave the engine computing the first batch of a four-minute `count(*)`
/// for nobody, with the session blocked behind it.
pub(crate) struct AbandonGuard<'a> {
    interrupter: &'a Interrupter,
    id: WorkId,
    armed: bool,
}

impl<'a> AbandonGuard<'a> {
    /// Arms the guard for task `id`.
    pub(crate) const fn new(interrupter: &'a Interrupter, id: WorkId) -> Self {
        Self {
            interrupter,
            id,
            armed: true,
        }
    }

    /// The reply has arrived: nothing left to interrupt.
    pub(crate) const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AbandonGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.interrupter.interrupt(self.id);
        }
    }
}

impl std::fmt::Debug for Interrupter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interrupter")
            .field("tripped", &self.tripped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}
