//! Everything that is not pixels: state, drivers, policy, executor.
//!
//! Lives apart from the views for one reason — the UI thread must never touch
//! any of it directly. A view emits an event, `workspace` turns that event into
//! a [`oxyn_core::Command`], and the command is dispatched **here**, on
//! the Tokio runtime ([I-05](../../../CLAUDE.md#i-05)).

use std::sync::Arc;

use anyhow::{Context as _, Result};
use oxyn_core::{
    Actor, CancelToken, Command, ConnectionConfig, ConnectionId, DefaultPolicy, DriverId,
    Environment, PolicyGate, ResultId,
};
use oxyn_data::ResultBuffer;
use oxyn_driver::DriverRegistry;
use oxyn_driver_postgres::PostgresDriver;
use oxyn_driver_sqlite::SqliteDriver;
use oxyn_exec::{ExecEvent, Executor, Outcome};
use oxyn_store::Store;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;

/// The name of the scratch connection opened at startup.
///
/// A workspace with no connection can do nothing at all, and phase 0 has no
/// connection picker yet. An in-memory SQLite base is the smallest thing that
/// makes the editor answer — and it is marked `Local`, so nothing about it can
/// be mistaken for a real system ([I-02](../../../CLAUDE.md#i-02)).
const BAC_A_SABLE: &str = "scratch (in-memory SQLite)";

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
    /// The scratch connection, ready before the window opens.
    scratch: ConnectionId,
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

        let policy = Arc::new(DefaultPolicy::new());
        let gate: Arc<dyn PolicyGate> = policy.clone();

        let executor = Executor::builder(Arc::clone(&store), gate)
            .with_drivers(Arc::new(drivers))
            .build();

        // Connections saved in a previous session are unknown to the policy
        // until they are registered. `DefaultPolicy` is closed by default, so
        // skipping this would deny every write with a message nobody could act
        // on ([ADR-0004](../../../docs/adr/0004-command-bus.md)).
        let known = executor
            .load_connections()
            .context("loading the saved connections")?;
        tracing::info!(connections = known, "saved connections registered");

        let scratch = ouvrir_le_bac_a_sable(&store, &policy, &executor)
            .context("preparing the scratch connection")?;

        Ok(Self {
            inner: Arc::new(Inner {
                executor,
                scratch,
                runtime,
            }),
        })
    }

    /// The connection the editor runs against until a picker exists.
    pub fn scratch(&self) -> ConnectionId {
        self.inner.scratch
    }

    /// The buffer behind a result id, shared with the grid **without a copy**.
    pub fn result(&self, result: ResultId) -> Option<Arc<ResultBuffer>> {
        self.inner.executor.result(result)
    }

    /// A receiver on the execution event stream.
    ///
    /// One per subscriber: `broadcast` gives every subscriber every event, so a
    /// view that starts late misses what came before but never steals it from
    /// another view.
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
        F: FnOnce(Result<Outcome, oxyn_core::OxynError>) + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        self.inner.runtime.spawn(async move {
            let issue = inner.executor.dispatch(actor, command, &cancel).await;
            then(issue);
        });
    }
}

/// Creates — or finds again — the scratch connection, and opens the gate on it.
///
/// Registering it with the policy is not optional: `DefaultPolicy` is closed by
/// default, so an unregistered connection has **every** mutating command denied,
/// with a message the user cannot act on
/// ([ADR-0004](../../../docs/adr/0004-command-bus.md)).
fn ouvrir_le_bac_a_sable(
    store: &Arc<Store>,
    policy: &Arc<DefaultPolicy>,
    executor: &Executor,
) -> Result<ConnectionId> {
    let ateliers = store.workspaces().list().context("listing workspaces")?;
    let atelier = match ateliers.into_iter().next() {
        Some(existant) => existant,
        None => store
            .workspaces()
            .create("oxyn")
            .context("creating the first workspace")?,
    };

    let connexions = store
        .connections()
        .list(atelier.id)
        .context("listing the workspace connections")?;

    let config = match connexions.into_iter().find(|c| c.name == BAC_A_SABLE) {
        Some(deja_la) => deja_la,
        None => {
            let neuve = ConnectionConfig::new(BAC_A_SABLE, DriverId::sqlite())
                .with_environment(Environment::Local)
                .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
            store
                .connections()
                .save(atelier.id, &neuve)
                .context("saving the scratch connection")?;
            neuve
        }
    };

    policy.register(&config);
    executor.register_connection(&config);
    tracing::info!(connection = %config.name, "scratch connection ready");

    Ok(config.id)
}
