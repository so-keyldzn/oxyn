//! Hrana 3 cursor transport for continuable previews, owned through cleanup.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{
    CancelToken, DriverId, ExecRequest, ExecStats, OxynError, Result, StatementHandle,
};
use oxyn_driver::Cursor;
use tokio::sync::{mpsc, oneshot};

use crate::http::Http;
use crate::session::Registered;

mod framing;
mod owner;
mod rows;
mod worker;

pub(crate) use owner::Tasks;
type Terminal = Arc<Mutex<Option<OxynError>>>;

pub(crate) struct Open {
    pub http: Arc<Http>,
    pub driver: DriverId,
    pub request: ExecRequest,
    pub handle: StatementHandle,
    pub token: CancelToken,
    pub registered: Registered,
}

struct Abandoned(Option<CancelToken>);
impl Drop for Abandoned {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}

pub(crate) struct StreamingCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    token: CancelToken,
    requests: mpsc::Sender<()>,
    replies: mpsc::Receiver<Option<RecordBatch>>,
    terminal: Terminal,
    started: Instant,
    stats: ExecStats,
    done: bool,
}

impl StreamingCursor {
    pub async fn start(open: Open, tasks: &Arc<Tasks>) -> Result<Self> {
        let started = Instant::now();
        let handle = open.handle;
        let bound = open
            .request
            .limits
            .timeout
            .unwrap_or(LIFETIME)
            .min(LIFETIME);
        let token = open.token.clone();
        let mut abandoned = Abandoned(Some(token.clone()));
        let terminal = Arc::new(Mutex::new(None));
        let (send, ready) = oneshot::channel();
        let (requests, receive) = mpsc::channel(1);
        let (reply, replies) = mpsc::channel(1);
        tasks.spawn(worker::run(
            open,
            send,
            receive,
            reply,
            Arc::clone(&terminal),
        ))?;
        let schema = tokio::select! {
            biased;
            () = token.cancelled() => return Err(OxynError::Cancelled),
            () = tokio::time::sleep(bound) => return Err(OxynError::Timeout { after: bound }),
            ready = ready => ready.map_err(|_| OxynError::Internal("the Hrana cursor opener stopped".into()))??,
        };
        abandoned.0 = None;
        Ok(Self {
            handle,
            schema,
            token,
            requests,
            replies,
            terminal,
            started,
            stats: ExecStats::default(),
            done: false,
        })
    }

    fn terminal_error(&mut self) -> OxynError {
        let cancelled = self.token.is_cancelled();
        self.done = true;
        self.token.cancel();
        self.terminal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| {
                if cancelled {
                    OxynError::Cancelled
                } else {
                    OxynError::Internal("the Hrana cursor stopped before its result ended".into())
                }
            })
    }
}

impl Drop for StreamingCursor {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

#[async_trait]
impl Cursor for StreamingCursor {
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
        if self.done {
            return Ok(None);
        }
        if self.token.is_cancelled() {
            return Err(self.terminal_error());
        }
        let mut abandoned = Abandoned(Some(self.token.clone()));
        if self.requests.send(()).await.is_err() {
            return Err(self.terminal_error());
        }
        let Some(batch) = self.replies.recv().await else {
            return Err(self.terminal_error());
        };
        abandoned.0 = None;
        self.stats.total_time = self.started.elapsed();
        if let Some(batch) = &batch {
            self.stats.record_batch(
                u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
            );
        } else {
            self.done = true;
        }
        Ok(batch)
    }
}

const LIFETIME: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests;
