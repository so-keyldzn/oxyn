//! An open session: execute, stop, ping.
//!
//! # Stateless, on purpose
//!
//! A session holds an HTTP client and what the server said at connection. It
//! keeps no Hrana stream between statements: each one opens its own, runs, and
//! closes it in the same request ([`crate::wire`]). So nothing a statement
//! does to a connection outlives it — and the statements that would
//! (`BEGIN`, `PRAGMA x = y`, `ATTACH`) are refused rather than accepted and
//! forgotten ([`crate::refuse`]).
//!
//! # Stopping
//!
//! The token a caller passes, or that [`Session::cancel`] trips, drops the
//! request. Turso has no call to interrupt a running statement, so the session
//! does not declare `SERVER_SIDE_CANCEL` and the executor does not call
//! `cancel` on it; the verdict is what the driver holds itself to: `Cancelled`
//! for a read, `OutcomeUnknown` for anything that may write
//! ([`crate::http`]).
//!
//! # Read-only is classification
//!
//! A read-only execution is refused if the request is not classified a read.
//! The server is not asked to enforce it (no `READ_ONLY_SESSION`): a database
//! token created with `--read-only` is the way to have Turso enforce it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecRequest, OxynError, PreviewShape, Result,
    StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session};

use crate::catalog::TursoCatalog;
use crate::convert::{Batches, Plan};
use crate::cursor::TursoCursor;
use crate::http::Http;
use crate::params;
use crate::refuse::refuse_session_state;
use crate::variant::capabilities;
use crate::wire::{self, Arg};

/// How long a ping may take before the server is reported unreachable.
const PING_BOUND: Duration = Duration::from_secs(10);

type Running = Arc<Mutex<HashMap<StatementHandle, CancelToken>>>;

/// Removes an execution from the running set when its request ends, whichever
/// way: the answer, an error, or the caller dropping the future.
struct Registered {
    running: Running,
    handle: StatementHandle,
}

impl Drop for Registered {
    fn drop(&mut self) {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.handle);
    }
}

/// An open Turso session.
#[derive(Debug)]
pub struct TursoSession {
    http: Arc<Http>,
    driver: DriverId,
    catalog: TursoCatalog,
    running: Running,
}

impl TursoSession {
    pub(crate) fn new(http: Arc<Http>, driver: DriverId, version: String) -> Self {
        let catalog = TursoCatalog::new(Arc::clone(&http), driver.clone(), version);
        Self {
            http,
            driver,
            catalog,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Executions still waiting for an answer. Diagnostics and tests.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl Session for TursoSession {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }

    /// Sends the statement and returns once its answer arrived and was read.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] for another language, a typed transaction
    /// verb, `PRAGMA x = y`, `ATTACH` or a value SQLite has no storage for;
    /// [`OxynError::PolicyDenied`] for a write under a read-only bound;
    /// [`OxynError::Cancelled`] or [`OxynError::OutcomeUnknown`] if `cancel`
    /// fires; the server's refusal, classified.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities().require_language(request.language)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        refuse_session_state(&request)?;
        if request.limits.read_only && request.is_mutating() {
            return Err(OxynError::PolicyDenied {
                reason: "this execution is bounded to read-only: \
                         a write must lift that bound explicitly"
                    .to_owned(),
            });
        }
        let args: Vec<Arg> = params::bind(&request.params)?;
        let body = wire::request_body(&request.text, &args)?;

        let started = Instant::now();
        let handle = StatementHandle::new();
        let token = cancel.child();
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(handle, token.clone());
        let _registered = Registered {
            running: Arc::clone(&self.running),
            handle,
        };

        let result = self
            .http
            .run(
                &self.driver,
                request.intent,
                body,
                request.limits.timeout,
                &token,
            )
            .await?;

        let mut rows = result.rows;
        let truncated = request.limits.max_rows.is_some_and(|max| rows.len() > max);
        if let Some(max) = request.limits.max_rows {
            rows.truncate(max);
        }
        // An answer that contradicts itself, to a statement that may have
        // taken effect, leaves the outcome in doubt: not a plain refusal.
        let plan = Plan::of(&result.cols, &rows).map_err(|error| {
            if request.intent.is_mutating() {
                OxynError::OutcomeUnknown(
                    "Turso's answer contradicts itself and the statement may have been \
                     applied; check the data before running it again"
                        .to_owned(),
                )
            } else {
                error
            }
        })?;
        let schema = plan.schema();
        // A statement with no result set reports what it changed.
        let written = result.cols.is_empty().then_some(result.affected_row_count);
        Ok(Box::new(TursoCursor::new(
            handle,
            schema,
            Batches::new(plan, rows),
            written,
            result.query_duration_ms,
            truncated,
            started,
        )))
    }

    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        self.catalog
            .preview_request(path, limit, shape, cancel)
            .await
    }

    /// Stops an execution on Oxyn's side: its request is dropped. Stopping a
    /// finished statement is not an error.
    ///
    /// Turso cannot be asked to interrupt a statement, so this never reaches
    /// the server — which is why `SERVER_SIDE_CANCEL` is not declared.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        let token = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&statement)
            .cloned();
        if let Some(token) = token {
            token.cancel();
        }
        Ok(())
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Measures a round trip with `SELECT 1`, authenticated like any request.
    ///
    /// # Errors
    /// Any transport or server error: a session whose ping fails is lost.
    async fn ping(&self) -> Result<Duration> {
        let start = Instant::now();
        let body = wire::request_body("SELECT 1", &[])?;
        // Bounded: a server that accepts the connection and then hangs would
        // hold the ping, and the session lock with it, for ever.
        self.http
            .run(
                &self.driver,
                StatementIntent::Read,
                body,
                Some(PING_BOUND),
                &CancelToken::new(),
            )
            .await?;
        Ok(start.elapsed())
    }

    /// Stops what still waits for an answer.
    ///
    /// # Errors
    /// None today: the HTTP client has no goodbye to say.
    async fn close(self: Box<Self>) -> Result<()> {
        let tokens: Vec<CancelToken> = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for token in tokens {
            token.cancel();
        }
        Ok(())
    }
}
