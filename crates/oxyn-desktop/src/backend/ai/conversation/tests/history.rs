//! What the history panel reads beyond the threads of the open connection.

use oxyn_core::{ConnectionConfig, DriverId, WorkspaceId};
use oxyn_store::conversations::Destination as StoredDestination;
use oxyn_store::{Conversation, Store};

use super::*;

/// Saves one titled thread about `connection`, named `name`.
fn thread_on(store: &Store, workspace: WorkspaceId, connection: ConnectionId, name: &str) {
    let destination = StoredDestination::provider(
        ProviderId::new("anthropic-1a2b3c4d").expect("provider id"),
        "Anthropic",
        "a-model",
    );
    let conversation = Conversation::new(workspace, destination, format!("About {name}"))
        .on_connection(connection, name);
    store
        .conversations()
        .save(&conversation)
        .expect("thread saved");
}

/// The launch's prune reaches the panel: how many threads went, and the rule's
/// own numbers. A launch that removed nothing has nothing to say.
#[test]
fn the_launch_prune_is_transmitted_with_its_rule() {
    let home = tempfile::tempdir().expect("a temporary workspace");
    let path = home.path().join("workspace.sqlite3");
    {
        let store = Store::open_at(&path).expect("a workspace on disk");
        let workspace = store.workspaces().create("oxyn").expect("workspace").id;
        // Three past the count bound: the oldest three go.
        let over = oxyn_store::RetentionPolicy::default().max_conversations + 3;
        for index in 0..over {
            let conversation = Conversation::new(
                workspace,
                StoredDestination::provider(
                    ProviderId::new("anthropic-1a2b3c4d").expect("provider id"),
                    "Anthropic",
                    "a-model",
                ),
                format!("Idle {index}"),
            )
            .on_connection(ConnectionId::new(), "billing");
            store
                .conversations()
                .save(&conversation)
                .expect("thread saved");
        }
    }

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_at(&path).expect("the same workspace");
    // The prune runs off the startup path: wait for it, bounded.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let pruned = loop {
        if let Some(pruned) = backend.ai_pruned_history() {
            break pruned;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the launch prune was never transmitted"
        );
        runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(20)));
    };
    let policy = oxyn_store::RetentionPolicy::default();
    assert_eq!(
        pruned,
        PrunedHistory {
            conversations: 3,
            max_conversations: policy.max_conversations,
            max_age_days: policy.max_age_days,
            max_bytes: policy.max_bytes,
        }
    );

    let empty = Backend::open_temporary().expect("temporary backend");
    runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
    assert_eq!(empty.ai_pruned_history(), None);
}

/// A deleted connection's threads come back under the name they kept; a live
/// connection's threads are not among them.
#[test]
fn a_deleted_connections_threads_are_listed_under_its_name() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let executor = Arc::clone(&backend.inner.executor);
    let store = executor.store();
    let workspace = executor.workspace();

    let live = ConnectionConfig::new("warehouse", DriverId::postgres());
    store
        .connections()
        .save(workspace, &live)
        .expect("connection saved");
    thread_on(store, workspace, live.id, "warehouse");
    thread_on(store, workspace, ConnectionId::new(), "billing (deleted)");

    let orphans = runtime.block_on(backend.ai_orphan_threads());
    assert_eq!(orphans.len(), 1, "{orphans:?}");
    assert_eq!(
        orphans[0].connection_name.as_deref(),
        Some("billing (deleted)")
    );
    assert_eq!(orphans[0].title, "About billing (deleted)");

    // Listed is not deletable: the id, sent through a live connection,
    // deletes nothing.
    runtime
        .block_on(backend.ai_delete_thread(live.id, &orphans[0].id))
        .expect("answers");
    assert_eq!(runtime.block_on(backend.ai_orphan_threads()).len(), 1);
}
