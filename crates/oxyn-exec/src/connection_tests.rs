//! Connection lifecycle handlers: `Store` access and credential resolution
//! move to the blocking pool, never inline on the dispatching worker (I-05).

use super::*;
use oxyn_core::{DefaultPolicy, DriverId};
use oxyn_driver_sqlite::SqliteDriver;

fn sqlite_config(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY)
}

#[tokio::test]
async fn create_connection_is_saved_and_present_in_the_store() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let policy = Arc::new(DefaultPolicy::new());
    let executor = Executor::builder(store.clone(), policy)
        .with_workspace(workspace)
        .build();
    let config = sqlite_config("primary");

    let outcome = executor
        .dispatch(
            Actor::Human,
            Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        )
        .await
        .expect("connection saved");
    assert!(matches!(outcome, Outcome::ConnectionSaved { connection } if connection == config.id));

    let stored = store
        .connections()
        .get(config.id)
        .expect("read back")
        .expect("present in the store");
    assert_eq!(stored.id, config.id);
}

#[tokio::test]
async fn connect_on_a_second_executor_resolves_from_the_store_without_the_cache() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let policy = Arc::new(DefaultPolicy::new());
    let config = sqlite_config("shared");
    policy.register(&config);

    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .expect("SQLite registration");
    let drivers = Arc::new(drivers);

    let writer = Executor::builder(store.clone(), policy.clone())
        .with_drivers(drivers.clone())
        .with_workspace(workspace)
        .build();
    let saved = writer
        .dispatch(
            Actor::Human,
            Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        )
        .await
        .expect("connection saved");
    assert!(matches!(saved, Outcome::ConnectionSaved { .. }));

    // A second executor over the *same* store, never told about this
    // connection: the cache miss must be resolved from the Store, and the
    // credential lookup, on the blocking pool.
    let reader = Executor::builder(store.clone(), policy)
        .with_drivers(drivers)
        .with_workspace(workspace)
        .build();
    let connected = reader
        .dispatch(
            Actor::Human,
            Command::Connect {
                connection: config.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("connect");
    assert!(matches!(connected, Outcome::Connected { connection, .. } if connection == config.id));
}

/// The cache gives the `PolicyGate` its environment (I-02): after concurrent
/// updates of one connection it must agree with the store, whatever order the
/// blocking tasks ran in. A cache written after the `.await`, outside the
/// blocking task, could end on one environment and the store on another.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_updates_leave_the_cache_agreeing_with_the_store() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let policy = Arc::new(DefaultPolicy::new());
    let executor = Arc::new(
        Executor::builder(store.clone(), policy)
            .with_workspace(workspace)
            .build(),
    );
    let config = sqlite_config("contended");
    store
        .connections()
        .save(workspace, &config)
        .expect("seed connection");
    executor.register_connection(&config);

    let updates = (0..64).map(|i| {
        let executor = Arc::clone(&executor);
        let environment = if i % 2 == 0 {
            Environment::Local
        } else {
            Environment::Development
        };
        let config = config.clone().with_environment(environment);
        tokio::spawn(async move {
            executor
                .dispatch(
                    Actor::Human,
                    Command::UpdateConnection {
                        config: Box::new(config),
                    },
                    &CancelToken::new(),
                )
                .await
        })
    });
    for update in futures::future::join_all(updates).await {
        assert!(matches!(
            update.expect("update task"),
            Ok(Outcome::ConnectionSaved { .. })
        ));
    }

    let stored = store
        .connections()
        .get(config.id)
        .expect("read back")
        .expect("present in the store");
    let probe = Command::Connect {
        connection: config.id,
    };
    assert_eq!(executor.environment_of(&probe), stored.environment);
}

