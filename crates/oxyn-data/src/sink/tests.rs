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
fn a_write_is_read_to_its_end_past_the_limit_and_past_a_stop() {
    let buffer = Arc::new(ResultBuffer::with_limits(
        schema(),
        BufferLimits::default().with_max_rows(5_usize),
    ));
    let sink = BatchSink::new(buffer.clone()).awaiting_terminal_outcome();
    let mut source = ScriptedSource::new(vec![batch_of(5), batch_of(5), batch_of(0)]);
    let cancel = CancelToken::new();
    // The Stop lands with the first batch: the source still decides.
    let outcome =
        block_on(sink.drain_with(&mut source, &cancel, |_| cancel.cancel())).expect("drain");
    assert_eq!(
        outcome,
        SinkOutcome::RowLimit,
        "a row past the limit was seen"
    );
    assert_eq!(source.pull_count, 4, "read to the end of the stream");
    assert_eq!(buffer.row_count(), 5, "nothing stored past the limit");
    assert!(buffer.is_complete());
}

#[test]
fn a_write_ends_on_its_source_verdict() {
    for (error, cancelled) in [
        (OxynError::Cancelled, true),
        (OxynError::OutcomeUnknown("cut".to_owned()), false),
    ] {
        let buffer = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let sink = BatchSink::new(buffer.clone()).awaiting_terminal_outcome();
        let cancel = CancelToken::new();
        cancel.cancel();
        let issue = block_on(sink.drain(&mut ScriptedSource::failing(error), &cancel));
        if cancelled {
            assert_eq!(issue.expect("a proven stop"), SinkOutcome::Cancelled);
        } else {
            let error = issue.expect_err("an ambiguous end stays an error");
            assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
        }
        assert!(buffer.is_complete());
    }
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
