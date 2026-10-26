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

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, DefaultPolicy,
    DriverId, ExecRequest, OxynError, PolicyGate, SessionId,
};
use oxyn_data::SinkOutcome;
use oxyn_driver::DriverRegistry;
use oxyn_driver_clickhouse::ClickhouseDriver;
use oxyn_driver_duckdb::DuckDbDriver;
use oxyn_driver_elasticsearch::ElasticsearchDriver;
use oxyn_driver_mongodb::MongoDriver;
use oxyn_driver_mysql::MysqlDriver;
use oxyn_driver_postgres::PostgresDriver;
use oxyn_driver_redis::RedisDriver;
use oxyn_driver_sqlite::SqliteDriver;
use oxyn_exec::{DispatchReport, ExecEvent, Executor, Outcome, PendingCommand};
use oxyn_secrets::{KeyringSecretStore, SecretStore};
use oxyn_store::Store;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use self::confirm::HostAnswer;
pub(crate) use self::confirm::{HostConfirm, NativeDialog};
pub(crate) use self::exit::ExitStep;
pub(crate) use self::windows::{CloseStep, WindowKey};
use crate::credentials::KeyringCredentials;
use crate::ipc::consoles::ConsoleSession;
use crate::ipc::{
    self, CommandOutcome, ConnectResponse, ConnectionDraft, ConnectionTest, DriverChoice, IpcError,
    OpenConnection, ResultColumn,
};

/// The assembled backend, shared by every IPC command.
#[derive(Clone)]
pub struct Backend {
    pub(crate) inner: Arc<Inner>,
}

/// A creation held by policy. Secrets stay in memory until the declaration
/// exists; rejecting or closing the owning window simply drops them (I-03).
/// No `Debug`: this value owns the secrets typed in the form.
pub(crate) struct PendingConnection {
    config: ConnectionConfig,
    secrets: BTreeMap<String, String>,
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
    pub(crate) pending_connections: Mutex<HashMap<CommandId, PendingConnection>>,
    /// Consoles and documents: document writers, the shutdown marker and the
    /// local writes it waits for.
    pub(crate) workbench: consoles::Workbench,
    /// Settings: the preferences applied locally, and connection edits or
    /// deletions awaiting a decision.
    pub(crate) settings: settings::SettingsState,
    /// AI: the assistant conversation of each connection, kept across a
    /// webview reload and never persisted (UX-SPEC).
    pub(crate) ai: ai::AiState,
    /// The host's dialog for critical decisions, and the one it may have open
    /// ([ADR-0037](../../../docs/adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)).
    pub(crate) confirmations: confirm::Confirmations,
    /// Metadata and results: the searches remembered for « next match ».
    pub(crate) results: results::ResultsState,
    /// The review sessions of `Drop…`, `Truncate…` and `Rename…` whose
    /// command waits for a decision (ADR-0042).
    pub(crate) object_operations: object_operations::ReviewSessions,
    /// The windows, and what each one owns: its sessions, commands, results
    /// and assistants (ADR-0043).
    pub(crate) windows: windows::WindowRegistry,
    /// Where each window stands and what it holds, for the workspace file.
    pub(crate) layouts: windows::Layouts,
    /// The tabs moved to a new window, until its webview adopts them.
    pub(crate) handoffs: windows::Handoffs,
    /// Local semantic ranking: the model's place, its download, the model
    /// in memory while used, and the relation vectors (ADR-0056).
    pub(crate) semantic: semantic::SemanticState,
    /// The editor's completion pass each connection has running, so that a
    /// newer one cancels the older (ADR-0057).
    pub(crate) completion_passes: completion_load::Passes,
}

// Not derived: `Executor` and `Store` reach connection configurations, and a
// `{backend:?}` added later is the documented way secrets leak (I-03).
impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

/// The windows a finished launch left, adopted for this one (ADR-0043).
///
/// An unreadable file costs the layout, never the launch: one window opens,
/// as before migration 18. When the file holds none, the first window takes
/// the object tab the preferences held.
fn adopt_layouts(
    store: &Store,
    workspace: oxyn_core::WorkspaceId,
    session: oxyn_core::AppSessionId,
) -> windows::Layouts {
    let adopted = store
        .windows()
        .adopt(workspace, session)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "window layouts unreadable; opening one window");
            Vec::new()
        });
    let inherited = if adopted.is_empty() {
        store
            .preferences()
            .load(workspace)
            .ok()
            .and_then(|snapshot| snapshot.preferences.object_location)
    } else {
        None
    };
    windows::Layouts::new(adopted, inherited)
}

