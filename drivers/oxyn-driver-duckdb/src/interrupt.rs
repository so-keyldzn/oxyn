//! Targeted interruption: stop only the targeted work, never the next one.
//!
//! # Why `duckdb_interrupt` alone is not enough
//!
//! `duckdb_interrupt` targets the **connection**: it raises a flag that the
//! running query reads, and DuckDB lowers it when the next query starts
//! (`ClientContext::InitialCleanup`, `src/main/client_context.cpp`, v1.5.6).
//! Two consequences:
//!
//! * **a late interruption is lost, or hits the neighbor.** A tab closed at the
//!   instant its query finishes: the carrier thread has moved on, and the flag
//!   lands on whatever runs next;
//! * **an early interruption is lost.** Raised between two statements of a
//!   batch, it is lowered when the next statement starts, which then runs to its
//!   end.
//!
//! # What this module guarantees
//!
//! Each task handed to the carrier thread gets a [`WorkId`]. The thread
//! declares, **under a lock**, which one it runs; an interruption for `id`
//! reaches the engine only if `id` is that one, checked under the same lock.
//! An interruption for a queued task marks it abandoned: the thread drops it
//! instead of running it for nobody. For a finished task, nothing happens.
//!
//! The early interruption is caught by a flag of the task,
//! [`Interrupter::tripped`], which the stream reads before starting each
//! statement.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use duckdb::InterruptHandle;
use parking_lot::Mutex;

/// The identity of a task handed to the carrier thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorkId(u64);

/// What the carrier thread is doing, seen under the lock.
#[derive(Default)]
struct Turn {
    running: Option<WorkId>,
    /// Queued tasks, and whether each was abandoned.
    queued: HashMap<WorkId, bool>,
}

/// A connection's interrupter, shared by the carrier thread and those who hand
/// it work.
pub(crate) struct Interrupter {
    engine: Arc<InterruptHandle>,
    turn: Mutex<Turn>,
    /// Raised by an interruption of the current task; lowered when a task
    /// starts. Survives the engine lowering its own flag.
    tripped: AtomicBool,
    next: AtomicU64,
}

impl Interrupter {
    pub(crate) fn new(engine: Arc<InterruptHandle>) -> Self {
        Self {
            engine,
            turn: Mutex::new(Turn::default()),
            tripped: AtomicBool::new(false),
            next: AtomicU64::new(0),
        }
    }

    /// Reserves a task's identity **before** it enters the queue: an
    /// interruption arriving between the send and the registration would find
    /// it neither running nor queued, and take it for finished.
    pub(crate) fn enqueue(&self) -> WorkId {
        let id = WorkId(self.next.fetch_add(1, Ordering::Relaxed));
        self.turn.lock().queued.insert(id, false);
        id
    }

    /// Removes a task that could not enter the queue.
    pub(crate) fn withdraw(&self, id: WorkId) {
        self.turn.lock().queued.remove(&id);
    }

    /// The carrier thread takes task `id`; `false` if it was abandoned while it
    /// waited, and must be dropped unexecuted.
    pub(crate) fn begin(&self, id: WorkId) -> bool {
        let mut turn = self.turn.lock();
        if turn.queued.remove(&id).unwrap_or(false) {
            return false;
        }
        turn.running = Some(id);
        self.tripped.store(false, Ordering::SeqCst);
        true
    }

    /// The carrier thread finished the current task, its statements and
    /// results released.
    pub(crate) fn end(&self) {
        self.turn.lock().running = None;
    }

    /// Interrupts task `id`, and it alone.
    ///
    /// Returns `true` when the task was still queued: it is then proven never
    /// to run.
    pub(crate) fn interrupt(&self, id: WorkId) -> bool {
        let mut turn = self.turn.lock();
        if turn.running == Some(id) {
            self.tripped.store(true, Ordering::SeqCst);
            // Under the lock: `end` then `begin` cannot start the next task
            // before the engine flag is raised for this one.
            self.engine.interrupt();
        } else if let Some(abandoned) = turn.queued.get_mut(&id) {
            *abandoned = true;
            return true;
        }
        false
    }

    /// Whether the current task was interrupted: read before each statement
    /// starts, so that an interruption raised between two statements is not
    /// lowered by the engine when the next one begins.
    pub(crate) fn tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }

    /// The current task, for tests waiting for the thread to pick it up.
    #[cfg(test)]
    pub(crate) fn running(&self) -> Option<WorkId> {
        self.turn.lock().running
    }
}

impl std::fmt::Debug for Interrupter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interrupter")
            .field("tripped", &self.tripped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// Interrupts a task if its wait is abandoned before the reply.
///
/// Carried by the future of a wait: a destroyed future cannot `await`, but it
/// can raise a flag. Without it, closing a tab during `execute` would leave the
/// engine computing a four-minute aggregate for nobody, the session blocked
/// behind it.
pub(crate) struct AbandonGuard<'a> {
    interrupter: &'a Interrupter,
    id: WorkId,
    armed: bool,
}

impl<'a> AbandonGuard<'a> {
    pub(crate) const fn new(interrupter: &'a Interrupter, id: WorkId) -> Self {
        Self {
            interrupter,
            id,
            armed: true,
        }
    }

    /// The reply arrived: nothing left to interrupt.
    pub(crate) const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AbandonGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.interrupter.interrupt(self.id);
        }
    }
}
