//! Everything that is not pixels: state, drivers, policy, executor.
//!
//! Lives apart from the views for one reason — the UI thread must never touch
//! any of it directly. A view emits an event, `workspace` turns that event into
//! a [`oxyn_core::Command`], and the command is dispatched **here**, on
//! the Tokio runtime ([I-05](../../../CLAUDE.md#i-05)).
//!
//! # No connection is opened for you
//!
//! Startup opens the local state and the driver registry, and stops there. Which
//! database to open is the user's decision, taken on the connection screen — an
//! imposed scratch base would answer a question nobody asked, and would put a
//! connection in the status bar that the user never chose
//! ([UX-SPEC](../../../docs/UX-SPEC.md)).

use std::sync::Arc;

use anyhow::{Context as _, Result};
use oxyn_core::{
    Actor, CancelToken, Capabilities, Command, CommandId, ConnectionConfig, ConnectionId,
    DefaultPolicy, Environment, OxynError, PolicyGate, ResultId, SessionId, SqlDialect,
};
use oxyn_data::ResultBuffer;
use oxyn_driver::DriverRegistry;
use oxyn_driver_postgres::PostgresDriver;
use oxyn_driver_sqlite::SqliteDriver;
use oxyn_exec::{ExecEvent, Executor, Outcome};
use oxyn_secrets::{KeyringSecretStore, SecretStore};
use oxyn_store::Store;
use oxyn_ui::connection_form::{ConnectionDraft, DriverChoice, SavedConnection};
use tokio::runtime::Runtime;
use tokio::sync::{broadcast, oneshot};

use crate::credentials::KeyringCredentials;
use crate::picker;

/// What the status bar is allowed to say about a connection.
///
/// Deliberately **not** the [`ConnectionConfig`]: that one carries parameter
/// values and a secret reference, and it also carries the
/// [`ConnectionId`] — none of which belongs on screen
/// ([I-03](../../../CLAUDE.md#i-03)). Narrowing the type here is what makes the
/// rule hold: the view cannot display what it was never handed.
#[derive(Debug, Clone)]
pub struct ConnectionDisplay {
    /// The name the user gave the connection.
    pub name: String,
    /// The protocol, as the driver names it.
    pub driver: String,
    /// How the connection is marked.
    pub environment: Environment,
    /// Whether writes are refused on it.
    pub read_only: bool,
}

impl ConnectionDisplay {
    /// What may be shown about a configuration.
    fn of(config: &ConnectionConfig) -> Self {
        Self {
            name: config.name.clone(),
            driver: config.driver.to_string(),
            environment: config.environment,
            read_only: config.read_only,
        }
    }
}

/// A connection that is open, with a session ready.
#[derive(Debug, Clone)]
pub struct OpenConnection {
    /// A workspace reserves its own query session beside the catalog/preview session.
    /// Console-only opens leave this empty.
    pub initial_console: Option<Box<OpenConnection>>,
    /// The session actually opened by the executor.
    pub session: SessionId,
    /// The connection the workspace will run against.
    pub connection: ConnectionId,
    /// What the status bar shows about it.
    pub display: ConnectionDisplay,
    /// Capabilities negotiated by the actual open session.
    pub capabilities: Capabilities,
    /// The SQL dialect of the connected driver.
    ///
    /// Resolved from the driver identifier once, here, rather than defaulting
    /// to `Ansi` at every call site: the dialect decides how a statement is
    /// read, and reading `EXPLAIN (ANALYZE) DELETE …` as ANSI loses exactly the
    /// distinction [I-07](../../../CLAUDE.md#i-07) depends on.
    pub dialect: SqlDialect,
    /// Executor-owned cache, acquired off the UI thread during connection.
    pub catalog: oxyn_catalog::SharedCatalog,
}

/// A connection may require approval before its local configuration is saved.
#[derive(Debug)]
pub enum ConnectionResponse {
    Open(OpenConnection),
    Approval {
        command: CommandId,
        config: Box<ConnectionConfig>,
        reason: String,
        preview: Option<oxyn_core::Preview>,
    },
}

/// The assembled backend, shared by every view.
///
/// Views and application lifecycle callbacks retain the runtime. Worker tasks
/// retain only Inner, so the runtime cannot be dropped on its own worker thread.
#[derive(Clone)]
pub struct Backend {
    inner: Arc<Inner>,
    // Tasks retain Inner, never their own runtime; its final drop must stay on the owner.
    runtime: Arc<Runtime>,
}

