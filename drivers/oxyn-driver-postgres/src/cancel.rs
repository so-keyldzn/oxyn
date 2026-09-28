//! Server-side cancellation, without ever hitting the neighboring query.
//!
//! # The window this module closes
//!
//! `pg_cancel_backend(pid)` targets a **server process**, not a query. Sending
//! the cancellation from [`Session::cancel`](oxyn_driver::Session::cancel) —
//! read the pid, open a second connection, then call the function — leaves a
//! window as long as a handshake: if the query finishes meanwhile, the stream
//! task returns its connection to the pool, the next query borrows it, and it is
//! the one the cancellation interrupts. Pressing Esc at the moment a query
//! finishes is enough.
//!
//! Re-reading the pid just before sending changes nothing: the next query runs
//! on **the same** process, hence under the same pid.
//!
//! # The rule
//!
//! **Only the stream task sends `pg_cancel_backend`, and it does so while
//! holding its connection**, already marked to be closed. `Session::cancel` only
//! fires the execution's token, then waits for the task's [`Verdict`]. Only two
//! outcomes:
//!
//! * the task had already observed the end of the stream — the token arrives
//!   too late, nothing is sent, and the connection goes back to the pool with
//!   no cancellation in flight;
//! * the task sees the token — it sends the cancellation **before** letting go
//!   of the connection, which is then closed and never serves anyone again.
//!
//! No query can therefore start on that process while a cancellation targets
//! it. The held connection guarantees it, not a delay.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementHandle, StatementIntent};
use sqlx::{ConnectOptions as _, Connection as _};
use tokio::sync::watch;

use crate::error::{map_connect_error, map_exec_error};
use crate::options::ConnectSpec;

/// Asks the server to interrupt another process's query.
pub(crate) const SQL_CANCEL_BACKEND: &str = "SELECT pg_catalog.pg_cancel_backend($1)";

/// What the stream task did with the cancellation it was asked for.
#[derive(Debug, Clone)]
pub(crate) enum Verdict {
    /// The stream ended without the server needing to be stopped.
    Finished,
    /// `pg_cancel_backend` went out while the connection was held.
    Cancelled,
    /// The cancellation request failed; the query may still be running.
    CancelFailed(Arc<OxynError>),
}

/// Where the stream task returns its [`Verdict`].
#[derive(Debug)]
pub(crate) struct VerdictSender(watch::Sender<Option<Verdict>>);

impl VerdictSender {
    /// Publishes the verdict. Cannot fail: a `Session::cancel` arriving later
    /// will find the execution missing from the registry.
    pub(crate) fn settle(&self, verdict: Verdict) {
        self.0.send_replace(Some(verdict));
    }
}

/// A running execution, as `Session::cancel` sees it.
#[derive(Debug, Clone)]
struct RunningExecution {
    /// The execution's **own** token: the one the stream task watches.
    token: CancelToken,
    verdict: watch::Receiver<Option<Verdict>>,
}

/// Associates each execution with the token that stops it.
///
/// Shared between the session — which cancels — and the stream tasks — which
/// remove themselves on leaving. Without that removal, a session open for a day
/// would accumulate one entry per executed query.
#[derive(Debug, Default)]
pub(crate) struct StatementRegistry {
    entries: Mutex<HashMap<StatementHandle, RunningExecution>>,
}

impl StatementRegistry {
    /// Records an execution that starts, and returns what publishes its verdict.
    pub(crate) fn register(&self, handle: StatementHandle, token: CancelToken) -> VerdictSender {
        let (sender, verdict) = watch::channel(None);
        self.lock()
            .insert(handle, RunningExecution { token, verdict });
        VerdictSender(sender)
    }

    /// Forgets a finished execution.
    pub(crate) fn forget(&self, handle: StatementHandle) {
        self.lock().remove(&handle);
    }

    /// Number of running executions. Reserved for diagnostics and tests.
    #[doc(hidden)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Stops an execution on the server, through the task that holds its
    /// connection.
    ///
    /// Returns once the task has ruled. Cancelling an unknown or already
    /// finished execution is not an error.
    ///
    /// # Errors
    /// That of the cancellation request, as the task received it from the
    /// server, attached to its original family.
    pub(crate) async fn cancel(&self, driver: &DriverId, handle: StatementHandle) -> Result<()> {
        let Some(RunningExecution { token, mut verdict }) = self.lock().get(&handle).cloned()
        else {
            return Ok(());
        };
        token.cancel();
        let issue = match verdict.wait_for(Option::is_some).await {
            Ok(rendered) => rendered.clone(),
            // The task disappeared without ruling: there is nothing left to cut.
            Err(_) => None,
        };
        match issue {
            Some(Verdict::CancelFailed(error)) => {
                Err(OxynError::driver(driver.clone(), error.class(), error))
            }
            Some(Verdict::Finished | Verdict::Cancelled) | None => Ok(()),
        }
    }

