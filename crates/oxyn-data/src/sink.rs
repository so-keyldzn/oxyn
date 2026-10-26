//! The bridge between a driver cursor and a [`ResultBuffer`], with
//! back-pressure and cancellation.
//!
//! It is the mechanism that keeps a `SELECT *` over 500 GB from blowing up
//! memory ([I-06](../../../CLAUDE.md#i-06)). It fits in one sentence: **the
//! next batch is only requested from the server if the buffer has room**. The
//! naive loop — read everything, then store everything — puts network
//! throughput in direct competition with available RAM, and the network wins.
//!
//! # What this sink does not do
//!
//! **It applies no timeout.** `ExecLimits::timeout` is applied by
//! `oxyn-exec`, which owns the runtime, the log and the right to issue the
//! server-side cancellation ([ARCHITECTURE](../../../docs/ARCHITECTURE.md#9-execution-and-threading-model)).
//! A timeout set here would only cancel the future, leaving the query running
//! and the lock held on the database side — exactly the defect the driver
//! contract forbids.
//!
//! **It resumes nothing after a cancellation.** An abandoned `next_batch`
//! future may have been abandoned **after** consuming bytes from the stream:
//! the driver's decoder is then out of sync. The sink knows it and refuses to
//! resume — the source must be destroyed. A resumption point is designed, not
//! improvised ([rust.md](../../../.claude/rules/rust.md)).

use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use futures::future::{BoxFuture, Either, select};
use oxyn_core::{CancelToken, ExecStats, OxynError};

use crate::buffer::{BatchIndex, Pressure, ResultBuffer};
use crate::error::DataError;

/// What produces batches: a driver cursor, a file decoder, a test generator.
///
/// # For `oxyn-driver`
///
/// `Cursor` is written with `#[async_trait]`, which desugars
/// `async fn next_batch` into exactly the signature below. The adaptation
/// therefore fits in:
///
/// ```ignore
/// impl BatchSource for Box<dyn Cursor> {
///     fn schema(&self) -> SchemaRef { (**self).schema() }
///     fn next_batch(&mut self) -> BoxFuture<'_, oxyn_core::Result<Option<RecordBatch>>> {
///         Box::pin((**self).next_batch())
///     }
///     fn stats(&self) -> ExecStats { (**self).stats() }
/// }
/// ```
///
/// This trait lives here rather than in `oxyn-driver` because `oxyn-data` does
/// not depend on `oxyn-driver` — it is the other way round — and because a
/// sink must be testable without a driver.
///
/// # Cancellation
///
/// [`next_batch`](Self::next_batch) must be abandonable. After abandonment,
/// the source is considered unusable: see the module note.
pub trait BatchSource: Send {
    /// The schema of the batches. Known before the first batch, it is what lets
    /// the grid draw its columns while the rows arrive.
    fn schema(&self) -> SchemaRef;

    /// The next batch, or `None` when the stream is exhausted.
    ///
    /// The return type is the desugaring of `async fn`: it keeps the trait
    /// compatible with `dyn`, which is a hard constraint of the workspace
    /// ([ARCHITECTURE §4.1](../../../docs/ARCHITECTURE.md#41-the-traits)).
    fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>>;

    /// What the source knows about the execution: server time, rows, bytes.
    ///
    /// Queried at the end of the stream, to close the buffer.
    fn stats(&self) -> ExecStats {
        ExecStats::default()
    }
}

