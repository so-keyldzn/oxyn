//! Paged summaries and separately loaded statements from local execution history.

use super::*;
use crate::encoding::parse_id;
use oxyn_core::{
    HistoryConnectionFilter, HistoryFilter, MAX_QUERY_DOCUMENT_BYTES, ResultId, WorkspaceId,
};
use rusqlite::OptionalExtension;

/// A history list row. It deliberately has no complete statement field.
#[derive(Clone)]
pub struct HistorySummary {
    /// Stable SQLite recording order.
    pub id: i64,
    /// Submission timestamp.
    pub ts: DateTime<Utc>,
    /// Original connection, even when it has since been removed.
    pub connection: Option<ConnectionId>,
    /// Historical display name.
    pub connection_name: Option<String>,
    /// At most 256 characters, never used for execution.
    pub statement_preview: String,
    /// Recorded outcome.
    pub status: HistoryStatus,
    /// Recorded error class; never inferred from message text.
    pub error_class: Option<ErrorClass>,
    /// Recorded elapsed time.
    pub duration: Option<Duration>,
    /// Recorded row count.
    pub rows: Option<u64>,
    /// Potentially retained result.
    pub result: Option<ResultId>,
    /// True for an ambiguous or otherwise unresolved mutating outcome.
    pub requires_reconciliation: bool,
    /// When the user confirmed having inspected the server state, if they did.
    pub reconciled_at: Option<DateTime<Utc>>,
}
impl std::fmt::Debug for HistorySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistorySummary")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("requires_reconciliation", &self.requires_reconciliation)
            .finish_non_exhaustive()
    }
}

/// One connection as history remembers it, existing or not.
#[derive(Debug, Clone)]
pub struct HistoryConnectionSummary {
    /// The recorded identity, usable as a [`HistoryFilter::connection`].
    pub connection: ConnectionId,
    /// The name copied at execution time, absent on a row that never carried one.
    pub name: Option<String>,
    /// Recording order of its most recent entry; also the pagination cursor.
    pub last_entry: i64,
    /// False once the connection has been removed, or when it belongs elsewhere.
    pub in_workspace: bool,
}

/// At most one bounded page of the connections history recorded.
#[derive(Debug, Clone)]
pub struct HistoryConnectionPage {
    /// The connections, most recently used first.
    pub entries: Vec<HistoryConnectionSummary>,
    /// Cursor for the next, less recently used page.
    pub next: Option<i64>,
}

/// At most one bounded history page.
#[derive(Debug, Clone)]
pub struct HistoryPage {
    /// The summaries, newest recording first.
    pub entries: Vec<HistorySummary>,
    /// Cursor for the next older page.
    pub next: Option<i64>,
}

