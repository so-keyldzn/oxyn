//! Query analysis: what a submission and a keystroke cost.
//!
//! # Why these three entry points
//!
//! `split` runs on every batch that reaches the executor, `classify` runs on
//! every submission — it is what the `PolicyGate` decides on — and
//! `current_statement` runs on **every keystroke** of the editor, because the
//! statement under the caret is part of the editor's live state. The last one
//! is therefore the only one of the three that sits under the 8 ms frame budget
//! of [PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-dinteraction); the
//! first two sit under the 100 ms visible-feedback budget.
//!
//! # Why several script sizes
//!
//! A one-statement script is the common case and says nothing about scaling; a
//! two-hundred-statement script is a migration file pasted into the editor, and
//! it is the case where a quadratic scanner would show. Both are measured, and
//! the caret bench deliberately puts the caret at the **end** of the script:
//! that is the worst case for a scanner that restarts from the beginning, and
//! it is also where a caret actually sits while someone types.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxyn_core::SqlDialect;
use oxyn_query::{classify, current_statement, format, split, words};

/// One block of realistic SQL: comments, quoted identifiers, a string holding a
/// semicolon, a dollar-quoted body, and a statement that is not a read.
const BLOCK: &str = r#"
-- Monthly revenue, by region.
WITH regional AS (
    SELECT r.region_id, sum(o.total_amount) AS revenue
    FROM orders o
    JOIN "public"."regions" r ON r.region_id = o.region_id
    WHERE o.placed_at >= '2026-01-01' AND o.status <> 'cancelled; refunded'
    GROUP BY r.region_id
)
SELECT region_id, revenue, revenue / nullif(lag(revenue) OVER (ORDER BY region_id), 0) AS ratio
FROM regional
ORDER BY revenue DESC
LIMIT 100;

/* A write, so the classifier has something to be careful about. */
UPDATE customers SET last_seen_at = now() WHERE id = 42;

CREATE OR REPLACE FUNCTION touch_updated_at() RETURNS trigger AS $body$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END;
$body$ LANGUAGE plpgsql;

INSERT INTO audit_log (actor, action, payload)
VALUES ('operator', 'refresh', '{"scope": "regional; full"}');
"#;

/// Script sizes: one block is the common submission, fifty blocks is a
/// migration file pasted whole.
const REPEATS: [usize; 3] = [1, 10, 50];

fn script(repeats: usize) -> String {
    BLOCK.repeat(repeats)
}

fn analysis(criterion: &mut Criterion) {
    let dialect = SqlDialect::Postgres;

    let mut group = criterion.benchmark_group("query_analysis");
    for repeats in REPEATS {
        let sql = script(repeats);
        let statements = split(&sql, dialect).len();
        println!(
            "# {repeats} block(s): {bytes} bytes, {statements} statements",
            bytes = sql.len(),
        );

        // Throughput in statements: the unit the user thinks in.
        group.throughput(Throughput::Elements(statements as u64));

        group.bench_with_input(BenchmarkId::new("split", repeats), &sql, |b, sql| {
            b.iter(|| black_box(split(black_box(sql), dialect).len()));
        });
        group.bench_with_input(BenchmarkId::new("classify", repeats), &sql, |b, sql| {
            b.iter(|| black_box(classify(black_box(sql), dialect)));
        });
        group.bench_with_input(BenchmarkId::new("words", repeats), &sql, |b, sql| {
            b.iter(|| black_box(words(black_box(sql), dialect).len()));
        });
        group.bench_with_input(BenchmarkId::new("format", repeats), &sql, |b, sql| {
            b.iter(|| black_box(format(black_box(sql), dialect)));
        });

        // The keystroke path. The caret sits at the very end of the script,
        // which is both the realistic position while typing and the worst case
        // for a scanner that restarts from byte zero.
        let caret = sql.len();
        group.bench_with_input(
            BenchmarkId::new("current_statement_at_end", repeats),
            &sql,
            |b, sql| {
                b.iter(|| black_box(current_statement(black_box(sql), dialect, caret)));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, analysis);
criterion_main!(benches);
