//! What reading back a batch **spilled to disk** costs.
//!
//! [PERFORMANCE](../../../docs/PERFORMANCE.md) states that "scrolling beyond
//! the memory budget reads a disk page" and that it "never re-runs the
//! query". The second half holds by construction and is proven elsewhere:
//! `page_read_is_local_audited_and_scoped_for_both_actors` reads back a
//! spilled batch **with no driver registered**, so no execution can slip in.
//!
//! This bench measures the half that was quantified nowhere: **how long this
//! read takes**. It is what decides whether fast scrolling stays smooth or
//! becomes a series of jolts, and the neighboring budget is the frame's —
//! 8 ms p99.
//!
//! # What it measures, and what it does not
//!
//! It measures the `ResultBuffer::batch` path on a non-resident batch: reading
//! the spill file and Arrow IPC decoding. It measures neither rendering nor
//! the trip through the bus — the first escapes `criterion`, the second is
//! scheduling. In other words, it gives the cost of the floor: what no
//! interface optimization will be able to bring down.
//!
//! # The sizes
//!
//! A 512-row batch is what drivers commonly produce; 8,192 rows is the case
//! of a wide column read in large blocks. A budget held on the small batch and
//! lost on the large one is a budget discovered while scrolling on the user's
//! machine.

use std::hint::black_box;
use std::sync::Arc;

use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxyn_data::{BatchIndex, ResultBuffer};

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("mot", DataType::Utf8, true),
    ]))
}

fn lot(lignes: usize) -> RecordBatch {
    let entiers = Int64Array::from_iter_values(0..lignes as i64);
    let mots: Vec<String> = (0..lignes)
        .map(|rang| format!("valeur_{rang:07}"))
        .collect();
    RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(entiers),
            Arc::new(StringArray::from(
                mots.iter().map(String::as_str).collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("the columns match the schema")
}

/// A buffer whose **last** batch spilled, and the index of that batch.
///
/// The one-byte budget forces spilling from the second batch on. The check
/// that follows is not a stylistic precaution: without it, the bench would
/// measure a **cached** read if the spill did not happen, announce a few
/// nanoseconds, and that figure would pass for an excellent result. That is
/// exactly the failure that already skewed a bench in this repository.
fn tampon_deborde(lignes: usize) -> (ResultBuffer, BatchIndex) {
    let tampon = ResultBuffer::new(schema(), 1);
    for _ in 0..4 {
        tampon.push(lot(lignes)).expect("batch accepted");
    }
    let dernier = BatchIndex::new(tampon.batch_count() - 1);
    assert!(
        !tampon.is_resident(dernier),
        "the bench must measure a disk read: this batch is still in memory"
    );
    (tampon, dernier)
}

fn read_back_a_spilled_batch(c: &mut Criterion) {
    let mut groupe = c.benchmark_group("spilled_page");

    for lignes in [512usize, 8_192] {
        let (tampon, position) = tampon_deborde(lignes);

        groupe.bench_with_input(BenchmarkId::from_parameter(lignes), &lignes, |banc, _| {
            banc.iter(|| {
                let lot = tampon
                    .batch(black_box(position))
                    .expect("the read succeeds")
                    .expect("the batch exists");
                // `num_rows` is O(1): a real value of the batch is consumed so
                // that decoding cannot be elided.
                black_box(lot.num_rows())
            });
        });
    }

    groupe.finish();
}

criterion_group!(benches, read_back_a_spilled_batch);
criterion_main!(benches);
