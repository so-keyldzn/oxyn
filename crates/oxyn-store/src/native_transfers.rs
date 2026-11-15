//! Bounded readable native job records. Every method blocks on the local store.
use crate::{Result, Store, StoreError};
use oxyn_core::{
    ConnectionId, NativeTransferFailure, NativeTransferId, NativeTransferJob,
    NativeTransferOperation, NativeTransferOutcome, NativeTransferStage, WorkspaceId,
};
use rusqlite::{OptionalExtension, params};

const MAX_JOB_BYTES: usize = 64 * 1024;
const MAX_PAGE: usize = 100;
/// Local native job history, separate from authentication and SQL text history.
#[derive(Debug)]
pub struct NativeTransfers<'a> {
    store: &'a Store,
}
fn invalid() -> StoreError {
    StoreError::Corrupted {
        field: "native_transfer_jobs",
        detail: "invalid native job record or ownership".into(),
    }
}
fn decode(payload: Option<String>) -> Result<NativeTransferJob> {
    let payload = payload.ok_or_else(invalid)?;
    if payload.len() > MAX_JOB_BYTES {
        return Err(invalid());
    }
    let job: NativeTransferJob = serde_json::from_str(&payload).map_err(|_| invalid())?;
    job.validate().map_err(|_| invalid())?;
    Ok(job)
}
impl<'a> NativeTransfers<'a> {
    pub(crate) const fn new(store: &'a Store) -> Self {
        Self { store }
    }
    /// Insert/update one immutable source identity; cross-workspace replacement is refused.
    ///
    /// # Errors
    /// Invalid/oversized records, changed source identity or storage failure.
    pub fn save(&self, workspace: WorkspaceId, job: &NativeTransferJob) -> Result<()> {
        job.validate().map_err(|_| invalid())?;
        let payload = serde_json::to_string(job).map_err(|_| invalid())?;
        if payload.len() > MAX_JOB_BYTES {
            return Err(invalid());
        }
        self.store.with_connection(|sql| {
            let changed=sql.execute("INSERT INTO native_transfer_jobs(id,workspace_id,connection_id,owner_id,session_id,created_at,payload) VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(id) DO UPDATE SET payload=excluded.payload
             WHERE native_transfer_jobs.workspace_id=excluded.workspace_id AND native_transfer_jobs.connection_id=excluded.connection_id AND native_transfer_jobs.owner_id=excluded.owner_id AND native_transfer_jobs.session_id=excluded.session_id AND native_transfer_jobs.created_at=excluded.created_at",params![job.id.to_string(),workspace.to_string(),job.connection.to_string(),job.owner.to_string(),job.session.to_string(),job.created_at,payload])?;
            if changed!=1 { return Err(invalid()); } Ok(())
        })
    }
    /// Read one workspace-owned job; no source is opened or restored.
    ///
    /// # Errors
    /// Corrupt/oversized payload, inconsistent identity or storage failure.
    pub fn get(
        &self,
        workspace: WorkspaceId,
        id: NativeTransferId,
    ) -> Result<Option<NativeTransferJob>> {
        self.store.with_connection(|sql| {
            let limit=i64::try_from(MAX_JOB_BYTES).map_err(|_|invalid())?;
            let row:Option<(String,String,String,String,Option<String>)>=sql.query_row("SELECT connection_id,owner_id,session_id,created_at,CASE WHEN length(CAST(payload AS BLOB))<=?3 THEN payload END FROM native_transfer_jobs WHERE workspace_id=?1 AND id=?2",params![workspace.to_string(),id.to_string(),limit],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional()?;
            row.map(|(connection,owner,session,created,payload)|{
                let job=decode(payload)?;
                if job.id!=id || job.connection.to_string()!=connection || job.owner.to_string()!=owner || job.session.to_string()!=session || job.created_at!=created { return Err(invalid()); } Ok(job)
            }).transpose()
        })
    }
    /// Read at most 100 newest records for one connection; deletion never erases history.
    ///
    /// # Errors
    /// Corrupt payload/identity or storage failure.
    pub fn list(
        &self,
        workspace: WorkspaceId,
        connection: ConnectionId,
    ) -> Result<Vec<NativeTransferJob>> {
        let ids=self.store.with_connection(|sql| {
            let mut statement=sql.prepare("SELECT id FROM native_transfer_jobs WHERE workspace_id=?1 AND connection_id=?2 ORDER BY created_at DESC,id DESC LIMIT ?3")?;
            let rows=statement.query_map(params![workspace.to_string(),connection.to_string(),i64::try_from(MAX_PAGE).map_err(|_|invalid())?],|row|row.get::<_,String>(0))?;
            rows.map(|row|row.map_err(StoreError::from).and_then(|id|id.parse().map_err(|_|invalid()))).collect::<Result<Vec<NativeTransferId>>>()
        })?;
        ids.into_iter()
            .map(|id| self.get(workspace, id)?.ok_or_else(invalid))
            .collect()
    }
    /// Whether startup must ask the human to inspect possible native writes.
    ///
    /// # Errors
    /// Unreadable JSON or storage failure; callers conservatively warn on failure.
    pub fn any_requires_reconciliation(&self, workspace: WorkspaceId) -> Result<bool> {
        self.store.with_connection(|sql| {
            let limit = i64::try_from(MAX_JOB_BYTES).map_err(|_| invalid())?;
            // CASE bounds SQLite's own JSON parser before untrusted workspace
            // payloads reach it. Corrupt/oversized records conservatively warn.
            Ok(sql.query_row("SELECT EXISTS(SELECT 1 FROM native_transfer_jobs WHERE workspace_id=?1 AND CASE WHEN length(CAST(payload AS BLOB))>?2 THEN 1 WHEN NOT json_valid(payload) THEN 1 ELSE json_extract(payload,'$.mayHaveChanged')=1 AND json_extract(payload,'$.reconciled')=0 AND (json_extract(payload,'$.outcome') IS NULL OR json_extract(payload,'$.outcome')<>'succeeded') END)", params![workspace.to_string(), limit], |row| row.get(0))?)
        })
    }
    /// Mark unfinished work from a prior application launch, never resume a PID or restore.
    ///
    /// # Errors
    /// Corrupt records or storage failure; the caller must surface failed recovery.
    pub fn interrupt_unfinished(&self, workspace: WorkspaceId) -> Result<usize> {
        let mut changed = 0;
        loop {
            let ids = self.store.with_connection(|sql| {
            let mut statement =
                sql.prepare("SELECT id FROM native_transfer_jobs WHERE workspace_id=?1 AND CASE WHEN length(CAST(payload AS BLOB))>?2 THEN 1 WHEN NOT json_valid(payload) THEN 1 ELSE COALESCE(json_extract(payload,'$.stage'),'')<>'finished' END LIMIT 100")?;
            let rows =
                statement.query_map(params![workspace.to_string(), i64::try_from(MAX_JOB_BYTES).map_err(|_| invalid())?], |row| row.get::<_, String>(0))?;
            rows.map(|row| {
                row.map_err(StoreError::from)
                    .and_then(|id| id.parse().map_err(|_| invalid()))
            })
            .collect::<Result<Vec<NativeTransferId>>>()
        })?;
            if ids.is_empty() {
                break;
            }
            for id in ids {
                let Some(mut job) = self.get(workspace, id)? else {
                    continue;
                };
                if job.stage == NativeTransferStage::Finished {
                    continue;
                }
                let before = matches!(
                    job.stage,
                    NativeTransferStage::Preparing | NativeTransferStage::Ready
                ) && !job.may_have_changed;
                job.may_have_changed |=
                    job.operation == NativeTransferOperation::Restore && !before;
                job.stage = NativeTransferStage::Finished;
                job.outcome = Some(if before {
                    NativeTransferOutcome::FailedBeforeStart
                } else {
                    NativeTransferOutcome::Interrupted
                });
                job.failure = Some(NativeTransferFailure::Interrupted);
                job.command = None;
                job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                self.save(workspace, &job)?;
                changed += 1;
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
