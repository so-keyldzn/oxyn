use super::*;
use crate::Executor;
use oxyn_core::{Actor, AgentId, AgentSessionId, DefaultPolicy, DriverId, ServerProfileSnapshot};
use oxyn_driver::DriverRegistry;
use oxyn_driver_postgres::PostgresDriver;
use oxyn_store::Store;
use std::sync::Arc;

fn setup() -> (Arc<Store>, Executor, ServerProfile) {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("profiles").expect("workspace").id;
    let mut registry = DriverRegistry::new();
    registry
        .register(Arc::new(PostgresDriver::new()))
        .expect("driver");
    let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .with_drivers(Arc::new(registry))
        .build();
    let profile = ServerProfile {
        id: ServerProfileId::new(),
        name: "Example".into(),
        driver: DriverId::postgres(),
        params: [("host".into(), "localhost".into())].into_iter().collect(),
    };
    (store, executor, profile)
}

async fn listed(executor: &Executor) -> Vec<ServerProfileSnapshot> {
    match executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::ListServerProfiles,
            &CancelToken::new(),
        )
        .await
        .expect("list")
    {
        Outcome::ServerProfilesListed { profiles } => profiles,
        _ => panic!("unexpected outcome"),
    }
}

#[tokio::test]
async fn human_profile_writes_are_local_and_audited_but_agents_are_refused() {
    let (store, executor, profile) = setup();
    let command = oxyn_core::Command::SaveServerProfile {
        profile: Box::new(profile.clone()),
        expected_revision: None,
    };
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    let denied = executor
        .dispatch(agent, command.clone(), &CancelToken::new())
        .await
        .expect("decision");
    assert!(denied.is_denied());
    assert!(listed(&executor).await.is_empty());
    let saved = executor
        .dispatch(Actor::Human, command, &CancelToken::new())
        .await
        .expect("save");
    assert!(matches!(saved, Outcome::ServerProfileSaved { snapshot } if snapshot.revision == 1));
    assert_eq!(listed(&executor).await.len(), 1);
    assert_eq!(executor.sessions.len(), 0, "grouping opens no database");
    assert!(store.journal().count().expect("audit") >= 4);
    for command in [
        oxyn_core::Command::ListServerProfiles,
        oxyn_core::Command::RemoveServerProfile {
            profile: profile.id,
            expected_revision: 1,
        },
        oxyn_core::Command::SetServerProfileMembership {
            connection: ConnectionId::new(),
            profile: Some(profile.id),
            expected_profile: None,
        },
    ] {
        assert!(
            executor
                .dispatch(agent, command, &CancelToken::new())
                .await
                .expect("decision")
                .is_denied()
        );
    }
    assert_eq!(listed(&executor).await.len(), 1);
}

#[tokio::test]
async fn secret_unknown_and_database_defaults_cannot_reach_persistence() {
    let (_, executor, profile) = setup();
    for key in ["password", "unrecognized", "database"] {
        let mut invalid = profile.clone();
        invalid
            .params
            .insert(key.into(), "synthetic-sensitive-value".into());
        let error = executor
            .dispatch(
                Actor::Human,
                oxyn_core::Command::SaveServerProfile {
                    profile: Box::new(invalid),
                    expected_revision: None,
                },
                &CancelToken::new(),
            )
            .await
            .expect_err("refused");
        assert!(!error.to_string().contains("synthetic-sensitive-value"));
    }
    assert!(listed(&executor).await.is_empty());
}

#[tokio::test]
async fn cancellation_before_dispatch_changes_no_profile() {
    let (_, executor, profile) = setup();
    let cancel = CancelToken::new();
    cancel.cancel();
    let _ = executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::SaveServerProfile {
                profile: Box::new(profile),
                expected_revision: None,
            },
            &cancel,
        )
        .await;
    assert!(listed(&executor).await.is_empty());
}

