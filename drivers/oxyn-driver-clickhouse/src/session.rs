//! An open session: execute, cancel, declare the database.
//!
//! # Stateless, on purpose
//!
//! A session holds an HTTP client and what the server said at connection; it
//! sends no `session_id` (ADR-0060). The database every request resolves
//! unqualified names in is the one the session **declares** — the form's, or
//! the one the context selector set — and it travels with each request. A
//! typed `USE`, `SET` or `CREATE TEMPORARY TABLE` would change nothing past
//! its own request, so it is refused before anything is sent
//! (`crate::refuse`).
//!
//! # Read-only is the server's
//!
//! A read-only execution carries `readonly = 1`: the server refuses writes,
//! DDL, settings in the statement, and table functions that reach out
//! (`url()`, observed on 26.3). `readonly = 2` would have let `url()` through.
//! An account the server already holds at `readonly = 1` or `2` is not bound
//! further: the server refuses any change of the level, so its writes are
//! refused by the account itself, and at level 2 `url()` stays possible — the
//! driver says so rather than pretend.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecLimits, ExecRequest, OxynError, PreviewShape, Result,
    StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session, SessionContext};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

use crate::catalog::ClickhouseCatalog;
use crate::cursor::ClickhouseCursor;
use crate::fetch::fetch;
use crate::http::{Call, Http};
use crate::params;
use crate::refuse::refuse_session_state;
use crate::stream::{self, DRAIN_BOUND, Execution, KILL_BOUND};
use crate::variant::ServerFacts;

/// Settings every request carries when the account accepts settings.
///
/// `output_format_binary_write_json_as_string`: a `JSON` column comes as its
/// JSON text, which the decoder reads like a `String`; without it, the column
/// is refused. `cancel_http_readonly_queries_on_client_close`: a read whose
/// connection closes is stopped by the server, which is what reaches it when
/// the account may not kill, or when a kill came before the statement
/// started. `output_format_binary_encode_types_in_binary_format` and
/// `send_progress_in_http_headers` are pinned at their defaults: a profile
/// that changed them would change the header the decoder reads, or send the
/// 200 before the statement ended.
const SESSION_SETTINGS: [(&str, &str); 4] = [
    ("output_format_binary_write_json_as_string", "1"),
    ("cancel_http_readonly_queries_on_client_close", "1"),
    ("output_format_binary_encode_types_in_binary_format", "0"),
    ("send_progress_in_http_headers", "0"),
];

/// How long a ping may take before the server is reported unreachable.
const PING_BOUND: Duration = Duration::from_secs(10);

/// An open ClickHouse session.
#[derive(Debug)]
pub struct ClickhouseSession {
    http: Arc<Http>,
    driver: DriverId,
    facts: ServerFacts,
    capabilities: Capabilities,
    catalog: ClickhouseCatalog,
    /// The database requests resolve in, and what the selector declared.
    place: Arc<Mutex<Place>>,
    running: Arc<Mutex<HashMap<StatementHandle, Running>>>,
    tasks: Mutex<JoinSet<()>>,
}

/// The database of the session's requests.
#[derive(Debug, Clone, Default)]
pub(crate) struct Place {
    /// What requests carry; `None` is the account's default database.
    pub(crate) database: Option<String>,
    /// What the context selector declared, if it declared anything.
    pub(crate) declared: Option<SessionContext>,
}

/// An execution `Session::cancel` can reach.
#[derive(Debug, Clone)]
struct Running {
    token: CancelToken,
    query_id: String,
}

impl ClickhouseSession {
    pub(crate) fn new(http: Arc<Http>, driver: DriverId, facts: ServerFacts) -> Self {
        let capabilities = facts.capabilities();
        let place = Arc::new(Mutex::new(Place {
            database: http.spec().database().map(str::to_owned),
            declared: None,
        }));
        let catalog = ClickhouseCatalog::new(
            Arc::clone(&http),
            driver.clone(),
            facts.clone(),
            capabilities,
        );
        Self {
            http,
            driver,
            facts,
            capabilities,
            catalog,
            place,
            running: Arc::new(Mutex::new(HashMap::new())),
            tasks: Mutex::new(JoinSet::new()),
        }
    }

