//! The task that runs one statement: send it, read its header, stream its
//! rows into batches, and stop it on the server when asked.
//!
//! # One task per execution, its handle held
//!
//! The task owns the HTTP response. The cursor receives its batches through a
//! one-slot channel — the back-pressure: the task reads the network only as
//! fast as the cursor drains. When the cursor goes away mid-stream, the
//! channel closes and the task kills the statement before it ends, so a closed
//! tab does not leave an aggregation running on the server. The session holds
//! every task in a `JoinSet`: none outlives it.
//!
//! # Stopping, and what a stop proves
//!
//! A stop sends `KILL QUERY WHERE query_id = …` as a separate request — the
//! `query_id` is the driver's, unique to this execution, so the kill cannot
//! land on another statement — and then waits, at most [`DRAIN_BOUND`], for the
//! targeted response to end. What the stop is reported as follows from the
//! statement's intent, never from the kill's answer alone:
//!
//! * a **read** ends on [`OxynError::Cancelled`]: it wrote nothing;
//! * anything else ends on [`OxynError::OutcomeUnknown`]: ClickHouse has no
//!   transaction to roll back, an `INSERT` keeps the blocks it already wrote,
//!   and a mutation (`ALTER … UPDATE`) keeps running after `KILL QUERY`
//!   ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md));
//! * a statement whose response completed before the kill took effect ran to
//!   its end, and is reported as such: a write that landed is not cancelled.

use std::sync::Arc;
use std::time::Duration;

use arrow::datatypes::{Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementIntent};
use reqwest::Response;
use tokio::sync::{mpsc, oneshot};

use crate::body::{Body, header};
use crate::decode::RowDecoder;
use crate::error::{
    QUERY_WAS_CANCELLED, map_decode_error, map_server_error, map_transport_error, protocol_error,
};
use crate::http::{Call, FORMAT, Failure, Http, Reply};
use crate::measure::{DecodeError, measure};
use crate::schema::schema_for;

/// A batch is closed at this many decoded bytes ([I-06](../../../CLAUDE.md#i-06)).
pub const BATCH_BYTE_BUDGET: usize = 1 << 20;
/// …or at this many rows, so that narrow rows still reach the grid early.
pub const BATCH_ROW_CEILING: usize = 8_192;
/// How long a stopped statement may take to end once the kill is sent.
pub(crate) const DRAIN_BOUND: Duration = Duration::from_secs(5);
/// How long the kill request itself may take.
pub(crate) const KILL_BOUND: Duration = Duration::from_secs(10);

/// The kill, with the statement's id bound as a parameter: the id is the
/// driver's own UUID, and is still never concatenated.
pub(crate) const SQL_KILL: &str = "KILL QUERY WHERE query_id = {id:String} ASYNC";

/// What a stopped write is reported as.
pub(crate) const STOPPED_WRITE: &str = "the statement was stopped after it reached the server: \
    ClickHouse has no transaction to roll it back, an INSERT keeps the blocks it already wrote, \
    and a mutation (ALTER … UPDATE/DELETE) goes on running after KILL QUERY — check the data, \
    and system.mutations, before running it again";

/// Why a result that chose its own format is refused.
const OWN_FORMAT: &str = "the statement chooses its own output format (a FORMAT clause): Oxyn \
    reads results as RowBinaryWithNamesAndTypes, so remove the clause to see the rows";

/// What the statement opened with: its schema, and the rows a write reports.
#[derive(Debug)]
pub struct Opening {
    /// The schema of every batch.
    pub schema: SchemaRef,
    /// Rows written, for a statement that returns no result set.
    pub written_rows: Option<u64>,
}

/// One execution, as the session hands it to the task.
#[derive(Debug)]
pub struct Execution {
    /// The endpoint and the account.
    pub http: Arc<Http>,
    /// The driver's identifier, for errors.
    pub driver: DriverId,
    /// The request.
    pub call: Call,
    /// What the statement does, from the executor's reclassification.
    pub intent: StatementIntent,
    /// Whether this account may kill its own statements.
    pub can_kill: bool,
    /// Whether the request asked for `JSON` as text: an account that takes no
    /// setting gets it in a binary form the decoder does not read.
    pub json_as_text: bool,
}

