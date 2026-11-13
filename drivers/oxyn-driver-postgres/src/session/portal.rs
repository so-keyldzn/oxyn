//! Demand-driven FETCH windows let idle preview release preserve temporary data.

use std::sync::Arc;
use std::time::Duration;

use arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use async_trait::async_trait;
use futures::TryStreamExt;
use oxyn_core::{
    CancelToken, ErrorClass, ExecRequest, ExecStats, OxynError, Result, StatementHandle,
    StatementIntent,
};
use oxyn_driver::Cursor;
use sqlx::{AssertSqlSafe, Executor as _, SqlSafeStr as _, Statement as _};
use std::sync::{Mutex, PoisonError};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

use super::{
    PostgresSession, SQL_BEGIN_READ_ONLY, SQL_RESET_SEARCH_PATH, SQL_ROLLBACK, backend_pid,
    race_cancel,
};
use crate::cancel::{Verdict, VerdictSender};
use crate::decode::BatchAssembler;
use crate::error::{Bound, map_exec_error, map_stream_error};
use crate::lease::Lease;
use crate::types::{PgDecoding, schema_for};

const WINDOW: usize = 200;
const CLEANUP: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Owners {
    closing: CancelToken,
    tasks: Mutex<JoinSet<()>>,
}

impl Owners {
    pub async fn close(&self) {
        self.closing.cancel();
        let _ = self.wait(&CancelToken::new()).await;
    }

    pub async fn wait(&self, cancel: &CancelToken) -> Result<()> {
        let mut tasks =
            std::mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        loop {
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(OxynError::Cancelled),
                result = tasks.join_next() => result,
            };
            match result {
                Some(Ok(())) => {}
                Some(Err(_)) => {
                    return Err(OxynError::Internal(
                        "a PostgreSQL preview owner stopped unexpectedly".into(),
                    ));
                }
                None => return Ok(()),
            }
        }
    }
}

impl PostgresSession {
    pub(super) async fn original_preview(
        &self,
        request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        if !request.limits.read_only || !request.params.is_empty() {
            return Err(OxynError::Config(
                "a PostgreSQL relation preview requires an unbound read-only request".into(),
            ));
        }
        self.capabilities.require_language(request.language)?;
        let mut connection = Lease::new(self.acquire(cancel).await?);
        let pid = race_cancel(cancel, async {
            backend_pid(&mut connection)
                .await
                .map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))
        })
        .await?;
        connection.taint();
        let mut began = false;
        let mut context_changed = false;
        let setup = async {
            if let Some(context) = self.context_statement() {
                sqlx::raw_sql(AssertSqlSafe(context).into_sql_str())
                    .execute(&mut *connection)
                    .await?;
                context_changed = true;
            }
            sqlx::raw_sql(SQL_BEGIN_READ_ONLY)
                .execute(&mut *connection)
                .await?;
            began = true;
            // Parse/Describe proves one statement before it is embedded in DECLARE.
            let prepared = (&mut *connection)
                .prepare(AssertSqlSafe(request.text.clone()).into_sql_str())
                .await?;
            let (schema, decodings) = schema_for(prepared.columns());
            let name = oxyn_catalog::quote_identifier(
                &format!("oxyn_preview_{}", StatementHandle::new()),
                oxyn_catalog::QuoteStyle::Double,
            );
            let scroll = if self.variant.flavor == crate::variant::PostgresFlavor::Redshift {
                ""
            } else {
                " NO SCROLL"
            };
            let declare = format!("DECLARE {name}{scroll} CURSOR FOR {}", request.text);
            sqlx::raw_sql(AssertSqlSafe(declare).into_sql_str())
                .execute(&mut *connection)
                .await?;
            Ok::<_, sqlx::Error>((name, schema, decodings))
        };
        let issue = race_cancel(cancel, async { Ok(setup.await) }).await;
        let (name, schema, decodings) = match issue {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                // A normal SQL/type refusal leaves a synchronizable protocol.
                // Closing it would destroy temporary objects for a mere typo.
                let restorable = matches!(
                    error,
                    sqlx::Error::Database(_)
                        | sqlx::Error::ColumnDecode { .. }
                        | sqlx::Error::Decode(_)
                );
                let restored = if restorable {
                    tokio::time::timeout(CLEANUP, async {
                        if began {
                            sqlx::raw_sql(SQL_ROLLBACK)
                                .execute(&mut *connection)
                                .await?;
                        }
                        if context_changed {
                            sqlx::raw_sql(SQL_RESET_SEARCH_PATH)
                                .execute(&mut *connection)
                                .await?;
                        }
                        Ok::<_, sqlx::Error>(())
                    })
                    .await
                    .is_ok_and(|result| result.is_ok())
                } else {
                    false
                };
                if restored {
                    connection.restored();
                } else {
                    connection.discard();
                }
                return Err(map_exec_error(&self.driver, StatementIntent::Read, error));
            }
            Err(error) => {
                return self
                    .stop_if_interrupted(&mut connection, pid, Err(error))
                    .await;
            }
        };
        let token = cancel.child();
        let handle = StatementHandle::new();
        let verdict = self.statements.register(handle, token.clone());
        let (send, events) = mpsc::channel(1);
        let (done, completed) = oneshot::channel();
        let (demand, pulls) = mpsc::channel(1);
        let pump = Pump {
            driver: self.driver.clone(),
            connection,
            pid,
            name,
            schema: Arc::clone(&schema),
            decodings,
            canceller: Arc::clone(&self.canceller),
            cancel: token.clone(),
            closing: self.portals.closing.clone(),
            verdict,
        };
        let statements = Arc::clone(&self.statements);
        let mut tasks = self
            .portals
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while let Some(result) = tasks.try_join_next() {
            if result.is_err() {
                tracing::warn!("a PostgreSQL preview owner stopped unexpectedly");
            }
        }
        tasks.spawn(async move {
            let outcome = pump.run(pulls, send).await;
            statements.forget(handle);
            let _ = done.send(outcome);
        });
        Ok(Box::new(PortalCursor {
            handle,
            schema,
            events,
            demand,
            completed: Some(completed),
            cancel: token,
            stats: ExecStats::default(),
        }))
    }
}