/// Why the sink stopped.
///
/// Only [`Exhausted`](Self::Exhausted) describes a whole result; every other
/// case produces a **truncated** result, marked as such in [`ExecStats`] and
/// therefore on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SinkOutcome {
    /// The source is exhausted: the result is complete.
    Exhausted,
    /// [`BufferLimits::max_rows`](crate::BufferLimits) is reached and rows
    /// beyond it were received: the result is certainly incomplete.
    RowLimit,
    /// [`BufferLimits::max_rows`](crate::BufferLimits) is reached before the
    /// end of the stream was observed: rows may or may not be missing.
    ///
    /// The sink says so when it was not asked to
    /// [confirm the end](BatchSink::with_end_confirmation), or when the source
    /// timed out while confirming it. It is still not a whole result: what is
    /// not proven complete is not exported.
    RowLimitUnverified,
    /// The buffer is saturated: memory budget reached, spilling forbidden or
    /// capped.
    Saturated,
    /// The [`CancelToken`] was triggered. **The source must no longer be used.**
    Cancelled,
}

impl SinkOutcome {
    /// Does the result contain every row of the query?
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Exhausted)
    }

    /// May rows be missing?
    #[must_use]
    pub const fn is_truncated(self) -> bool {
        !self.is_complete()
    }
}

/// What the sink just stored, for the caller that wants to hand control back
/// to the interface between two batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BatchProgress {
    /// Position of the batch in the buffer.
    pub index: BatchIndex,
    /// Rows of this batch.
    pub rows: usize,
    /// Total rows available after this batch.
    pub total_rows: usize,
}

/// Drains a [`BatchSource`] into a [`ResultBuffer`].
#[derive(Debug)]
pub struct BatchSink {
    buffer: Arc<ResultBuffer>,
    /// Was a `next_batch` abandoned in flight?
    ///
    /// Resuming after that would read an out-of-sync stream: the sink refuses.
    aborted: AtomicBool,
    confirm_end_at_limit: bool,
    awaits_terminal_outcome: bool,
}

impl BatchSink {
    /// A new sink feeding `buffer`.
    #[must_use]
    pub fn new(buffer: Arc<ResultBuffer>) -> Self {
        Self {
            buffer,
            aborted: AtomicBool::new(false),
            confirm_end_at_limit: false,
            awaits_terminal_outcome: false,
        }
    }

    /// Reads the source to its own end, whatever stops the result.
    ///
    /// For a statement whose effect survives a stop — a write. Abandoning its
    /// source on a cancellation, or once the buffer is full, would throw away
    /// the one answer that says whether the write was applied, and would cut
    /// the drain some drivers need to commit it. In this mode:
    ///
    /// * the token is not raced against the source: the driver watches it,
    ///   stops the server, and ends on its own verdict — a source error
    ///   [`OxynError::is_cancelled`] becomes [`SinkOutcome::Cancelled`], an
    ///   ambiguous one stays an error, and a statement that ended first ends as
    ///   if nothing had been asked;
    /// * once the buffer stops taking rows, the rest is read and discarded.
    ///
    /// **The caller bounds the wait**, as it bounds every other: this sink
    /// applies no timeout (see the module note).
    #[must_use]
    pub fn awaiting_terminal_outcome(mut self) -> Self {
        self.awaits_terminal_outcome = true;
        self
    }

    /// Probes for end-of-stream at the row limit, on the same source.
    ///
    /// At most one nonempty batch is read and discarded beyond the buffer limit.
    /// Cancellation and the caller's timeout still apply; no extra rows are stored.
    /// The probe makes the server produce one more batch: the caller only asks
    /// for it where that has no side effect. Without it, a stop at the limit is
    /// [`SinkOutcome::RowLimitUnverified`].
    #[must_use]
    pub fn with_end_confirmation(mut self) -> Self {
        self.confirm_end_at_limit = true;
        self
    }

    /// The buffer being fed. Shareable for reading during draining.
    #[must_use]
    pub fn buffer(&self) -> &Arc<ResultBuffer> {
        &self.buffer
    }

    /// Drains the source until exhaustion, saturation or cancellation.
    ///
    /// # Errors
    ///
    /// Propagates the source's error, or the buffer's if writing the spill file
    /// fails. Saturation and cancellation are **not** errors: they are
    /// [`SinkOutcome`]s.
    pub async fn drain(
        &self,
        source: &mut dyn BatchSource,
        ct: &CancelToken,
    ) -> Result<SinkOutcome, OxynError> {
        self.drain_with(source, ct, |_| {}).await
    }

