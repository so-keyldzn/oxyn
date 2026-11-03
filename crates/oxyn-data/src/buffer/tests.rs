use arrow::array::Int32Array;
use arrow::datatypes::{DataType, Field, Schema};

use super::*;

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]))
}

fn batch_of(start: i32, rows: usize) -> RecordBatch {
    let values: Vec<i32> = (0..rows)
        .map(|i| start.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
        .collect();
    RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(values))])
        .expect("the column matches the schema built just above")
}

fn value_at(rb: &RecordBatch, line: usize) -> i32 {
    rb.column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("column 0 is an Int32Array by construction")
        .value(line)
}

#[test]
fn a_new_buffer_is_empty() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    assert!(buffer.is_empty());
    assert_eq!(buffer.row_count(), 0);
    assert_eq!(buffer.batch_count(), 0);
    assert!(!buffer.is_complete());
    assert!(buffer.locate(0).is_none());
    assert!(buffer.has_capacity());
}

#[test]
fn an_empty_batch_is_ignored_without_error() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    assert_eq!(
        buffer.push(batch_of(0, 0)).expect("empty batch accepted"),
        None
    );
    assert_eq!(buffer.batch_count(), 0);
}

#[test]
fn a_batch_with_the_wrong_schema_is_refused() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    let other = Arc::new(Schema::new(vec![Field::new("n", DataType::Utf8, false)]));
    let wrong = RecordBatch::try_new(
        other,
        vec![Arc::new(arrow::array::StringArray::from(vec!["a"]))],
    )
    .expect("batch built for the test");

    match buffer.push(wrong) {
        Err(DataError::SchemaMismatch { .. }) => {}
        other => panic!("expected SchemaMismatch, got {other:?}"),
    }
}

/// The batch boundary is the case `locate` misses when written by hand:
/// last row of a batch, first row of the next.
#[test]
fn locate_is_exact_on_batch_boundaries() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    for (start, rows) in [(0, 3), (100, 1), (200, 4)] {
        buffer.push(batch_of(start, rows)).expect("batch accepted");
    }
    assert_eq!(buffer.row_count(), 8);

    let expected = [
        (0, 0, 0),
        (1, 0, 1),
        (2, 0, 2), // last row of batch 0
        (3, 1, 0), // batch 1 contains only one row
        (4, 2, 0), // first row of batch 2
        (7, 2, 3), // last row of the result
    ];
    for (line, batch, offset) in expected {
        assert_eq!(
            buffer.locate(line),
            Some((BatchIndex::new(batch), offset)),
            "row {line}"
        );
    }
    assert_eq!(buffer.locate(8), None, "a row beyond does not exist");
    assert_eq!(buffer.locate(usize::MAX), None);
}

/// The same walk, but checking the value read back: `locate` can be
/// consistent with itself and point to the wrong batch.
#[test]
fn every_row_reads_back_to_its_value() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    buffer.push(batch_of(0, 5)).expect("batch accepted");
    buffer.push(batch_of(1_000, 5)).expect("batch accepted");

    let expected: Vec<i32> = (0..5).chain(1_000..1_005).collect();
    for (line, expected_value) in expected.into_iter().enumerate() {
        let (rb, offset) = buffer
            .row(line)
            .expect("read back")
            .expect("the row exists");
        assert_eq!(value_at(&rb, offset), expected_value, "row {line}");
    }
}

/// Tiny budget: everything spills, and everything is read back anyway.
#[test]
fn a_tiny_budget_spills_everything_without_losing_anything() {
    let buffer = ResultBuffer::new(schema(), 1);
    for start in [0, 100, 200, 300] {
        buffer.push(batch_of(start, 10)).expect("batch accepted");
    }

    assert_eq!(buffer.spilled_batches(), 4, "no batch fits in memory");
    assert_eq!(buffer.resident_bytes(), 0);
    assert!(buffer.spilled_bytes() > 0);
    assert_eq!(buffer.row_count(), 40);

    // Walk out of order, so as not to depend on the cache.
    for line in [39_usize, 0, 25, 10, 9, 30] {
        let (rb, offset) = buffer
            .row(line)
            .expect("reading back from disk")
            .expect("the row exists");
        let block = i32::try_from(line / 10).unwrap_or(0) * 100;
        let in_block = i32::try_from(line % 10).unwrap_or(0);
        assert_eq!(value_at(&rb, offset), block + in_block, "row {line}");
    }
}

/// The budget fills then spills: the first batches stay in memory, which
/// is what guarantees the fast first display.
#[test]
fn the_first_batches_stay_resident() {
    let sample = batch_of(0, 64);
    let size = sample.get_array_memory_size();
    // Three quarters remain resident: enough for two batches, not three.
    let buffer = ResultBuffer::new(schema(), size * 3);

    for start in [0, 100, 200, 300] {
        buffer.push(batch_of(start, 64)).expect("batch accepted");
    }

    assert_eq!(buffer.batch_count(), 4);
    assert_eq!(buffer.spilled_batches(), 2, "only the last ones spill");
    assert!(buffer.is_resident(BatchIndex::new(0)));
    assert!(buffer.is_resident(BatchIndex::new(1)));
    assert!(!buffer.is_resident(BatchIndex::new(3)));
}

