//! Everything that is not the webview: state, drivers, policy, executor.
//!
//! The IPC commands in [`crate::commands`] are thin: they parse what the front
//! sent and call this module, which turns it into a [`Command`] dispatched on
//! the command bus ([I-01](../../../CLAUDE.md#i-01)). Nothing here reaches a
//! driver, the store or the keyring except through the executor — the one
//! exception, writing a draft's secrets, goes through [`KeyringCredentials`],
//! the single audited place for it.
//!
//! # No connection is opened for you
//!
//! Startup opens the local state and the driver registry, and stops there.
//! Which database to open is the user's decision
//! ([UX-SPEC](../../../docs/UX-SPEC.md)).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, DefaultPolicy,
    DriverId, ExecRequest, OxynError, PolicyGate, QueryLanguage, SessionId,
};
use oxyn_data::SinkOutcome;
use oxyn_driver::DriverRegistry;
use oxyn_driver_postgres::PostgresDriver;
use oxyn_driver_sqlite::SqliteDriver;
use oxyn_exec::{DispatchReport, ExecEvent, Executor, Outcome};
use oxyn_secrets::{KeyringSecretStore, SecretStore};
use oxyn_store::Store;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::credentials::KeyringCredentials;
use crate::ipc::{
    self, CommandOutcome, ConnectResponse, ConnectionDraft, ConnectionTest, DriverChoice, IpcError,
    OpenConnection, ResultColumn,
};

/// The assembled backend, shared by every IPC command.
#[derive(Clone)]
pub struct Backend {
    pub(crate) inner: Arc<Inner>,
}

/// State shared by every feature module under `backend/`.
///
/// A feature that needs state adds **one** field here, holding its own type
/// from its own module: this struct is the one place every feature touches.
pub(crate) struct Inner {
    /// Shared by `Arc`: an agent's command sink holds a handle for the length
    /// of a conversation, on the same executor as the interface
    /// ([ADR-0004](../../../docs/adr/0004-command-bus.md)).
    pub(crate) executor: Arc<Executor>,
    /// The only authority on which database types exist
    /// ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    pub(crate) drivers: Arc<DriverRegistry>,
    /// Closed by default: a connection created after startup must be
    /// registered, or every write on it is denied.
    pub(crate) policy: Arc<DefaultPolicy>,
    pub(crate) credentials: Arc<KeyringCredentials>,
    /// Cancellation tokens of the commands still running, by the id the front
    /// chose. Cancelling reaches the server through the executor; dropping a
    /// future would not ([I-13](../../../CLAUDE.md#i-13)).
    pub(crate) running: Mutex<tracking::Tracker>,
    /// Connection configurations awaiting a decision. Kept here rather than
    /// sent to the front: a configuration carries parameters the webview has
    /// no reason to hold ([I-03](../../../CLAUDE.md#i-03)).
    pub(crate) pending_connections: Mutex<HashMap<CommandId, ConnectionConfig>>,
    /// Consoles and documents: document writers, the shutdown marker and the
    /// local writes it waits for.
    pub(crate) workbench: consoles::Workbench,
    /// Settings: the preferences applied locally, and connection edits or
    /// deletions awaiting a decision.
    pub(crate) settings: settings::SettingsState,
    /// AI: the assistant conversation of each connection, kept across a
    /// webview reload and never persisted (UX-SPEC).
    pub(crate) ai: ai::AiState,
    /// Metadata and results: the searches remembered for « next match ».
    pub(crate) results: results::ResultsState,
}

// Not derived: `Executor` and `Store` reach connection configurations, and a
// `{backend:?}` added later is the documented way secrets leak (I-03).
impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

impl Backend {
    /// Opens the local state and assembles the executor.
    ///
    /// Must run inside the Tokio runtime's context. Blocking, and deliberately
    /// so: it runs once, before the window exists, and a failure here must
    /// reach the user as text rather than as a window onto nothing.
    ///
    /// # Errors
    /// Local state unreadable, driver registry inconsistent, keyring absent.
    pub fn open() -> Result<Self> {
        KeyringSecretStore::availability().context("the system keyring is unavailable")?;
        Self::assemble(
            Arc::new(Store::open_default().context("opening the local workspace state")?),
            Arc::new(KeyringSecretStore::new()),
        )
    }