#[tokio::test]
async fn database_discovery_requires_an_open_seed_and_refuses_agents() {
    let (_, executor, _) = setup();
    let command = oxyn_core::Command::DiscoverDatabases {
        connection: ConnectionId::new(),
    };
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    assert!(
        executor
            .dispatch(agent, command.clone(), &CancelToken::new())
            .await
            .expect("decision")
            .is_denied()
    );
    assert!(
        executor
            .dispatch(Actor::Human, command, &CancelToken::new())
            .await
            .is_err()
    );
    assert_eq!(executor.sessions.len(), 0);
}

#[tokio::test]
async fn database_child_creation_keeps_production_approval_and_agent_refusal() {
    let (_, executor, profile) = setup();
    let command = oxyn_core::Command::CreateDatabaseConnection {
        profile: profile.id,
        seed: ConnectionId::new(),
        config: Box::new(oxyn_core::ConnectionConfig::new(
            "Reviewed child",
            DriverId::postgres(),
        )),
    };
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    assert!(
        executor
            .dispatch(agent, command.clone(), &CancelToken::new())
            .await
            .expect("agent decision")
            .is_denied()
    );
    let held = executor
        .dispatch(Actor::Human, command, &CancelToken::new())
        .await
        .expect("human decision");
    assert!(
        matches!(held, Outcome::NeedsApproval { preview: Some(preview), .. } if preview.connection == "Reviewed child")
    );
    assert_eq!(executor.sessions.len(), 0);
}

#[tokio::test]
async fn discovery_refuses_a_saved_destination_that_differs_from_the_live_session() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store
        .workspaces()
        .create("live seed")
        .expect("workspace")
        .id;
    let mut registry = DriverRegistry::new();
    registry
        .register(Arc::new(oxyn_driver_sqlite::SqliteDriver::new()))
        .expect("driver");
    let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .with_drivers(Arc::new(registry))
        .build();
    let mut config = oxyn_core::ConnectionConfig::new("seed", DriverId::sqlite())
        .with_environment(oxyn_core::Environment::Local)
        .with_param("path", ":memory:");
    executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        )
        .await
        .expect("save");
    executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::Connect {
                connection: config.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect("open");
    assert!(matches!(
        executor
            .dispatch(
                Actor::Human,
                oxyn_core::Command::DiscoverDatabases {
                    connection: config.id
                },
                &CancelToken::new()
            )
            .await,
        Err(OxynError::NotSupported { .. })
    ));
    config.params.insert(
        "path".into(),
        "file:changed?mode=memory&cache=shared".into(),
    );
    executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::UpdateConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        )
        .await
        .expect("edit saved destination");
    let error = executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::DiscoverDatabases {
                connection: config.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect_err("live destination is stale");
    assert!(error.to_string().contains("reconnect"));
    assert_eq!(
        executor.sessions.len(),
        1,
        "editing never retargets an existing session"
    );
}

#[tokio::test]
async fn unopened_seed_discovery_never_decodes_a_hostile_workspace_row() {
    let directory = tempfile::tempdir().expect("test directory");
    let path = directory.path().join("workspace.oxyn");
    let store = Arc::new(Store::open_at(&path).expect("store"));
    let workspace = store.workspaces().create("seed").expect("workspace").id;
    let config = oxyn_core::ConnectionConfig::new("unopened", DriverId::postgres());
    store
        .connections()
        .save(workspace, &config)
        .expect("saved seed");
    let raw = rusqlite::Connection::open(&path).expect("fixture connection");
    raw.execute(
        "UPDATE connections SET params = ?1 WHERE id = ?2",
        rusqlite::params![r#""synthetic-sensitive-seed-value""#, config.id.to_string()],
    )
    .expect("hostile fixture");
    let executor = Executor::builder(store, Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .build();
    let error = executor
        .dispatch(
            Actor::Human,
            oxyn_core::Command::DiscoverDatabases {
                connection: config.id,
            },
            &CancelToken::new(),
        )
        .await
        .expect_err("unopened seed");
    assert!(!error.to_string().contains("synthetic-sensitive-seed-value"));
    assert_eq!(executor.sessions.len(), 0);
}