struct Inner {
    executor: Executor,
    /// Held for the connection screen: the registry is the only authority on
    /// which database types exist ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    drivers: Arc<DriverRegistry>,
    /// Held because `DefaultPolicy` is closed by default: a connection created
    /// after startup must be registered, or every write on it is denied.
    policy: Arc<DefaultPolicy>,
    /// The one place that reads or writes the keyring.
    credentials: Arc<KeyringCredentials>,
    saved: Vec<(ConnectionId, SavedConnection)>,
    preferences: oxyn_core::PreferencesSnapshot,
    pending_local_writes: Arc<PendingLocalWrites>,
}

#[derive(Default)]
struct PendingLocalWrites {
    count: std::sync::atomic::AtomicUsize,
    changed: tokio::sync::Notify,
}

struct LocalWriteGuard(Arc<PendingLocalWrites>);
impl Drop for LocalWriteGuard {
    fn drop(&mut self) {
        self.0
            .count
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

// `Debug` is derived nowhere here on purpose: `Executor` and `Store` reach
// connection configurations, and a `{cfg:?}` added six months from now is the
// documented way secrets leak ([I-03](../../../CLAUDE.md#i-03)).
impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

impl Backend {
    /// Opens the local state and assembles the executor.
    ///
    /// Blocking, and deliberately so: it runs once, before the window exists.
    /// Every failure here is a startup failure the user must see as text rather
    /// than as a window that opens onto nothing.
    ///
    /// # Errors
    /// Local state unreadable, driver registry inconsistent, keyring absent.
    pub fn open() -> Result<Self> {
        KeyringSecretStore::availability().context("the system keyring is unavailable")?;
        Self::assemble(
            Arc::new(Store::open_default().context("opening the local workspace state")?),
            Arc::new(KeyringSecretStore::new()),
            oxyn_data::DEFAULT_MEMORY_BUDGET,
        )
    }

    /// Explicit ephemeral workspace for local QA; never touches saved state or keyring.
    /// Database connections remain the user's explicit choice.
    pub fn open_temporary() -> Result<Self> {
        Self::assemble(
            Arc::new(Store::open_in_memory().context("opening temporary workspace state")?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
            oxyn_data::DEFAULT_MEMORY_BUDGET,
        )
    }

    #[cfg(test)]
    pub(crate) fn temporary_with_memory_budget(bytes: usize) -> Result<Self> {
        Self::assemble(
            Arc::new(Store::open_in_memory()?),
            Arc::new(oxyn_secrets::MemorySecretStore::new()),
            bytes,
        )
    }

    fn assemble(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        memory_budget: usize,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("oxyn-exec")
            .build()
            .context("starting the async runtime")?;

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

        // The workspace is chosen **before** the executor is built. Left to its
        // default, `ExecutorBuilder` mints a fresh `WorkspaceId`, and every
        // connection would be written to — and listed from — a workspace that
        // exists nowhere on disk: saving would appear to work and nothing would
        // ever come back.
        let atelier = atelier_courant(&store)?;
        let preferences = store
            .preferences()
            .load(atelier)
            .context("loading workspace preferences")?;

        let executor = Executor::builder(Arc::clone(&store), gate)
            .with_drivers(Arc::clone(&drivers))
            .with_credentials(Arc::clone(&credentials) as Arc<_>)
            .with_workspace(atelier)
            .with_memory_budget(memory_budget)
            .build();

        // Connections saved in a previous session are unknown to the policy
        // until they are registered. `DefaultPolicy` is closed by default, so
        // skipping this would deny every write with a message nobody could act
        // on ([ADR-0004](../../../docs/adr/0004-command-bus.md)).
        let known = executor
            .load_connections()
            .context("loading the saved connections")?;
        let configs = store
            .connections()
            .list(atelier)
            .context("listing the saved connections")?;
        let saved = configs
            .iter()
            .map(|config| (config.id, picker::saved_connection(config)))
            .collect();
        for config in configs {
            policy.register(&config);
        }
        tracing::info!(connections = known, "saved connections registered");

        let backend = Self {
            inner: Arc::new(Inner {
                executor,
                drivers,
                policy,
                credentials,
                saved,
                preferences,
                pending_local_writes: Arc::new(PendingLocalWrites::default()),
            }),
            runtime: Arc::new(runtime),
        };
        let weak = Arc::downgrade(&backend.inner);
        backend.runtime.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                let _ = tokio::task::spawn_blocking(move || inner.executor.prune_results()).await;
            }
        });
        Ok(backend)
    }

