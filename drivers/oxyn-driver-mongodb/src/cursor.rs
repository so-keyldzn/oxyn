//! The documents of one execution, as a stream of byte-bounded batches.
//!
//! A server batch — `firstBatch`, then each `getMore` — holds at most 16 MiB
//! of documents (MongoDB manual, cursor batches, consulted on 2026-10-22). The
//! `Feed` keeps **one** such batch, as owned documents, and the cursor turns
//! it into Arrow batches of at most [`BATCH_BYTE_BUDGET`] bytes: the client
//! never holds more than a server batch plus an Arrow batch, whatever the size
//! of the result ([I-06](../../../CLAUDE.md#i-06)). Nothing calls `to_vec` on
//! a cursor.
//!
//! The first documents are the **sample** the schema is inferred from
//! ([`crate::decode`]): they are read before [`Session::execute`] returns,
//! since the schema is known before the first batch, and they are the first
//! rows streamed.
//!
//! A `bulkWrite` streams its per-operation results the same way, each
//! `getMore` under the execution's logical session; its first row is the
//! reply's summary.
//!
//! [`Session::execute`]: oxyn_driver::Session::execute

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use bson::raw::RawBsonRef;
use bson::{RawDocument, RawDocumentBuf};
use futures::StreamExt as _;
use mongodb::raw_batch_cursor::SessionRawBatchCursor;
use oxyn_core::{
    DriverId, ErrorClass, ExecStats, OxynError, Result, StatementHandle, StatementIntent,
};
use oxyn_driver::Cursor;

use crate::cancel::{Leased, Registry, Running};
use crate::decode::{Assembler, Plan};
use crate::error::{MongoError, map_statement_error, write_concern_failure};

/// Bytes beyond which a batch is closed: the PostgreSQL and MySQL drivers'
/// budget.
pub const BATCH_BYTE_BUDGET: usize = 1 << 20;

/// Rows beyond which a batch is closed whatever its size.
pub const BATCH_ROW_CEILING: usize = 8_192;

/// Documents the schema inference reads at most before the first batch.
pub const SAMPLE_DOCUMENTS: usize = 1_000;

/// Bytes of documents the inference reads at most: a sample of 16 MiB
/// documents stops at the first.
pub const SAMPLE_BYTES: usize = 8 << 20;

/// Where the documents come from.
enum Source {
    /// A server cursor, iterated in the logical session that opened it, and
    /// given back when the variant is dropped: after the cursor, declared
    /// first. Boxed: one per execution, and the session is large.
    Server {
        cursor: Box<SessionRawBatchCursor>,
        session: Box<Leased>,
    },
    /// A reply that is the whole result, already in `pending`, or a cursor
    /// the server has no more batches for.
    Done,
}

/// The documents not yet streamed, and where the next ones come from.
pub(crate) struct Feed {
    pending: VecDeque<RawDocumentBuf>,
    source: Source,
    driver: DriverId,
    intent: StatementIntent,
    running: Arc<Running>,
    /// The command carried a password: no server text in its errors.
    withheld: bool,
    /// The next server batch is a `bulkWrite`'s first reply, whose top-level
    /// fields become the first row.
    summarized: bool,
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("pending", &self.pending.len())
            .field("open", &matches!(self.source, Source::Server { .. }))
            .finish_non_exhaustive()
    }
}

impl Feed {
    /// A result held whole in one reply.
    pub(crate) fn reply(
        document: RawDocumentBuf,
        driver: DriverId,
        intent: StatementIntent,
        running: Arc<Running>,
    ) -> Self {
        Self {
            pending: VecDeque::from([document]),
            source: Source::Done,
            driver,
            intent,
            running,
            withheld: false,
            summarized: false,
        }
    }

    /// The batches of a server cursor; when `summarized`, the first reply's
    /// fields outside `cursor` first, as one row.
    pub(crate) fn server(
        cursor: SessionRawBatchCursor,
        session: Leased,
        driver: DriverId,
        intent: StatementIntent,
        withheld: bool,
        summarized: bool,
    ) -> Self {
        let running = Arc::clone(session.running());
        Self {
            pending: VecDeque::new(),
            source: Source::Server {
                cursor: Box::new(cursor),
                session: Box::new(session),
            },
            driver,
            intent,
            running,
            withheld,
            summarized,
        }
    }