#[tokio::test]
async fn delete_connection_reports_existed_and_removes_it_from_the_store() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let policy = Arc::new(DefaultPolicy::new());
    let executor = Executor::builder(store.clone(), policy)
        .with_workspace(workspace)
        .build();
    let config = sqlite_config("to-delete");
    store
        .connections()
        .save(workspace, &config)
        .expect("seed connection");
    executor.register_connection(&config);

    let outcome = executor
        .dispatch(
            Actor::Human,
            Command::DeleteConnection {
                connection: config.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("delete");
    assert!(matches!(
        outcome,
        Outcome::ConnectionDeleted { existed: true, .. }
    ));
    assert!(
        store
            .connections()
            .get(config.id)
            .expect("read back")
            .is_none()
    );
}

fn sqlite_executor(store: &Arc<Store>) -> Executor {
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .expect("SQLite registration");
    Executor::builder(Arc::clone(store), Arc::new(DefaultPolicy::new()))
        .with_drivers(Arc::new(drivers))
        .with_workspace(workspace)
        .build()
}

#[tokio::test]
async fn a_tested_configuration_opens_closes_and_leaves_nothing_behind() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let executor = sqlite_executor(&store);
    // Unknown to the executor and to the policy, marked production by default:
    // the human tries it all the same, since nothing is written.
    let config = ConnectionConfig::new("draft", DriverId::sqlite())
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);

    let outcome = executor
        .dispatch(
            Actor::Human,
            Command::TestConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        )
        .await
        .expect("the configuration opens");
    assert!(matches!(outcome, Outcome::ConnectionTested { connection } if connection == config.id));
    assert!(
        executor.sessions().is_empty(),
        "no session outlives the test"
    );
    assert!(
        store
            .connections()
            .get(config.id)
            .expect("read back")
            .is_none(),
        "a test saves nothing"
    );
}

/// Resolves a fixed password, standing in for a draft's keyring entry.
struct CanaryCredentials;

const CANARY: &str = "canary-password-8f2d";

impl CredentialResolver for CanaryCredentials {
    fn resolve(&self, _config: &ConnectionConfig) -> Result<oxyn_driver::Credentials> {
        Ok(oxyn_driver::Credentials::new().with_password(CANARY))
    }
}

#[tokio::test]
async fn a_failed_test_is_classified_and_quotes_no_secret() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let mut drivers = DriverRegistry::new();
    drivers
        .register(Arc::new(SqliteDriver::new()))
        .expect("SQLite registration");
    let executor = Executor::builder(Arc::clone(&store), Arc::new(DefaultPolicy::new()))
        .with_drivers(Arc::new(drivers))
        .with_credentials(Arc::new(CanaryCredentials))
        .with_workspace(workspace)
        .build();
    let config = ConnectionConfig::new("draft", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(
            SqliteDriver::PATH,
            "/nonexistent-oxyn-test-dir/nested/db.sqlite",
        )
        .with_secret_ref("keychain://oxyn/draft");

    let error = executor
        .dispatch(
            Actor::Human,
            Command::TestConnection {
                config: Box::new(config),
            },
            &CancelToken::new(),
        )
        .await
        .expect_err("a file under a missing directory does not open");
    // The class is the driver's, carried as data: SQLite reports a file it
    // cannot open as a connection failure, and the front reads `retryable`
    // from that class, never from the message.
    assert!(matches!(error, OxynError::Connection(_)), "{error}");
    assert_eq!(error.class(), oxyn_core::ErrorClass::Transient);
    let shown = error.to_string();
    assert!(!shown.contains(CANARY), "secret in the message: {shown}");
    assert!(
        !shown.contains("keychain://"),
        "reference in the message: {shown}"
    );
    assert!(executor.sessions().is_empty());
}

#[tokio::test]
async fn an_agent_may_not_test_a_configuration() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let executor = sqlite_executor(&store);
    let config = sqlite_config("agent-chosen host");
    let agent = Actor::agent(oxyn_core::AgentId::new(), oxyn_core::AgentSessionId::new());

    let outcome = executor
        .dispatch(
            agent,
            Command::TestConnection {
                config: Box::new(config),
            },
            &CancelToken::new(),
        )
        .await
        .expect("a refusal is an outcome");
    assert!(outcome.is_denied(), "{outcome:?}");
    assert!(executor.sessions().is_empty(), "nothing was opened");
}

#[test]
fn create_connection_without_a_runtime_fails_and_writes_nothing() {
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    let workspace = store
        .workspaces()
        .create("connections")
        .expect("workspace")
        .id;
    let policy = Arc::new(DefaultPolicy::new());
    let executor = Executor::builder(store, policy)
        .with_workspace(workspace)
        .build();
    let config = sqlite_config("no-runtime");

    let outcome = futures::executor::block_on(executor.dispatch(
        Actor::Human,
        Command::CreateConnection {
            config: Box::new(config.clone()),
        },
        &CancelToken::new(),
    ));
    assert!(matches!(outcome, Err(OxynError::Config(_))));
    assert!(
        executor
            .store()
            .connections()
            .get(config.id)
            .expect("read back")
            .is_none()
    );
}
