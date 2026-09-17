//! Row-to-batch conversion: what Oxyn adds on top of SQLite itself.
//!
//! # What is measured, and how the number is obtained
//!
//! `ColumnBuilder` is `pub(crate)`, and a bench is a separate crate. Nothing is
//! made public for the sake of the measurement: opening an API for a measuring
//! tool is a permanent cost for a temporary gain. The conversion cost is
//! therefore obtained by **subtraction** between two runs over the same rows:
//!
//! * `oxyn` — the driver's public API: `Session::execute`, then every
//!   `RecordBatch` drained from the `Cursor`. This is Oxyn *plus* SQLite.
//! * `rusqlite` — the same statement on a raw `rusqlite::Connection`, touching
//!   every value of every row through `Row::get_ref` and accounting its size
//!   exactly like the driver does, but building no Arrow array. This is SQLite
//!   alone.
//!
//! `oxyn - rusqlite` is an **estimate** of the conversion cost, not a direct
//! measurement. It also contains the worker-thread channel round trips — one
//! per batch, so ~31 for 250 000 rows at 8 192 rows per batch, which is
//! negligible against the per-value work.
//!
//! # Why an in-memory database, and why this many rows
//!
//! In-memory: no page cache, no filesystem, no server — the only thing left in
//! the signal is CPU time. The driver names its in-memory database after the
//! connection (`file:oxyn-memory-<id>?mode=memory&cache=shared`), so a single
//! session holds it alive for the whole bench process and the tables are
//! written once.
//!
//! Row count: a thousand rows fit in L2 and would measure the cache. The mixed
//! table produces 48.4 MiB of Arrow buffers at 250 000 rows and 186.7 MiB at
//! 1 000 000 — the bench prints both figures at startup — well past the 24 MiB
//! system level cache of an Apple M1 Max. The two sizes are run side by side on
//! purpose: if the per-row cost is the same at both, the bench is measuring the
//! algorithm and not the cache.
//!
//! Per-type benches use a single column and are a **decomposition**, not a
//! headline number: a narrow table has a smaller footprint, so its absolute
//! per-row cost is optimistic.

use std::hint::black_box;

use criterion::{
    BenchmarkId, Criterion, SamplingMode, Throughput, criterion_group, criterion_main,
};
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest, QueryLanguage,
};
use oxyn_driver::{Credentials, Driver, Session};
use oxyn_driver_sqlite::SqliteDriver;
use rusqlite::Connection;
use rusqlite::types::ValueRef;
use tokio::runtime::Runtime;

/// Rows of the headline table, and of every per-type table.
const ROWS: usize = 250_000;

/// Rows of the second headline size, used to show the per-row cost is flat.
const ROWS_LARGE: usize = 1_000_000;

/// Filler text, so that the long-text column is not a handful of bytes.
const LOREM: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor";

/// A table of the bench: its DDL, and the projection that fills it.
struct Table {
    /// Name, used as the bench id.
    name: &'static str,
    /// Column list, with declared types — SQLite's only type hint.
    columns: &'static str,
    /// Expressions fed by the row counter `i`.
    projection: String,
    /// How many rows to generate.
    rows: usize,
}

/// The headline table: one column per storage class, plus a frequently null one.
const MIXED_COLUMNS: &str =
    "id INTEGER, ratio REAL, label TEXT, notes TEXT, payload BLOB, optional INTEGER";

/// A ~80-character text value, deterministic on purpose: no `random()`, so two
/// runs read exactly the same bytes.
fn long_text() -> String {
    format!("'note-' || i || '-' || substr('{LOREM}', 1, 60)")
}

/// One column per storage class, plus a frequently null one.
fn mixed_projection() -> String {
    format!(
        "i, i * 1.5, 'row-' || i, {}, zeroblob(64), CASE WHEN i % 7 = 0 THEN NULL ELSE i % 1000 END",
        long_text()
    )
}

