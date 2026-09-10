//! Paged summaries and separately loaded statements from local execution history.

use super::*;
use oxyn_core::{HistoryFilter, MAX_QUERY_DOCUMENT_BYTES, ResultId};
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
                substr(statement,1,256) AS statement,intent,duration_ms,row_count,status,substr(error,1,512) AS error,error_class,result_id
                FROM query_history WHERE (?1 IS NULL OR connection_id=?1) AND statement LIKE ?2 ESCAPE '\\'
                AND (?3 IS NULL OR ts>=?3) AND (?4='' OR status=?4 OR (?4='ambiguous' AND error_class='ambiguous'))
                AND (?5 IS NULL OR id<?5) AND (?7=0 OR result_id IS NOT NULL) ORDER BY id DESC LIMIT ?6")?;
            let mut entries = query.query_and_then(params![filter.connection.map(|id| id.to_string()), search, cutoff, filter.status.as_str(), filter.before, i64::from(filter.limit)+1, filter.results_only], |row| -> Result<HistorySummary> {
                let entry = depuis_ligne(row)?;
                let reconcile = entry.record.requires_reconciliation();
                let record = entry.record;
                Ok(HistorySummary { id: entry.id, ts: record.ts, connection: record.connection, connection_name: record.connection_name,
                    statement_preview: record.statement, status: record.status, error_class: record.error_class, duration: record.duration,
                    rows: record.rows, result: record.result, requires_reconciliation: reconcile })
            })?.collect::<Result<Vec<_>>>()?;
            let more = entries.len() > usize::from(filter.limit);
            if more { entries.pop(); }
            let next = if more { entries.last().map(|entry| entry.id) } else { None };
            Ok(HistoryPage { entries, next })
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
}
