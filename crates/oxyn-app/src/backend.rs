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
    Actor, CancelToken, Command, ConnectionConfig, ConnectionId, DefaultPolicy, Environment,
    OxynError, PolicyGate, ResultId,
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
    /// The connection the workspace will run against.
    pub connection: ConnectionId,
    /// What the status bar shows about it.
    pub display: ConnectionDisplay,
}

/// The assembled backend, shared by every view.
///
/// Cloneable by `Arc`: the views hold it, the runtime holds it, and neither owns
/// it. Dropping the last handle shuts the runtime down.
#[derive(Clone)]
pub struct Backend {
    inner: Arc<Inner>,
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
    /// Kept alive for as long as the backend: dropping it would abort every
    /// in-flight statement, and a dropped future does not cancel a server-side
    /// query ([I-13](../../../CLAUDE.md#i-13)).
    runtime: Runtime,
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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("oxyn-exec")
            .build()
            .context("starting the async runtime")?;

        let store = Arc::new(Store::open_default().context("opening the local workspace state")?);

        let mut drivers = DriverRegistry::new();
        drivers
            .register(Arc::new(SqliteDriver::new()))
            .context("registering the SQLite driver")?;
        drivers
            .register(Arc::new(PostgresDriver::new()))
            .context("registering the PostgreSQL driver")?;
        tracing::info!(drivers = drivers.len(), "driver registry ready");
        let drivers = Arc::new(drivers);

        // Refusing to start rather than falling back to a file: a silent
        // downgrade would put every password of every user on disk the first
        // time a platform backend misbehaved
        // ([SECURITY](../../../docs/SECURITY.md#secrets)).
        KeyringSecretStore::availability().context("the system keyring is unavailable")?;
        let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new());
        let credentials = Arc::new(KeyringCredentials::new(secrets));

        let policy = Arc::new(DefaultPolicy::new());
        let gate: Arc<dyn PolicyGate> = policy.clone();

        // The workspace is chosen **before** the executor is built. Left to its
        // default, `ExecutorBuilder` mints a fresh `WorkspaceId`, and every
        // connection would be written to — and listed from — a workspace that
        // exists nowhere on disk: saving would appear to work and nothing would
        // ever come back.
        let atelier = atelier_courant(&store)?;

        let executor = Executor::builder(Arc::clone(&store), gate)
            .with_drivers(Arc::clone(&drivers))
            .with_credentials(Arc::clone(&credentials) as Arc<_>)
            .with_workspace(atelier)
            .build();

        // Connections saved in a previous session are unknown to the policy
        // until they are registered. `DefaultPolicy` is closed by default, so
        // skipping this would deny every write with a message nobody could act
        // on ([ADR-0004](../../../docs/adr/0004-command-bus.md)).
        let known = executor
            .load_connections()
            .context("loading the saved connections")?;
        for config in store
            .connections()
            .list(atelier)
            .context("listing the saved connections")?
        {
            policy.register(&config);
        }
        tracing::info!(connections = known, "saved connections registered");

        Ok(Self {
            inner: Arc::new(Inner {
                executor,
                drivers,
                policy,
                credentials,
                runtime,
            }),
        })
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
        let configs = self
            .inner
            .executor
            .store()
            .connections()
            .list(self.inner.executor.workspace())
            .context("listing the saved connections")?;

        Ok(configs
            .iter()
            .map(|config| (config.id, picker::saved_connection(config)))
            .collect())
    }

    /// Creates the connection a draft describes and opens a session on it.
    ///
    /// Returns immediately; the outcome arrives on the channel. Everything
    /// costly — the keyring write, the command bus, the server handshake —
    /// happens on the runtime, never on the UI thread
    /// ([I-05](../../../CLAUDE.md#i-05)).
    #[must_use]
    pub fn connect(&self, draft: ConnectionDraft) -> oneshot::Receiver<Result<OpenConnection>> {
        let (envoi, reception) = oneshot::channel();
        let inner = Arc::clone(&self.inner);

        self.inner.runtime.spawn(async move {
            let issue = ouvrir(&inner, draft).await;
            // The receiver is gone when the window closed mid-connect. Nothing
            // to report to, and nothing broken: the session is closed with the
            // backend.
            let _ = envoi.send(issue);
        });

        reception
    }

    /// Opens a session on a connection that already exists.
    ///
    /// # Panics
    /// Never: an unknown connection comes back as an error on the channel.
    #[must_use]
    pub fn reconnect(&self, connection: ConnectionId) -> oneshot::Receiver<Result<OpenConnection>> {
        let (envoi, reception) = oneshot::channel();
        let inner = Arc::clone(&self.inner);

        self.inner.runtime.spawn(async move {
            let issue = rouvrir(&inner, connection).await;
            let _ = envoi.send(issue);
        });

        reception
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
    pub fn dispatch<F>(&self, actor: Actor, command: Command, cancel: CancelToken, then: F)
    where
        F: FnOnce(Result<Outcome, OxynError>) + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        self.inner.runtime.spawn(async move {
            let issue = inner.executor.dispatch(actor, command, &cancel).await;
            then(issue);
        });
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
async fn ouvrir(inner: &Inner, draft: ConnectionDraft) -> Result<OpenConnection> {
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
    let cancel = CancelToken::new();
    inner
        .executor
        .dispatch(
            Actor::Human,
            Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            &cancel,
        )
        .await
        .context("saving the connection")?;

    // The gate learns about the connection here. `DefaultPolicy` is closed by
    // default: skipping this denies every write on a connection the user just
    // created, with a message they cannot act on.
    inner.policy.register(&config);

    ouvrir_la_session(inner, config).await
}

/// Reopens a connection the store already knows.
async fn rouvrir(inner: &Inner, connection: ConnectionId) -> Result<OpenConnection> {
    let config = inner
        .executor
        .store()
        .connections()
        .get(connection)
        .context("reading the saved connection")?
        .context("this connection is no longer in the workspace")?;

    inner.policy.register(&config);
    ouvrir_la_session(inner, config).await
}

/// The half both paths share: `Command::Connect`, then what may be displayed.
async fn ouvrir_la_session(inner: &Inner, config: ConnectionConfig) -> Result<OpenConnection> {
    let cancel = CancelToken::new();
    inner
        .executor
        .dispatch(
            Actor::Human,
            Command::Connect {
                connection: config.id,
            },
            &cancel,
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

    Ok(OpenConnection {
        connection: config.id,
        display: ConnectionDisplay::of(&config),
    })
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;

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