struct PortalCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    events: mpsc::Receiver<RecordBatch>,
    demand: mpsc::Sender<()>,
    completed: Option<oneshot::Receiver<Result<()>>>,
    cancel: CancelToken,
    stats: ExecStats,
}

impl Drop for PortalCursor {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[async_trait]
impl Cursor for PortalCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
    fn stats(&self) -> ExecStats {
        self.stats
    }
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.completed.is_none() {
            return Ok(None);
        }
        let _ = self.demand.try_send(());
        let batch = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(OxynError::Cancelled),
            batch = self.events.recv() => batch,
        };
        if let Some(batch) = batch {
            self.stats.record_batch(
                u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
            );
            return Ok(Some(batch));
        }
        if let Some(done) = self.completed.take() {
            done.await.map_err(|_| {
                OxynError::Internal("the PostgreSQL preview owner stopped before cleanup".into())
            })??;
        }
        Ok(None)
    }
}

struct Pump {
    driver: oxyn_core::DriverId,
    connection: Lease,
    pid: i32,
    name: String,
    schema: SchemaRef,
    decodings: Vec<PgDecoding>,
    canceller: Arc<crate::cancel::BackendCanceller>,
    cancel: CancelToken,
    closing: CancelToken,
    verdict: VerdictSender,
}

impl Pump {
    async fn run(
        mut self,
        mut pulls: mpsc::Receiver<()>,
        send: mpsc::Sender<RecordBatch>,
    ) -> Result<()> {
        let reading = async {
            loop {
                tokio::select! {
                    biased;
                    () = self.cancel.cancelled() => return Ok(()),
                    () = self.closing.cancelled() => return Ok(()),
                    pull = pulls.recv() => if pull.is_none() { return Ok(()); },
                }
                let fetch = format!("FETCH FORWARD {WINDOW} FROM {}", self.name);
                let mut rows =
                    sqlx::query(AssertSqlSafe(fetch).into_sql_str()).fetch(&mut *self.connection);
                let mut batch = BatchAssembler::new(Arc::clone(&self.schema), &self.decodings);
                let mut count = 0;
                loop {
                    let row = tokio::select! {
                        biased;
                        () = self.cancel.cancelled() => return Ok(()),
                        () = self.closing.cancelled() => return Ok(()),
                        row = rows.try_next() => row.map_err(|error| map_stream_error(&self.driver, StatementIntent::Read, true, Bound::Internal, error))?,
                    };
                    let Some(row) = row else {
                        break;
                    };
                    count += 1;
                    batch.push(&row).map_err(|error| {
                        OxynError::driver(self.driver.clone(), ErrorClass::Permanent, error)
                    })?;
                    if batch.bytes() >= crate::cursor::BATCH_BYTE_BUDGET {
                        let batch = batch.finish().map_err(|error| {
                            OxynError::driver(self.driver.clone(), ErrorClass::Permanent, error)
                        })?;
                        tokio::select! {
                            biased;
                            () = self.cancel.cancelled() => return Ok(()),
                            () = self.closing.cancelled() => return Ok(()),
                            sent = send.send(batch) => if sent.is_err() { return Ok(()); },
                        }
                    }
                }
                if !batch.is_empty() {
                    let batch = batch.finish().map_err(|error| {
                        OxynError::driver(self.driver.clone(), ErrorClass::Permanent, error)
                    })?;
                    tokio::select! {
                        biased;
                        () = self.cancel.cancelled() => return Ok(()),
                        () = self.closing.cancelled() => return Ok(()),
                        sent = send.send(batch) => if sent.is_err() { return Ok(()); },
                    }
                }
                if count < WINDOW {
                    return Ok(());
                }
            }
        };
        let outcome: Result<()> = reading.await;
        // sqlx drains the current bounded FETCH before sending ROLLBACK. This
        // closes our non-holdable portal and preserves pre-existing temp data.
        let deadline = tokio::time::Instant::now() + CLEANUP;
        let restored = tokio::time::timeout(CLEANUP / 2, async {
            sqlx::raw_sql(SQL_ROLLBACK)
                .execute(&mut *self.connection)
                .await?;
            sqlx::raw_sql(SQL_RESET_SEARCH_PATH)
                .execute(&mut *self.connection)
                .await?;
            Ok::<_, sqlx::Error>(())
        })
        .await;
        match restored {
            Ok(Ok(())) => {
                self.connection.restored();
                self.verdict.settle(Verdict::Finished);
                outcome
            }
            _ => {
                self.connection.discard();
                let cancelled =
                    tokio::time::timeout_at(deadline, self.canceller.cancel_backend(self.pid))
                        .await;
                match cancelled {
                    Ok(Ok(())) => self.verdict.settle(Verdict::Cancelled),
                    Ok(Err(error)) => self.verdict.settle(Verdict::CancelFailed(Arc::new(error))),
                    Err(_) => {
                        self.verdict
                            .settle(Verdict::CancelFailed(Arc::new(OxynError::Timeout {
                                after: CLEANUP,
                            })))
                    }
                }
                Err(OxynError::Config("the PostgreSQL preview could not restore its session; reconnect before continuing".into()))
            }
        }
    }
}