#[test]
fn forbidden_spilling_produces_back_pressure() {
    let buffer = ResultBuffer::with_limits(
        schema(),
        BufferLimits::default()
            .with_memory_budget(1)
            .without_spill(),
    );
    match buffer.push(batch_of(0, 10)) {
        Err(DataError::Full { reason }) => assert!(reason.contains("spilling is disabled")),
        other => panic!("expected Full, got {other:?}"),
    }
    assert!(buffer.has_capacity(), "nothing has been accepted yet");
}

#[test]
fn the_spill_quota_is_respected() {
    let buffer = ResultBuffer::with_limits(
        schema(),
        BufferLimits::default()
            .with_memory_budget(1)
            .with_max_spill_bytes(16_u64),
    );
    match buffer.push(batch_of(0, 1_000)) {
        Err(DataError::Full { reason }) => assert_eq!(reason, "spill quota reached"),
        other => panic!("expected Full, got {other:?}"),
    }
}

#[test]
fn the_row_limit_trims_the_batch_and_declares_truncation() {
    let buffer =
        ResultBuffer::with_limits(schema(), BufferLimits::default().with_max_rows(12_usize));
    buffer.push(batch_of(0, 10)).expect("batch accepted");
    buffer.push(batch_of(100, 10)).expect("batch trimmed");

    assert_eq!(buffer.row_count(), 12);
    assert!(buffer.stats().truncated);
    assert!(!buffer.has_capacity());
    assert_eq!(buffer.remaining_rows(), Some(0));

    match buffer.push(batch_of(200, 1)) {
        Err(DataError::Full { reason }) => assert_eq!(reason, "row limit reached"),
        other => panic!("expected Full, got {other:?}"),
    }
}

#[test]
fn a_truncation_survives_the_driver_stats() {
    let buffer =
        ResultBuffer::with_limits(schema(), BufferLimits::default().with_max_rows(5_usize));
    buffer.push(batch_of(0, 10)).expect("batch trimmed");
    assert!(buffer.stats().truncated);

    // The driver, for its part, saw nothing.
    buffer.mark_complete(ExecStats {
        rows: 10,
        truncated: false,
        ..ExecStats::default()
    });

    assert!(
        buffer.stats().truncated,
        "a driver must not be able to erase an observed truncation"
    );
}

#[test]
fn nothing_is_added_after_closing() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    buffer.push(batch_of(0, 4)).expect("batch accepted");
    buffer.mark_complete(ExecStats::default());

    assert!(buffer.is_complete());
    assert!(!buffer.has_capacity());
    match buffer.push(batch_of(100, 4)) {
        Err(DataError::AlreadyComplete) => {}
        other => panic!("expected AlreadyComplete, got {other:?}"),
    }
}

#[test]
fn a_missing_position_returns_none() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    buffer.push(batch_of(0, 2)).expect("batch accepted");
    assert!(
        buffer
            .batch(BatchIndex::new(7))
            .expect("no error")
            .is_none()
    );
    assert!(buffer.row(99).expect("no error").is_none());
}

/// The `Debug` must never print a value from the database
/// ([I-03](../../../CLAUDE.md#i-03)).
#[test]
fn debug_shows_no_value() {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    buffer.push(batch_of(424_242, 3)).expect("batch accepted");
    let rendered = format!("{buffer:?}");
    assert!(!rendered.contains("424242"), "{rendered}");
    assert!(rendered.contains("rows: 3"), "{rendered}");
}

/// The grid reads while the sink pushes: the buffer must stay consistent,
/// and above all `locate` must never return a position out of the bounds
/// of the published batches.
#[test]
fn concurrent_reads_and_writes_stay_consistent() {
    // Budget small enough for almost all batches to spill: the test thus
    // exercises the disk read **during** the disk write.
    let buffer = Arc::new(ResultBuffer::new(schema(), 128));
    let writer = Arc::clone(&buffer);

    let producer = std::thread::spawn(move || {
        for i in 0..64_i32 {
            writer.push(batch_of(i * 10, 10)).expect("batch accepted");
        }
        writer.mark_complete(ExecStats::default());
    });

    while !buffer.is_complete() {
        let rows = buffer.row_count();
        for line in (0..rows).step_by(7) {
            let (position, offset) = buffer.locate(line).expect("announced row received");
            let rb = buffer
                .batch(position)
                .expect("read back")
                .expect("the located batch exists");
            assert!(offset < rb.num_rows());
        }
    }

    producer.join().expect("the producer does not panic");
    assert_eq!(buffer.row_count(), 640);
}
