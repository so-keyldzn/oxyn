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
