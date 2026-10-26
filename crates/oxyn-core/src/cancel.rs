//! Cooperative, hierarchical cancellation.
//!
//! A cancellation token clones, is shared between threads, and branches into
//! children: cancelling a parent cancels all its descendants, never the
//! reverse. That is what makes it possible to close a tab — hence to cancel
//! the session that goes with it — without having to find every query it
//! launched.
//!
//! This token cancels **nothing by itself**. It signals. It is up to the driver
//! to turn the signal into `pg_cancel_backend`, `KILL QUERY` or
//! `sqlite3_interrupt`: a "Cancel" button that only drops the client-side
//! future leaves the query running, the connection taken and the lock held
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! # Why not `tokio-util`
//!
//! `tokio-util`'s `CancellationToken` does exactly this, but `tokio-util` is
//! not in this crate's dependency contract. The implementation fits in an
//! `AtomicBool` and a [`tokio::sync::Notify`].

use std::fmt;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use tokio::sync::Notify;

/// Shared state of a token and its clones.
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
    /// **Weak** references: a long-lived parent must not keep alive the tokens
    /// of a thousand queries already finished.
    children: Mutex<Vec<Weak<Inner>>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
            children: Mutex::new(Vec::new()),
        }
    }
}

/// Clonable, shareable, hierarchical cancellation token.
///
/// Cloning a token gives a **view** of the same state: cancelling a clone
/// cancels the original. To get an independently cancellable token, use
/// [`CancelToken::child`].
#[derive(Clone)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

impl CancelToken {
    /// Creates a root token, not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::new()),
        }
    }

    /// Requests cancellation, here and for all descendants.
    ///
    /// Idempotent: subsequent calls do nothing. Never goes up to the parent.
    pub fn cancel(&self) {
        Self::cancel_inner(&self.inner);
    }

    fn cancel_inner(inner: &Arc<Inner>) {
        // The flag is set **before** taking the children's lock; that is what
        // guarantees that a child created concurrently is born cancelled rather
        // than escaping the cancellation. See `child`.
        if inner.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        inner.notify.notify_waiters();

        // The lock is released before going down: recursing under a parent
        // lock would bring nothing and freezes the tree during propagation.
        let children = {
            let mut guard = inner.children.lock();
            std::mem::take(&mut *guard)
        };
        for weak in children {
            if let Some(child) = weak.upgrade() {
                Self::cancel_inner(&child);
            }
        }
    }

    /// Has cancellation been requested?
    ///
    /// To be checked in any somewhat long decoding loop: it is the only point
    /// where a blocking driver can yield.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Waits for cancellation.
    ///
    /// Returns **immediately** if cancellation already happened: that is the
    /// case that gets missed, and it turns a cancellation into a deadlock.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let mut waiting = pin!(self.inner.notify.notified());
            // Registration must come before re-reading the flag. Without
            // `enable()`, `notified()` only registers on the first poll, and a
            // cancellation occurring between the re-read and the poll would wake
            // nobody.
            waiting.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            waiting.await;
            if self.is_cancelled() {
                return;
            }
            // Woken without cancellation: only `cancel` notifies, so this case
            // should not happen. We loop again rather than return on a
            // cancellation that did not take place.
        }
    }

    /// Creates a child token.
    ///
    /// Cancelling the parent cancels the child; cancelling the child leaves the
    /// parent intact. If the parent is **already** cancelled, the child is born
    /// cancelled.
    #[must_use]
    pub fn child(&self) -> Self {
        let child = Arc::new(Inner::new());

        let mut guard = self.inner.children.lock();
        // Purge finished children: otherwise a long-running session would
        // accumulate one `Weak` per executed query.
        guard.retain(|weak| weak.strong_count() > 0);

        if self.inner.cancelled.load(Ordering::SeqCst) {
            // No need to register it: propagation already happened.
            child.cancelled.store(true, Ordering::SeqCst);
        } else {
            guard.push(Arc::downgrade(&child));
        }
        drop(guard);

        Self { inner: child }
    }

    /// Number of children still alive. For tests and diagnostics only.
    #[doc(hidden)]
    #[must_use]
    pub fn live_children(&self) -> usize {
        self.inner
            .children
            .lock()
            .iter()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