fn tables() -> Vec<Table> {
    vec![
        Table {
            name: "mixed_250k",
            columns: MIXED_COLUMNS,
            projection: mixed_projection(),
            rows: ROWS,
        },
        Table {
            name: "mixed_1m",
            columns: MIXED_COLUMNS,
            projection: mixed_projection(),
            rows: ROWS_LARGE,
        },
        Table {
            name: "int64",
            columns: "v INTEGER",
            projection: "i".to_owned(),
            rows: ROWS,
        },
        Table {
            name: "float64",
            columns: "v REAL",
            projection: "i * 1.5".to_owned(),
            rows: ROWS,
        },
        Table {
            name: "text_short",
            columns: "v TEXT",
            projection: "'row-' || i".to_owned(),
            rows: ROWS,
        },
        Table {
            name: "text_long",
            columns: "v TEXT",
            projection: long_text(),
            rows: ROWS,
        },
        Table {
            name: "blob64",
            columns: "v BLOB",
            projection: "zeroblob(64)".to_owned(),
            rows: ROWS,
        },
        Table {
            name: "int64_nulls",
            columns: "v INTEGER",
            projection: "CASE WHEN i % 7 = 0 THEN NULL ELSE i END".to_owned(),
            rows: ROWS,
        },
    ]
}

impl Table {
    fn create(&self) -> String {
        format!("CREATE TABLE {} ({})", self.name, self.columns)
    }

    /// A recursive CTE generates the rows inside SQLite: no round trip per row,
    /// and the same bytes on both sides of the comparison.
    fn insert(&self) -> String {
        format!(
            "INSERT INTO {name} WITH RECURSIVE seq(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM seq WHERE i + 1 < {rows}) SELECT {projection} FROM seq",
            name = self.name,
            rows = self.rows,
            projection = self.projection,
        )
    }

    fn select(&self) -> String {
        format!("SELECT * FROM {}", self.name)
    }
}

/// Exec limits of an export: no row cap, no timeout, read only.
fn unbounded_read() -> ExecLimits {
    let mut limits = ExecLimits::unbounded();
    limits.read_only = true;
    limits
}

/// Exec limits of a write: no row cap, no timeout, writes allowed.
fn unbounded_write() -> ExecLimits {
    ExecLimits::unbounded()
}

/// A live session over a private in-memory database, filled once.
struct Fixture {
    runtime: Runtime,
    session: Box<dyn Session>,
    cancel: CancelToken,
}

impl Fixture {
    fn new(tables: &[Table]) -> Self {
        // A current-thread runtime: the driver owns its own connection thread,
        // and a multi-threaded scheduler would only add scheduling noise.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let cancel = CancelToken::new();
        let driver = SqliteDriver::new();
        let config = ConnectionConfig::new("bench", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);

        let session = runtime
            .block_on(driver.connect(&config, &Credentials::new(), &cancel))
            .expect("open the in-memory database");

        for table in tables {
            runtime
                .block_on(run(session.as_ref(), &table.create(), &cancel))
                .expect("create the table");
            runtime
                .block_on(run(session.as_ref(), &table.insert(), &cancel))
                .expect("fill the table");
        }

        Self {
            runtime,
            session,
            cancel,
        }
    }

    /// Drains every batch of a statement and returns rows and Arrow bytes.
    fn drain(&self, sql: &str) -> (usize, usize) {
        self.runtime
            .block_on(drain(self.session.as_ref(), sql, &self.cancel))
    }

    /// Executes and stops at the first batch — what the grid needs to paint.
    fn first_batch(&self, sql: &str) -> usize {
        self.runtime
            .block_on(first_batch(self.session.as_ref(), sql, &self.cancel))
    }
}

/// Runs a statement to completion, discarding its batches.
async fn run(session: &dyn Session, sql: &str, cancel: &CancelToken) -> oxyn_core::Result<()> {
    let request = ExecRequest::new(QueryLanguage::SQL, sql).with_limits(unbounded_write());
    let mut cursor = session.execute(request, cancel).await?;
    while cursor.next_batch().await?.is_some() {}
    Ok(())
}

/// The measured path: every batch produced by the driver, touched then dropped.
async fn drain(session: &dyn Session, sql: &str, cancel: &CancelToken) -> (usize, usize) {
    let request = ExecRequest::new(QueryLanguage::SQL, sql).with_limits(unbounded_read());
    let mut cursor = session
        .execute(request, cancel)
        .await
        .expect("the statement compiles");
    let mut rows = 0usize;
    let mut bytes = 0usize;
    while let Some(batch) = cursor.next_batch().await.expect("a batch") {
        rows += batch.num_rows();
        bytes += batch.get_array_memory_size();
        black_box(&batch);
    }
    (rows, bytes)
}

