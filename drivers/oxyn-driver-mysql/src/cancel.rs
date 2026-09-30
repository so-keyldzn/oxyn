//! Server-side cancellation, without ever hitting the next statement.
//!
//! # The window this module closes
//!
//! `KILL QUERY <id>` targets a **connection**, not a statement (ADR-0050 §7).
//! Sent while the targeted statement is already finishing, it could land on the
//! next statement of the same connection. Two rules close that window, the
//! PostgreSQL driver's `cancel.rs` rule plus one:
//!
//! 1. **Only the task that holds the connection sends the kill**, and it keeps
//!    holding it until the kill's effect is settled: no other statement of the
//!    session can start on that connection meanwhile. `Session::cancel` only
//!    fires the execution's token and waits for the task's [`Verdict`].
//! 2. **The connection is kept only when the kill is proven consumed**: the
//!    task goes on reading the targeted statement until the server answers
//!    `ER_QUERY_INTERRUPTED` (1317) on it. A natural end, another error, or no
//!    answer within [`DRAIN_BOUND`] means the kill may still be pending: the
//!    connection is closed. Keeping it when the kill is proven spares the
//!    user's open transaction, which closing would silently roll back
//!    (ADR-0050 §7, amended on 2026-09-30).
//!
//! The kill goes out from a **second connection of the same account**, opened
//! for it: an account can always kill its own threads, without
//! `CONNECTION_ADMIN`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mysql_async::prelude::Queryable as _;
use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementHandle, StatementIntent};
use tokio::sync::watch;

use crate::connection::connect_bare;
use crate::error::map_exec_error;
use crate::options::ConnectSpec;

/// How long the task waits, after the kill, for the server to confirm it.
///
/// The server checks the kill flag between blocks of rows: a confirmation
/// takes milliseconds. Beyond this, something else is going on, and the
/// connection is closed rather than trusted.
pub(crate) const DRAIN_BOUND: Duration = Duration::from_secs(5);

/// How long opening the killing connection may take.
const KILL_CONNECT_BOUND: Duration = Duration::from_secs(10);

/// Was the kill's effect observed on the targeted statement?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainVerdict {
    /// The server answered 1317 on it: the kill is spent, the connection is
    /// safe to keep.
    Consumed,
    /// Anything else: the kill may still be pending on the connection.
    Unproven,
}

/// What asks the server to stop a statement.
///
/// Carries the connection parameters because the kill opens its **own**
/// connection: the session's only connection is precisely the busy one.
#[derive(Debug)]
pub(crate) struct Killer {
    spec: ConnectSpec,
    driver: DriverId,
}

impl Killer {
    pub(crate) const fn new(spec: ConnectSpec, driver: DriverId) -> Self {
        Self { spec, driver }
    }

    /// Checks that a second connection of this account opens, which is what
    /// `SERVER_SIDE_CANCEL` promises (ADR-0050 §7).
    pub(crate) async fn probe(&self) -> bool {
        match tokio::time::timeout(KILL_CONNECT_BOUND, connect_bare(&self.spec)).await {
            Ok(Ok(conn)) => {
                let _ = conn.disconnect().await;
                true
            }
            _ => false,
        }
    }

    /// Sends `KILL QUERY <id>` from a second connection.
    ///
    /// Killing a thread whose statement already ended is not an error for the
    /// server. The id is an integer the driver read from the server itself
    /// (`CONNECTION_ID()`), formatted from a `u64`: nothing received is
    /// concatenated as text ([I-10](../../../CLAUDE.md#i-10)). `KILL` takes no
    /// placeholder in the text protocol.
    ///
    /// # Errors
    /// [`OxynError::Connection`] if the second connection does not open in
    /// time, or the server's refusal.
    pub(crate) async fn kill_query(&self, id: u64) -> Result<()> {
        self.send(kill_statement(id)).await
    }

    /// Sends `KILL CONNECTION <id>`: the last resort when a `KILL QUERY` could
    /// not be proven spent.
    ///
    /// A `KILL QUERY` that reaches the server before the statement has begun
    /// is lost, and the statement then runs to its end — observed against
    /// MySQL 8.4 on 2026-09-30 (RESEARCH-NOTES). The connection is being
    /// closed anyway: ending it on the server is what stops the statement for
    /// sure.
    ///
    /// # Errors
    /// As [`Self::kill_query`].
    pub(crate) async fn kill_connection(&self, id: u64) -> Result<()> {
        self.send(format!("KILL CONNECTION {id}")).await
    }

    async fn send(&self, statement: String) -> Result<()> {
        let mut conn = tokio::time::timeout(KILL_CONNECT_BOUND, connect_bare(&self.spec))
            .await
            .map_err(|_| {
                OxynError::Connection(
                    "the connection that sends KILL did not open in time".to_owned(),
                )
            })??;
        let issue = conn.query_drop(statement).await;
        // Closed in every case: it has no further use.
        let _ = conn.disconnect().await;
        issue.map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, &error))
    }
}