impl Backend {
    /// Opens the local state and assembles the executor.
    ///
    /// Must run inside the Tokio runtime's context. Blocking, and deliberately
    /// so: it runs once, before the window exists, and a failure here must
    /// reach the user as text rather than as a window onto nothing.
    ///
    /// `confirm` is the host's dialog for critical decisions. No default: a
    /// backend without one does not exist, so no critical decision passes for
    /// want of a dialog (ADR-0037).
    ///
    /// # Errors
    /// Local state unreadable, driver registry inconsistent, keyring absent.
    pub(crate) fn open(confirm: Arc<dyn HostConfirm>) -> Result<Self> {
        KeyringSecretStore::availability().context("the system keyring is unavailable")?;
        let backend = Self::assemble_with(
            Arc::new(Store::open_default().context("opening the local workspace state")?),
            Arc::new(KeyringSecretStore::new()),
            confirm::Confirmations::new(confirm, confirm::Timing::HOST),
            oxyn_data::DEFAULT_MEMORY_BUDGET,
        )?;
        backend.place_models();
        Ok(backend)
    }

    /// An ephemeral workspace for local QA: never touches saved state or keyring.
    ///
    /// # Errors
    /// If the in-memory state cannot be created.
    pub(crate) fn open_temporary_with(confirm: Arc<dyn HostConfirm>) -> Result<Self> {
        let backend =
            Self::open_in_memory(confirm::Confirmations::new(confirm, confirm::Timing::HOST))?;
        // The model is the machine's, not the workspace's: see `place_models`.
        backend.place_models();
        Ok(backend)
    }

    /// A temporary workspace whose host confirms every critical decision at
    /// once: the tests that are not about the dialog.
    ///
    /// # Errors
    /// If the in-memory state cannot be created.
    #[cfg(test)]
    pub(crate) fn open_temporary() -> Result<Self> {
        Self::open_in_memory(confirm::Confirmations::confirming())
    }

    /// A temporary workspace on a scripted host and its own timing: the
    /// tests of the dialog itself.
    ///
    /// # Errors
    /// If the in-memory state cannot be created.
    #[cfg(test)]
    pub(crate) fn open_scripted(
        confirm: Arc<confirm::ScriptedConfirm>,
        timing: confirm::Timing,
    ) -> Result<Self> {
        Self::open_in_memory(confirm::Confirmations::new(confirm, timing))
    }