    /// A poisoned lock must not propagate another task's panic: the table stays
    /// usable, and losing an entry costs less than an unusable session
    /// ([I-09](../../../CLAUDE.md#i-09)).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<StatementHandle, RunningExecution>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What asks the server to stop a query.
///
/// Carries the connection parameters because the cancellation opens its **own**
/// connection: borrowing the pool's would make the cancellation wait behind the
/// queries it must cut.
///
/// Called only by whoever **holds** the targeted connection — the stream task,
/// or introspection — and has marked it to be closed: that is what guarantees
/// the pid serves no other query while sending.
#[derive(Debug)]
pub(crate) struct BackendCanceller {
    spec: ConnectSpec,
    driver: DriverId,
    #[cfg(test)]
    gate: Mutex<Option<tests_gate::CancelGate>>,
}

impl BackendCanceller {
    /// Prepares a session's canceller.
    pub(crate) fn new(spec: ConnectSpec, driver: DriverId) -> Self {
        Self {
            spec,
            driver,
            #[cfg(test)]
            gate: Mutex::new(None),
        }
    }

    /// Asks the server to interrupt the query of process `backend_pid`.
    ///
    /// Interrupting an already finished query is **not** an error: the server
    /// returns `false` and nothing is made of it. What matters is that no query
    /// survives a tab being closed.
    ///
    /// # Errors
    /// [`OxynError::Connection`] if the second connection does not open,
    /// [`OxynError::Driver`] if the server refuses the call — typically for lack
    /// of rights on a pid belonging to another role.
    pub(crate) async fn cancel_backend(&self, backend_pid: i32) -> Result<()> {
        #[cfg(test)]
        self.pass_gate(backend_pid).await;

        let mut connection = self
            .spec
            .options()
            .connect()
            .await
            .map_err(|error| map_connect_error(&error))?;

        let issue = sqlx::query(SQL_CANCEL_BACKEND)
            .bind(backend_pid)
            .fetch_optional(&mut connection)
            .await;

        // Closed in every case: this connection has no further use, and letting
        // it go would open one per cancellation.
        let _ = connection.close().await;

        issue.map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))?;
        Ok(())
    }
}

/// The barrier that makes the cancellation window reproducible in a test.
#[cfg(test)]
pub(crate) mod tests_gate {
    use tokio::sync::oneshot;

    use super::BackendCanceller;

    /// Holds back the next cancellation just before it opens its connection.
    #[derive(Debug)]
    pub(crate) struct CancelGate {
        reached: oneshot::Sender<i32>,
        proceed: oneshot::Receiver<()>,
    }

    impl BackendCanceller {
        /// Arms the barrier: the next cancellation signals the pid it targets,
        /// then waits for the green light.
        pub(crate) fn hold_next_cancel(&self) -> (oneshot::Receiver<i32>, oneshot::Sender<()>) {
            let (reached, reached_rx) = oneshot::channel();
            let (proceed_tx, proceed) = oneshot::channel();
            *self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(CancelGate { reached, proceed });
            (reached_rx, proceed_tx)
        }

        pub(super) async fn pass_gate(&self, backend_pid: i32) {
            let gate = self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(CancelGate { reached, proceed }) = gate {
                let _ = reached.send(backend_pid);
                let _ = proceed.await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_execution_does_not_stay_in_the_registry() {
        // Without that removal, a session open for a day accumulates one entry
        // per executed query.
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        let _verdict = registry.register(handle, CancelToken::new());
        assert_eq!(registry.len(), 1);
        registry.forget(handle);
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn cancelling_an_unknown_execution_must_cost_nothing() {
        // "Cancelling an already finished statement is not an error."
        let registry = StatementRegistry::default();
        registry
            .cancel(&DriverId::postgres(), StatementHandle::new())
            .await
            .expect("nothing to cancel");
    }

    #[tokio::test]
    async fn cancelling_fires_the_execution_token_and_waits_for_its_verdict() {
        // The cancellation does not go out from here: the task holding the
        // connection sends it. `cancel` must therefore wake it, then return
        // what it observed — not an assumed success.
        let registry = Arc::new(StatementRegistry::default());
        let target_handle = StatementHandle::new();
        let neighbor = StatementHandle::new();
        let token = CancelToken::new();
        let neighbor_token = CancelToken::new();
        let verdict = registry.register(target_handle, token.clone());
        let _neighbor_verdict = registry.register(neighbor, neighbor_token.clone());

        let task = tokio::spawn({
            let token = token.clone();
            async move {
                token.cancelled().await;
                verdict.settle(Verdict::CancelFailed(Arc::new(OxynError::Connection(
                    "refused".to_owned(),
                ))));
            }
        });

        let issue = registry.cancel(&DriverId::postgres(), target_handle).await;
        let error = match issue {
            Ok(()) => panic!("the task's failure must surface"),
            Err(error) => error,
        };
        assert_eq!(error.class(), oxyn_core::ErrorClass::Transient);
        assert!(token.is_cancelled());
        assert!(
            !neighbor_token.is_cancelled(),
            "only the targeted execution is cancelled"
        );
        task.await.expect("the task rules");
    }

    #[tokio::test]
    async fn a_task_gone_without_verdict_does_not_block_the_cancellation() {
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        let verdict = registry.register(handle, CancelToken::new());
        drop(verdict);
        registry
            .cancel(&DriverId::postgres(), handle)
            .await
            .expect("nothing left to cut");
    }
}