    /// An ephemeral workspace for local QA: never touches saved state or keyring.
    ///
    /// # Errors
    /// If the in-memory state cannot be created.
    pub fn open_temporary() -> Result<Self> {
        Self::assemble(
            Arc::new(Store::open_in_memory().context("opening temporary workspace state")?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
        )
    }

    /// A workspace on disk, for the tests that must read the file itself.
    ///
    /// # Errors
    /// If the state cannot be created at `path`.
    #[cfg(test)]
    pub(crate) fn open_at(path: &std::path::Path) -> Result<Self> {
        Self::assemble(
            Arc::new(Store::open_at(path).context("opening workspace state")?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
        )
    }

    fn assemble(store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        let mut drivers = DriverRegistry::new();
        drivers
            .register(Arc::new(SqliteDriver::new()))
            .context("registering the SQLite driver")?;
        drivers
            .register(Arc::new(PostgresDriver::new()))
            .context("registering the PostgreSQL driver")?;
        tracing::info!(drivers = drivers.len(), "driver registry ready");
        let drivers = Arc::new(drivers);

        let credentials = Arc::new(KeyringCredentials::new(secrets));
        let policy = Arc::new(DefaultPolicy::new());
        let gate: Arc<dyn PolicyGate> = policy.clone();

        // Chosen before the executor is built: left to its default, the
        // builder mints a workspace that exists nowhere on disk, and saved
        // connections would never come back.
        let workspace = current_workspace(&store)?;
        // Observed, then recorded, before anything else writes: a session
        // begun later would count itself (ADR-0021).
        let local = recovery::LocalWork::begin(&store, workspace)?;

        let executor = Executor::builder(Arc::clone(&store), gate)
            .with_drivers(Arc::clone(&drivers))
            .with_credentials(Arc::clone(&credentials) as Arc<_>)
            .with_workspace(workspace)
            .build();

        let known = executor
            .load_connections()
            .context("loading the saved connections")?;
        for config in store
            .connections()
            .list(workspace)
            .context("listing the saved connections")?
        {
            policy.register(&config);
        }
        tracing::info!(connections = known, "saved connections registered");

        let backend = Self {
            inner: Arc::new(Inner {
                executor: Arc::new(executor),
                drivers,
                policy,
                credentials,
                running: Mutex::default(),
                pending_connections: Mutex::new(HashMap::new()),
                workbench: consoles::Workbench::new(local),
                settings: settings::SettingsState::default(),
                ai: ai::AiState::default(),
                results: results::ResultsState::default(),
            }),
        };
        backend.start_heartbeat();
        backend.prune_conversations();
        // Results nobody reads any more are released on a timer, off the IPC
        // threads (ADR-0017). A weak handle: the task ends with the backend.
        let weak = Arc::downgrade(&backend.inner);
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                // The same tick journals the outcomes of commands whose caller
                // dropped them: without it, an abandoned agent command has a
                // decision in the audit journal and never an outcome.
                let _ = tokio::task::spawn_blocking(move || {
                    inner.executor.prune_results();
                    inner.executor.journal_abandoned();
                })
                .await;
            }
        });
        Ok(backend)
    }

    /// Applies the assistant's disk budget to this workspace, once per launch
    /// (docs/PERFORMANCE.md: 200 threads, 90 idle days, 32 MiB).
    ///
    /// At startup rather than after each exchange: the budget bounds months of
    /// use, not one session, and a delete in the middle of a conversation
    /// could take the thread the user is reading. Off the startup path and the
    /// UI thread (I-05); a weak handle, so it ends with the backend.
    fn prune_conversations(&self) {
        let weak = Arc::downgrade(&self.inner);
        tauri::async_runtime::spawn_blocking(move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let executor = &inner.executor;
            let policy = oxyn_store::RetentionPolicy::default();
            match executor
                .store()
                .conversations()
                .prune(executor.workspace(), policy)
            {
                Ok(report) if !report.is_empty() => {
                    tracing::info!(
                        conversations = report.conversations,
                        turns = report.turns,
                        bytes = report.bytes,
                        "assistant history pruned to its budget"
                    );
                    // Not only the log: the history panel says it
                    // (docs/UX-SPEC.md).
                    *inner.ai.pruned.lock() = crate::ipc::ai::PrunedHistory::of(report, policy);
                }
                Ok(_) => {}
                // Counted, never quoted: a SQLite message may carry a path.
                Err(_) => tracing::warn!("the assistant history could not be pruned"),
            }
        });
    }

    /// The database types the connection screen may offer, from the registry.
    #[must_use]
    pub fn driver_choices(&self) -> Vec<DriverChoice> {
        self.inner
            .drivers
            .sorted()
            .iter()
            .map(|driver| DriverChoice::of(driver.metadata()))
            .collect()
    }

    /// The connections saved in the current workspace.
    ///
    /// Read from the store on each call: a list cached at startup would miss
    /// the connection created a minute ago.
    ///
    /// # Errors
    /// If the local state cannot be read.
    /// Blocking: call it from the blocking pool, never from the main thread
    /// (I-05).
    pub fn saved_connections(&self) -> Result<Vec<ipc::settings::ConnectionSummary>, IpcError> {
        let executor = &self.inner.executor;
        let configs = executor
            .store()
            .connections()
            .list(executor.workspace())
            .map_err(|error| {
                IpcError::invalid(format!("listing the saved connections: {error}"))
            })?;
        Ok(configs
            .iter()
            .map(|config| {
                ipc::settings::ConnectionSummary::of(
                    config,
                    self.inner.drivers.metadata(&config.driver),
                )
            })
            .collect())
    }

    /// Writes the draft's secrets, saves the connection, opens a session.
    ///
    /// In that order: the secret reference is part of the configuration, so
    /// the keyring write comes first. A failure after it leaves an unreferenced
    /// entry, which is harmless; the reverse would leave a saved connection
    /// whose password does not exist.
    pub async fn connect(
        &self,
        id: CommandId,
        draft: ConnectionDraft,
    ) -> Result<ConnectResponse, IpcError> {
        let inner = &self.inner;
        let cancel = self.track(id)?;

        let mut config = config_from(&draft)?;
        if !draft.secrets.is_empty() {
            let reference = inner
                .credentials
                .store_secrets(&config, &draft.secrets)
                .map_err(IpcError::from)?;
            config = config.with_secret_ref(reference.as_str());
        }

        // Through the command bus, like everything else. Writing to the store
        // directly is the second execution path I-01 forbids.
        let outcome = inner
            .executor
            .dispatch_as(
                id,
                Actor::Human,
                Command::CreateConnection {
                    config: Box::new(config.clone()),
                },
                &cancel,
            )
            .await?;

        match outcome {
            Outcome::NeedsApproval {
                command,
                reason,
                mut preview,
            } => {
                if let Some(preview) = &mut preview {
                    preview.connection = config.name.clone();
                }
                inner.pending_connections.lock().insert(command, config);
                Ok(ConnectResponse::Approval {
                    command: command.to_string(),
                    reason,
                    preview: preview.map(Into::into),
                })
            }
            Outcome::ConnectionSaved { .. } => {
                inner.policy.register(&config);
                self.open_session(config, &cancel)
                    .await
                    .map(ConnectResponse::Open)
            }
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("The connection was not saved")),
        }
    }

    /// Opens a session on the draft and closes it, saving nothing.
    ///
    /// The draft's secrets must reach the driver through the keyring, the one
    /// place the executor resolves them from: they are written under the
    /// draft's own fresh id, then forgotten whatever the test gave. A crash
    /// in between leaves an unreferenced entry, harmless like the one
    /// [`Self::connect`] may leave. Cancellable under `id`, like an opening.
    pub async fn test_connection(
        &self,
        id: CommandId,
        draft: ConnectionDraft,
    ) -> Result<ConnectionTest, IpcError> {
        let inner = &self.inner;
        let cancel = self.track(id)?;

        let mut config = config_from(&draft)?;
        // Read only whatever the form says: opening SQLite read-write creates a
        // mistyped file, and a test that leaves one behind is not a test. A
        // file that does not exist yet fails here, and says so.
        config.read_only = true;
        let stored = if draft.secrets.is_empty() {
            None
        } else {
            let lookup = config.clone();
            let secrets = draft.secrets;
            let reference = self
                .on_blocking_pool(move |backend| {
                    backend
                        .inner
                        .credentials
                        .store_secrets(&lookup, &secrets)
                        .map_err(IpcError::from)
                })
                .await?;
            config = config.with_secret_ref(reference.as_str());
            Some(reference)
        };

        let started = std::time::Instant::now();
        let outcome = inner
            .executor
            .dispatch_as(
                id,
                Actor::Human,
                Command::TestConnection {
                    config: Box::new(config),
                },
                &cancel,
            )
            .await;
        let elapsed = started.elapsed();

        if let Some(reference) = stored {
            let forgotten = self
                .on_blocking_pool(move |backend| {
                    backend
                        .inner
                        .credentials
                        .forget_secrets(reference.as_str())
                        .map_err(IpcError::from)
                })
                .await;
            // Counted, never quoted: the entry is unreachable either way.
            if forgotten.is_err() {
                tracing::warn!("the secrets of a tested draft could not be forgotten");
            }
        }

        match outcome {
            Ok(Outcome::ConnectionTested { .. }) => Ok(ConnectionTest::Succeeded {
                elapsed_ms: ipc::millis(elapsed),
            }),
            Ok(Outcome::Denied { reason, .. }) => Err(IpcError::invalid(reason)),
            Ok(_) => Err(IpcError::invalid(
                "The executor did not test the connection",
            )),
            Err(OxynError::Cancelled) => Ok(ConnectionTest::Cancelled),
            Err(error) => Ok(ConnectionTest::Failed {
                message: error.to_string(),
                class: error.class().as_str(),
                retryable: error.is_retryable(),
            }),
        }
    }

    /// Completes, or rejects, a connection setup the policy held back.
    pub async fn decide_connection(
        &self,
        command: CommandId,
        approved: bool,
    ) -> Result<Option<ConnectResponse>, IpcError> {
        let inner = &self.inner;
        let Some(config) = inner.pending_connections.lock().remove(&command) else {
            return Err(IpcError::invalid("No connection is awaiting this decision"));
        };
        if !approved {
            inner.executor.reject(command);
            return Ok(None);
        }
        // Registered under the pending command's id: `cancel(command)` must
        // reach the approval and the session opening that follows. Refused,
        // the decision stays pending rather than losing its configuration.
        let cancel = match self.track(command) {
            Ok(cancel) => cancel,
            Err(error) => {
                inner.pending_connections.lock().insert(command, config);
                return Err(error);
            }
        };
        match inner.executor.approve("human", command, &cancel).await? {
            Outcome::ConnectionSaved { .. } => {}
            Outcome::Denied { reason, .. } => return Err(IpcError::invalid(reason)),
            _ => return Err(IpcError::invalid("The connection was not saved")),
        }
        inner.policy.register(&config);
        self.open_session(config, &cancel)
            .await
            .map(|open| Some(ConnectResponse::Open(open)))
    }

    /// Opens a session on a connection the store already knows.
    pub async fn reconnect(
        &self,
        id: CommandId,
        connection: ConnectionId,
    ) -> Result<OpenConnection, IpcError> {
        let inner = &self.inner;
        let cancel = self.track(id)?;
        let config = self.read_config(connection).await?;
        inner.policy.register(&config);
        self.open_session(config, &cancel).await
    }

    /// `Command::Connect`, then what the workspace may know about it.
    async fn open_session(
        &self,
        config: ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<OpenConnection, IpcError> {
        let inner = &self.inner;
        let outcome = inner
            .executor
            .dispatch(
                Actor::Human,
                Command::Connect {
                    connection: config.id,
                },
                cancel,
            )
            .await
            // The server's own words, code included: the audience reads
            // PostgreSQL errors, and a paraphrase removes what tells them what
            // to fix. The class is kept: `anyhow` would flatten `retryable`.
            .map_err(|error| IpcError {
                retryable: error.is_retryable(),
                message: format!("opening a session on \"{}\": {error}", config.name),
            })?;

        let Outcome::Connected { session, .. } = outcome else {
            return Err(IpcError::invalid("The executor did not open a session"));
        };
        // The first console gets its own session: catalog and preview work
        // stay out of its transaction, and it may change context where the
        // catalog's session may not (ADR-0015, ADR-0019).
        let console = match self.open_console(CommandId::new(), config.id).await {
            Ok(console) => console,
            Err(error) => {
                let _ = self.close_console(config.id, session).await;
                return Err(error);
            }
        };
        let capabilities = inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("The opened session is no longer registered"))?
            .capabilities();
        tracing::info!(
            connection = %config.name,
            driver = %config.driver,
            environment = %config.environment,
            "connection open"
        );
        Ok(OpenConnection {
            connection: config.id.to_string(),
            session: session.to_string(),
            name: config.name.clone(),
            driver: config.driver.to_string(),
            environment: config.environment,
            read_only: config.read_only,
            privacy_tier: config.privacy_tier,
            capabilities: ipc::capability_names(capabilities),
            console,
        })
    }

    /// Closes the sessions of a connection.
    pub async fn disconnect(&self, connection: ConnectionId) -> Result<CommandOutcome, IpcError> {
        // An agent's tools act on a session of this connection: none outlives
        // the sessions it was given.
        self.inner.ai.release_agents(connection);
        self.run(CommandId::new(), Command::Disconnect { connection })
            .await
    }

    /// Runs the SQL the user wrote, as written.
    pub async fn execute(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        sql: String,
    ) -> Result<CommandOutcome, IpcError> {
        let config = self.read_config(connection).await?;
        let dialect = oxyn_query::dialect_for(&config.driver);
        let mut request = ExecRequest::new(QueryLanguage::Sql(dialect), sql);
        request.limits.read_only = config.read_only;
        self.run(
            id,
            Command::Execute {
                connection,
                session,
                request: Box::new(request),
            },
        )
        .await
    }

    /// Resolves a human decision against the exact pending command.
    ///
    /// An agent's call may be waiting on it: it is told what the decision did,
    /// and answers its model with that.
    pub async fn decide(
        &self,
        command: CommandId,
        approved: bool,
    ) -> Result<CommandOutcome, IpcError> {
        let inner = &self.inner;
        if !approved {
            let answer = inner.ai.decisions.answer(command);
            inner.executor.reject(command);
            answer.report(DispatchReport::Denied {
                command,
                reason: "the user rejected this statement".to_owned(),
            });
            return Ok(CommandOutcome::Denied {
                reason: "Operation rejected".into(),
            });
        }
        // Tracked before the waiting agent's answer is taken: refused, the
        // decision stays pending and the agent still waits on it.
        let cancel = self.track(command)?;
        let answer = inner.ai.decisions.answer(command);
        let decided = inner.executor.approve("human", command, &cancel).await;
        answer.report(DispatchReport::decided(command, &decided));
        let outcome = decided?;
        self.hold_shown(&outcome);
        Ok(describe(outcome))
    }

    /// A receiver on the execution event stream.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ExecEvent> {
        self.inner.executor.subscribe()
    }

    /// Dispatches a command under a cancellable token the front can reach.
    pub(crate) async fn run(
        &self,
        id: CommandId,
        command: Command,
    ) -> Result<CommandOutcome, IpcError> {
        let inner = &self.inner;
        let cancel = self.track(id)?;
        // Held until the write ends: the shutdown waits for it (ADR-0021).
        let _local = self.local_write(&command);
        match inner
            .executor
            .dispatch_as(id, Actor::Human, command, &cancel)
            .await
        {
            Ok(outcome) => {
                // Held for the webview, which holds nothing itself.
                self.hold_shown(&outcome);
                Ok(describe(outcome))
            }
            Err(OxynError::Cancelled) => Ok(CommandOutcome::Cancelled),
            Err(error) => Err(error.into()),
        }
    }

    /// Reads the store: only from the blocking pool, or through
    /// [`Self::read_config`] from an async body (I-05).
    pub(crate) fn config(&self, connection: ConnectionId) -> Result<ConnectionConfig, IpcError> {
        #[cfg(test)]
        config_reads::record(connection);
        self.inner
            .executor
            .store()
            .connections()
            .get(connection)
            .map_err(|error| IpcError::invalid(format!("reading the connection: {error}")))?
            .ok_or_else(|| IpcError::invalid("This connection is no longer in the workspace"))
    }

    /// [`Self::config`] off the runtime worker: a store waiting on a lock — the
    /// startup pruning writes at the same moment — must not stall every other
    /// command on that worker (ARCHITECTURE, the thread model).
    pub(crate) async fn read_config(
        &self,
        connection: ConnectionId,
    ) -> Result<ConnectionConfig, IpcError> {
        self.on_blocking_pool(move |backend| backend.config(connection))
            .await
    }

    /// Resolved from the driver, never defaulted to ANSI: the dialect decides
    /// how `EXPLAIN (ANALYZE) DELETE …` is classified ([I-07](../../../CLAUDE.md#i-07)).
    pub(crate) fn dialect_of(
        &self,
        connection: ConnectionId,
    ) -> Result<oxyn_core::SqlDialect, IpcError> {
        Ok(oxyn_query::dialect_for(&self.config(connection)?.driver))
    }
}