/// `KILL QUERY` for a connection id.
pub(crate) fn kill_statement(id: u64) -> String {
    format!("KILL QUERY {id}")
}

/// What the stream task did with the cancellation it was asked for.
#[derive(Debug, Clone)]
pub(crate) enum Verdict {
    /// The stream ended without the server needing to be stopped.
    Finished,
    /// The kill went out while the connection was held.
    Cancelled,
    /// The kill failed; the statement may still be running.
    CancelFailed(Arc<OxynError>),
}

/// Where the stream task returns its [`Verdict`].
#[derive(Debug)]
pub(crate) struct VerdictSender(watch::Sender<Option<Verdict>>);

impl VerdictSender {
    /// Publishes the verdict.
    pub(crate) fn settle(&self, verdict: Verdict) {
        self.0.send_replace(Some(verdict));
    }
}

/// A running execution, as `Session::cancel` sees it.
#[derive(Debug, Clone)]
struct RunningExecution {
    token: CancelToken,
    verdict: watch::Receiver<Option<Verdict>>,
}

/// Associates each execution with the token that stops it.
///
/// Shared between the session — which cancels — and the stream tasks — which
/// remove themselves on leaving.
#[derive(Debug, Default)]
pub(crate) struct StatementRegistry {
    entries: Mutex<HashMap<StatementHandle, RunningExecution>>,
}

impl StatementRegistry {
    /// Records an execution that starts.
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

    /// Number of running executions. Diagnostics and tests.
    #[doc(hidden)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Stops an execution on the server, through the task that holds its
    /// connection. Cancelling an unknown or finished execution is not an
    /// error.
    ///
    /// # Errors
    /// That of the kill, as the task received it.
    pub(crate) async fn cancel(&self, driver: &DriverId, handle: StatementHandle) -> Result<()> {
        let Some(RunningExecution { token, mut verdict }) = self.lock().get(&handle).cloned()
        else {
            return Ok(());
        };
        token.cancel();
        let issue = match verdict.wait_for(Option::is_some).await {
            Ok(settled) => settled.clone(),
            Err(_) => None,
        };
        match issue {
            Some(Verdict::CancelFailed(error)) => {
                Err(OxynError::driver(driver.clone(), error.class(), error))
            }
            Some(Verdict::Finished | Verdict::Cancelled) | None => Ok(()),
        }
    }

    /// A poisoned lock must not propagate another task's panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<StatementHandle, RunningExecution>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kill_statement_carries_only_an_integer() {
        assert_eq!(kill_statement(42), "KILL QUERY 42");
        assert_eq!(kill_statement(u64::MAX), format!("KILL QUERY {}", u64::MAX));
    }

    #[test]
    fn a_finished_execution_does_not_stay_in_the_registry() {
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        let _verdict = registry.register(handle, CancelToken::new());
        assert_eq!(registry.len(), 1);
        registry.forget(handle);
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn cancelling_an_unknown_execution_costs_nothing() {
        StatementRegistry::default()
            .cancel(&DriverId::mysql(), StatementHandle::new())
            .await
            .expect("nothing to cancel");
    }

    #[tokio::test]
    async fn cancelling_fires_the_execution_token_and_returns_the_task_verdict() {
        let registry = Arc::new(StatementRegistry::default());
        let target = StatementHandle::new();
        let neighbor = StatementHandle::new();
        let token = CancelToken::new();
        let neighbor_token = CancelToken::new();
        let verdict = registry.register(target, token.clone());
        let _neighbor = registry.register(neighbor, neighbor_token.clone());

        let task = tokio::spawn({
            let token = token.clone();
            async move {
                token.cancelled().await;
                verdict.settle(Verdict::CancelFailed(Arc::new(OxynError::Connection(
                    "refused".to_owned(),
                ))));
            }
        });

        let error = registry
            .cancel(&DriverId::mysql(), target)
            .await
            .expect_err("the task's failure surfaces");
        assert_eq!(error.class(), oxyn_core::ErrorClass::Transient);
        assert!(token.is_cancelled());
        assert!(
            !neighbor_token.is_cancelled(),
            "only the target is cancelled"
        );
        task.await.expect("the task rules");
    }

    #[tokio::test]
    async fn a_task_gone_without_verdict_does_not_block_the_cancellation() {
        let registry = StatementRegistry::default();
        let handle = StatementHandle::new();
        drop(registry.register(handle, CancelToken::new()));
        registry
            .cancel(&DriverId::mysql(), handle)
            .await
            .expect("nothing left to cut");
    }
}
