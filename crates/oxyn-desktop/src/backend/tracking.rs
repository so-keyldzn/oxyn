//! The cancellation tokens the front reaches by the command id it chose.
//!
//! Three races live here, each of which used to lose a cancellation or cancel
//! the wrong command:
//!
//! - an id tracked twice: the second token replaced the first, and the first
//!   command could no longer be cancelled. Refused now;
//! - a guard releasing an entry that is no longer its own: removed the token of
//!   whoever held the id after it. Each entry carries a generation, and only
//!   its own guard removes it;
//! - a cancellation arriving before `track`: the front sends `cancel` as soon
//!   as the user asks, while the command may still be reading its connection
//!   on the blocking pool. It was answered « not running » and forgotten, and
//!   the command then ran to completion. It is retained now, for a short while,
//!   and applied when the id is tracked — unless the id already ran: a pending
//!   approval keeps the id of the command that asked for it, and a « Stop »
//!   clicked as that command ended must not cancel the approval that follows.

use std::collections::{HashMap, VecDeque};
use std::ops::Deref;
use std::time::{Duration, Instant};

use oxyn_core::{CancelToken, CommandId};

use super::{Backend, Inner};
use crate::ipc::IpcError;

/// How many early cancellations are retained. The front sends ids of its own,
/// and one that cancels in a loop must grow nothing.
const EARLY_CANCELS: usize = 64;

/// How long an early cancellation waits for its command. Well above the time a
/// command spends before `track` — a store read on the blocking pool — and
/// short enough that a stray one does not wait for an id forever.
const EARLY_CANCEL_WINDOW: Duration = Duration::from_secs(30);

/// How many released ids are remembered, so that a cancel arriving after its
/// command ended is not mistaken for one arriving before it.
const RELEASED: usize = 256;

/// The running commands, and the cancellations that arrived before them.
#[derive(Default)]
pub(crate) struct Tracker {
    running: HashMap<CommandId, Entry>,
    early: VecDeque<(CommandId, Instant)>,
    released: VecDeque<CommandId>,
    next_generation: u64,
}

struct Entry {
    token: CancelToken,
    generation: u64,
}

impl Tracker {
    /// Registers `id`, cancelled at once if a cancellation was waiting for it.
    ///
    /// Returns `None` if `id` is already running: a second command under the
    /// same id would make the first unreachable.
    fn register(&mut self, id: CommandId, now: Instant) -> Option<(CancelToken, u64)> {
        if self.running.contains_key(&id) {
            return None;
        }
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        let token = CancelToken::new();
        if self.take_early(id, now) {
            token.cancel();
        }
        self.running.insert(
            id,
            Entry {
                token: token.clone(),
                generation,
            },
        );
        Some((token, generation))
    }

    /// Cancels `id` if it runs, and retains the request if it has not run yet.
    ///
    /// Returns whether a running command was reached.
    fn cancel(&mut self, id: CommandId, now: Instant) -> bool {
        if let Some(entry) = self.running.get(&id) {
            entry.token.cancel();
            return true;
        }
        if self.released.contains(&id) {
            return false;
        }
        self.forget_expired(now);
        if !self.early.iter().any(|(early, _)| *early == id) {
            if self.early.len() >= EARLY_CANCELS {
                self.early.pop_front();
            }
            self.early.push_back((id, now));
        }
        false
    }

    /// Removes `id` only if the entry is still the one `generation` created.
    fn release(&mut self, id: CommandId, generation: u64) {
        if self
            .running
            .get(&id)
            .is_some_and(|entry| entry.generation == generation)
        {
            self.running.remove(&id);
            if self.released.len() >= RELEASED {
                self.released.pop_front();
            }
            self.released.push_back(id);
        }
    }

    fn take_early(&mut self, id: CommandId, now: Instant) -> bool {
        self.forget_expired(now);
        match self.early.iter().position(|(early, _)| *early == id) {
            Some(index) => {
                self.early.remove(index);
                true
            }
            None => false,
        }
    }

    fn forget_expired(&mut self, now: Instant) {
        while self
            .early
            .front()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) > EARLY_CANCEL_WINDOW)
        {
            self.early.pop_front();
        }
    }
}

/// A tracked command: its token, released from the tracker when dropped,
/// however the dispatch ends.
pub(crate) struct Tracked<'a> {
    inner: &'a Inner,
    id: CommandId,
    generation: u64,
    token: CancelToken,
}

