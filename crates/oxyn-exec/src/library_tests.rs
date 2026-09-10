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