/// Runs an execution to its end.
///
/// `opened` receives the schema, or the failure that came first; `batches`
/// then receives every batch and, last, the failure that ended the stream if
/// one did. The task returns when the stream ends, is stopped, or is
/// abandoned.
pub async fn run(
    execution: Execution,
    token: CancelToken,
    opened: oneshot::Sender<Result<Opening>>,
    batches: mpsc::Sender<Result<RecordBatch>>,
) {
    if token.is_cancelled() {
        // Nothing was sent: whatever the statement, nothing was applied.
        let _ = opened.send(Err(OxynError::Cancelled));
        return;
    }
    let reply = {
        let send = execution.http.send(&execution.call);
        tokio::pin!(send);
        tokio::select! {
            biased;
            reply = &mut send => reply,
            () = token.cancelled() => {
                let outcome = stop_before_reply(&execution, send).await;
                let _ = opened.send(outcome);
                return;
            }
        }
    };
    let reply = match reply {
        Ok(reply) => reply,
        Err(failure) => {
            let _ = opened.send(Err(failure_error(&execution, &failure)));
            return;
        }
    };
    let Some(format) = reply.format.as_deref() else {
        // No result set: a write or a DDL, already complete.
        let _ = opened.send(Ok(Opening {
            schema: Arc::new(Schema::empty()),
            written_rows: reply.written_rows,
        }));
        return;
    };
    if format != FORMAT {
        execution.kill().await;
        let _ = opened.send(Err(protocol_error(
            &execution.driver,
            execution.intent,
            OWN_FORMAT,
        )));
        return;
    }
    Rows::new(execution, reply)
        .stream(token, opened, batches)
        .await;
}

/// The stop of a statement whose response had not come yet.
async fn stop_before_reply(
    execution: &Execution,
    send: std::pin::Pin<&mut impl Future<Output = std::result::Result<Reply, Failure>>>,
) -> Result<Opening> {
    execution.kill().await;
    let ended = tokio::time::timeout(DRAIN_BOUND, send).await;
    match ended {
        // It ran to its end before the kill took effect.
        Ok(Ok(reply)) if reply.format.is_none() => Ok(Opening {
            schema: Arc::new(Schema::empty()),
            written_rows: reply.written_rows,
        }),
        Ok(Err(Failure::Server(error))) if error.code != Some(QUERY_WAS_CANCELLED) => {
            Err(map_server_error(&execution.driver, execution.intent, error))
        }
        // Killed, cut, still running, or about to stream rows nobody waits
        // for any more.
        _ => Err(stopped(execution.intent)),
    }
}

/// What a statement stopped after it was sent is reported as.
pub(crate) fn stopped(intent: StatementIntent) -> OxynError {
    if intent.is_read_only() {
        OxynError::Cancelled
    } else {
        OxynError::OutcomeUnknown(STOPPED_WRITE.to_owned())
    }
}

/// The error of a request that failed before its body.
fn failure_error(execution: &Execution, failure: &Failure) -> OxynError {
    match failure {
        Failure::Transport(error) => {
            map_transport_error(&execution.driver, execution.intent, true, error)
        }
        Failure::Server(error) => {
            map_server_error(&execution.driver, execution.intent, error.clone())
        }
    }
}

impl Execution {
    /// Kills this execution on the server, when the account may.
    ///
    /// An account that may not still has its response dropped by the caller:
    /// a read then ends at its next write to the closed connection, or at
    /// once under `cancel_http_readonly_queries_on_client_close`.
    async fn kill(&self) {
        if self.can_kill {
            // The stop is reported from the statement's own end, whatever
            // the kill answers.
            let _ = kill(&self.http, &self.call.query_id).await;
        }
    }
}