impl History<'_> {
    /// Applies literal filters in SQLite, returning no complete SQL bodies.
    pub fn page(
        &self,
        filter: &HistoryFilter,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<HistoryPage> {
        filter
            .validate()
            .map_err(|error| crate::StoreError::Corrupted {
                field: "history filter",
                detail: error.to_string(),
            })?;
        let search = format!("%{}%", escape_like(&filter.search));
        let cutoff = filter.days.and_then(|days| {
            Utc::now().checked_sub_signed(chrono::TimeDelta::days(i64::from(days)))
        });
        self.store.with_connection_cancellable(cancel, |connection| {
            let mut query = connection.prepare("SELECT id,ts,connection_id,substr(connection_name,1,256) AS connection_name,actor_kind,actor_id,language,
                substr(statement,1,256) AS statement,intent,duration_ms,row_count,status,substr(error,1,512) AS error,error_class,result_id,reconciled_at
                FROM query_history WHERE (?1 IS NULL OR connection_id=?1) AND statement LIKE ?2 ESCAPE '\\'
                AND (?3 IS NULL OR ts>=?3) AND (?4='' OR status=?4 OR (?4='ambiguous' AND error_class='ambiguous'))
                AND (?5 IS NULL OR id<?5) AND (?7=0 OR result_id IS NOT NULL) ORDER BY id DESC LIMIT ?6")?;
            let mut entries = query.query_and_then(params![filter.connection.map(|id| id.to_string()), search, cutoff, filter.status.as_str(), filter.before, i64::from(filter.limit)+1, filter.results_only], |row| -> Result<HistorySummary> {
                let entry = depuis_ligne(row)?;
                let reconcile = entry.record.requires_reconciliation();
                let record = entry.record;
                Ok(HistorySummary { id: entry.id, ts: record.ts, connection: record.connection, connection_name: record.connection_name,
                    statement_preview: record.statement, status: record.status, error_class: record.error_class, duration: record.duration,
                    rows: record.rows, result: record.result, requires_reconciliation: reconcile,
                    reconciled_at: record.reconciled_at })
            })?.collect::<Result<Vec<_>>>()?;
            let more = entries.len() > usize::from(filter.limit);
            if more { entries.pop(); }
            let next = if more { entries.last().map(|entry| entry.id) } else { None };
            Ok(HistoryPage { entries, next })
        })
    }

    /// Lists the distinct connections history holds, one bounded page at a time.
    ///
    /// The answer is not a subset of the saved connections: it is what
    /// `query_history` recorded, so it keeps a connection the workspace has
    /// deleted and a connection that belongs to another workspace. That is the
    /// whole point — the identity returned here is the only one that can be
    /// given to [`HistoryFilter::connection`] to find those entries again.
    ///
    /// `in_workspace` is resolved against `connections` for `workspace`, so a
    /// caller can say so on screen instead of leaving the user with a filter
    /// that looks broken.
    ///
    /// The name is the most recently recorded one: renaming a connection makes
    /// older entries carry an older name, and offering the stale one would name
    /// something the user no longer recognises.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`], or [`crate::StoreError::Corrupted`] on an
    /// out-of-bounds filter or an unparsable stored identifier.
    pub fn connections(
        &self,
        workspace: WorkspaceId,
        filter: &HistoryConnectionFilter,
        cancel: &oxyn_core::CancelToken,
    ) -> Result<HistoryConnectionPage> {
        filter
            .validate()
            .map_err(|error| crate::StoreError::Corrupted {
                field: "history connection filter",
                detail: error.to_string(),
            })?;
        self.store.with_connection_cancellable(cancel, |connection| {
            let mut query = connection.prepare(
                "SELECT h.connection_id AS connection_id,
                    substr((SELECT n.connection_name FROM query_history n
                        WHERE n.connection_id=h.connection_id AND n.connection_name IS NOT NULL
                        ORDER BY n.id DESC LIMIT 1),1,256) AS connection_name,
                    MAX(h.id) AS last_entry,
                    EXISTS(SELECT 1 FROM connections c WHERE c.id=h.connection_id AND c.workspace_id=?1) AS in_workspace
                FROM query_history h WHERE h.connection_id IS NOT NULL
                GROUP BY h.connection_id HAVING ?2 IS NULL OR MAX(h.id)<?2
                ORDER BY last_entry DESC LIMIT ?3")?;
            let mut entries = query.query_and_then(params![workspace.to_string(), filter.before, i64::from(filter.limit)+1], |row| -> Result<HistoryConnectionSummary> {
                let raw: String = row.get("connection_id")?;
                Ok(HistoryConnectionSummary {
                    connection: parse_id(&raw, "query_history.connection_id")?,
                    name: row.get("connection_name")?,
                    last_entry: row.get("last_entry")?,
                    in_workspace: row.get("in_workspace")?,
                })
            })?.collect::<Result<Vec<_>>>()?;
            let more = entries.len() > usize::from(filter.limit);
            if more { entries.pop(); }
            let next = if more { entries.last().map(|entry| entry.last_entry) } else { None };
            Ok(HistoryConnectionPage { entries, next })
        })
    }

    /// Loads one complete historical statement; oversized SQL is never opened partially.
    pub fn get(&self, id: i64, cancel: &oxyn_core::CancelToken) -> Result<Option<HistoryEntry>> {
        self.store
            .with_connection_cancellable(cancel, |connection| {
                let transaction = connection.unchecked_transaction()?;
                let bytes: Option<i64> = transaction
                    .query_row(
                        "SELECT length(CAST(statement AS BLOB)) FROM query_history WHERE id=?1",
                        [id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if bytes.is_some_and(|bytes| {
                    usize::try_from(bytes).map_or(true, |bytes| bytes > MAX_QUERY_DOCUMENT_BYTES)
                }) {
                    return Err(crate::StoreError::Corrupted {
                        field: "query_history.statement",
                        detail: "query exceeds the 1 MiB editor limit".into(),
                    });
                }
                transaction
                    .query_row(&format!("{SELECT_COLONNES} WHERE id=?1"), [id], |row| {
                        Ok(depuis_ligne(row))
                    })
                    .optional()?
                    .transpose()
            })
    }

    /// Whether any recorded execution still needs its server state inspected —
    /// the rows the library marks `needs_inspection`.
    ///
    /// Reads three columns, stopping at the first match. A clean success and a
    /// row the user reconciled are excluded in SQL, since neither qualifies;
    /// everything else is decided by the same rule as
    /// [`HistoryRecord::requires_reconciliation`].
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the read fails.
    pub fn any_requires_reconciliation(&self) -> Result<bool> {
        self.store.with_connection(|connection| {
            let mut query = connection.prepare(
                "SELECT intent, status, error_class FROM query_history
                 WHERE (status <> 'succeeded' OR error_class IS NOT NULL)
                   AND reconciled_at IS NULL",
            )?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                let intent: String = row.get("intent")?;
                let status: String = row.get("status")?;
                let error_class: Option<String> = row.get("error_class")?;
                if super::requires_reconciliation(
                    intent_from_text(&intent),
                    HistoryStatus::from_text(&status),
                    error_class.as_deref().map(error_class_from_text),
                ) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }
}