    /// Like [`drain`](Self::drain), calling `on_batch` after each stored
    /// batch.
    ///
    /// It is the hook `oxyn-exec` needs to emit
    /// [`Event::BatchReady`](oxyn_core::Event) without waiting for the end of
    /// the stream — "the grid displays from the first `RecordBatch`".
    ///
    /// `on_batch` runs **inside** the draining loop: what is put there delays
    /// the next batch. Sending on a channel there, yes; drawing there, no.
    ///
    /// # Errors
    ///
    /// Those of [`drain`](Self::drain), plus [`OxynError::Internal`] if the
    /// source was already abandoned in flight during a previous call.
    pub async fn drain_with<F>(
        &self,
        source: &mut dyn BatchSource,
        ct: &CancelToken,
        mut on_batch: F,
    ) -> Result<SinkOutcome, OxynError>
    where
        F: FnMut(BatchProgress),
    {
        if self.aborted.load(Ordering::SeqCst) {
            return Err(OxynError::Internal(
                "batch source was abandoned mid-flight and cannot be resumed".to_owned(),
            ));
        }
        if self.awaits_terminal_outcome {
            return self.drain_to_end(source, on_batch).await;
        }

        loop {
            if ct.is_cancelled() {
                self.seal(source, true);
                return Ok(SinkOutcome::Cancelled);
            }

            // Back-pressure: the question to the server is only asked if the
            // answer has somewhere to go.
            let confirming_end = match self.buffer.pressure() {
                Pressure::Ready => false,
                // The buffer already dropped rows of a batch that crossed the
                // limit: the truncation is observed, there is nothing to probe.
                Pressure::RowLimit if self.buffer.stats().truncated => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::RowLimit);
                }
                Pressure::RowLimit if self.confirm_end_at_limit => true,
                Pressure::RowLimit => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::RowLimitUnverified);
                }
                Pressure::Saturated => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::Saturated);
                }
                Pressure::Complete => return Ok(SinkOutcome::Exhausted),
            };

            // The block bounds the mutable borrow of `source` by the read
            // future: without it, nothing could touch the source in the
            // branches below.
            let received = {
                let batch = pin!(source.next_batch());
                let cancel = pin!(ct.cancelled());
                match select(batch, cancel).await {
                    Either::Left((received, _)) => Some(received),
                    Either::Right(((), _)) => None,
                }
            };

            let Some(received) = received else {
                // The read future was just abandoned, perhaps after consuming
                // bytes from the stream: the source is burnt.
                self.aborted.store(true, Ordering::SeqCst);
                self.seal(source, true);
                return Ok(SinkOutcome::Cancelled);
            };

            let batch = match received {
                Ok(Some(batch)) => batch,
                Ok(None) => {
                    self.seal(source, false);
                    // A driver bounded by `ExecLimits::max_rows` ends its
                    // stream there and says in its stats whether rows were
                    // left behind: the end of *that* stream is not the end of
                    // the result.
                    return Ok(if source.stats().truncated {
                        SinkOutcome::RowLimit
                    } else {
                        SinkOutcome::Exhausted
                    });
                }
                Err(error) if error.is_cancelled() => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::Cancelled);
                }
                // The N rows arrived; only the answer about the next one did
                // not. That is a limit left unverified, not a failed read.
                Err(OxynError::Timeout { .. }) if confirming_end => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::RowLimitUnverified);
                }
                Err(error) => {
                    // Rows already received stay readable; the buffer is
                    // closed so that the interface stops waiting for more.
                    self.seal(source, true);
                    return Err(error);
                }
            };

            if confirming_end {
                if batch.num_rows() == 0 {
                    continue;
                }
                self.seal(source, true);
                return Ok(SinkOutcome::RowLimit);
            }

            match self.buffer.push(batch) {
                // Empty batch: the source is allowed to produce one, there is
                // nothing to report.
                Ok(None) => {}
                Ok(Some(index)) => {
                    // Read back from the buffer, never from the pushed batch:
                    // the buffer decides how many rows it keeps.
                    let rows = self.buffer.batch_rows(index).unwrap_or(0);
                    let total_rows = self.buffer.row_count();
                    on_batch(BatchProgress {
                        index,
                        rows,
                        total_rows,
                    });
                }
                Err(DataError::Full { .. }) => {
                    let outcome = match self.buffer.pressure() {
                        Pressure::RowLimit => SinkOutcome::RowLimit,
                        _ => SinkOutcome::Saturated,
                    };
                    self.seal(source, true);
                    return Ok(outcome);
                }
                Err(other) => {
                    self.seal(source, true);
                    return Err(other.into());
                }
            }
        }
    }

    /// The loop of [`awaiting_terminal_outcome`](Self::awaiting_terminal_outcome).
    async fn drain_to_end<F>(
        &self,
        source: &mut dyn BatchSource,
        mut on_batch: F,
    ) -> Result<SinkOutcome, OxynError>
    where
        F: FnMut(BatchProgress),
    {
        // Why the buffer stopped taking rows, once it did; and a failure to
        // store, reported only after the source has said how it ended.
        let mut stopped: Option<SinkOutcome> = None;
        let mut store_failure: Option<OxynError> = None;
        loop {
            if stopped.is_none() && store_failure.is_none() {
                stopped = match self.buffer.pressure() {
                    Pressure::Ready => None,
                    Pressure::RowLimit if self.buffer.stats().truncated => {
                        Some(SinkOutcome::RowLimit)
                    }
                    Pressure::RowLimit => Some(SinkOutcome::RowLimitUnverified),
                    Pressure::Saturated => Some(SinkOutcome::Saturated),
                    Pressure::Complete => return Ok(SinkOutcome::Exhausted),
                };
            }

            let batch = match source.next_batch().await {
                Ok(Some(batch)) => batch,
                Ok(None) => {
                    let truncated = stopped.is_some() || source.stats().truncated;
                    self.seal(source, truncated);
                    if let Some(failure) = store_failure {
                        return Err(failure);
                    }
                    return Ok(stopped.unwrap_or(if truncated {
                        SinkOutcome::RowLimit
                    } else {
                        SinkOutcome::Exhausted
                    }));
                }
                Err(error) if error.is_cancelled() => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::Cancelled);
                }
                Err(error) => {
                    self.seal(source, true);
                    return Err(error);
                }
            };

            if store_failure.is_some() {
                continue;
            }
            if let Some(outcome) = stopped {
                // A row the limit kept out: the result is certainly cut.
                if outcome == SinkOutcome::RowLimitUnverified && batch.num_rows() > 0 {
                    stopped = Some(SinkOutcome::RowLimit);
                }
                continue;
            }

            match self.buffer.push(batch) {
                Ok(None) => {}
                Ok(Some(index)) => {
                    let rows = self.buffer.batch_rows(index).unwrap_or(0);
                    let total_rows = self.buffer.row_count();
                    on_batch(BatchProgress {
                        index,
                        rows,
                        total_rows,
                    });
                }
                Err(DataError::Full { .. }) => {
                    stopped = Some(match self.buffer.pressure() {
                        Pressure::RowLimit => SinkOutcome::RowLimit,
                        _ => SinkOutcome::Saturated,
                    });
                }
                Err(other) => store_failure = Some(other.into()),
            }
        }
    }

    /// Closes the buffer with the source's measurements.
    fn seal(&self, source: &dyn BatchSource, truncated: bool) {
        if truncated {
            self.buffer.mark_truncated();
        }
        self.buffer.mark_complete(source.stats());
    }
}

#[cfg(test)]
mod tests;