/// The configuration a draft describes, secrets aside, under a fresh id.
fn config_from(draft: &ConnectionDraft) -> Result<ConnectionConfig, IpcError> {
    let driver = DriverId::new(&draft.driver)
        .map_err(|error| IpcError::invalid(format!("unknown driver: {error}")))?;
    // The tier and the read-only flag are the user's choices on the form;
    // dropped here, the connection would silently open under the defaults
    // (I-04).
    let mut config = ConnectionConfig::new(draft.name.clone(), driver)
        .with_environment(draft.environment)
        .with_privacy_tier(draft.privacy_tier);
    config.read_only = draft.read_only;
    for (key, value) in &draft.values {
        config = config.with_param(key, value);
    }
    Ok(config)
}

/// The workspace the session works in — the first one, or a new one.
fn current_workspace(store: &Arc<Store>) -> Result<oxyn_core::WorkspaceId> {
    let workspaces = store.workspaces().list().context("listing workspaces")?;
    match workspaces.into_iter().next() {
        Some(existing) => Ok(existing.id),
        None => Ok(store
            .workspaces()
            .create("oxyn")
            .context("creating the first workspace")?
            .id),
    }
}

/// What the front needs to know about an outcome, and nothing more.
pub(crate) fn describe(outcome: Outcome) -> CommandOutcome {
    match outcome {
        Outcome::Executed {
            result,
            buffer,
            stats,
            sink,
            ..
        } => CommandOutcome::Executed {
            result: result.to_string(),
            columns: buffer
                .schema()
                .fields()
                .iter()
                .map(|field| ResultColumn {
                    name: field.name().clone(),
                    data_type: field.data_type().to_string(),
                    nullable: field.is_nullable(),
                })
                .collect(),
            rows: stats.rows,
            elapsed_ms: ipc::millis(stats.total_time),
            complete: matches!(sink, SinkOutcome::Exhausted),
            cancelled: matches!(sink, SinkOutcome::Cancelled),
            truncated: stats.truncated,
        },
        Outcome::NeedsApproval {
            command,
            reason,
            preview,
        } => CommandOutcome::NeedsApproval {
            command: command.to_string(),
            reason,
            preview: preview.map(Into::into),
        },
        Outcome::Denied { reason, .. } => CommandOutcome::Denied { reason },
        Outcome::CatalogRefreshed { .. } => CommandOutcome::CatalogRefreshed,
        Outcome::Exported { rows, bytes, .. } => CommandOutcome::Exported { rows, bytes },
        Outcome::Cancelled { .. } => CommandOutcome::Cancelled,
        _ => CommandOutcome::Done,
    }
}

