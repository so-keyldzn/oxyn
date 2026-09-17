//! What an execution leaves behind when its future is dropped before it ends.
//!
//! `JoinSet::abort_all`, the losing branch of a `select!`, a runtime shutting
//! down: each drops the future at its current `.await`, and nothing written
//! after that `.await` ever runs. The executor's cleanup — unregistering the
//! statement, closing the result, announcing the end — lives after one, so it
//! lives here too, in a guard whose `Drop` does it when the normal path did not.
//!
//! # Why the guard never calls the server
//!
//! `Drop` cannot await, and cancellation on the server is a round trip. The
//! guard does not spawn one either, for two reasons:
//!
//! * **The driver already owns it.** The cursor is owned by the dropped future,
//!   so it is dropped with it, and the driver contract makes that drop reach
//!   the engine: SQLite interrupts a step in flight, PostgreSQL's stream task
//!   sees its token and sends `pg_cancel_backend` itself. The guard cancels the
//!   execution's token as well, which every driver was handed and watches.
//! * **A late cancel by handle can hit the wrong statement.** Sent after the
//!   statement has ended, a server cancel races the next borrower of the same
//!   connection. Everything the guard does is keyed on what belongs to this
//!   execution alone — its own child token, its own handle — so running it
//!   late, or twice, touches nothing else.
//!
//! A detached task would also outlive the application's shutdown with nobody
//! holding its handle, which `rust.md` forbids.
//!
//! # What it does not do
//!
//! It does not retry, and it does not report a write as cleanly failed: an
//! abandoned write may have been applied ([I-13](../../../CLAUDE.md#i-13)). The
//! end is announced as [`Event::Cancelled`], the same as an interrupted write
//! on the normal path, and the query history keeps the entry `Running`, which
//! it already treats as needing reconciliation before any replay.
//!
//! # The audit outcome, written later
//!
//! A command with a decision in the audit journal and no outcome is a hole in
//! the trail. Writing it from `Drop` would do disk I/O on whichever thread drops
//! the future — at shutdown, possibly the UI thread (I-05). So [`OutcomeGuard`]
//! only builds the record in memory and queues it in [`AbandonedOutcomes`];
//! [`Executor::journal_abandoned`](crate::Executor::journal_abandoned) writes
//! the queue from the blocking pool.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use oxyn_core::{
    CancelToken, CommandId, ConnectionId, ErrorClass, Event, ExecStats, StatementHandle,
};
use oxyn_data::ResultBuffer;
use oxyn_store::JournalRecord;
use parking_lot::Mutex;

use crate::cancel::CancelRegistry;
use crate::events::EventBus;

/// Armed for the lifetime of one execution; settled by its normal path.
///
/// Created before the driver is called, so an abandonment during preparation
/// is covered too. [`track`](Self::track) attaches the statement once the
/// cursor exists.
pub(crate) struct AbandonGuard<'a> {
    running: &'a CancelRegistry,
    events: &'a EventBus,
    command: CommandId,
    connection: ConnectionId,
    /// This execution's own token — a child, so cancelling it spares the tab.
    token: CancelToken,
    drained: Option<(StatementHandle, Arc<ResultBuffer>)>,
    settled: bool,
}

impl<'a> AbandonGuard<'a> {
    pub(crate) const fn new(
        running: &'a CancelRegistry,
        events: &'a EventBus,
        command: CommandId,
        connection: ConnectionId,
        token: CancelToken,
    ) -> Self {
        Self {
            running,
            events,
            command,
            connection,
            token,
            drained: None,
            settled: false,
        }
    }

    /// Attaches the registered statement and the buffer it drains into.
    pub(crate) fn track(&mut self, statement: StatementHandle, buffer: Arc<ResultBuffer>) {
        self.drained = Some((statement, buffer));
    }

    /// The normal path reached its end: unregister, and disarm.
    ///
    /// Consumes the guard, so the abandonment branch of `Drop` cannot follow.
    pub(crate) fn settle(mut self) {
        if let Some((statement, _)) = self.drained.take() {
            self.running.finish(statement);
        }
        self.settled = true;
    }
}

impl Drop for AbandonGuard<'_> {
    /// Acts only when the future was dropped before [`settle`](Self::settle).
    ///
    /// Synchronous and infallible by construction: no await, no lock that
    /// poisons, nothing that panics — a panic here, during an unwind, would
    /// abort the process.
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.token.cancel();
        if let Some((statement, buffer)) = self.drained.take() {
            if !buffer.is_complete() {
                // Whoever still holds the result must stop waiting for rows,
                // and must not read what arrived as the whole result.
                buffer.mark_truncated();
                buffer.mark_complete(ExecStats::default());
            }
            self.running.finish(statement);
        }
        tracing::debug!(command = %self.command, "execution abandoned by its caller");
        self.events
            .publish(self.command, Some(self.connection), Event::Cancelled);
    }
}

