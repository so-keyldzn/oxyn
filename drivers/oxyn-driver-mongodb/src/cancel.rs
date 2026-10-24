//! Stopping a command on the server: `killSessions` on the command's own
//! logical session.
//!
//! Every execution runs in a **logical session of its own**, started by the
//! driver; the server tags each operation, and the cursor it opens, with that
//! session's id. `killSessions` names the id: the server interrupts what runs
//! under it and closes its cursors — "Killing a session kills any in-progress
//! operations in the session and closes any open cursors associated with these
//! operations" (MongoDB manual, `killSessions`, consulted on 2026-10-22).
//!
//! Nothing is added to the user's command to find it again. A `comment` would
//! have made it findable in `currentOp`, at the price of sending another
//! command than the one typed — and a `comment` the user wrote would have to
//! be overwritten. The session id is part of the wire envelope, which the
//! driver owns.
//!
//! Under access control, "the command only kills the sessions owned by the
//! user": the session id is combined with the authenticated user, so a session
//! can only stop its own commands, and needs no privilege to. Atlas M0 and Flex
//! clusters do not support the command; [`probe`] finds out at connection, and
//! the session then does not declare `SERVER_SIDE_CANCEL`.
//!
//! # A stop names an id the library reuses
//!
//! The library pools logical sessions: the id an execution gives back is the
//! one the next execution starts under. A `killSessions` still on its way when
//! the stopped execution ends would interrupt the next one — another command
//! than the one the user stopped. The execution therefore holds its logical
//! session through a [`Leased`], and gives it back through its [`Running`]:
//! while a stop is in flight, the session is parked there and returns to the
//! pool only once the server answered the stop. A stop that got no answer — the
//! grace ran out, the connection broke, the stopping future was dropped — may
//! still reach the server: its session never returns to the pool, it is kept
//! in the [`Quarantine`] until the Oxyn session ends.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bson::{Bson, Document, doc};
use mongodb::error::ErrorKind;
use mongodb::{Client, ClientSession};
use oxyn_core::{CancelToken, OxynError, StatementHandle};
use parking_lot::Mutex;

/// One execution the session may have to stop, and the lease on its logical
/// session.
pub(crate) struct Running {
    /// The logical session's id document, `{"id": <UUID>}`.
    lsid: Document,
    /// Set once `killSessions` was sent for it: the server's `Interrupted`
    /// is then the user's stop, not a failure.
    killed: AtomicBool,
    lease: Mutex<Lease<ClientSession>>,
    quarantine: Arc<Quarantine>,
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running")
            .field("killed", &self.was_killed())
            .finish_non_exhaustive()
    }
}

impl Running {
    /// Was the stop sent?
    pub(crate) fn was_killed(&self) -> bool {
        self.killed.load(Ordering::Acquire)
    }

    /// A stop begins, unless the execution already gave its session back:
    /// nothing runs under the id any more, and a stop would only reach
    /// whoever took it next.
    fn begin_stop(&self) -> Option<Stop<'_>> {
        self.lease.lock().begin_stop().then_some(Stop {
            running: self,
            answered: false,
        })
    }

    fn settle(&self, settled: Option<Settle<ClientSession>>) {
        match settled {
            // Dropped here, outside the lock: the library checks it back in.
            Some(Settle::Return(session)) => drop(session),
            Some(Settle::Quarantine(session)) => self.quarantine.0.lock().push(session),
            None => {}
        }
    }
}

/// A stop in flight; dropped when its request ended, however it ended.
struct Stop<'a> {
    running: &'a Running,
    /// The server answered the request, a refusal included.
    answered: bool,
}

impl Drop for Stop<'_> {
    fn drop(&mut self) {
        let settled = self.running.lease.lock().end_stop(self.answered);
        self.running.settle(settled);
    }
}

/// The logical session of one execution, given back through its [`Running`]
/// when dropped.
pub(crate) struct Leased {
    /// `None` only once dropped.
    session: Option<ClientSession>,
    running: Arc<Running>,
}

impl Leased {
    /// Leases `session` to one execution.
    pub(crate) fn new(session: ClientSession, quarantine: &Arc<Quarantine>) -> Self {
        let running = Arc::new(Running {
            lsid: session.id().clone(),
            killed: AtomicBool::new(false),
            lease: Mutex::new(Lease::default()),
            quarantine: Arc::clone(quarantine),
        });
        Self {
            session: Some(session),
            running,
        }
    }

    /// The session, for the library's calls.
    ///
    /// # Errors
    /// [`OxynError::Internal`] never in practice: only `drop` takes the
    /// session out.
    pub(crate) fn session(&mut self) -> oxyn_core::Result<&mut ClientSession> {
        self.session.as_mut().ok_or_else(|| {
            OxynError::Internal("an execution's logical session was already given back".to_owned())
        })
    }

    /// What a stop of this execution needs.
    pub(crate) const fn running(&self) -> &Arc<Running> {
        &self.running
    }
}

impl Drop for Leased {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let settled = self.running.lease.lock().release(session);
            self.running.settle(settled);
        }
    }
}

/// The logical sessions a stop without answer may still reach: never given
/// back to the pool. One per such stop — a stop the server left unanswered for
/// [`KILL_GRACE`] — dropped with the Oxyn session, whose client goes with them.
#[derive(Default)]
pub(crate) struct Quarantine(Mutex<Vec<ClientSession>>);

impl std::fmt::Debug for Quarantine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Quarantine")
            .field(&self.0.lock().len())
            .finish()
    }
}