/// The threads [`Backend::config`] ran on, per connection: a store read made
/// on the thread that polls an async body is invisible on the dev machine, and
/// only a record of where it ran shows it (I-05).
#[cfg(test)]
pub(crate) mod config_reads {
    use std::thread::ThreadId;

    use oxyn_core::ConnectionId;
    use parking_lot::Mutex;

    static READS: Mutex<Vec<(ConnectionId, ThreadId)>> = Mutex::new(Vec::new());

    pub(crate) fn record(connection: ConnectionId) {
        READS.lock().push((connection, std::thread::current().id()));
    }

    /// The threads that read `connection`, oldest first.
    pub(crate) fn of(connection: ConnectionId) -> Vec<ThreadId> {
        READS
            .lock()
            .iter()
            .filter(|(read, _)| *read == connection)
            .map(|(_, thread)| *thread)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use oxyn_core::{Environment, ResultId};

    use super::*;
    use crate::ipc::results::ResultWindow;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts")
    }

    fn draft(environment: Environment) -> ConnectionDraft {
        ConnectionDraft {
            driver: "sqlite".into(),
            name: "IPC regression".into(),
            environment,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: BTreeMap::new(),
        }
    }

    fn open(
        runtime: &tokio::runtime::Runtime,
        backend: &Backend,
        environment: Environment,
    ) -> OpenConnection {
        runtime.block_on(async {
            match backend
                .connect(CommandId::new(), draft(environment))
                .await
                .expect("connects")
            {
                ConnectResponse::Open(open) => open,
                ConnectResponse::Approval { command, .. } => {
                    let command = command.parse().expect("an id the backend minted");
                    match backend
                        .decide_connection(command, true)
                        .await
                        .expect("approved")
                    {
                        Some(ConnectResponse::Open(open)) => open,
                        _ => panic!("an approved connection opens"),
                    }
                }
            }
        })
    }