    /// Snapshot read at startup, before the UI exists; no I/O is performed here.
    pub fn initial_preferences(&self) -> oxyn_core::PreferencesSnapshot {
        self.inner.preferences.clone()
    }

    /// Workspace owning local documents and preferences.
    pub fn workspace_id(&self) -> oxyn_core::WorkspaceId {
        self.inner.executor.workspace()
    }

    /// Waits for already submitted preference and document writes, even after their views close.
    pub async fn wait_for_local_writes(&self) {
        loop {
            let mut notified = std::pin::pin!(self.inner.pending_local_writes.changed.notified());
            notified.as_mut().enable();
            if self
                .inner
                .pending_local_writes
                .count
                .load(std::sync::atomic::Ordering::SeqCst)
                == 0
            {
                return;
            }
            notified.await;
        }
    }

    /// The database types the connection screen may offer.
    ///
    /// Read from the registry and nowhere else: a list written by hand would
    /// keep offering a driver after it was removed, and the failure would happen
    /// at connect time with an unhelpful message
    /// ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    #[must_use]
    pub fn driver_choices(&self) -> Vec<DriverChoice> {
        self.inner
            .drivers
            .sorted()
            .iter()
            .map(|driver| picker::driver_choice(driver.metadata()))
            .collect()
    }

    /// The connections already saved, in the order the screen lists them.
    ///
    /// # Errors
    /// If the local state cannot be read.
    pub fn saved_connections(&self) -> Result<Vec<(ConnectionId, SavedConnection)>> {
        Ok(self.inner.saved.clone())
    }

    /// Creates the connection a draft describes and opens a session on it.
    ///
    /// Returns immediately; the outcome arrives on the channel. Everything
    /// costly — the keyring write, the command bus, the server handshake —
    /// happens on the runtime, never on the UI thread
    /// ([I-05](../../../CLAUDE.md#i-05)).
    #[must_use]
    pub fn connect(
        &self,
        draft: ConnectionDraft,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<ConnectionResponse>> {
        let (envoi, reception) = oneshot::channel();
        let inner = Arc::clone(&self.inner);

        self.runtime.spawn(async move {
            let issue = ouvrir(&inner, draft, &cancel).await;
            // The receiver is gone when the window closed mid-connect. Nothing
            // to report to, and nothing broken: the session is closed with the
            // backend.
            if let Err(Ok(response)) = envoi.send(issue) {
                release_connection_response(&inner, response).await;
            }
        });

        reception
    }

    /// Opens a session on a connection that already exists.
    ///
    /// # Panics
    /// Never: an unknown connection comes back as an error on the channel.
    #[must_use]
    pub fn reconnect(
        &self,
        connection: ConnectionId,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<ConnectionResponse>> {
        let (envoi, reception) = oneshot::channel();
        let inner = Arc::clone(&self.inner);

        self.runtime.spawn(async move {
            let issue = rouvrir(&inner, connection, &cancel, true)
                .await
                .map(ConnectionResponse::Open);
            if let Err(Ok(response)) = envoi.send(issue) {
                release_connection_response(&inner, response).await;
            }
        });

        reception
    }

    /// Opens only the independent session owned by a new console.
    #[must_use]
    pub fn reconnect_console(
        &self,
        connection: ConnectionId,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<ConnectionResponse>> {
        let (sender, receiver) = oneshot::channel();
        let inner = self.inner.clone();
        self.runtime.spawn(async move {
            let outcome = rouvrir(&inner, connection, &cancel, false)
                .await
                .map(ConnectionResponse::Open);
            if let Err(Ok(response)) = sender.send(outcome) {
                release_connection_response(&inner, response).await;
            }
        });
        receiver
    }

    /// Completes an explicitly approved connection setup, without bypassing the gate.
    pub fn approve_connection(
        &self,
        command: CommandId,
        config: ConnectionConfig,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<ConnectionResponse>> {
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let result = async {
                match inner.executor.approve("human", command, &cancel).await? {
                    Outcome::ConnectionSaved { .. } => {}
                    Outcome::Denied { reason, .. } => anyhow::bail!("{reason}"),
                    _ => anyhow::bail!("The connection was not saved"),
                }
                inner.policy.register(&config);
                ouvrir_la_session(&inner, config, &cancel)
                    .await
                    .map(ConnectionResponse::Open)
            }
            .await;
            if let Err(Ok(response)) = sender.send(result) {
                release_connection_response(&inner, response).await;
            }
        });
        receiver
    }

    /// Releases an opening that its requesting view no longer needs.
    pub fn release_open_connection(&self, open: OpenConnection) {
        if let Some(console) = open.initial_console {
            drop(self.dispatch(
                CommandId::new(),
                Command::CloseSession {
                    connection: console.connection,
                    session: console.session,
                },
                CancelToken::new(),
            ));
        }
        drop(self.dispatch(
            CommandId::new(),
            Command::CloseSession {
                connection: open.connection,
                session: open.session,
            },
            CancelToken::new(),
        ));
    }

    /// The buffer behind a result id, shared with the grid **without a copy**.
    #[must_use]
    pub fn result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.inner.executor.result(result)
    }

    /// A receiver on the execution event stream.
    ///
    /// One per subscriber: `broadcast` gives every subscriber every event, so a
    /// view that starts late misses what came before but never steals it from
    /// another view.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ExecEvent> {
        self.inner.executor.subscribe()
    }