    fn fail(&self, error: &mongodb::error::Error) -> OxynError {
        map_statement_error(
            &self.driver,
            self.intent,
            error,
            self.running.was_killed(),
            self.withheld,
        )
    }

    /// Reads server batches until the sample bounds are reached or the
    /// cursor is exhausted.
    ///
    /// # Errors
    /// The server's, classified.
    pub(crate) async fn fill_sample(&mut self) -> Result<()> {
        let mut bytes = 0_usize;
        loop {
            if self.pending.len() >= SAMPLE_DOCUMENTS || bytes >= SAMPLE_BYTES {
                return Ok(());
            }
            let before = self.pending.len();
            if !self.load_next().await? {
                return Ok(());
            }
            // An empty batch from an open cursor is a server waiting for
            // data: the sample is what has arrived, not what may come.
            if self.pending.len() == before {
                return Ok(());
            }
            bytes = self
                .pending
                .iter()
                .skip(before)
                .fold(bytes, |sum, document| {
                    sum.saturating_add(document.as_bytes().len())
                });
        }
    }

    /// The sample: the first documents held, at most [`SAMPLE_DOCUMENTS`] and
    /// about [`SAMPLE_BYTES`] — always the first one, whatever its size. A
    /// server batch may bring more; they are streamed, not sampled, so the
    /// sample's size does not depend on how the server cut its batches.
    pub(crate) fn sample(&self) -> impl Iterator<Item = &bson::RawDocument> {
        let mut bytes = 0_usize;
        self.pending
            .iter()
            .take(SAMPLE_DOCUMENTS)
            .take_while(move |document| {
                let within = bytes < SAMPLE_BYTES;
                bytes = bytes.saturating_add(document.as_bytes().len());
                within
            })
            .map(AsRef::as_ref)
    }

    /// The next document, reading the next server batch when needed.
    async fn next_document(&mut self) -> Result<Option<RawDocumentBuf>> {
        loop {
            if let Some(document) = self.pending.pop_front() {
                return Ok(Some(document));
            }
            if !self.load_next().await? {
                return Ok(None);
            }
        }
    }

    /// Reads one server batch into `pending`; `false` once there is none.
    async fn load_next(&mut self) -> Result<bool> {
        let Source::Server { cursor, session } = &mut self.source else {
            return Ok(false);
        };
        let session = session.session()?;
        let next = cursor.stream(session).next().await;
        match next {
            None => {
                self.source = Source::Done;
                Ok(false)
            }
            Some(Err(error)) => {
                self.source = Source::Done;
                Err(self.fail(&error))
            }
            Some(Ok(batch)) => {
                let reply = batch.as_raw_document();
                let summarized = std::mem::take(&mut self.summarized);
                if summarized
                    && !self.intent.is_read_only()
                    && let Some(failure) = write_concern_failure(reply, self.withheld)
                {
                    self.source = Source::Done;
                    return Err(failure);
                }
                take_batch(&mut self.pending, reply, summarized)
                    .map_err(|message| self.protocol(message))?;
                Ok(true)
            }
        }
    }

    fn protocol(&self, message: impl Into<String>) -> OxynError {
        let class = if self.intent.is_read_only() {
            ErrorClass::Permanent
        } else {
            ErrorClass::Ambiguous
        };
        OxynError::driver(self.driver.clone(), class, MongoError::protocol(message))
    }
}