impl Deref for Tracked<'_> {
    type Target = CancelToken;

    fn deref(&self) -> &CancelToken {
        &self.token
    }
}

impl Drop for Tracked<'_> {
    fn drop(&mut self) {
        self.inner.running.lock().release(self.id, self.generation);
    }
}

impl Backend {
    /// Makes `id` reachable by [`Backend::cancel`] until the guard drops.
    ///
    /// # Errors
    /// If a command is already running under `id`: its token must stay the
    /// one `cancel` reaches.
    pub(crate) fn track(&self, id: CommandId) -> Result<Tracked<'_>, IpcError> {
        let registered = self.inner.running.lock().register(id, Instant::now());
        let (token, generation) = registered
            .ok_or_else(|| IpcError::invalid("A command with this id is already running"))?;
        Ok(Tracked {
            inner: &self.inner,
            id,
            generation,
            token,
        })
    }

    /// Cancels a running command. Returns whether it was still running.
    ///
    /// A command not yet tracked is cancelled as soon as it is: the request is
    /// retained for a short while (see the module documentation).
    #[must_use]
    pub fn cancel(&self, id: CommandId) -> bool {
        self.inner.running.lock().cancel(id, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_track_of_a_running_id_is_refused() {
        let mut tracker = Tracker::default();
        let id = CommandId::new();
        let now = Instant::now();
        let (first, _) = tracker.register(id, now).expect("a fresh id registers");
        assert!(tracker.register(id, now).is_none(), "the id is running");
        assert!(tracker.cancel(id, now));
        assert!(first.is_cancelled(), "the first command is still reachable");
    }

    #[test]
    fn a_release_leaves_an_entry_it_did_not_create() {
        let mut tracker = Tracker::default();
        let id = CommandId::new();
        let now = Instant::now();
        let (_, stale) = tracker.register(id, now).expect("registers");
        tracker.release(id, stale);
        let (current, generation) = tracker
            .register(id, now)
            .expect("released, registers again");
        assert_ne!(stale, generation);

        tracker.release(id, stale);
        assert!(tracker.cancel(id, now), "the stale guard removed nothing");
        assert!(current.is_cancelled());
        tracker.release(id, generation);
        assert!(!tracker.cancel(id, now), "its own guard removes it");
    }

    #[test]
    fn a_cancel_before_track_cancels_the_command_when_it_starts() {
        let mut tracker = Tracker::default();
        let id = CommandId::new();
        let now = Instant::now();
        assert!(!tracker.cancel(id, now), "nothing runs yet");
        let (token, generation) = tracker.register(id, now).expect("registers");
        assert!(token.is_cancelled(), "the early cancellation applies");

        tracker.release(id, generation);
        let (again, _) = tracker.register(id, now).expect("registers");
        assert!(!again.is_cancelled(), "an early cancellation applies once");
    }

    #[test]
    fn a_cancel_after_the_command_ended_spares_the_approval_that_follows() {
        let mut tracker = Tracker::default();
        let id = CommandId::new();
        let now = Instant::now();
        let (_, generation) = tracker.register(id, now).expect("registers");
        tracker.release(id, generation);
        assert!(!tracker.cancel(id, now), "the command already ended");
        let (approval, _) = tracker.register(id, now).expect("the approval registers");
        assert!(!approval.is_cancelled(), "a late cancel is not retained");
    }

    #[test]
    fn an_early_cancel_expires_and_stays_bounded() {
        let mut tracker = Tracker::default();
        let id = CommandId::new();
        let then = Instant::now();
        assert!(!tracker.cancel(id, then));
        let later = then + EARLY_CANCEL_WINDOW + Duration::from_secs(1);
        let (token, _) = tracker.register(id, later).expect("registers");
        assert!(!token.is_cancelled(), "a stale cancellation is forgotten");

        for _ in 0..EARLY_CANCELS * 2 {
            assert!(!tracker.cancel(CommandId::new(), then));
        }
        assert_eq!(tracker.early.len(), EARLY_CANCELS);
    }

    #[test]
    fn the_backend_applies_an_early_cancel_and_refuses_a_duplicate() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let id = CommandId::new();
        assert!(!backend.cancel(id), "nothing runs under this id yet");
        let tracked = backend.track(id).expect("a fresh id is tracked");
        assert!(tracked.is_cancelled(), "the early cancellation applies");
        assert!(backend.track(id).is_err(), "a running id is not reused");
        drop(tracked);
        assert!(!backend.cancel(id), "the guard released the id");
    }
}