    /// Dispatches a command and hands the outcome to `then`, off the UI thread.
    ///
    /// The whole point of this function is that it **returns immediately**. The
    /// dispatch happens on the runtime; `then` runs there too, so it must not
    /// touch a view — it sends, and the UI reads what it sent
    /// ([I-05](../../../CLAUDE.md#i-05)).
    pub fn dispatch(
        &self,
        id: CommandId,
        command: Command,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<Outcome, OxynError>> {
        let pending = if matches!(
            command,
            Command::WriteWorkspacePreferences { .. }
                | Command::SaveQueryDocument { .. }
                | Command::CloseQueryDocument {
                    expected_revision: None,
                    ..
                }
                | Command::DeleteQueryDocument { .. }
                | Command::WriteDocument { .. }
        ) {
            self.inner
                .pending_local_writes
                .count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(LocalWriteGuard(self.inner.pending_local_writes.clone()))
        } else {
            None
        };
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let outcome = inner
                .executor
                .dispatch_as(id, Actor::Human, command, &cancel)
                .await;
            drop(pending);
            let _ = sender.send(outcome);
        });
        receiver
    }

    /// Resolves a human decision against the exact pending command.
    pub fn decide(
        &self,
        id: CommandId,
        approved: bool,
        cancel: CancelToken,
    ) -> oneshot::Receiver<Result<Outcome, OxynError>> {
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let outcome = if approved {
                inner.executor.approve("human", id, &cancel).await
            } else {
                inner.executor.reject(id);
                Ok(Outcome::Denied {
                    command: id,
                    reason: "Operation rejected".into(),
                })
            };
            let _ = sender.send(outcome);
        });
        receiver
    }
}

/// Releases an opening or approval that could not be delivered to its UI receiver.
async fn release_connection_response(inner: &Inner, response: ConnectionResponse) {
    match response {
        ConnectionResponse::Open(open) => {
            let mut sessions = vec![(open.connection, open.session)];
            if let Some(console) = open.initial_console {
                sessions.push((console.connection, console.session));
            }
            for (connection, session) in sessions {
                if let Err(error) = inner
                    .executor
                    .dispatch(
                        Actor::Human,
                        Command::CloseSession {
                            connection,
                            session,
                        },
                        &CancelToken::new(),
                    )
                    .await
                {
                    tracing::warn!(error = %error, "unused session could not be closed cleanly");
                }
            }
        }
        ConnectionResponse::Approval { command, .. } => {
            inner.executor.reject(command);
        }
    }
}

/// The workspace the session works in — the first one, or a new one.
///
/// Oxyn has no workspace switcher yet; when it does, the choice moves to the UI
/// and this function becomes its default.
fn atelier_courant(store: &Arc<Store>) -> Result<oxyn_core::WorkspaceId> {
    let ateliers = store.workspaces().list().context("listing workspaces")?;
    match ateliers.into_iter().next() {
        Some(existant) => Ok(existant.id),
        None => Ok(store
            .workspaces()
            .create("oxyn")
            .context("creating the first workspace")?
            .id),
    }
}