/// Sends `KILL QUERY` for `query_id`, bounded by [`KILL_BOUND`].
///
/// Killing a statement that already ended is no error for the server, and a
/// kill that fails is not the statement's failure: the caller reports the
/// stop from the statement's own end.
///
/// # Errors
/// [`OxynError::Timeout`] if the server does not answer in time, or the
/// server's refusal.
pub async fn kill(http: &Http, query_id: &str) -> Result<()> {
    let call = Call {
        text: SQL_KILL.to_owned(),
        query_id: uuid::Uuid::new_v4().to_string(),
        params: vec![("id", query_id.to_owned())],
        ..Call::default()
    };
    let driver = DriverId::clickhouse();
    let outcome = tokio::time::timeout(KILL_BOUND, async {
        match http.send(&call).await {
            Ok(mut reply) => {
                while let Ok(Some(_)) = reply.response.chunk().await {}
                Ok(())
            }
            Err(Failure::Server(error)) => {
                Err(map_server_error(&driver, StatementIntent::Read, error))
            }
            Err(Failure::Transport(error)) => Err(map_transport_error(
                &driver,
                StatementIntent::Read,
                true,
                &error,
            )),
        }
    })
    .await;
    outcome.unwrap_or(Err(OxynError::Timeout { after: KILL_BOUND }))
}

/// The rows of a 200 response.
struct Rows {
    execution: Execution,
    response: Response,
    body: Body,
    zone: Option<String>,
}

impl Rows {
    fn new(execution: Execution, reply: Reply) -> Self {
        Self {
            body: Body::new(reply.exception_tag.as_deref()),
            zone: reply.zone,
            execution,
            response: reply.response,
        }
    }

    async fn stream(
        mut self,
        token: CancelToken,
        opened: oneshot::Sender<Result<Opening>>,
        batches: mpsc::Sender<Result<RecordBatch>>,
    ) {
        let mut decoder = match self.open(&token).await {
            Ok(decoder) => decoder,
            Err(error) => {
                let _ = opened.send(Err(error));
                return;
            }
        };
        let schema = schema_for(decoder.columns(), self.zone.as_deref());
        if opened
            .send(Ok(Opening {
                schema,
                written_rows: None,
            }))
            .is_err()
        {
            // `execute` was dropped before the schema came back.
            self.execution.kill().await;
            return;
        }
        if let Err(error) = self.rows(&mut decoder, &token, &batches).await {
            let _ = batches.send(Err(error)).await;
        }
    }

    /// Reads the header: the columns' names and types.
    async fn open(&mut self, token: &CancelToken) -> Result<RowDecoder> {
        let (columns, end) = loop {
            match header(self.body.ready()) {
                Ok(Some(parsed)) => break parsed,
                Ok(None) if self.body.rows_closed() => return Err(self.closed_early()),
                Ok(None) => self.read(token).await?,
                Err(error) => {
                    self.execution.kill().await;
                    return Err(map_decode_error(
                        &self.execution.driver,
                        self.execution.intent,
                        &error,
                    ));
                }
            }
        };
        self.body.consume(end);
        let json_as_text = self.execution.json_as_text;
        if let Some(column) = columns.iter().find(|column| {
            column.ty.unsupported().is_some() || (!json_as_text && column.ty.contains_json())
        }) {
            let message = format!(
                "column `{}` has type `{}`, which this driver does not decode yet: cast it \
                 in the query, for instance `toString({})`",
                column.name, column.raw_type, column.name
            );
            self.execution.kill().await;
            return Err(protocol_error(
                &self.execution.driver,
                self.execution.intent,
                &message,
            ));
        }
        let schema = schema_for(&columns, self.zone.as_deref());
        Ok(RowDecoder::new(columns, schema, self.zone.as_deref()))
    }