/// Who holds an execution's logical session, between its end and its stops.
#[derive(Debug)]
struct Lease<S> {
    /// The execution gave the session back.
    released: bool,
    /// Stops sent and not yet ended.
    stops: usize,
    /// A stop ended without the server's answer.
    unanswered: bool,
    /// The session given back while a stop was in flight.
    parked: Option<S>,
}

impl<S> Default for Lease<S> {
    fn default() -> Self {
        Self {
            released: false,
            stops: 0,
            unanswered: false,
            parked: None,
        }
    }
}

/// Where a session goes once nothing can stop it any more.
#[derive(Debug, PartialEq, Eq)]
enum Settle<S> {
    /// Back to the pool.
    Return(S),
    /// Kept out of the pool: a stop may still reach its id.
    Quarantine(S),
}

impl<S> Lease<S> {
    fn begin_stop(&mut self) -> bool {
        if self.released {
            return false;
        }
        self.stops = self.stops.saturating_add(1);
        true
    }

    fn end_stop(&mut self, answered: bool) -> Option<Settle<S>> {
        self.stops = self.stops.saturating_sub(1);
        self.unanswered |= !answered;
        if self.stops > 0 {
            return None;
        }
        let session = self.parked.take()?;
        Some(self.verdict(session))
    }

    fn release(&mut self, session: S) -> Option<Settle<S>> {
        self.released = true;
        if self.stops > 0 {
            self.parked = Some(session);
            return None;
        }
        Some(self.verdict(session))
    }

    const fn verdict(&self, session: S) -> Settle<S> {
        if self.unanswered {
            Settle::Quarantine(session)
        } else {
            Settle::Return(session)
        }
    }
}

/// The executions of one Oxyn session, by handle.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    running: Mutex<HashMap<StatementHandle, Arc<Running>>>,
}

impl Registry {
    pub(crate) fn insert(&self, handle: StatementHandle, running: Arc<Running>) {
        self.running.lock().insert(handle, running);
    }

    pub(crate) fn remove(&self, handle: StatementHandle) {
        self.running.lock().remove(&handle);
    }

    pub(crate) fn get(&self, handle: StatementHandle) -> Option<Arc<Running>> {
        self.running.lock().get(&handle).cloned()
    }
}

/// Sends `killSessions` for one execution, unless it already ended.
///
/// The flag is raised **before** the command leaves: the reply the stopped
/// command gets may arrive before this one's. The execution's logical session
/// stays out of the pool until the request ended (module documentation).
///
/// # Errors
/// The library's error when the server refuses the command or cannot be
/// reached.
pub(crate) async fn kill(stopper: &Client, running: &Running) -> mongodb::error::Result<()> {
    let Some(mut stop) = running.begin_stop() else {
        return Ok(());
    };
    running.killed.store(true, Ordering::Release);
    #[cfg(test)]
    tokio::time::sleep(Duration::from_millis(
        KILL_DELAY_MS.load(std::sync::atomic::Ordering::Relaxed),
    ))
    .await;
    // Bounded: a server that does not answer the stop leaves it unconfirmed,
    // never a cancellation that waits forever.
    let admin = stopper.database("admin");
    let send = admin.run_command(doc! { "killSessions": [ Bson::Document(running.lsid.clone()) ] });
    let outcome = match tokio::time::timeout(KILL_GRACE, send).await {
        Ok(outcome) => outcome.map(drop),
        Err(_) => Err(mongodb::error::Error::custom(Unconfirmed)),
    };
    // A refusal is an answer; a cut or a timeout leaves the request possibly
    // delivered, and applied later.
    stop.answered = match &outcome {
        Ok(()) => true,
        Err(error) => matches!(error.kind.as_ref(), ErrorKind::Command(_)),
    };
    outcome
}

/// How long a test holds every `killSessions` back: a slow stop, which the
/// race it may lose against the next statement needs to be seen.
#[cfg(test)]
pub(crate) static KILL_DELAY_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// How long a stopped command is awaited after `killSessions`: past it, the
/// stop is reported unconfirmed.
pub(crate) const KILL_GRACE: Duration = Duration::from_secs(5);

/// The stop was sent and the server did not answer the stopped command in
/// time: whether it ended is not known. Carried as the library's `Custom`
/// error, which `crate::error::map_error` reads as a stop.
#[derive(Debug)]
pub(crate) struct Unconfirmed;

/// Awaits a library call; on `cancel`, sends `killSessions` and **keeps
/// awaiting the call**: the server's answer to the stopped command is what
/// proves the stop, not the return of this function
/// ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
pub(crate) async fn race<T>(
    stopper: &Client,
    running: &Running,
    cancel: &CancelToken,
    call: impl Future<Output = mongodb::error::Result<T>>,
) -> mongodb::error::Result<T> {
    let mut call = std::pin::pin!(call);
    tokio::select! {
        outcome = &mut call => outcome,
        () = cancel.cancelled() => {
            if let Err(error) = kill(stopper, running).await {
                tracing::debug!(
                    target: "oxyn::driver::mongodb",
                    code = ?crate::error::server_code(&error),
                    "killSessions failed"
                );
            }
            match tokio::time::timeout(KILL_GRACE, call).await {
                Ok(outcome) => outcome,
                Err(_) => Err(mongodb::error::Error::custom(Unconfirmed)),
            }
        }
    }
}

/// Can this connection stop its own commands?
///
/// Sends `killSessions` for a session id nobody holds — a fresh UUID: the
/// server answers it like any other, and kills nothing ("If the user has no
/// session that match, the killSessions has no effect").
pub(crate) async fn probe(stopper: &Client) -> bool {
    let nobody = doc! { "id": bson::Uuid::new() };
    stopper
        .database("admin")
        .run_command(doc! { "killSessions": [ nobody ] })
        .await
        .is_ok()
}

#[cfg(test)]
mod tests;