    fn open_in_memory(confirmations: confirm::Confirmations) -> Result<Self> {
        Self::assemble_with(
            Arc::new(Store::open_in_memory().context("opening temporary workspace state")?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
            confirmations,
            oxyn_data::DEFAULT_MEMORY_BUDGET,
        )
    }

    /// A temporary workspace whose results spill to disk past `memory_budget`
    /// bytes: the tests of pages read back from disk or from the page cache.
    ///
    /// # Errors
    /// If the in-memory state cannot be created.
    #[cfg(test)]
    pub(crate) fn open_temporary_spilling(memory_budget: usize) -> Result<Self> {
        Self::assemble_with(
            Arc::new(Store::open_in_memory().context("opening temporary workspace state")?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
            confirm::Confirmations::confirming(),
            memory_budget,
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

    /// [`Self::assemble_with`] on a host that confirms every critical
    /// decision at once: the tests that are not about the dialog.
    #[cfg(test)]
    fn assemble(store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        Self::assemble_with(
            store,
            secrets,
            confirm::Confirmations::confirming(),
            oxyn_data::DEFAULT_MEMORY_BUDGET,
        )
    }

    fn assemble_with(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        confirmations: confirm::Confirmations,
        memory_budget: usize,
    ) -> Result<Self> {
        let policy = Arc::new(DefaultPolicy::new());
        let gate: Arc<dyn PolicyGate> = policy.clone();
        Self::assemble_parts(
            store,
            secrets,
            confirmations,
            memory_budget,
            policy,
            gate,
            shipped_drivers()?,
        )
    }

    #[cfg(test)]
    fn assemble_with_test_gate(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        gate: Arc<dyn PolicyGate>,
    ) -> Result<Self> {
        Self::assemble_parts(
            store,
            secrets,
            confirm::Confirmations::confirming(),
            oxyn_data::DEFAULT_MEMORY_BUDGET,
            Arc::new(DefaultPolicy::new()),
            gate,
            shipped_drivers()?,
        )
    }

    /// A temporary workspace that also knows `driver`: the tests of a
    /// language no shipped driver speaks yet.
    ///
    /// # Errors
    /// If the in-memory state cannot be created, or `driver` takes the
    /// identifier of a shipped one.
    #[cfg(test)]
    pub(crate) fn open_temporary_with_driver(driver: Arc<dyn oxyn_driver::Driver>) -> Result<Self> {
        let mut drivers = shipped_drivers()?;
        drivers
            .register(driver)
            .context("registering the test driver")?;
        let policy = Arc::new(DefaultPolicy::new());
        let gate: Arc<dyn PolicyGate> = policy.clone();
        Self::assemble_parts(
            Arc::new(Store::open_in_memory().context("opening temporary workspace state")?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
            confirm::Confirmations::confirming(),
            oxyn_data::DEFAULT_MEMORY_BUDGET,
            policy,
            gate,
            drivers,
        )
    }

    fn assemble_parts(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        confirmations: confirm::Confirmations,
        memory_budget: usize,
        policy: Arc<DefaultPolicy>,
        gate: Arc<dyn PolicyGate>,
        drivers: DriverRegistry,
    ) -> Result<Self> {
        tracing::info!(drivers = drivers.len(), "driver registry ready");
        let drivers = Arc::new(drivers);

        let credentials = Arc::new(KeyringCredentials::new(secrets));

        // Chosen before the executor is built: left to its default, the
        // builder mints a workspace that exists nowhere on disk, and saved
        // connections would never come back.
        let workspace = current_workspace(&store)?;
        // Observed, then recorded, before anything else writes: a session
        // begun later would count itself (ADR-0021).
        let local = recovery::LocalWork::begin(&store, workspace)?;
        let layouts = adopt_layouts(&store, workspace, local.session);

        let executor = Executor::builder(Arc::clone(&store), gate)
            .with_drivers(Arc::clone(&drivers))
            .with_credentials(Arc::clone(&credentials) as Arc<_>)
            .with_workspace(workspace)
            .with_memory_budget(memory_budget)
            .with_app_session(local.session)
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

        // Subscribed before anything runs: a state published earlier would
        // be missed, and a transaction left out of the exit (ADR-0043).
        let consoles = Arc::new(exit::ConsoleTransactions::new(executor.subscribe()));
        tauri::async_runtime::spawn(Arc::clone(&consoles).follow());

        let backend = Self {
            inner: Arc::new(Inner {
                executor: Arc::new(executor),
                drivers,
                policy,
                credentials,
                running: Mutex::default(),
                pending_connections: Mutex::new(HashMap::new()),
                workbench: consoles::Workbench::new(local, consoles),
                settings: settings::SettingsState::default(),
                ai: ai::AiState::default(),
                confirmations,
                results: results::ResultsState::default(),
                object_operations: object_operations::ReviewSessions::default(),
                windows: windows::WindowRegistry::default(),
                layouts,
                handoffs: windows::Handoffs::default(),
                semantic: semantic::SemanticState::default(),
                completion_passes: completion_load::Passes::default(),
            }),
        };
        backend.start_heartbeat();
        backend.prune_conversations();
        backend.load_user_agents();
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
                let pruned = Arc::clone(&inner);
                let _ = tokio::task::spawn_blocking(move || {
                    pruned.executor.prune_results();
                    pruned.executor.journal_abandoned();
                })
                .await;
                // A review session outlives its command only until the
                // approval expires (ADR-0042).
                Self { inner }.release_stale_reviews().await;
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
            config.secret_ref = Some(KeyringCredentials::fresh_reference(config.id));
        }
        let pending = PendingConnection {
            config: config.clone(),
            secrets: draft.secrets,
        };

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
                inner.pending_connections.lock().insert(command, pending);
                Ok(ConnectResponse::Approval {
                    command: command.to_string(),
                    reason,
                    preview: preview.map(Into::into),
                })
            }
            Outcome::ConnectionSaved { .. } => Ok(self.finish_creation(id, pending, &cancel).await),
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("The connection was not saved")),
        }
    }

    /// Opens a session on the draft and closes it, saving nothing.
    ///
    /// The draft's secrets must reach the driver through the keyring, the one
    /// place the executor resolves them from: they are written under the
    /// draft's own fresh id, then forgotten whatever the test gave. A crash
    /// in between leaves an unreferenced entry. Cancellable under `id`, like
    /// an opening.
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
        if !inner.pending_connections.lock().contains_key(&command) {
            return Err(IpcError::invalid("No connection is awaiting this decision"));
        }
        if !approved {
            inner.pending_connections.lock().remove(&command);
            inner.executor.reject(command);
            return Ok(None);
        }
        // Registered under the pending command's id, before the host is asked:
        // `cancel(command)` must reach the approval and the session opening
        // that follows, and no other command may take this id while the
        // dialog is open. Refused, the decision stays pending.
        let cancel = self.track(command)?;
        // Before the configuration is taken: a busy dialog leaves it pending.
        let answer = self.confirm_held(command).await?;
        let Some(pending) = inner.pending_connections.lock().remove(&command) else {
            return Err(IpcError::invalid("No connection is awaiting this decision"));
        };
        if answer == HostAnswer::Refused {
            inner.executor.reject(command);
            return Ok(None);
        }
        match inner.executor.approve("human", command, &cancel).await? {
            Outcome::ConnectionSaved { .. } => {}
            Outcome::Denied { reason, .. } => return Err(IpcError::invalid(reason)),
            _ => return Err(IpcError::invalid("The connection was not saved")),
        }
        Ok(Some(self.finish_creation(command, pending, &cancel).await))
    }