    /// Decodes every row, emitting a batch each time one is full.
    async fn rows(
        &mut self,
        decoder: &mut RowDecoder,
        token: &CancelToken,
        batches: &mpsc::Sender<Result<RecordBatch>>,
    ) -> Result<()> {
        let execution_intent = self.execution.intent;
        loop {
            loop {
                let ready = self.body.ready();
                let Some(length) = measure(decoder.columns(), ready).map_err(|error| {
                    map_decode_error(&self.execution.driver, execution_intent, &error)
                })?
                else {
                    break;
                };
                let row = ready.get(..length).ok_or_else(|| {
                    map_decode_error(
                        &self.execution.driver,
                        execution_intent,
                        &DecodeError::Truncated,
                    )
                })?;
                decoder.append(row).map_err(|error| {
                    map_decode_error(&self.execution.driver, execution_intent, &error)
                })?;
                self.body.consume(length);
                if decoder.bytes() >= BATCH_BYTE_BUDGET || decoder.rows() >= BATCH_ROW_CEILING {
                    self.emit(decoder, token, batches).await?;
                }
            }
            if self.body.rows_closed() {
                break;
            }
            let read = tokio::select! {
                biased;
                () = batches.closed() => None,
                read = self.read(token) => Some(read),
            };
            match read {
                Some(read) => read?,
                // The cursor is gone: nobody reads the rest.
                None => {
                    self.execution.kill().await;
                    return Ok(());
                }
            }
        }
        if decoder.rows() > 0 {
            self.emit(decoder, token, batches).await?;
        }
        if let Some(error) = self.body.exception() {
            return Err(map_server_error(
                &self.execution.driver,
                execution_intent,
                error,
            ));
        }
        if self.body.is_drained() {
            Ok(())
        } else {
            Err(map_decode_error(
                &self.execution.driver,
                execution_intent,
                &DecodeError::Truncated,
            ))
        }
    }

    /// Hands a full batch to the cursor, waiting for it to take it.
    async fn emit(
        &mut self,
        decoder: &mut RowDecoder,
        token: &CancelToken,
        batches: &mpsc::Sender<Result<RecordBatch>>,
    ) -> Result<()> {
        let batch = decoder.finish().map_err(|error| {
            map_decode_error(&self.execution.driver, self.execution.intent, &error)
        })?;
        let sent = tokio::select! {
            biased;
            () = token.cancelled() => None,
            sent = batches.send(Ok(batch)) => Some(sent.is_ok()),
        };
        match sent {
            None => Err(self.stop().await),
            Some(true) => Ok(()),
            // The cursor is gone; the loop sees the closed channel next.
            Some(false) => Ok(()),
        }
    }

    /// Reads the next chunk, or stops the statement if `token` fires first.
    async fn read(&mut self, token: &CancelToken) -> Result<()> {
        let chunk = tokio::select! {
            biased;
            () = token.cancelled() => None,
            chunk = self.response.chunk() => Some(chunk),
        };
        let Some(chunk) = chunk else {
            return Err(self.stop().await);
        };
        match chunk {
            Ok(Some(bytes)) => self.body.push(&bytes),
            Ok(None) => self.body.end(),
            Err(error) => {
                return Err(map_transport_error(
                    &self.execution.driver,
                    self.execution.intent,
                    true,
                    &error,
                ));
            }
        }
        Ok(())
    }

    /// Kills the statement, waits briefly for its response to end, and says
    /// what the stop means.
    async fn stop(&mut self) -> OxynError {
        self.execution.kill().await;
        let _ = tokio::time::timeout(DRAIN_BOUND, async {
            while let Ok(Some(_)) = self.response.chunk().await {}
        })
        .await;
        stopped(self.execution.intent)
    }

    /// The body ended before the header did.
    fn closed_early(&self) -> OxynError {
        match self.body.exception() {
            Some(error) => map_server_error(&self.execution.driver, self.execution.intent, error),
            None => map_decode_error(
                &self.execution.driver,
                self.execution.intent,
                &DecodeError::Truncated,
            ),
        }
    }
}
