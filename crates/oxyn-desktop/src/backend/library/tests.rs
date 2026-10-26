use super::*;
use crate::ipc::library::{DocumentChange, HistoryStatusChoice};

#[test]
fn a_page_lists_titles_without_bodies_and_history_records_what_ran() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let document = DocumentId::new();
    runtime
        .block_on(backend.save_query_document(
            CommandId::new(),
            document,
            None,
            DocumentChange {
                document: document.to_string(),
                revision: 1,
                title: "monthly.sql".into(),
                text: "SELECT 'a body the list never carries'".into(),
                connection: None,
                named: true,
            },
        ))
        .expect("saved");
    let page = runtime
        .block_on(backend.list_query_documents(
            CommandId::new(),
            DocumentQuery {
                saved_only: true,
                open_only: false,
                search: "monthly".into(),
                before: None,
                limit: None,
            },
        ))
        .expect("listed");
    assert_eq!(page.entries.len(), 1);
    let json = serde_json::to_string(&page).expect("serializable");
    assert!(!json.contains("a body the list never carries"), "{json}");

    let history = runtime
        .block_on(backend.read_history(
            CommandId::new(),
            None,
            HistoryQuery {
                connection: None,
                search: String::new(),
                days: Some(7),
                status: HistoryStatusChoice::All,
                before: None,
                limit: Some(0),
                results_only: false,
            },
        ))
        .expect_err("a page of zero is a malformed request");
    assert!(!history.retryable);

    let expired = runtime
        .block_on(backend.open_retained_result(ConnectionId::new(), ResultId::new()))
        .expect("answered");
    assert!(matches!(expired, RetainedResult::Expired));
}

fn all_history() -> HistoryQuery {
    HistoryQuery {
        connection: None,
        search: String::new(),
        days: None,
        status: HistoryStatusChoice::All,
        before: None,
        limit: None,
        results_only: false,
    }
}

#[test]
fn history_and_recent_results_read_the_same_execution_without_replaying_it() {
    use oxyn_core::QueryLanguage;
    use oxyn_store::history::HistoryRecord;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = std::sync::Arc::new(oxyn_store::Store::open_in_memory().expect("store"));
    let mut kept = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "SELECT 1")
        .succeeded(std::time::Duration::from_millis(3), Some(1));
    kept.result = Some(ResultId::new());
    let kept = store.history().record(&kept).expect("recorded");
    let without = store
        .history()
        .record(&HistoryRecord::new(
            &Actor::Human,
            QueryLanguage::SQL,
            "SELECT 2",
        ))
        .expect("recorded");
    let backend = Backend::assemble(
        store,
        std::sync::Arc::new(oxyn_secrets::MemorySecretStore::new()),
    )
    .expect("backend");

    // The front sends the field by its camelCase name.
    let query: HistoryQuery = serde_json::from_value(serde_json::json!({
        "connection": null,
        "search": "",
        "days": null,
        "status": "all",
        "before": null,
        "limit": null,
        "resultsOnly": true,
    }))
    .expect("deserializable");
    let recent = runtime
        .block_on(backend.read_history(CommandId::new(), None, query))
        .expect("listed");
    let ids: Vec<i64> = recent.entries.iter().map(|row| row.id).collect();
    assert_eq!(ids, vec![kept], "only the run carrying a result");
    assert!(recent.entries.iter().all(|row| row.result.is_some()));

    let history = runtime
        .block_on(backend.read_history(CommandId::new(), None, all_history()))
        .expect("listed");
    let ids: Vec<i64> = history.entries.iter().map(|row| row.id).collect();
    assert!(ids.contains(&kept) && ids.contains(&without), "{ids:?}");
}

#[test]
fn an_agent_statement_says_so_in_the_list_and_in_full() {
    use oxyn_core::{AgentId, AgentSessionId, QueryLanguage};
    use oxyn_store::history::HistoryRecord;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let store = std::sync::Arc::new(oxyn_store::Store::open_in_memory().expect("store"));
    let human = store
        .history()
        .record(&HistoryRecord::new(
            &Actor::Human,
            QueryLanguage::SQL,
            "SELECT 1",
        ))
        .expect("recorded");
    let agent = store
        .history()
        .record(&HistoryRecord::new(
            &Actor::agent(AgentId::new(), AgentSessionId::new()),
            QueryLanguage::SQL,
            "SELECT 2",
        ))
        .expect("recorded");
    let backend = Backend::assemble(
        store,
        std::sync::Arc::new(oxyn_secrets::MemorySecretStore::new()),
    )
    .expect("backend");

    let page = runtime
        .block_on(backend.read_history(CommandId::new(), None, all_history()))
        .expect("listed");
    let from_agent = |id| {
        page.entries
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.from_agent)
    };
    assert_eq!(from_agent(agent), Some(true));
    assert_eq!(from_agent(human), Some(false));
    let json = serde_json::to_value(&page).expect("serializable");
    assert!(
        json["entries"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["fromAgent"] == true)),
        "{json}"
    );

    let detail = runtime
        .block_on(backend.read_history_entry(agent))
        .expect("read");
    let json = serde_json::to_value(&detail).expect("serializable");
    assert_eq!(json["fromAgent"], true, "{json}");
    let detail = runtime
        .block_on(backend.read_history_entry(human))
        .expect("read");
    assert!(!detail.from_agent);
}