    async fn finish_creation(
        &self,
        id: CommandId,
        pending: PendingConnection,
        cancel: &CancelToken,
    ) -> ConnectResponse {
        let PendingConnection { config, secrets } = pending;
        self.inner.policy.register(&config);
        if !secrets.is_empty() {
            let target = config.clone();
            if let Err(error) = self
                .on_blocking_pool(move |backend| {
                    backend
                        .inner
                        .credentials
                        .store_secrets(&target, &secrets)
                        .map(|_| ())
                        .map_err(IpcError::from)
                })
                .await
            {
                return ConnectResponse::Saved {
                    connection: ipc::SavedConnection::of(&config),
                    message: format!(
                        "The connection was saved without its secrets: type them again. {}",
                        error.message
                    ),
                    retryable: error.retryable,
                };
            }
        }
        self.open_saved(id, config, cancel).await
    }

    /// Opens a connection just saved.
    ///
    /// A failed opening does not unsave it: the answer names the saved
    /// connection, so that connecting again from the same form edits it
    /// rather than saving one more.
    async fn open_saved(
        &self,
        id: CommandId,
        config: ConnectionConfig,
        cancel: &CancelToken,
    ) -> ConnectResponse {
        let saved = ipc::SavedConnection::of(&config);
        match self.open_session(id, config, cancel).await {
            Ok(open) => ConnectResponse::Open(open),
            Err(error) => ConnectResponse::Saved {
                connection: saved,
                message: error.message,
                retryable: error.retryable,
            },
        }
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
        self.open_session(id, config, &cancel).await
    }

    /// `Command::Connect`, then what the workspace may know about it.
    ///
    /// The opening of `id`, if a window recorded it, names its connection
    /// first: from before its first session until its window adopts them,
    /// another window's cleanup does not disconnect them and a deletion is
    /// refused (ADR-0043).
    async fn open_session(
        &self,
        id: CommandId,
        config: ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<OpenConnection, IpcError> {
        self.inner
            .windows
            .bind_opening(id, config.id, cancel)
            .await?;
        let session = self.catalog_session(&config, cancel).await?;
        // The first console gets its own session: catalog and preview work
        // stay out of its transaction, and it may change context where the
        // catalog's session may not (ADR-0015, ADR-0019). Under the opening's
        // token: a window closed meanwhile cancels both sessions, not one.
        let console = match self
            .open_console_under(CommandId::new(), config.id, cancel)
            .await
        {
            Ok(console) => console,
            Err(error) => {
                let _ = self.close_console(config.id, session).await;
                return Err(error);
            }
        };
        self.workspace_of(&config, session, console)
    }

    /// The session of a workspace's catalog and previews: `Command::Connect`.
    pub(crate) async fn catalog_session(
        &self,
        config: &ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<SessionId, IpcError> {
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
                missing_module: error.missing_module().map(str::to_owned),
                message: format!("opening a session on \"{}\": {error}", config.name),
            })?;

        let Outcome::Connected { session, .. } = outcome else {
            return Err(IpcError::invalid("The executor did not open a session"));
        };
        // Cancelled while the server answered — its window closed: nobody
        // will use the session, and it must not outlive the gesture.
        if cancel.is_cancelled() {
            self.close_console(config.id, session).await?;
            return Err(IpcError::invalid("Opening the connection was cancelled."));
        }
        Ok(session)
    }

