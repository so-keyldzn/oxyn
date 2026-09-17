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

/// The page size of every library list, as the GPUI library used it.
pub const LIBRARY_PAGE: u16 = 100;

impl Backend {
    async fn read_local(&self, id: CommandId, command: Command) -> Result<Outcome, IpcError> {
        let cancel = self.track(id);
        let _running = super::Running {
            inner: &self.inner,
            id,
        };
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
            results_only: false,
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
                },
            ))
            .expect_err("a page of zero is a malformed request");
        assert!(!history.retryable);

        let expired = runtime
            .block_on(backend.open_retained_result(ConnectionId::new(), ResultId::new()))
            .expect("answered");
        assert!(matches!(expired, RetainedResult::Expired));
    }
}