/// Compiles the statement and returns as soon as the first batch is available.
///
/// This is the path behind the « premières lignes affichées » budget of
/// [PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-dinteraction): the driver
/// resolves the column types on a first probe pass, so the first batch costs
/// strictly more per row than the ones that follow. Dropping the cursor right
/// after interrupts the statement, which is what closing a tab does.
async fn first_batch(session: &dyn Session, sql: &str, cancel: &CancelToken) -> usize {
    let request = ExecRequest::new(QueryLanguage::SQL, sql).with_limits(unbounded_read());
    let mut cursor = session
        .execute(request, cancel)
        .await
        .expect("the statement compiles");
    let batch = cursor
        .next_batch()
        .await
        .expect("a batch")
        .expect("the table is not empty");
    black_box(&batch);
    batch.num_rows()
}

/// The reference: SQLite alone, every value touched, no Arrow built.
fn reference(connection: &Connection, sql: &str) -> (usize, usize) {
    let mut statement = connection
        .prepare_cached(sql)
        .expect("the statement compiles");
    let width = statement.column_count();
    let mut rows = statement.query([]).expect("the query starts");
    let mut count = 0usize;
    let mut bytes = 0usize;
    while let Some(row) = rows.next().expect("a row") {
        for index in 0..width {
            let value = row.get_ref(index).expect("a value");
            bytes += value_bytes(value);
            black_box(value);
        }
        count += 1;
    }
    (count, bytes)
}

/// Mirrors the driver's own byte accounting, so the reference does the same
/// per-value work minus the Arrow append.
fn value_bytes(value: ValueRef<'_>) -> usize {
    match value {
        ValueRef::Null => 0,
        ValueRef::Integer(_) | ValueRef::Real(_) => size_of::<i64>(),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
    }
}

/// The same tables, on a raw connection, filled the same way.
fn reference_connection(tables: &[Table]) -> Connection {
    let connection = Connection::open_in_memory().expect("open an in-memory database");
    for table in tables {
        connection
            .execute_batch(&table.create())
            .expect("create the table");
        connection
            .execute_batch(&table.insert())
            .expect("fill the table");
    }
    connection
}

fn row_to_batch(criterion: &mut Criterion) {
    let tables = tables();
    let fixture = Fixture::new(&tables);
    let reference_db = reference_connection(&tables);

    let mut group = criterion.benchmark_group("row_to_batch");
    // Flat sampling, not linear: an iteration reads a whole table — 210 ms for
    // the million-row one — and criterion's linear mode would need
    // `n(n+1)/2` iterations for `n` samples, which is minutes for nothing. Flat
    // sampling keeps the volume, which is what carries the meaning, and still
    // gives fifty samples to build a confidence interval on a machine that is
    // never perfectly idle.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(50);

    for table in &tables {
        let sql = table.select();
        let (rows, arrow_bytes) = fixture.drain(&sql);
        assert_eq!(rows, table.rows, "the table has the expected row count");
        println!(
            "# {name}: {rows} rows, {mib:.1} MiB of Arrow buffers",
            name = table.name,
            mib = arrow_bytes as f64 / (1024.0 * 1024.0),
        );

        group.throughput(Throughput::Elements(table.rows as u64));
        group.bench_with_input(BenchmarkId::new("oxyn", table.name), &sql, |b, sql| {
            b.iter(|| black_box(fixture.drain(sql)));
        });
        group.bench_with_input(BenchmarkId::new("rusqlite", table.name), &sql, |b, sql| {
            b.iter(|| black_box(reference(&reference_db, sql)));
        });
    }

    group.finish();

    // The other half of the question: not « how fast is the whole table », but
    // « how long before anything can be shown ». The two are different budgets
    // and the second one is the one the user feels.
    let mut first = criterion.benchmark_group("first_batch");
    first.sampling_mode(SamplingMode::Flat);
    first.sample_size(50);
    for table in tables.iter().filter(|t| t.name.starts_with("mixed")) {
        let sql = table.select();
        let rows = fixture.first_batch(&sql);
        println!("# first batch of {name}: {rows} rows", name = table.name);
        first.throughput(Throughput::Elements(rows as u64));
        first.bench_with_input(BenchmarkId::new("oxyn", table.name), &sql, |b, sql| {
            b.iter(|| black_box(fixture.first_batch(sql)));
        });
    }
    first.finish();
}

criterion_group!(benches, row_to_batch);
criterion_main!(benches);