/// Appends the documents of one cursor reply — `firstBatch` or `nextBatch` —
/// to `pending`; when `summarized`, the reply's fields outside `cursor` first,
/// as one document. A `bulkWrite`'s counts and `nErrors` are there, so a row
/// bound that stops the per-operation results never hides how many failed.
///
/// # Errors
/// What cannot be read, worded for [`MongoError::protocol`].
fn take_batch(
    pending: &mut VecDeque<RawDocumentBuf>,
    reply: &RawDocument,
    summarized: bool,
) -> std::result::Result<(), String> {
    let cursor = reply
        .get_document("cursor")
        .map_err(|_| "a cursor reply holds no cursor document".to_owned())?;
    let documents = cursor
        .get_array("firstBatch")
        .or_else(|_| cursor.get_array("nextBatch"))
        .map_err(|_| "a cursor reply holds no batch".to_owned())?;
    if summarized {
        let mut summary = RawDocumentBuf::new();
        for element in reply {
            let (key, value) =
                element.map_err(|error| format!("a cursor reply cannot be read: {error}"))?;
            if key.as_str() != "cursor" {
                summary.append(key, value);
            }
        }
        pending.push_back(summary);
    }
    for item in documents {
        match item {
            Ok(RawBsonRef::Document(document)) => pending.push_back(document.to_owned()),
            Ok(_) => return Err("a cursor batch holds a value that is not a document".to_owned()),
            Err(error) => return Err(format!("a cursor batch cannot be read: {error}")),
        }
    }
    Ok(())
}

/// The batches of one execution.
pub struct MongoCursor {
    handle: StatementHandle,
    plan: Plan,
    assembler: Assembler,
    feed: Feed,
    registry: Arc<Registry>,
    driver: DriverId,
    rows: u64,
    stats: ExecStats,
    started: Instant,
    done: bool,
    /// A failure met after rows were accumulated, reported once they are.
    failed: Option<OxynError>,
}

impl std::fmt::Debug for MongoCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoCursor")
            .field("handle", &self.handle)
            .field("rows", &self.rows)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl MongoCursor {
    pub(crate) fn new(
        handle: StatementHandle,
        plan: Plan,
        feed: Feed,
        registry: Arc<Registry>,
        driver: DriverId,
        started: Instant,
    ) -> Self {
        let assembler = plan.assembler();
        Self {
            handle,
            plan,
            assembler,
            feed,
            registry,
            driver,
            rows: 0,
            stats: ExecStats::default(),
            started,
            done: false,
            failed: None,
        }
    }

    fn close_batch(&mut self) -> Result<Option<RecordBatch>> {
        let batch = self.assembler.finish().map_err(|error| {
            OxynError::Internal(format!(
                "a MongoDB batch disagrees with its schema: {error}"
            ))
        })?;
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
        self.stats.total_time = self.started.elapsed();
        Ok(Some(batch))
    }

    fn finish(&mut self) {
        self.done = true;
        self.registry.remove(self.handle);
    }
}

impl Drop for MongoCursor {
    fn drop(&mut self) {
        // The library sends `killCursors` for a cursor dropped unexhausted;
        // the handle stops naming anything to cancel.
        self.registry.remove(self.handle);
    }
}

#[async_trait]
impl Cursor for MongoCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        self.plan.schema()
    }

    /// The rows accumulated before a failure are delivered first, as their own
    /// batch; the failure comes at the next call. A refused document never
    /// costs the rows the user could have seen.
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        if self.done {
            return Ok(None);
        }
        loop {
            if self.assembler.rows() > 0
                && (self.assembler.bytes() >= BATCH_BYTE_BUDGET
                    || self.assembler.rows() >= BATCH_ROW_CEILING)
            {
                return self.close_batch();
            }
            let failure = match self.feed.next_document().await {
                Ok(None) => {
                    self.finish();
                    if self.assembler.rows() > 0 {
                        return self.close_batch();
                    }
                    self.stats.total_time = self.started.elapsed();
                    return Ok(None);
                }
                Ok(Some(document)) => {
                    self.rows = self.rows.saturating_add(1);
                    match self.assembler.push(&document, self.rows) {
                        Ok(()) => continue,
                        Err(error) => {
                            OxynError::driver(self.driver.clone(), ErrorClass::Permanent, error)
                        }
                    }
                }
                Err(error) => error,
            };
            self.finish();
            if self.assembler.rows() > 0 {
                self.failed = Some(failure);
                return self.close_batch();
            }
            return Err(failure);
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

#[cfg(test)]
mod tests;
