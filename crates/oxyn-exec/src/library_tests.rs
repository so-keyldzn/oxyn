//! Local library commands cross the bus without a registered database driver.

use super::*;
use oxyn_core::{DefaultPolicy, DocumentFilter, HistoryFilter, QueryDocumentUpdate, QueryLanguage};

#[tokio::test]
async fn documents_and_history_are_local_and_workspace_scoped() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("library").expect("workspace").id;
    let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .build();
    let cancel = CancelToken::new();
    let document = DocumentId::new();
    let update = QueryDocumentUpdate {
        expected_revision: None,
        document,
        revision: 1,
        title: "Report".into(),
        language: QueryLanguage::SQL,
        text: "DELETE FROM invoices".into(),
        connection: None,
        save_named: true,
        is_open: true,
        provenance: None,
    };
    let saved = executor
        .dispatch(
            Actor::Human,
            Command::SaveQueryDocument {
                workspace,
                update: Box::new(update.clone()),
            },
            &cancel,
        )
        .await
        .expect("save without execution");
    assert!(
        matches!(saved, Outcome::DocumentOpened { document: doc } if doc.content == update.text)
    );
    let list = executor
        .dispatch(
            Actor::Human,
            Command::ListQueryDocuments {
                workspace,
                filter: Box::new(DocumentFilter::default()),
            },
            &cancel,
        )
        .await
        .expect("list");
    assert!(matches!(list, Outcome::QueryDocumentsListed { page } if page.entries.len() == 1));
    for command in [
        Command::OpenDocument {
            workspace: WorkspaceId::new(),
            document,
        },
        Command::SaveQueryDocument {
            workspace: WorkspaceId::new(),
            update: Box::new(update),
        },
    ] {
        assert!(
            executor
                .dispatch(Actor::Human, command, &cancel)
                .await
                .is_err()
        );
    }
    let history = executor
        .dispatch(
            Actor::Human,
            Command::ReadHistory {
                filter: Box::new(HistoryFilter::default()),
            },
            &cancel,
        )
        .await
        .expect("history");
    assert!(matches!(history, Outcome::HistoryListed { page } if page.entries.is_empty()));
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::ReadHistoryEntry { entry: 1 },
                &cancel
            )
            .await
            .is_err()
    );
    executor
        .dispatch(
            Actor::Human,
            Command::DeleteQueryDocument {
                workspace,
                document,
                revision: 2,
            },
            &cancel,
        )
        .await
        .expect("delete");
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::OpenDocument {
                    workspace,
                    document
                },
                &cancel
            )
            .await
            .is_err()
    );
}

/// Declaring a write reconciled is a statement about the server only the user
/// can make: an agent is refused, and the refusal is journaled (I-13, I-02).
#[tokio::test]
async fn only_the_human_reconciles_an_unresolved_write_through_the_bus() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store
        .workspaces()
        .create("reconcile")
        .expect("workspace")
        .id;
    let executor = Executor::builder(store.clone(), Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .build();
    let cancel = CancelToken::new();
    let entry = store
        .history()
        .record(
            &oxyn_store::history::HistoryRecord::new(
                &Actor::Human,
                QueryLanguage::SQL,
                "INSERT INTO t VALUES (1)",
            )
            .with_intent(oxyn_core::StatementIntent::Write)
            .failed(&OxynError::Timeout {
                after: std::time::Duration::from_secs(30),
            }),
        )
        .expect("expired write");

    let agent = Actor::agent(oxyn_core::AgentId::new(), oxyn_core::AgentSessionId::new());
    let refused = executor
        .dispatch(agent, Command::ReconcileHistoryEntry { entry }, &cancel)
        .await
        .expect("policy answer");
    assert!(matches!(refused, Outcome::Denied { .. }), "{refused:?}");
    assert!(store.history().any_requires_reconciliation().expect("scan"));
    let trace = store.journal().recent(1).expect("audit").remove(0);
    assert_eq!(trace.record.command_kind, "ReconcileHistoryEntry");
    assert!(trace.record.actor_kind.is_agent());
    assert_eq!(trace.record.decision, oxyn_store::PolicyOutcome::Denied);

    let done = executor
        .dispatch(
            Actor::Human,
            Command::ReconcileHistoryEntry { entry },
            &cancel,
        )
        .await
        .expect("reconciled");
    assert!(matches!(done, Outcome::HistoryEntryReconciled { entry: id } if id == entry));
    assert!(!store.history().any_requires_reconciliation().expect("scan"));
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::ReconcileHistoryEntry { entry: entry + 1 },
                &cancel,
            )
            .await
            .is_err(),
        "an absent entry is not silently acknowledged"
    );
}

#[tokio::test]
async fn retained_result_is_never_reexecuted_and_checks_the_original_connection() {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let workspace = store.workspaces().create("results").expect("workspace").id;
    let executor = Executor::builder(store, Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .build();
    let owner = ConnectionId::new();
    let result = ResultId::new();
    let buffer = Arc::new(ResultBuffer::new(
        Arc::new(arrow::datatypes::Schema::empty()),
        65536,
    ));
    buffer.mark_complete(ExecStats::default());
    executor.results.write().insert(
        result,
        StoredResult {
            connection: owner,
            buffer: buffer.clone(),
        },
    );
    let cancel = CancelToken::new();
    let opened = executor
        .dispatch(
            Actor::Human,
            Command::OpenRetainedResult {
                connection: owner,
                result,
            },
            &cancel,
        )
        .await
        .expect("local open");
    assert!(
        matches!(opened, Outcome::RetainedResultOpened { result: found, buffer: retained } if found == result && Arc::ptr_eq(&retained, &buffer))
    );
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::OpenRetainedResult {
                    connection: ConnectionId::new(),
                    result
                },
                &cancel
            )
            .await
            .is_err()
    );
    executor.results.write().remove(&result);
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::OpenRetainedResult {
                    connection: owner,
                    result
                },
                &cancel
            )
            .await
            .is_err()
    );
    assert_eq!(executor.sessions.len(), 0);
}