    /// What a workspace may know about its connection, its catalog's session
    /// and its first console.
    pub(crate) fn workspace_of(
        &self,
        config: &ConnectionConfig,
        session: SessionId,
        console: ConsoleSession,
    ) -> Result<OpenConnection, IpcError> {
        let inner = &self.inner;
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

    /// Runs the text the user wrote, as written, in the language the session
    /// speaks ([`oxyn_query::console_language`]).
    pub async fn execute(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        sql: String,
    ) -> Result<CommandOutcome, IpcError> {
        let config = self.read_config(connection).await?;
        let capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("This session is no longer open"))?
            .capabilities();
        let language = oxyn_query::console_language(&config.driver, capabilities)
            .ok_or_else(|| IpcError::invalid(consoles::NO_CONSOLE_LANGUAGE))?;
        let mut request = ExecRequest::new(language, sql);
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
        let decided = self.decide_held(command, approved).await;
        // Approved, rejected or expired: a review session held for this
        // command is no longer needed (ADR-0042). A busy host dialog leaves
        // the command pending, and its session with it.
        let pending = self.inner.executor.approvals().peek(command).is_some();
        if !pending {
            self.release_review(command).await;
        }
        decided
    }

    async fn decide_held(
        &self,
        command: CommandId,
        approved: bool,
    ) -> Result<CommandOutcome, IpcError> {
        let inner = &self.inner;
        let reject = |reason: &str| {
            let answer = inner.ai.decisions.answer(command);
            inner.executor.reject(command);
            answer.report(DispatchReport::Denied {
                command,
                reason: "the user rejected this statement".to_owned(),
            });
            Ok(CommandOutcome::Denied {
                reason: reason.to_owned(),
            })
        };
        if !approved {
            return reject("Operation rejected");
        }
        // A connection's creation, edit or deletion completes on its own path:
        // approved here, its secrets and the policy would never follow it.
        if inner.pending_connections.lock().contains_key(&command)
            || inner.settings.pending_changes.lock().contains_key(&command)
        {
            return Err(IpcError::invalid(
                "This decision belongs to the connection screen",
            ));
        }
        // Tracked before the host is asked and before the waiting agent's
        // answer is taken: no other command may take this id while the dialog
        // is open, and refused, the decision stays pending and the agent still
        // waits on it. A busy dialog leaves it untouched too.
        let cancel = self.track(command)?;
        if self.confirm_held(command).await? == HostAnswer::Refused {
            return reject("Not confirmed in the Oxyn dialog: nothing was run");
        }
        // An approved console statement runs from here: the exit lists its
        // session as `Unknown` until it answers (ADR-0043).
        let _running = match inner.executor.approvals().peek(command) {
            Some(PendingCommand {
                command: Command::Execute { session, .. },
                ..
            }) => Some(inner.workbench.consoles.run_on(session)?),
            _ => None,
        };
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

    /// The language this connection's consoles speak, from what its driver
    /// declares at best; SQL in the driver's dialect for a driver that names
    /// none. A session narrows its capabilities, never to another language.
    pub(crate) fn language_of(&self, driver: &DriverId) -> oxyn_core::QueryLanguage {
        self.inner
            .drivers
            .get(driver)
            .and_then(|known| oxyn_query::console_language(driver, known.capabilities()))
            .unwrap_or_else(|| oxyn_core::QueryLanguage::Sql(oxyn_query::dialect_for(driver)))
    }
}

/// The configuration a draft describes, secrets aside, under a fresh id.
/// The drivers Oxyn ships.
fn shipped_drivers() -> Result<DriverRegistry> {
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .context("registering the SQLite driver")?;
    drivers
        .register(Arc::new(PostgresDriver::new()))
        .context("registering the PostgreSQL driver")?;
    drivers
        .register(Arc::new(MysqlDriver::new()))
        .context("registering the MySQL driver")?;
    drivers
        .register(Arc::new(ElasticsearchDriver::new()))
        .context("registering the Elasticsearch driver")?;
    drivers
        .register(Arc::new(DuckDbDriver::new()))
        .context("registering the DuckDB driver")?;
    drivers
        .register(Arc::new(ClickhouseDriver::new()))
        .context("registering the ClickHouse driver")?;
    drivers
        .register(Arc::new(RedisDriver::new()))
        .context("registering the Redis driver")?;
    drivers
        .register(Arc::new(MongoDriver::new()))
        .context("registering the MongoDB driver")?;
    Ok(drivers)
}

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
            row_limit_unverified: matches!(sink, SinkOutcome::RowLimitUnverified),
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
mod tests;

mod ai;
mod completion_load;
pub(crate) mod confirm;
mod consoles;
mod documents;
mod exit;
mod library;
mod metadata;
mod object_operations;
mod proposal;
mod recovery;
mod results;
mod semantic;
mod settings;
mod tracking;
pub(crate) mod windows;