    fn ids(open: &OpenConnection) -> (ConnectionId, SessionId) {
        (
            open.connection.parse().expect("connection id"),
            open.session.parse().expect("session id"),
        )
    }

    #[test]
    fn a_query_runs_pages_and_survives_an_error() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let open = open(&runtime, &backend, Environment::Local);
        let (connection, session) = ids(&open);

        let outcome = runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<5000) SELECT x, NULL AS empty_value FROM n".into(),
            ))
            .expect("executes");
        let CommandOutcome::Executed {
            result,
            rows,
            columns,
            ..
        } = outcome
        else {
            panic!("a SELECT executes, got {outcome:?}");
        };
        assert_eq!(rows, 5000);
        assert_eq!(columns.len(), 2);

        let result: ResultId = result.parse().expect("result id");
        let ResultWindow::Page(page) = runtime
            .block_on(backend.read_result_page(connection, result, 4990, 100))
            .expect("page")
        else {
            panic!("a held result pages");
        };
        assert_eq!(page.rows.len(), 10, "the page stops at the last row");
        assert_eq!(page.total_rows, 5000);
        let json = serde_json::to_string(&page.rows[0]).expect("serializable");
        assert_eq!(json, r#"["4991",null]"#);

        let ResultWindow::Page(huge) = runtime
            .block_on(backend.read_result_page(connection, result, 0, usize::MAX))
            .expect("page")
        else {
            panic!("a held result pages");
        };
        assert_eq!(
            huge.rows.len(),
            results::MAX_PAGE_ROWS,
            "a page is bounded whatever the front asks"
        );

        assert!(
            runtime
                .block_on(backend.execute(
                    CommandId::new(),
                    connection,
                    session,
                    "SELECT missing FROM nowhere".into()
                ))
                .is_err()
        );
        assert!(matches!(
            runtime.block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "SELECT 1".into()
            )),
            Ok(CommandOutcome::Executed { .. })
        ));
    }

    #[test]
    fn a_production_write_waits_for_the_exact_decision() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let open = open(&runtime, &backend, Environment::Production);
        let (connection, session) = ids(&open);

        let outcome = runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "CREATE TABLE approved (id INTEGER)".into(),
            ))
            .expect("policy answers");
        let CommandOutcome::NeedsApproval {
            command, preview, ..
        } = outcome
        else {
            panic!("a production write needs approval, got {outcome:?}");
        };
        assert_eq!(preview.expect("a preview").connection, "IPC regression");

        let command: CommandId = command.parse().expect("command id");
        let decided = runtime
            .block_on(backend.decide(command, true))
            .expect("approved");
        assert!(matches!(decided, CommandOutcome::Executed { .. }));

        let outcome = runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "DROP TABLE approved".into(),
            ))
            .expect("policy answers");
        let CommandOutcome::NeedsApproval { command, .. } = outcome else {
            panic!("a production drop needs approval");
        };
        let rejected = runtime
            .block_on(backend.decide(command.parse().expect("command id"), false))
            .expect("rejected");
        assert!(matches!(rejected, CommandOutcome::Denied { .. }));
        assert!(
            runtime
                .block_on(backend.execute(
                    CommandId::new(),
                    connection,
                    session,
                    "SELECT * FROM approved".into()
                ))
                .is_ok(),
            "the rejected drop did not run"
        );
    }

    #[test]
    fn cancelling_reaches_the_running_statement() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let open = open(&runtime, &backend, Environment::Local);
        let (connection, session) = ids(&open);
        let id = CommandId::new();
        let mut events = backend.subscribe();

        let running = runtime.spawn({
            let backend = backend.clone();
            async move {
                backend
                    .execute(id, connection, session,
                        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT x FROM n".into())
                    .await
            }
        });
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let event = events.recv().await.expect("event stream");
                    if event.command == id
                        && matches!(event.event, oxyn_core::Event::BatchReady { .. })
                    {
                        break;
                    }
                }
            })
            .await
            .expect("a first batch arrives");
        });
        assert!(backend.cancel(id), "the command is still running");
        let outcome = runtime
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(10), running).await
            })
            .expect("cancellation deadline")
            .expect("task joins")
            .expect("a cancelled run is not an error");
        assert!(matches!(
            outcome,
            CommandOutcome::Cancelled
                | CommandOutcome::Executed {
                    cancelled: true,
                    ..
                }
        ));
        assert!(
            !backend.cancel(id),
            "a finished command leaves no token behind"
        );
    }

    #[test]
    fn a_tested_draft_is_not_saved() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let tested = runtime
            .block_on(backend.test_connection(CommandId::new(), draft(Environment::Production)))
            .expect("a test answers");
        assert!(
            matches!(tested, ConnectionTest::Succeeded { .. }),
            "{tested:?}"
        );
        assert!(
            backend.saved_connections().expect("list").is_empty(),
            "testing saves nothing"
        );
    }

    #[test]
    fn testing_a_missing_sqlite_file_creates_nothing() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("mistyped.sqlite");
        let mut draft = draft(Environment::Local);
        draft
            .values
            .insert("path".to_owned(), path.display().to_string());
        let tested = runtime
            .block_on(backend.test_connection(CommandId::new(), draft))
            .expect("a test answers");
        assert!(
            matches!(tested, ConnectionTest::Failed { .. }),
            "{tested:?}"
        );
        assert!(!path.exists(), "a test must not create the database file");
    }

    #[test]
    fn a_failed_test_is_classified_and_quotes_no_secret() {
        const CANARY: &str = "canary-password-3c71";
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        // Port 1 on the loopback: refused at once, without a server.
        let draft = ConnectionDraft {
            driver: "postgres".into(),
            name: "Unreachable".into(),
            environment: Environment::Production,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [
                ("host".to_owned(), "127.0.0.1".to_owned()),
                ("port".to_owned(), "1".to_owned()),
                ("user".to_owned(), "oxyn".to_owned()),
                ("database".to_owned(), "oxyn".to_owned()),
            ]
            .into_iter()
            .collect(),
            secrets: [("password".to_owned(), CANARY.to_owned())]
                .into_iter()
                .collect(),
        };
        let tested = runtime
            .block_on(backend.test_connection(CommandId::new(), draft))
            .expect("a failed test is an answer, not an IPC error");
        let ConnectionTest::Failed {
            message,
            class,
            retryable,
        } = &tested
        else {
            panic!("nothing listens on port 1, got {tested:?}");
        };
        assert!(
            !message.contains(CANARY),
            "secret in the message: {message}"
        );
        assert!(["transient", "permanent", "ambiguous"].contains(class));
        assert_eq!(*retryable, *class == "transient");
        let serialized = serde_json::to_string(&tested).expect("serializable");
        assert!(
            !serialized.contains(CANARY),
            "secret on the wire: {serialized}"
        );
    }

    /// A current-thread runtime polls every async body on the test thread:
    /// a read of the saved connection made there would stall every command
    /// sharing its worker.
    #[test]
    fn reconnecting_reads_the_saved_connection_off_the_async_thread() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let (connection, _) = ids(&open(&runtime, &backend, Environment::Local));
        let before = config_reads::of(connection).len();

        runtime
            .block_on(backend.reconnect(CommandId::new(), connection))
            .expect("reconnects");

        let reads = config_reads::of(connection);
        let async_thread = std::thread::current().id();
        assert!(reads.len() > before, "reconnect reads through `config`");
        assert!(
            !reads.contains(&async_thread),
            "the saved connection was read on the thread that polls async bodies"
        );
    }

    #[test]
    fn an_unknown_pending_connection_is_refused() {
        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        assert!(
            runtime
                .block_on(backend.decide_connection(CommandId::new(), true))
                .is_err()
        );
    }
}

mod ai;
mod consoles;
mod documents;
mod library;
mod metadata;
mod proposal;
mod recovery;
mod results;
mod settings;
mod tracking;
