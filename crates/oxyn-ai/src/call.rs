//! Which tool call a dispatch and its events belong to.
//!
//! An external agent runs its tool calls over HTTP, one connection each: two
//! of them can be in flight at once. Every step of a call — announced, shown
//! with its statement, held for the user, reported — must land on **that**
//! call's row. Matched on « the call in progress », a second call's
//! announcement overwrote the first's: its report landed on the wrong row, a
//! row stayed « running » for ever, and an approval card attached to a call
//! that never asked for one.
//!
//! So a call carries an identity from its translation to its report: on the
//! events the observer receives ([`crate::AgentEvent::CommandSubmitted`],
//! [`crate::AgentEvent::CommandReported`]) and on the dispatch the sink
//! receives ([`crate::CommandSink::dispatch`]). The same value in all three
//! places is what lets a host key its rows on it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::OwnedMutexGuard;

/// A tool call's identity within this process.
///
/// Minted by Oxyn for each call, never taken from the agent or the provider:
/// an identifier a caller chooses can repeat another call's, by mistake or on
/// purpose, and two rows would then share their events again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallId(u64);

impl CallId {
    /// A new identity, distinct from every other minted in this process.
    #[must_use]
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Atomic addition wraps rather than panicking; 2^64 calls in one
        // process is not a case.
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// One tool call, as a [`crate::CommandSink`] receives it.
///
/// Its [`id`](Self::id), and — for an external agent's call — the turn it
/// holds in the agent's order of calls, which the sink gives back
/// ([`awaiting_user`](Self::awaiting_user)) when the call starts waiting on
/// the user.
pub struct CallHandle {
    id: CallId,
    /// The agent's order of calls, held from the gate's check to the
    /// executor's answer (`external::mcp::turn`).
    order: Mutex<Option<OwnedMutexGuard<()>>>,
}

impl std::fmt::Debug for CallHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallHandle")
            .field("id", &self.id)
            .field("in_order", &self.lock().is_some())
            .finish()
    }
}

impl CallHandle {
    /// A call holding nothing yet.
    #[must_use]
    pub fn new(id: CallId) -> Self {
        Self {
            id,
            order: Mutex::new(None),
        }
    }

    /// The identity its events carry.
    #[must_use]
    pub fn id(&self) -> CallId {
        self.id
    }

    /// Says the executor now holds a request of this call for the user, and
    /// gives back the turn it held in its agent's order of calls.
    ///
    /// The order kept a second call from slipping between the gate's « no
    /// request of this agent waits » and the executor's answer. Once the
    /// executor holds the request, that check refuses every next call by
    /// itself: holding the order through the user's decision — up to the
    /// request's lifetime — only queued the agent's other calls until they
    /// timed out on its side. Call it **after** the executor answered, never
    /// before. Does nothing when no order is held.
    pub fn awaiting_user(&self) {
        self.release_order();
    }

    /// Holds `guard` until the call ends or waits on the user.
    pub(crate) fn hold(&self, guard: OwnedMutexGuard<()>) {
        *self.lock() = Some(guard);
    }

    /// Gives back the turn held, if still held: the call has its answer.
    pub(crate) fn release_order(&self) {
        drop(self.lock().take());
    }

    fn lock(&self) -> MutexGuard<'_, Option<OwnedMutexGuard<()>>> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn identities_never_repeat() {
        let first = CallId::fresh();
        let second = CallId::fresh();
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn waiting_on_the_user_gives_the_order_back() {
        let order = Arc::new(tokio::sync::Mutex::new(()));
        let call = CallHandle::new(CallId::fresh());
        call.hold(Arc::clone(&order).lock_owned().await);
        assert!(order.try_lock().is_err(), "held by the call");
        call.awaiting_user();
        assert!(order.try_lock().is_ok(), "given back");
        // A second time is harmless.
        call.awaiting_user();
    }
}
