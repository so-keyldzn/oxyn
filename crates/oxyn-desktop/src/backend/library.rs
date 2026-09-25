//! The local library: saved queries, working copies and execution history.
//!
//! Every read goes through the command bus and returns a bounded page without
//! complete bodies ([ADR-0014](../../../../docs/adr/0014-documents-et-historique.md)).
//! Nothing here executes anything: a history entry is read to be **copied**
//! into a console, never replayed ([I-13](../../../../CLAUDE.md#i-13)).

use oxyn_core::{
    Actor, Command, CommandId, ConnectionId, DocumentFilter, DocumentId, HistoryConnectionFilter,
    HistoryFilter, ResultId,
};
use oxyn_exec::Outcome;

use super::Backend;
use crate::ipc::IpcError;
use crate::ipc::library::{
    DocumentList, DocumentQuery, HistoryConnectionList, HistoryDetail, HistoryList, HistoryQuery,
    RetainedResult,
};

/// The page size of every library list.
pub const LIBRARY_PAGE: u16 = 100;

impl Backend {
    async fn read_local(&self, id: CommandId, command: Command) -> Result<Outcome, IpcError> {
        let cancel = self.track(id)?;
        let outcome = self
            .inner
            .executor
            .dispatch_as(id, Actor::Human, command, &cancel)
            .await?;
        match outcome {
            // A local read the policy holds back is refused, not left pending:
            // nobody is going to be asked about a list.
            Outcome::NeedsApproval { command, .. } => {
                self.inner.executor.reject(command);
                Err(IpcError::invalid(
                    "The policy does not allow reading the local library.",
                ))
            }
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            other => Ok(other),
        }
    }

    /// A page of saved queries or open working copies.
    pub async fn list_query_documents(
        &self,
        id: CommandId,
        query: DocumentQuery,
    ) -> Result<DocumentList, IpcError> {
        let before = query
            .before
            .as_deref()
            .map(|cursor| {
                cursor
                    .parse::<DocumentId>()
                    .map_err(|error| IpcError::invalid(format!("invalid cursor: {error}")))
            })
            .transpose()?;
        let filter = DocumentFilter {
            saved_only: query.saved_only,
            open_only: query.open_only,
            search: query.search,
            before,
            limit: query.limit.unwrap_or(LIBRARY_PAGE),
        };
        filter.validate()?;
        match self
            .read_local(
                id,
                Command::ListQueryDocuments {
                    workspace: self.inner.executor.workspace(),
                    filter: Box::new(filter),
                },
            )
            .await?
        {
            Outcome::QueryDocumentsListed { page } => Ok(page.into()),
            _ => Err(IpcError::invalid("Unexpected response to a library read")),
        }
    }

    /// A page of local history, for all connections or one.
    pub async fn read_history(
        &self,
        id: CommandId,
        connection: Option<ConnectionId>,
        query: HistoryQuery,
    ) -> Result<HistoryList, IpcError> {
        let filter = HistoryFilter {
            results_only: query.results_only,
            connection,
            search: query.search,
            days: query.days,
            status: query.status.into(),
            before: query.before,
            limit: query.limit.unwrap_or(LIBRARY_PAGE),
        };
        filter.validate()?;
        match self
            .read_local(
                id,
                Command::ReadHistory {
                    filter: Box::new(filter),
                },
            )
            .await?
        {
            Outcome::HistoryListed { page } => Ok(page.into()),
            _ => Err(IpcError::invalid("Unexpected response to a history read")),
        }
    }

    /// One execution in full, to be copied — never replayed.
    pub async fn read_history_entry(&self, entry: i64) -> Result<HistoryDetail, IpcError> {
        match self
            .read_local(CommandId::new(), Command::ReadHistoryEntry { entry })
            .await?
        {
            Outcome::HistoryEntryRead { entry } => Ok((*entry).into()),
            _ => Err(IpcError::invalid("Unexpected response to a history read")),
        }
    }

    /// Records that the user inspected the server state of an unresolved write,
    /// so the next launch stops warning about it. Retries nothing (I-13).
    pub async fn reconcile_history_entry(&self, entry: i64) -> Result<(), IpcError> {
        let command = Command::ReconcileHistoryEntry { entry };
        let _local = self.local_write(&command);
        match self.read_local(CommandId::new(), command).await? {
            Outcome::HistoryEntryReconciled { .. } => Ok(()),
            _ => Err(IpcError::invalid(
                "Unexpected response to reconciling a history entry",
            )),
        }
    }

    /// Reopens a result still retained, through the bus; runs no query and
    /// opens no session. A released buffer answers `Expired`.
    pub async fn open_retained_result(
        &self,
        connection: ConnectionId,
        result: ResultId,
    ) -> Result<RetainedResult, IpcError> {
        // Checked before dispatch rather than read from the error text: an
        // expired buffer is a state, not a message to parse.
        if self.inner.executor.result(result).is_none() {
            return Ok(RetainedResult::Expired);
        }
        match self
            .read_local(
                CommandId::new(),
                Command::OpenRetainedResult { connection, result },
            )
            .await?
        {
            Outcome::RetainedResultOpened { result, buffer } => Ok(RetainedResult::Open {
                result: result.to_string(),
                columns: buffer
                    .schema()
                    .fields()
                    .iter()
                    .map(|field| crate::ipc::ResultColumn {
                        name: field.name().clone(),
                        data_type: field.data_type().to_string(),
                        nullable: field.is_nullable(),
                    })
                    .collect(),
                rows: buffer.row_count(),
                complete: buffer.is_complete(),
                truncated: buffer.stats().truncated,
            }),
            _ => Err(IpcError::invalid("Unexpected response to opening a result")),
        }
    }

    /// The connections history recorded, removed ones included.
    pub async fn list_history_connections(
        &self,
        before: Option<i64>,
    ) -> Result<HistoryConnectionList, IpcError> {
        let filter = HistoryConnectionFilter {
            before,
            limit: LIBRARY_PAGE,
        };
        filter.validate()?;
        match self
            .read_local(
                CommandId::new(),
                Command::ListHistoryConnections {
                    workspace: self.inner.executor.workspace(),
                    filter,
                },
            )
            .await?
        {
            Outcome::HistoryConnectionsListed { page } => Ok(page.into()),
            _ => Err(IpcError::invalid("Unexpected response to a history read")),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