/// Writes the secrets, saves the connection, opens the session.
///
/// In that order, and it matters: the secret reference is part of the
/// configuration, so the keyring write has to happen before the configuration is
/// persisted. A failure after the keyring write leaves an orphan entry, which is
/// harmless — an unreferenced secret is unreachable — where the reverse would
/// leave a saved connection whose password does not exist.
async fn ouvrir(
    inner: &Inner,
    draft: ConnectionDraft,
    cancel: &CancelToken,
) -> Result<ConnectionResponse> {
    let mut config = picker::config_from_draft(&draft)
        .context("the connection screen named a driver the registry does not know")?;

    let secrets = picker::secrets_of(&draft);
    if !secrets.is_empty() {
        let reference = inner
            .credentials
            .store_secrets(&config, secrets)
            .context("writing the connection secrets to the keyring")?;
        config = config.with_secret_ref(reference.as_str());
    }

    // Through the command bus, like everything else — including at startup.
    // Writing to the store directly here is exactly the second execution path
    // [I-01](../../../CLAUDE.md#i-01) forbids, and it is the tempting shortcut.
    let outcome = inner
        .executor
        .dispatch(
            Actor::Human,
            Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            cancel,
        )
        .await
        .context("saving the connection")?;

    match outcome {
        Outcome::NeedsApproval {
            command,
            reason,
            mut preview,
        } => {
            if let Some(preview) = &mut preview {
                preview.connection = config.name.clone();
            }
            return Ok(ConnectionResponse::Approval {
                command,
                config: Box::new(config),
                reason,
                preview,
            });
        }
        Outcome::ConnectionSaved { .. } => {}
        Outcome::Denied { reason, .. } => anyhow::bail!("{reason}"),
        _ => anyhow::bail!("The connection was not saved"),
    }

    // The gate learns about the connection here. `DefaultPolicy` is closed by
    // default: skipping this denies every write on a connection the user just
    // created, with a message they cannot act on.
    inner.policy.register(&config);

    ouvrir_la_session(inner, config, cancel)
        .await
        .map(ConnectionResponse::Open)
}

/// Reopens a connection the store already knows.
async fn rouvrir(
    inner: &Inner,
    connection: ConnectionId,
    cancel: &CancelToken,
    workspace: bool,
) -> Result<OpenConnection> {
    let config = inner
        .executor
        .store()
        .connections()
        .get(connection)
        .context("reading the saved connection")?
        .context("this connection is no longer in the workspace")?;

    inner.policy.register(&config);
    if workspace {
        ouvrir_la_session(inner, config, cancel).await
    } else {
        ouvrir_une_session(inner, config, cancel).await
    }
}

/// Keep catalog/preview work out of the first console's transaction as well.
async fn ouvrir_la_session(
    inner: &Inner,
    config: ConnectionConfig,
    cancel: &CancelToken,
) -> Result<OpenConnection> {
    let mut workspace = ouvrir_une_session(inner, config.clone(), cancel).await?;
    match ouvrir_une_session(inner, config, cancel).await {
        Ok(console) => {
            workspace.initial_console = Some(Box::new(console));
            Ok(workspace)
        }
        Err(error) => {
            let _ = inner
                .executor
                .dispatch(
                    Actor::Human,
                    Command::CloseSession {
                        connection: workspace.connection,
                        session: workspace.session,
                    },
                    &CancelToken::new(),
                )
                .await;
            Err(error)
        }
    }
}

