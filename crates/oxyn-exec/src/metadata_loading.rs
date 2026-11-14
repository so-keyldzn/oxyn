//! Manual metadata preferences use the same owned local-worker path as settings.

#[path = "metadata_loading/jobs.rs"]
pub(crate) mod jobs;
#[path = "metadata_loading/preload.rs"]
mod preload;

use oxyn_core::{CancelToken, ConnectionId, MetadataLoadingPolicy, Result};

use super::{Executor, Outcome};

impl Executor {
    pub(super) async fn read_metadata_policy(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let workspace = self.workspace;
        let snapshot = self
            .local_worker(cancel, move |cancel| {
                if cancel.is_cancelled() {
                    return Err(oxyn_store::StoreError::Cancelled);
                }
                store.metadata_policies().load(workspace, connection)
            })
            .await?;
        self.metadata_jobs.observe(connection, snapshot.revision);
        Ok(Outcome::MetadataPolicy { snapshot })
    }

    pub(super) async fn write_metadata_policy(
        &self,
        connection: ConnectionId,
        policy: &MetadataLoadingPolicy,
        expected_revision: u64,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        policy.validate()?;
        let store = self.store.clone();
        let workspace = self.workspace;
        let policy = policy.clone();
        let snapshot = self
            .local_worker(cancel, move |cancel| {
                if cancel.is_cancelled() {
                    return Err(oxyn_store::StoreError::Cancelled);
                }
                store
                    .metadata_policies()
                    .save(workspace, connection, &policy, expected_revision)
            })
            .await?;
        self.metadata_jobs.observe(connection, snapshot.revision);
        Ok(Outcome::MetadataPolicy { snapshot })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        Actor, AgentId, AgentSessionId, Command, ConnectionConfig, DefaultPolicy, DriverId,
        MetadataLevel,
    };
    use oxyn_store::Store;
    use std::sync::Arc;

    #[tokio::test]
    async fn saving_preferences_opens_no_driver_and_preserves_connection_authority() {
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let workspace = store.workspaces().create("metadata").expect("workspace").id;
        let source = ConnectionConfig::new("source", DriverId::sqlite())
            .read_only()
            .with_secret_ref("synthetic-reference");
        store
            .connections()
            .save(workspace, &source)
            .expect("source");
        // No driver is registered: an accidental connection/introspection fails.
        let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
            .with_workspace(workspace)
            .build();
        let policy = MetadataLoadingPolicy {
            default: MetadataLevel::Extended,
            ..Default::default()
        };
        let write = Command::WriteMetadataPolicy {
            connection: source.id,
            policy: Box::new(policy.clone()),
            expected_revision: 0,
        };
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        for command in [
            write.clone(),
            Command::ReadMetadataPolicy {
                connection: source.id,
            },
        ] {
            assert!(
                executor
                    .dispatch(agent, command, &CancelToken::new())
                    .await
                    .expect("policy")
                    .is_denied()
            );
        }
        assert_eq!(
            store
                .metadata_policies()
                .load(workspace, source.id)
                .expect("unchanged")
                .revision,
            0
        );
        let answer = executor
            .dispatch(Actor::Human, write.clone(), &CancelToken::new())
            .await
            .expect("save");
        assert!(
            matches!(answer, Outcome::MetadataPolicy { snapshot } if snapshot.revision == 1 && snapshot.policy == policy)
        );
        assert!(
            executor
                .dispatch(Actor::Human, write, &CancelToken::new())
                .await
                .is_err(),
            "stale write"
        );
        assert!(executor.sessions.is_empty());
        assert_eq!(
            store.connections().get(source.id).expect("source"),
            Some(source)
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

    #[tokio::test]
    async fn a_pre_cancelled_policy_write_does_not_publish_a_new_revision() {
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let workspace = store.workspaces().create("metadata").expect("workspace").id;
        let source = ConnectionConfig::new("source", DriverId::sqlite());
        store
            .connections()
            .save(workspace, &source)
            .expect("source");
        let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
            .with_workspace(workspace)
            .build();
        let cancel = CancelToken::new();
        cancel.cancel();
        let _ = executor
            .dispatch(
                Actor::Human,
                Command::WriteMetadataPolicy {
                    connection: source.id,
                    policy: Box::default(),
                    expected_revision: 0,
                },
                &cancel,
            )
            .await;
        assert_eq!(
            store
                .metadata_policies()
                .load(workspace, source.id)
                .expect("unchanged")
                .revision,
            0
        );
    }
}