    /// What the server said at connection.
    #[must_use]
    pub const fn facts(&self) -> &ServerFacts {
        &self.facts
    }

    /// Executions still running. Diagnostics and tests.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// The settings of one request.
    fn settings(
        &self,
        limits: &ExecLimits,
        intent: StatementIntent,
    ) -> Vec<(&'static str, String)> {
        if !self.facts.takes_settings() {
            return Vec::new();
        }
        let mut settings: Vec<(&'static str, String)> = SESSION_SETTINGS
            .iter()
            .map(|(name, value)| (*name, (*value).to_owned()))
            .collect();
        if limits.read_only && self.facts.can_bound_reads() {
            settings.push(("readonly", "1".to_owned()));
        } else if intent.is_read_only() && self.facts.can_bound_reads() {
            // `cancel_http_readonly_queries_on_client_close` acts only on a
            // request at `readonly` above 0 (observed on 26.3): without it, an
            // aggregate whose connection closed runs to its end. Level 2
            // keeps the statement's own `SETTINGS`, `url()` and `remote()`,
            // and refuses a write the classification took for a read.
            settings.push(("readonly", "2".to_owned()));
        }
        if let Some(timeout) = limits.timeout {
            // The server's own bound, in whole seconds rounded up: it stops
            // the statement even when the account may not kill it. The
            // executor's deadline, a little shorter, still decides the outcome.
            let seconds = timeout
                .as_secs()
                .saturating_add(u64::from(timeout.subsec_nanos() > 0));
            settings.push(("max_execution_time", seconds.max(1).to_string()));
        }
        settings
    }

    fn database(&self) -> Option<String> {
        self.place
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .database
            .clone()
    }
}

#[async_trait]
impl Session for ClickhouseSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Sends the statement and returns once its result header arrived — or,
    /// for a statement without result set, once it completed.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] for another language, a bound value
    /// ClickHouse cannot receive as text, or a typed `USE`, `SET` or
    /// `CREATE TEMPORARY TABLE`; [`OxynError::Config`] when the bound values
    /// do not match the statement's `{name:Type}` placeholders;
    /// [`OxynError::Cancelled`]; the server's refusal, classified.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        refuse_session_state(&request.text)?;
        let bound = if request.params.is_empty() {
            Vec::new()
        } else {
            params::bind(&request.text, &request.params)?
        };
        if request.limits.read_only && !self.capabilities.contains(Capabilities::READ_ONLY_SESSION)
        {
            return Err(OxynError::NotSupported {
                capability: "a read-only bound".to_owned(),
            });
        }