/// What the audit journal says of a command whose caller stopped waiting.
///
/// Classed [`ErrorClass::Ambiguous`]: the statement may have been applied, and
/// nothing reading the journal may take it for a clean failure (I-13).
pub(crate) const ABANDONED_OUTCOME: &str = "abandoned by its caller; outcome unknown";

/// Most outcomes waiting for the journal; one per abandoned command.
///
/// Far above what a second of abandonments produces — `JoinSet::abort_all`
/// on an agent's connections is a handful — and small enough that a writer
/// that stopped cannot grow memory without bound.
pub(crate) const MAX_PENDING_OUTCOMES: usize = 256;

/// Audit outcomes built on drop, waiting for a thread allowed to write them.
#[derive(Default)]
pub(crate) struct AbandonedOutcomes {
    queue: Mutex<VecDeque<JournalRecord>>,
    /// Outcomes dropped since start: queue full, or its lock held on drop.
    lost: AtomicUsize,
}

impl AbandonedOutcomes {
    /// Queues without blocking; never waits for the lock.
    fn push(&self, record: JournalRecord) {
        let Some(mut queue) = self.queue.try_lock() else {
            self.lose("the queue was being emptied");
            return;
        };
        if queue.len() >= MAX_PENDING_OUTCOMES {
            drop(queue);
            self.lose("the queue is full");
            return;
        }
        queue.push_back(record);
    }

    /// A count and a reason, never the command: its text is audit content.
    fn lose(&self, reason: &'static str) {
        let lost = self.lost.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        tracing::error!(
            lost,
            reason,
            "an abandoned command's audit outcome was lost"
        );
    }

    /// Everything queued so far, leaving the queue empty.
    pub(crate) fn take(&self) -> VecDeque<JournalRecord> {
        std::mem::take(&mut *self.queue.lock())
    }
}

impl std::fmt::Debug for AbandonedOutcomes {
    /// Counts only: a queued record carries the statement text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbandonedOutcomes")
            .field("lost", &self.lost.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// Armed around one command's execution; settled once its outcome is known.
pub(crate) struct OutcomeGuard<'a> {
    outcomes: &'a AbandonedOutcomes,
    command: CommandId,
    actor: &'a oxyn_core::Actor,
    request: &'a oxyn_core::Command,
    approved_by: Option<&'a str>,
    started: Instant,
    settled: bool,
}

impl<'a> OutcomeGuard<'a> {
    pub(crate) fn new(
        outcomes: &'a AbandonedOutcomes,
        command: CommandId,
        actor: &'a oxyn_core::Actor,
        request: &'a oxyn_core::Command,
        approved_by: Option<&'a str>,
    ) -> Self {
        Self {
            outcomes,
            command,
            actor,
            request,
            approved_by,
            started: Instant::now(),
            settled: false,
        }
    }

    /// The outcome is known and will be journaled by the caller.
    pub(crate) fn settle(mut self) {
        self.settled = true;
    }
}

impl Drop for OutcomeGuard<'_> {
    /// Builds the record in memory and queues it; no disk, no blocking lock.
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut record = crate::executor::outcome_record(
            self.command,
            self.actor,
            self.request,
            self.started.elapsed(),
            None,
            self.approved_by,
        );
        record.error = Some(ABANDONED_OUTCOME.to_owned());
        record.error_class = Some(ErrorClass::Ambiguous);
        self.outcomes.push(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{Actor, Command};

    fn record() -> JournalRecord {
        crate::executor::outcome_record(
            CommandId::new(),
            &Actor::Human,
            &Command::Disconnect {
                connection: ConnectionId::new(),
            },
            std::time::Duration::ZERO,
            None,
            None,
        )
    }

    #[test]
    fn a_full_queue_counts_what_it_drops_and_keeps_its_bound() {
        let outcomes = AbandonedOutcomes::default();
        for _ in 0..MAX_PENDING_OUTCOMES + 3 {
            outcomes.push(record());
        }
        assert_eq!(outcomes.lost.load(Ordering::Relaxed), 3);
        assert_eq!(outcomes.take().len(), MAX_PENDING_OUTCOMES);
        assert!(outcomes.take().is_empty());
    }

    #[test]
    fn a_drop_never_waits_for_the_queue_lock() {
        let outcomes = AbandonedOutcomes::default();
        let held = outcomes.queue.lock();
        // A blocking `lock` would deadlock here, a timed one would stall.
        let started = Instant::now();
        outcomes.push(record());
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "the drop waited for the lock"
        );
        drop(held);
        assert_eq!(outcomes.lost.load(Ordering::Relaxed), 1);
        assert!(outcomes.take().is_empty());
    }
}