/// The half both paths share: `Command::Connect`, then what may be displayed.
async fn ouvrir_une_session(
    inner: &Inner,
    config: ConnectionConfig,
    cancel: &CancelToken,
) -> Result<OpenConnection> {
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
        // The server's own words, code included: the audience for this product
        // reads PostgreSQL errors, and a reassuring paraphrase would remove the
        // one thing that tells them what to fix.
        .with_context(|| format!("opening a session on « {} »", config.name))?;

    tracing::info!(
        connection = %config.name,
        driver = %config.driver,
        environment = %config.environment,
        "connection open"
    );

    let Outcome::Connected { session, .. } = outcome else {
        anyhow::bail!("The executor did not open a session");
    };
    let capabilities = inner
        .executor
        .sessions()
        .get(session)
        .context("The opened session is no longer registered")?
        .capabilities();
    let catalog = inner
        .executor
        .catalog(config.id)
        .context("The opened connection has no catalog cache")?;
    let mut display = ConnectionDisplay::of(&config);
    display.read_only |= capabilities.contains(Capabilities::READ_ONLY_SESSION);
    Ok(OpenConnection {
        initial_console: None,
        catalog,
        capabilities,
        dialect: oxyn_query::dialect_for(&config.driver),
        session,
        connection: config.id,
        display,
    })
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;

    fn test_backend() -> Backend {
        let store = Arc::new(Store::open_in_memory().expect("in-memory workspace"));
        let workspace = atelier_courant(&store).expect("workspace");
        let mut drivers = DriverRegistry::new();
        drivers
            .register(Arc::new(SqliteDriver::new()))
            .expect("driver");
        let drivers = Arc::new(drivers);
        let policy = Arc::new(DefaultPolicy::new());
        let credentials = Arc::new(KeyringCredentials::new(Arc::new(
            oxyn_secrets::MemorySecretStore::new(),
        )));
        let executor = Executor::builder(store, policy.clone())
            .with_workspace(workspace)
            .with_drivers(drivers.clone())
            .with_credentials(credentials.clone())
            .build();
        Backend {
            inner: Arc::new(Inner {
                executor,
                drivers,
                policy,
                credentials,
                saved: Vec::new(),
                preferences: oxyn_core::PreferencesSnapshot::default(),
                pending_local_writes: Arc::new(PendingLocalWrites::default()),
            }),
            runtime: Arc::new(
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("runtime"),
            ),
        }
    }

    fn open_test_connection(backend: &Backend, environment: Environment) -> OpenConnection {
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: "UI regression".into(),
            environment,
            values: [("path".into(), ":memory:".into())].into_iter().collect(),
            secrets: Default::default(),
        };
        match backend
            .runtime
            .block_on(backend.connect(draft, CancelToken::new()))
            .expect("response")
            .expect("connection")
        {
            ConnectionResponse::Open(open) => open,
            ConnectionResponse::Approval {
                command, config, ..
            } => {
                let result = backend
                    .runtime
                    .block_on(backend.approve_connection(command, *config, CancelToken::new()))
                    .expect("response")
                    .expect("approval");
                let ConnectionResponse::Open(open) = result else {
                    panic!("connection must open after approval")
                };
                open
            }
        }
    }

    fn run(backend: &Backend, open: &OpenConnection, text: &str) -> Result<Outcome, OxynError> {
        let command = crate::workspace::execution_command(
            open.connection,
            open.session,
            false,
            open.dialect,
            text.to_owned(),
            Vec::new(),
        );
        backend
            .runtime
            .block_on(backend.dispatch(CommandId::new(), command, CancelToken::new()))
            .expect("response")
    }

    #[test]
    fn ui_reuses_the_open_session_and_survives_a_query_error() {
        let backend = test_backend();
        let open = open_test_connection(&backend, Environment::Local);
        assert!(
            matches!(run(&backend, &open, "SELECT 42 AS answer"), Ok(Outcome::Executed { stats, .. }) if stats.rows == 1)
        );
        assert!(run(&backend, &open, "SELECT missing FROM nonexistent").is_err());
        assert!(matches!(
            run(&backend, &open, "SELECT 1"),
            Ok(Outcome::Executed { .. })
        ));
        assert!(matches!(
            run(&backend, &open, "CREATE TABLE notes (id INTEGER)"),
            Ok(Outcome::Executed { .. })
        ));
        assert!(
            matches!(run(&backend, &open, "SELECT * FROM notes"), Ok(Outcome::Executed { stats, .. }) if stats.rows == 0)
        );
    }

    #[test]
    fn production_write_waits_for_the_exact_human_decision() {
        let backend = test_backend();
        let open = open_test_connection(&backend, Environment::Production);
        let Outcome::NeedsApproval {
            command, preview, ..
        } = run(&backend, &open, "CREATE TABLE approved (id INTEGER)").expect("policy response")
        else {
            panic!("approval required")
        };
        assert_eq!(preview.expect("preview").connection, "UI regression");
        assert!(run(&backend, &open, "SELECT * FROM approved").is_err());
        let outcome = backend
            .runtime
            .block_on(backend.decide(command, true, CancelToken::new()))
            .expect("response")
            .expect("approval");
        assert!(matches!(outcome, Outcome::Executed { .. }));
        assert!(run(&backend, &open, "SELECT * FROM approved").is_ok());
        let Outcome::NeedsApproval { command, .. } =
            run(&backend, &open, "DROP TABLE approved").expect("policy response")
        else {
            panic!("approval required")
        };
        let rejected = backend
            .runtime
            .block_on(backend.decide(command, false, CancelToken::new()))
            .expect("response")
            .expect("rejection");
        assert!(matches!(rejected, Outcome::Denied { .. }));
        assert!(run(&backend, &open, "SELECT * FROM approved").is_ok());
    }

    #[test]
    fn ui_cancel_reaches_sqlite_and_session_can_be_reused() {
        let backend = test_backend();
        let open = open_test_connection(&backend, Environment::Local);
        let cancel = CancelToken::new();
        let mut events = backend.subscribe();
        let id = CommandId::new();
        let command = crate::workspace::execution_command(open.connection, open.session, false, open.dialect,
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT x FROM n".into(), Vec::new());
        let receiver = backend.dispatch(id, command, cancel.clone());
        backend.runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let event = events.recv().await.expect("event");
                    if event.command == id
                        && matches!(event.event, oxyn_core::Event::BatchReady { .. })
                    {
                        break;
                    }
                }
            })
            .await
            .expect("first batch");
            cancel.cancel();
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), receiver)
                .await
                .expect("cancellation deadline")
                .expect("response");
            assert!(matches!(
                outcome,
                Ok(Outcome::Executed {
                    sink: oxyn_data::SinkOutcome::Cancelled,
                    ..
                }) | Err(OxynError::Cancelled)
            ));
        });
        assert!(run(&backend, &open, "SELECT 1").is_ok());
    }

    #[test]
    fn laffichage_dune_connexion_ne_porte_ni_identifiant_ni_parametre() {
        // Ce que la barre d'état reçoit est tout ce qu'elle *peut* montrer :
        // la règle tient parce que le type est étroit, pas parce que la vue est
        // disciplinée ([I-03](../../../CLAUDE.md#i-03)).
        let config = ConnectionConfig::new("prod", DriverId::postgres())
            .with_environment(Environment::Production)
            .with_param("host", "db.interne")
            .with_secret_ref("connection:0123");

        let vue = ConnectionDisplay::of(&config);
        let rendu = format!("{vue:?}");

        assert!(!rendu.contains("db.interne"), "paramètre exposé : {rendu}");
        assert!(
            !rendu.contains("connection:0123"),
            "référence exposée : {rendu}"
        );
        assert!(
            !rendu.contains(&config.id.to_string()),
            "identifiant exposé : {rendu}"
        );
        assert_eq!(vue.environment, Environment::Production);
    }

    #[test]
    fn une_connexion_sans_environnement_reste_de_production() {
        // Jamais l'inverse : une connexion dont le marquage manque vaut
        // production, sinon un oubli ouvrirait les écritures
        // ([I-02](../../../CLAUDE.md#i-02)).
        let config = ConnectionConfig::new("sans marquage", DriverId::sqlite());
        assert_eq!(
            ConnectionDisplay::of(&config).environment,
            Environment::Production
        );
    }
}

