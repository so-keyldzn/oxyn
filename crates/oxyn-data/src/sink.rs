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
}

impl BatchSink {
    /// A new sink feeding `buffer`.
    #[must_use]
    pub fn new(buffer: Arc<ResultBuffer>) -> Self {
        Self {
            buffer,
            aborted: AtomicBool::new(false),
            confirm_end_at_limit: false,
        }
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

    /// Closes the buffer with the source's measurements.
    fn seal(&self, source: &dyn BatchSource, truncated: bool) {
        if truncated {
            self.buffer.mark_truncated();
        }
        self.buffer.mark_complete(source.stats());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use futures::executor::block_on;

    use super::*;
    use crate::buffer::BufferLimits;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]))
    }

    fn batch_of(rows: usize) -> RecordBatch {
        let values: Vec<i32> = (0..rows)
            .map(|i| i32::try_from(i).unwrap_or(i32::MAX))
            .collect();
        RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(values))])
            .expect("the column matches the schema built just above")
    }

    /// Scripted source that counts how many times a batch was requested from
    /// it: it is **this counter** that proves back-pressure, not the buffer's
    /// content.
    #[derive(Debug)]
    struct ScriptedSource {
        remaining: Vec<RecordBatch>,
        pull_count: usize,
        error: Option<OxynError>,
        /// A driver that capped the stream itself and left rows behind.
        truncated: bool,
    }

    impl ScriptedSource {
        fn new(batches: Vec<RecordBatch>) -> Self {
            let mut remaining = batches;
            remaining.reverse();
            Self {
                remaining,
                pull_count: 0,
                error: None,
                truncated: false,
            }
        }

        fn failing(error: OxynError) -> Self {
            Self {
                remaining: Vec::new(),
                pull_count: 0,
                error: Some(error),
                truncated: false,
            }
        }
    }

    impl BatchSource for ScriptedSource {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>> {
            self.pull_count = self.pull_count.saturating_add(1);
            if let Some(error) = self.error.take() {
                return Box::pin(async move { Err(error) });
            }
            let next = self.remaining.pop();
            Box::pin(async move { Ok(next) })
        }

        fn stats(&self) -> ExecStats {
            ExecStats {
                batches: u64::try_from(self.pull_count).unwrap_or(u64::MAX),
                truncated: self.truncated,
                ..ExecStats::default()
            }
        }
    }

    #[test]
    fn an_exhausted_source_closes_the_result() {
        let buf = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let sink = BatchSink::new(Arc::clone(&buf));
        let mut source = ScriptedSource::new(vec![batch_of(10), batch_of(10), batch_of(5)]);
        let ct = CancelToken::new();

        let outcome = block_on(sink.drain(&mut source, &ct)).expect("draining without error");

        assert_eq!(outcome, SinkOutcome::Exhausted);
        assert!(outcome.is_complete());
        assert_eq!(buf.row_count(), 25);
        assert!(buf.is_complete());
        assert!(!buf.stats().truncated);
    }

    #[test]
    fn bounded_preview_confirms_end_without_accepting_extra_rows() {
        for (batches, expected, calls) in [
            (vec![batch_of(2)], SinkOutcome::Exhausted, 2),
            (
                vec![batch_of(2), batch_of(1), batch_of(10)],
                SinkOutcome::RowLimit,
                2,
            ),
            (vec![batch_of(3), batch_of(10)], SinkOutcome::RowLimit, 1),
        ] {
            let buffer = Arc::new(ResultBuffer::with_limits(
                schema(),
                BufferLimits::default().with_max_rows(2_usize),
            ));
            let sink = BatchSink::new(buffer.clone()).with_end_confirmation();
            let mut source = ScriptedSource::new(batches);
            let outcome = block_on(sink.drain(&mut source, &CancelToken::new())).expect("drain");
            assert_eq!(outcome, expected);
            assert_eq!(source.pull_count, calls);
            assert_eq!(buffer.row_count(), 2);
            assert_eq!(buffer.stats().truncated, expected == SinkOutcome::RowLimit);
        }
    }

    /// Issue #122: at the receive limit N, only an observed end of stream
    /// makes a result whole; N + 1 rows and more stay truncated.
    #[test]
    fn the_end_probe_tells_exactly_n_rows_from_more() {
        const N: usize = 10;
        for (total, expected) in [
            (N - 1, SinkOutcome::Exhausted),
            (N, SinkOutcome::Exhausted),
            (N + 1, SinkOutcome::RowLimit),
            (N * 50, SinkOutcome::RowLimit),
        ] {
            let buffer = Arc::new(ResultBuffer::with_limits(
                schema(),
                BufferLimits::default().with_max_rows(N),
            ));
            let sink = BatchSink::new(buffer.clone()).with_end_confirmation();
            // Batches that land exactly on the limit: the probe is the only
            // way to know what follows.
            let batches = (0..total)
                .step_by(5)
                .map(|start| batch_of((total - start).min(5)))
                .collect();
            let mut source = ScriptedSource::new(batches);
            let outcome = block_on(sink.drain(&mut source, &CancelToken::new())).expect("drain");
            assert_eq!(outcome, expected, "{total} rows");
            assert_eq!(buffer.row_count(), total.min(N), "{total} rows");
            assert_eq!(
                buffer.stats().truncated,
                outcome.is_truncated(),
                "{total} rows"
            );
            assert!(source.pull_count <= N / 5 + 1, "one batch probed at most");
        }
    }

    /// The driver applies `ExecLimits::max_rows` too: it ends its stream at
    /// the bound and says whether rows were left behind.
    #[test]
    fn the_end_of_a_capped_stream_is_not_the_end_of_the_result() {
        let buffer = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_max_rows(10_usize),
        ));
        let sink = BatchSink::new(buffer.clone()).with_end_confirmation();
        let mut source = ScriptedSource::new(vec![batch_of(5), batch_of(5)]);
        source.truncated = true;
        let outcome = block_on(sink.drain(&mut source, &CancelToken::new())).expect("drain");
        assert_eq!(outcome, SinkOutcome::RowLimit);
        assert!(buffer.stats().truncated);
    }

    /// Rows up to the limit, then a server that times out before saying
    /// whether more follow.
    struct SlowTail {
        served: bool,
    }

    impl BatchSource for SlowTail {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>> {
            if std::mem::replace(&mut self.served, true) {
                return Box::pin(async {
                    Err(OxynError::Timeout {
                        after: std::time::Duration::from_secs(30),
                    })
                });
            }
            Box::pin(async { Ok(Some(batch_of(2))) })
        }
    }

    #[test]
    fn a_probe_cut_by_the_timeout_leaves_the_limit_unverified_not_failed() {
        let buffer = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_max_rows(2_usize),
        ));
        let sink = BatchSink::new(buffer.clone()).with_end_confirmation();
        let mut source = SlowTail { served: false };
        let outcome = block_on(sink.drain(&mut source, &CancelToken::new())).expect("drain");
        assert_eq!(outcome, SinkOutcome::RowLimitUnverified);
        assert_eq!(buffer.row_count(), 2);
        assert!(buffer.stats().truncated, "never presented as whole");
    }

    #[test]
    fn without_the_probe_a_stop_at_the_limit_is_not_called_missing_rows() {
        let buffer = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_max_rows(10_usize),
        ));
        let sink = BatchSink::new(buffer.clone());
        let mut source = ScriptedSource::new(vec![batch_of(5), batch_of(5)]);
        let outcome = block_on(sink.drain(&mut source, &CancelToken::new())).expect("drain");
        assert_eq!(outcome, SinkOutcome::RowLimitUnverified);
        assert!(outcome.is_truncated(), "unverified is never complete");
        assert_eq!(source.pull_count, 2, "no read beyond the limit");
        assert!(buffer.stats().truncated);
    }

    #[test]
    fn cancelling_at_preview_limit_does_not_probe_or_claim_completion() {
        let buffer = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_max_rows(2_usize),
        ));
        let sink = BatchSink::new(buffer.clone()).with_end_confirmation();
        let mut source = ScriptedSource::new(vec![batch_of(2)]);
        let cancel = CancelToken::new();
        let outcome =
            block_on(sink.drain_with(&mut source, &cancel, |_| cancel.cancel())).expect("drain");
        assert_eq!(outcome, SinkOutcome::Cancelled);
        assert_eq!(source.pull_count, 1);
        assert!(buffer.stats().truncated);
    }

    #[test]
    fn every_batch_is_reported_as_soon_as_it_arrives() {
        let buf = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let sink = BatchSink::new(Arc::clone(&buf));
        let mut source = ScriptedSource::new(vec![batch_of(3), batch_of(4)]);
        let ct = CancelToken::new();

        let mut seen = Vec::new();
        block_on(sink.drain_with(&mut source, &ct, |progress| seen.push(progress)))
            .expect("draining without error");

        assert_eq!(seen.len(), 2);
        assert_eq!(seen.first().map(|p| (p.rows, p.total_rows)), Some((3, 3)));
        assert_eq!(seen.get(1).map(|p| (p.rows, p.total_rows)), Some((4, 7)));
    }

    /// The test that carries the promise: when the buffer refuses, the next
    /// batch **is not requested** from the server.
    #[test]
    fn a_saturated_buffer_stops_requesting_batches() {
        let buf = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default()
                .with_memory_budget(1)
                .without_spill(),
        ));
        let sink = BatchSink::new(Arc::clone(&buf));
        let mut source = ScriptedSource::new(vec![batch_of(10); 50]);
        let ct = CancelToken::new();

        let outcome = block_on(sink.drain(&mut source, &ct)).expect("draining without error");

        assert_eq!(outcome, SinkOutcome::Saturated);
        assert!(outcome.is_truncated());
        assert_eq!(
            source.pull_count, 1,
            "a single batch requested: the refusal must come up before the next request"
        );
        assert!(buf.stats().truncated);
        assert!(buf.is_complete(), "the interface must stop waiting");
    }

    #[test]
    fn the_row_limit_stops_draining() {
        let buf = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_max_rows(15_usize),
        ));
        let sink = BatchSink::new(Arc::clone(&buf));
        let mut source = ScriptedSource::new(vec![batch_of(10); 20]);
        let ct = CancelToken::new();

        let outcome = block_on(sink.drain(&mut source, &ct)).expect("draining without error");

        assert_eq!(outcome, SinkOutcome::RowLimit);
        assert_eq!(buf.row_count(), 15);
        assert!(buf.stats().truncated);
        assert_eq!(
            source.pull_count, 2,
            "two batches are enough to reach 15 rows; the third must not be requested"
        );
    }

    #[test]
    fn a_prior_cancellation_requests_no_batch() {
        let buf = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let sink = BatchSink::new(Arc::clone(&buf));
        let mut source = ScriptedSource::new(vec![batch_of(10)]);
        let ct = CancelToken::new();
        ct.cancel();

        let outcome = block_on(sink.drain(&mut source, &ct)).expect("draining without error");

        assert_eq!(outcome, SinkOutcome::Cancelled);
        assert_eq!(source.pull_count, 0);
        assert!(buf.is_empty());
        assert!(buf.stats().truncated);
    }

    /// Source that never answers, and cancels at the first poll: it is the
    /// server that does not hand back control while the user presses `Esc`.
    #[derive(Debug)]
    struct SilentSource {
        ct: CancelToken,
    }

    impl BatchSource for SilentSource {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>> {
            let ct = self.ct.clone();
            Box::pin(async move {
                ct.cancel();
                std::future::pending::<Result<Option<RecordBatch>, OxynError>>().await
            })
        }
    }

    #[test]
    fn an_in_flight_cancellation_burns_the_source() {
        let buf = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let sink = BatchSink::new(Arc::clone(&buf));
        let ct = CancelToken::new();
        let mut source = SilentSource { ct: ct.clone() };

        let outcome = block_on(sink.drain(&mut source, &ct)).expect("draining without error");
        assert_eq!(outcome, SinkOutcome::Cancelled);

        // Resuming would read a stream whose decoder may be out of sync.
        let ct2 = CancelToken::new();
        let resumption = block_on(sink.drain(&mut source, &ct2));
        assert!(resumption.is_err(), "resuming must be refused");
    }

    #[test]
    fn a_source_error_propagates_but_closes_the_buffer() {
        let buf = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let sink = BatchSink::new(Arc::clone(&buf));
        let mut source = ScriptedSource::failing(OxynError::Query("boom".to_owned()));
        let ct = CancelToken::new();

        let error = block_on(sink.drain(&mut source, &ct)).expect_err("the error must propagate");
        assert!(matches!(error, OxynError::Query(_)));
        assert!(
            buf.is_complete(),
            "without closing, the interface waits for a batch that will not come"
        );
        assert!(buf.stats().truncated);
    }
}