        let started = Instant::now();
        let handle = StatementHandle::new();
        let query_id = uuid::Uuid::new_v4().to_string();
        let token = cancel.child();
        let truncated = Arc::new(AtomicBool::new(false));
        let call = Call {
            text: request.text,
            query_id: query_id.clone(),
            database: self.database(),
            settings: self.settings(&request.limits, request.intent),
            params: Vec::new(),
            bound,
        };
        let execution = Execution {
            http: Arc::clone(&self.http),
            driver: self.driver.clone(),
            call,
            intent: request.intent,
            can_kill: self.facts.can_kill,
            json_as_text: self.facts.takes_settings(),
            max_rows: request.limits.max_rows,
            truncated: Arc::clone(&truncated),
        };
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                handle,
                Running {
                    token: token.clone(),
                    query_id,
                },
            );

        let (opened_tx, opened_rx) = oneshot::channel();
        let (batches_tx, batches_rx) = mpsc::channel(1);
        let running = Arc::clone(&self.running);
        {
            let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
            // Reaps the tasks that already ended, so the set does not grow
            // with every statement of a long session.
            while tasks.try_join_next().is_some() {}
            tasks.spawn(async move {
                stream::run(execution, token, opened_tx, batches_tx).await;
                running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&handle);
            });
        }

        let opening = opened_rx.await.map_err(|_| {
            OxynError::Internal("the execution task ended without an answer".to_owned())
        })??;
        Ok(Box::new(ClickhouseCursor::new(
            handle,
            opening.schema,
            batches_rx,
            opening.written_rows,
            truncated,
            started,
        )))
    }

    /// Asks the server to stop an execution: `KILL QUERY` on its own
    /// `query_id`, and the execution's token, which makes its task report
    /// the stop. Stopping a finished statement is not an error.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the account may not kill; the kill's
    /// own failure.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.capabilities
            .require(Capabilities::SERVER_SIDE_CANCEL)?;
        let running = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&statement)
            .cloned();
        let Some(running) = running else {
            return Ok(());
        };
        running.token.cancel();
        stream::kill(&self.http, &running.query_id).await
    }

    /// Declares the database unqualified names resolve in.
    ///
    /// ClickHouse has no catalog level. The database is checked against
    /// `system.databases`, its name bound as a parameter
    /// ([I-10](../../../CLAUDE.md#i-10)); requests then carry it. Going back
    /// to the server default restores the form's database.
    ///
    /// # Errors
    /// [`OxynError::Config`] for a catalog level, [`OxynError::Query`] for a
    /// database the server does not list, [`OxynError::Cancelled`].
    async fn set_context(&self, context: &SessionContext, cancel: &CancelToken) -> Result<()> {
        if context.catalog().is_some() {
            return Err(OxynError::Config(
                "ClickHouse has no catalog level: a context names a database only".to_owned(),
            ));
        }
        let database = match context.namespace() {
            Some(database) => {
                let call = Call {
                    text: "SELECT name FROM system.databases WHERE name = {database:String}"
                        .to_owned(),
                    query_id: uuid::Uuid::new_v4().to_string(),
                    params: vec![("database", database.to_owned())],
                    ..Call::default()
                };
                let batches =
                    fetch(&self.http, &self.driver, &call, self.facts.can_kill, cancel).await?;
                if batches.iter().all(|batch| batch.num_rows() == 0) {
                    return Err(OxynError::Query(
                        "the server lists no such database".to_owned(),
                    ));
                }
                Some(database.to_owned())
            }
            None => self.http.spec().database().map(str::to_owned),
        };
        let mut place = self.place.lock().unwrap_or_else(PoisonError::into_inner);
        place.database = database;
        place.declared = Some(context.clone());
        Ok(())
    }

    fn context(&self) -> Option<SessionContext> {
        self.place
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .declared
            .clone()
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

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Measures a round trip with `SELECT 1`, authenticated like any request.
    ///
    /// # Errors
    /// Any transport or server error: a session whose ping fails is lost.
    async fn ping(&self) -> Result<Duration> {
        let start = Instant::now();
        let call = Call {
            text: "SELECT 1".to_owned(),
            query_id: uuid::Uuid::new_v4().to_string(),
            ..Call::default()
        };
        // Bounded: a server that accepts the connection and then hangs would
        // hold the ping, and the session lock with it, for ever. `SELECT 1`
        // left behind holds nothing on the server.
        let cancel = CancelToken::new();
        let probe = fetch(
            &self.http,
            &self.driver,
            &call,
            self.facts.can_kill,
            &cancel,
        );
        tokio::time::timeout(PING_BOUND, probe)
            .await
            .map_err(|_| OxynError::Timeout { after: PING_BOUND })??;
        Ok(start.elapsed())
    }

    /// Stops what still runs, then lets the tasks end.
    ///
    /// # Errors
    /// None today: the HTTP client has no goodbye to say.
    async fn close(self: Box<Self>) -> Result<()> {
        let running: Vec<Running> = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for statement in running {
            statement.token.cancel();
        }
        let mut tasks =
            std::mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        let bound = KILL_BOUND.saturating_add(DRAIN_BOUND);
        if tokio::time::timeout(bound, async { while tasks.join_next().await.is_some() {} })
            .await
            .is_err()
        {
            tasks.shutdown().await;
        }
        Ok(())
    }
}