#[cfg(test)]
mod preference_lifecycle_tests {
    use super::*;
    use oxyn_core::{PreferencesSnapshot, ReadingDensity};

    #[test]
    fn discarded_view_receivers_do_not_lose_the_latest_preference_write() {
        let backend = Backend::open_temporary().expect("backend");
        for revision in 1..=30 {
            let mut snapshot = PreferencesSnapshot {
                revision,
                ..Default::default()
            };
            snapshot.preferences.reading_density = if revision == 30 {
                ReadingDensity::Comfortable
            } else {
                ReadingDensity::Compact
            };
            drop(backend.dispatch(
                CommandId::new(),
                Command::WriteWorkspacePreferences {
                    workspace: backend.workspace_id(),
                    snapshot: Box::new(snapshot),
                },
                CancelToken::new(),
            ));
        }
        backend.runtime.block_on(backend.wait_for_local_writes());
        let outcome = backend
            .runtime
            .block_on(backend.dispatch(
                CommandId::new(),
                Command::ReadWorkspacePreferences {
                    workspace: backend.workspace_id(),
                },
                CancelToken::new(),
            ))
            .expect("response")
            .expect("read");
        let Outcome::WorkspacePreferences { snapshot } = outcome else {
            panic!("snapshot")
        };
        assert_eq!(snapshot.revision, 30);
        assert_eq!(
            snapshot.preferences.reading_density,
            ReadingDensity::Comfortable
        );
    }
}

#[cfg(test)]
mod console_tests;

mod documents;
pub(crate) use documents::DocumentWriter;
