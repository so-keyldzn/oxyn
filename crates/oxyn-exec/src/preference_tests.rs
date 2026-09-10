//! Local preference routing and actor policy, without any database driver.

use super::*;
use oxyn_core::{AgentId, AgentSessionId, DefaultPolicy, PreferencesSnapshot, ReadingDensity};

#[tokio::test]
async fn only_human_preference_writes_are_saved_and_the_workspace_is_checked() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store
        .workspaces()
        .create("preferences")
        .expect("workspace")
        .id;
    let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .build();
    let mut snapshot = PreferencesSnapshot {
        revision: 1,
        ..Default::default()
    };
    snapshot.preferences.reading_density = ReadingDensity::Comfortable;
    let command = Command::WriteWorkspacePreferences {
        workspace,
        snapshot: Box::new(snapshot.clone()),
    };
    let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
    assert!(matches!(
        executor
            .dispatch(agent, command.clone(), &CancelToken::new())
            .await
            .expect("policy response"),
        Outcome::Denied { .. }
    ));
    assert_eq!(
        store
            .preferences()
            .load(workspace)
            .expect("unchanged")
            .revision,
        0
    );
    assert!(matches!(
        executor
            .dispatch(Actor::Human, command, &CancelToken::new())
            .await
            .expect("human write"),
        Outcome::WorkspacePreferences { .. }
    ));
    let read = executor
        .dispatch(
            agent,
            Command::ReadWorkspacePreferences { workspace },
            &CancelToken::new(),
        )
        .await
        .expect("local read");
    assert!(
        matches!(read, Outcome::WorkspacePreferences { snapshot: loaded } if loaded == snapshot)
    );
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::ReadWorkspacePreferences {
                    workspace: WorkspaceId::new()
                },
                &CancelToken::new()
            )
            .await
            .is_err()
    );
    assert!(
        store
            .journal()
            .recent(8)
            .expect("audit")
            .iter()
            .all(|entry| entry.record.statement.is_none())
    );
}
