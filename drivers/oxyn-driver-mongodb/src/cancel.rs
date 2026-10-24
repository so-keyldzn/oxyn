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

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bson::{Bson, Document, doc};
use mongodb::Client;
use oxyn_core::{CancelToken, StatementHandle};
use parking_lot::Mutex;

/// One execution the session may have to stop.
#[derive(Debug)]
pub(crate) struct Running {
    /// The logical session's id document, `{"id": <UUID>}`.
    lsid: Document,
    /// Set once `killSessions` was sent for it: the server's `Interrupted`
    /// is then the user's stop, not a failure.
    killed: AtomicBool,
}

impl Running {
    pub(crate) fn new(lsid: Document) -> Self {
        Self {
            lsid,
            killed: AtomicBool::new(false),
        }
    }

    /// Was the stop sent?
    pub(crate) fn was_killed(&self) -> bool {
        self.killed.load(Ordering::Acquire)
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

/// Sends `killSessions` for one execution.
///
/// The flag is raised **before** the command leaves: the reply the stopped
/// command gets may arrive before this one's.
///
/// # Errors
/// The library's error when the server refuses the command or cannot be
/// reached.
pub(crate) async fn kill(stopper: &Client, running: &Running) -> mongodb::error::Result<()> {
    running.killed.store(true, Ordering::Release);
    // Bounded: a server that does not answer the stop leaves it unconfirmed,
    // never a cancellation that waits forever.
    let admin = stopper.database("admin");
    let send = admin.run_command(doc! { "killSessions": [ Bson::Document(running.lsid.clone()) ] });
    match tokio::time::timeout(KILL_GRACE, send).await {
        Ok(outcome) => outcome.map(drop),
        Err(_) => Err(mongodb::error::Error::custom(Unconfirmed)),
    }
}

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
