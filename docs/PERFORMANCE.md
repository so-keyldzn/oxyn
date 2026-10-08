# Performance budgets

> **Authority**: the numeric thresholds beyond which a behavior is a
> defect, and not an acceptable slowness.

Invariants involved: [I-05](../CLAUDE.md#i-05), [I-06](../CLAUDE.md#i-06).
Derives from [ADR-0002](adr/0002-arrow-result-model.md) for everything that concerns
results.

> **Status of the numbers.** The values below remain **decided
> budgets**, derived from the thresholds of human perception. A first measurement
> campaign took place on **2026-09-10**; it is recorded
> [further down](#measurement-campaign-of-2026-09-10) and amended no budget. It
> only covers pure code: the row-to-batch conversion of the SQLite driver and
> query analysis. A second campaign, on **2026-09-11**, measured
> cold start, the cached catalog node and idle memory — its
> numbers are in [IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md) and carried over
> into the table below. **The frame budget, cold start and
> the idle application were measured on the GPUI interface, removed on
> 2026-09-18 ([ADR-0029](adr/0029-interface-tauri-shadcn.md)): they say
> nothing about the Tauri webview, which has not been measured yet** — see
> [Comparison with the budgets](#comparison-with-the-budgets). A third measurement, on
> **2026-09-15**, compared the memory budget with the memory of the **process**
> and no longer with the buffer's accounting alone; it is recorded
> [further down](#memory-measurement-of-2026-09-15). A budget
> contradicted by a measurement is amended **through an ADR** — never by adjusting it
> silently to make a test pass.

## Why budgets and not "best practices"

"Perceived latency is a feature" ([VISION](VISION.md)) means nothing
as long as no number lets us say we missed it. A performance regression
without a threshold is never detected: it accumulates in
15 ms slices nobody notices, until the product has
become slow without any commit being to blame.

## Interaction budgets

The thresholds come from perception: ~16 ms is the frame at 60 Hz, ~100 ms is
the limit of the "instant" reaction, ~1 s is where attention drops off.

| Interaction | Budget | What happens beyond it |
|---|---|---|
| Interface frame during a continuous interaction (scrolling, typing, resizing) | **8 ms** p99 | visible stutter; it is the number-one symptom of a violation of [I-05](../CLAUDE.md#i-05) |
| Visible feedback after a click or a keystroke | **100 ms** | the user clicks again, thinking they missed |
| First rows displayed after launching a query | **300 ms** after the server's first response | past this delay, the user no longer connects their action with the result |
| Window opening on cold start | **1 s** | a native client that starts more slowly than a web client loses its main argument |
| Expanding an already cached catalog node | **50 ms** | navigating the tree must feel local |

An operation that cannot hold its budget does not miss it silently: it
shows progress and remains cancellable. **A long, cancellable operation
is acceptable; a long, frozen operation is not.**

## Memory budgets

| Situation | Budget | Failure mode |
|---|---|---|
| In-memory `ResultBuffer`, per result | **256 MB** by default, configurable ([ADR-0002](adr/0002-arrow-result-model.md)); beyond, spill to Arrow IPC read back off the UI thread ([ADR-0012](adr/0012-lecture-pages-resultats.md)) | [I-06](../CLAUDE.md#i-06): `SELECT *` on a large table triggers the OOM killer, the process dies without a trace, the user loses their work |
| Scrolling beyond the memory budget | **one disk page read**, never a new execution | re-running the query is doubly wrong: the cost is arbitrary, and a `SELECT` may not be idempotent |
| Results kept without a reader | 16 results, 256 MiB of resident/cached batches and 1 GiB of IPC cumulated; check after execution and periodically ([ADR-0017](adr/0017-retention-resultats.md)) | accumulation of unused results |
| Catalog cache per connection | 1,024 scopes and 50,000 metadata objects; eviction of the least recently published scope ([ARCHITECTURE](ARCHITECTURE.md)) | ten connections on databases with tens of thousands of objects make the RSS grow without a ceiling |
| Assistant conversations, reading back | **16 turns per page**: at worst 21 MiB read from disk and 36.3 MiB decoded per call, whatever the thread; no thread is read in one piece. Calculation detail on `MAX_TURN_PAGE` (`oxyn-store`, 2026-09-16) | [I-06](../CLAUDE.md#i-06): a thread read back in bulk and sent to the interface allocates up to 1 GiB; a file written by a third party without a bound made the allocation arbitrary |
| Assistant conversations, reading back a branch | **16 exchanges per page**: at worst 16.0 MiB read from disk and 16.0 MiB decoded per call — the question alone bounds the calculation (1 MiB), the rest of the exchange fits in 410 bytes read and 546 decoded. A branch holds at most 256 exchanges, and the versions of an exchange are listed without question or answer. Calculation detail on `MAX_EXCHANGE_PAGE` (`oxyn-store`, 2026-09-18) | [I-06](../CLAUDE.md#i-06): a tree-shaped thread read back in one piece grows with the number of regenerations, which nothing bounds on the caller side |
| Assistant conversations, on disk | 200 threads, 90 days of inactivity and 32 MiB of transcript per workspace, applied by `Conversations::prune`; 512 turns per thread and `stop_reason` ≤ 256 bytes held by the file. Measurement: a 12-exchange thread weighs 64 KiB of transcript, 72 KiB of file (2026-09-16) | an assistant history that grows endlessly over a session of several months |
| Cached DDL definitions per connection | 16 definitions and 16 MiB of SQL + notes; eviction of old values, including invalidated ones ([ADR-0018](adr/0018-apercu-ddl.md)) | accumulation of large scripts while navigating |
| Local embedding model ([ADR-0056](adr/0056-local-cpu-embeddings-for-context-selection.md)) | **nothing** while semantic ranking is off (the default); once on, loaded when the assistant panel opens or at the first question, and dropped **5 minutes** after its last use. Budget amended by ADR-0056 on 2026-10-08, footprint: **300 MB** at most while loaded — the weights are a read-only mapping of `model.bpk`, file pages outside the footprint; measured 157–179 MB loaded, 248–268 MB after embedding 256 names (macOS 26.2, M1 Max, release, 2026-10-08; 553–643 MB before the mapping, when burn-store copied the file into the heap); **250 MB** at most kept after unloading, by the system allocator and not by Oxyn's code — measured about 240 MB, regions `malloc` keeps as `MALLOC_LARGE (empty)`; the one-time conversion, peaking at 1.45 GB with about 1.19 GB kept by the allocator afterwards, runs **in a child process** and leaves nothing in Oxyn's. Relation vectors: in memory only, keyed by a 32-byte SHA-256 of the model and the text, at most 50,000 in two generations — 50,000 × 1,536 bytes plus 2 × 32,768 buckets of 41 bytes, about **79.5 MB**, whatever the comments' length (`vectors.rs`, 2026-10-07). The semantic step of a question: **2 s** at most, loading included | a model held all day for a question asked in the morning; a conversion's 1.2 GB kept for the rest of the session; above these budgets, the ADR is reconsidered |
| Idle application, one open connection, no query | stable over time | growth at rest is a leak; it shows over a session of several hours, not in tests |

The retention of initial batches and decoded pages shares the result's
budget according to [ADR-0012](adr/0012-lecture-pages-resultats.md). The batch index,
decoding temporaries and references held by readers must
be counted in process measurements; an assertion on the cache does not
prove RSS stability.

## What is measured, and how

- **`criterion`** for pure-code benchmarks: parsing, formatting,
  conversion to `RecordBatch`, schema diff. They are the only reproducible
  measurements on a development machine.
- **The row-to-batch conversion is an expected hot spot**, not a given:
  drivers built on row-by-row clients go through every
  value of every row there ([DRIVER-CONTRACT](DRIVER-CONTRACT.md#3-it-produces-arrow-recordbatches-streamed)).
  It is the first place to measure, and the last to optimize without measurement.
- **System instruments** (Instruments, `perf`) for rendering and
  the interface. A `criterion` benchmark does not see the webview's rendering: it
  measures nothing useful about the interface.
- **No latency measurement against a real database is a benchmark**: the
  network and the server's state dominate the signal. What is measured is the
  time spent **in Oxyn**, not the round-trip time.

The full protocol is in [`/benchmark`](../.claude/commands/benchmark.md).

## Measurement campaign of 2026-09-10

> This section records **what was measured, and what was not**. It
> changes no budget: none of the budgets compared was contradicted.

### Conditions

| | |
|---|---|
| Machine | Apple M1 Max, 10 cores, 64 GiB, macOS 26.2 (25C56) |
| Toolchain | `rustc 1.98.1`, `bench` profile (`lto = "thin"`, `codegen-units = 1`) |
| Instrument | `criterion 0.8.2`, 50 samples, flat sampling, 2 s warm-up |
| Load | no concurrent compilation — neither `rustc` nor `cargo` — and 55 to 71 % idle CPU time during the measurement |
| Reproducibility | two successive campaigns; deviation of medians ≤ 4 % |

The **load average** of this machine is structurally above 20
(busy graphical session) without the cores being taken: it is not a
usable indicator here, and idle CPU time served as the criterion.
As a result, **the numbers are valid to ±5 %**. That is enough to compare
budgets counted in milliseconds; it would not be to settle an
optimization promising 3 %.

### Bench 1 — row-to-batch conversion, SQLite driver

`drivers/oxyn-driver-sqlite/benches/row_to_batch.rs`. **In-memory** database, table
with six columns (integer, float, short text, long text, 64-byte BLOB,
integer with one `NULL` in seven), filled by a recursive CTE hence identical from one
run to the next.

`ColumnBuilder` is `pub(crate)` and was **not** made public for the bench.
The conversion cost is obtained by **subtraction** between two passes over the
same rows: `oxyn` through the driver's public API, `rusqlite` on a raw
connection that touches every value without building Arrow. **The difference is an
estimate, not a direct measurement**; it also contains the round trips of the
channel to the carrier thread — one per batch, i.e. ~31 for 250,000 rows, hence
negligible.

| Table | Rows | Arrow produced | `oxyn` | `rusqlite` | Difference | Difference / value |
|---|---|---|---|---|---|---|
| mixed, 6 columns | 250,000 | 48.4 MiB | 54.7 ms | 34.6 ms | 20.1 ms | 13.4 ns |
| mixed, 6 columns | 1,000,000 | 186.7 MiB | 211.3 ms | 139.1 ms | 72.3 ms | 12.0 ns |
| `INTEGER` | 250,000 | 1.9 MiB | 14.0 ms | 8.7 ms | 5.3 ms | 21.1 ns |
| `REAL` | 250,000 | 1.9 MiB | 16.7 ms | 9.3 ms | 7.3 ms | 29.3 ns |
| short `TEXT` (~11 B) | 250,000 | 4.4 MiB | 17.3 ms | 10.1 ms | 7.2 ms | 29.0 ns |
| long `TEXT` (~80 B) | 250,000 | 20.8 MiB | 18.3 ms | 11.1 ms | 7.2 ms | 28.6 ns |
| `BLOB` 64 B | 250,000 | 17.4 MiB | 18.8 ms | 11.2 ms | 7.6 ms | 30.5 ns |
| `INTEGER` with `NULL` | 250,000 | 2.0 MiB | 14.2 ms | 9.2 ms | 5.0 ms | 20.0 ns |

Medians. The single-column tables are an **indicative breakdown**: their
footprint is smaller, and their per-value cost carries the whole fixed per-row
cost instead of sharing it among six columns.

What these numbers establish:

- **211 ns per six-column row**, i.e. **4.7 million rows per
  second** and about 880 MiB/s of Arrow buffers produced;
- **the per-row cost is identical at 250,000 and at 1,000,000 rows**
  (219 ns versus 211 ns). The bench therefore measures the algorithm and not the cache — at
  186.7 MiB, the table far exceeds the machine's 24 MiB of system-level
  cache;
- **the conversion weighs 34 % of the driver's total time**, the rest being
  SQLite's own iteration. The Oxyn layer costs **1.5 times** the
  raw pass over the same rows. It is a real hot spot, and it is not a
  pathology;
- the per-value cost hardly depends on the text length (29.0 ns at
  11 bytes, 28.6 ns at 80): it is dominated by the **per-value** work, not
  by copying bytes.

### Bench 2 — first batch

Same file, `first_batch` group: the time between `execute` and the first
available `RecordBatch`, i.e. what the grid waits for before it can
paint. The first batch is the most expensive, since it is the one during which
the driver resolves column types with a probe pass.

| Table | Rows in the first batch | Measurement |
|---|---|---|
| mixed, 250,000 rows | 8,192 | **2.60 ms** |
| mixed, 1,000,000 rows | 8,192 | **2.57 ms** |

The result does not depend on the table size: that is what streaming
promises ([I-06](../CLAUDE.md#i-06)), and the bench observes it.

### Bench 3 — query analysis

`crates/oxyn-query/benches/analysis.rs`, PostgreSQL dialect. The text is a
realistic block — comments, quoted identifier, string containing a
semicolon, `$body$` body, a write — repeated 1, 10 and 50 times.

| Function | 4 statements (820 B) | 40 statements (8.2 KiB) | 200 statements (41 KiB) |
|---|---|---|---|
| `split` | 1.13 µs | 11.5 µs | 55.4 µs |
| `words` | 1.71 µs | 14.6 µs | 66.5 µs |
| `current_statement` (cursor at end of text) | 1.44 µs | 11.5 µs | 56.4 µs |
| `format` | 2.23 µs | 22.0 µs | 108.7 µs |
| `classify` | 52.6 µs | 525.7 µs | 2.67 ms |

Everything is **linear** in text size: no quadratic scanner. `classify`
costs ~13 µs per statement, two orders of magnitude above splitting —
it is `sqlparser`'s parsing, and it is expected.

> **Non-regression check of 2026-09-15.** The three benches were rerun
> after this session's fixes — nine code defects, several of them
> in `oxyn-app` and `oxyn-core`. The numbers agree with those recorded
> above: catalog node at 10,000 relations **6.55 µs** (versus 6.70),
> read back of a spilled 8,192-row batch **35.7 µs** (versus 37.3),
> `classify` on 200 statements **2.69 ms** (versus 2.67). The differences are
> those of a `--quick` run versus a full campaign, not a shift.
>
> This check replaces no open measurement: it only says that none of
> these three paths regressed.

### Comparison with the budgets

> **Three verdicts concern the removed interface.** The frame, cold window
> opening and the idle application were measured on 2026-09-11 on
> the GPUI interface, removed on 2026-09-18
> ([ADR-0029](adr/0029-interface-tauri-shadcn.md)). They say nothing about the
> Tauri webview, which has not been measured yet
> ([plan](IMPLEMENTATION-PLAN.md#migration-to-the-tauri-interface)). The other
> rows measure the core, which the interface change did not touch.

| Budget | Verdict | Based on |
|---|---|---|
| First rows displayed — **300 ms** | **confirmed** for SQLite | 2.6 ms for the first batch, whatever the table size: 0.9 % of the budget |
| Visible feedback after a keystroke — **100 ms** | **confirmed for the `oxyn-query` part** | 56 µs for the current statement on a 200-statement script. The interface part is not measured |
| Frame during an interaction — **8 ms** p99 | **held** (2026-09-11): 0 hitch at rest and while typing, 1 under continuous resizing. Typing produced 17, one of them 50 ms, before [ADR-0024](adr/0024-autosauvegarde-au-repos-de-frappe.md) | a `criterion` bench on the interface measures nothing: it takes `xcrun xctrace record --template "Animation Hitches" --attach <pid>`, which works without a graphical interface. Typing **has** been covered since, and it is what revealed the overrun fixed by ADR-0024: the sentence that called it "to be covered" came before the measurement. What remains uncovered is **scrolling a populated grid** — reading back a spilled batch costs 4.5 µs there (next row), but the full scroll + render sequence has not been observed under an instrument |
| Cold window opening — **1 s** | **235–274 ms** (2026-09-11, `dev` profile, three launches) | measured by timestamping between the process launch and the log's `window ready` — no specialized instrument needed |
| Cached catalog node — **50 ms** | **6.70 µs** at 10,000 relations (2026-09-11) | `cargo bench -p oxyn-catalog --bench cached_node`; reading the cache consumes six millionths of the budget, so the bottleneck of a slow node is elsewhere |
| `ResultBuffer` — **256 MB** then spill | **confirmed on the RSS, at its real value** (2026-09-15): **2 GiB** pushed into a buffer at the default budget of **256 MB** make the RSS grow by **195 MiB** — below the budget —, **1.84 GiB** going to disk | [detailed measurement below](#memory-measurement-of-2026-09-15). The measurement remains **manual** — automating it would require an exception to [I-03](../CLAUDE.md#i-03) or to `unsafe_code = "deny"`, an unsettled trade-off |
| Scrolling = one page read | **held**: **4.5 µs** for a 512-row batch, **37.3 µs** for 8,192 (2026-09-14) | Two halves, proven separately. **That it is a read**: `page_read_is_local_audited_and_scoped_for_both_actors` reads back a spilled batch with **no driver registered** — no re-execution can slip in. **What it costs**: `cargo bench -p oxyn-data --bench spilled_page`, i.e. 0.06 % of the frame budget on an ordinary batch. *Caveat*: the spill file has just been written, so the system's page cache serves it hot. That is the real case of back-and-forth scrolling; a read back after eviction would cost more, and is not measured here |
| Idle application, stable over time | **82 MiB, stable over one minute** (2026-09-11) | a signal, not a proof: a slow leak shows over a session of several hours. The order of magnitude, however, is now known |

**No budget was contradicted. No budget was changed.**

## Memory measurement of 2026-09-15

Until then the memory budget was only checked through the **internal accounting
of the buffer** — `resident_bytes()` — never against the memory actually held by
the process. Accounting can be right and the process still grow:
that is exactly the failure mode [I-06](../CLAUDE.md#i-06) names.

### Conditions

`ResultBuffer` bounded to a memory budget of **4 MiB**, `max_rows` lifted,
disk spill allowed. **800 batches of 65,536 `i64` integers** pushed, i.e.
**400 MiB** of data produced. The budget is deliberately small: what is
tested is the spill **mechanism**, not the 256 MB value.

### Result

| Quantity | Measurement |
|---|---|
| Growth of the process RSS | **4.25 MiB** (4,456,448 bytes) |
| Volume actually spilled to disk | **404 MiB** (423,582,360 bytes) |
| `resident_bytes()` at the end of the run | below the budget |

**Sensitivity check.** The same scenario, with a budget of 400 MiB instead
of 4 MiB, makes the RSS climb by **317 MiB** and the assertion fail. The measurement
therefore discriminates by a factor of ~75: it is not a test that passes no matter
what.

**What this establishes:** [I-06](../CLAUDE.md#i-06) holds on the memory of the
**process**, and not only on the buffer's accounting. 400 MiB go through
a 4 MiB buffer without the process growing by more than its budget.

### The same measurement at the real budget

The small budget tests the mechanism; it says nothing of the value the product
uses. The measurement was therefore redone **at the default budget**, the one this
document sets:

| Quantity | Measurement |
|---|---|
| Buffer budget | **256 MB** (`DEFAULT_MEMORY_BUDGET`, 268,435,456 bytes) |
| Data pushed | **2 GiB** (4,096 batches of 65,536 `i64`, 2,147,483,648 bytes) |
| Growth of the process RSS | **195 MiB** (204,324,864 bytes) — **below the budget** |
| Volume actually spilled to disk | **1.84 GiB** (1,978,316,104 bytes) |
| Duration | 2.1 s |

Eight times the budget goes through the buffer, and the process grows by **9.1 %** of
what went through it — less than the budget itself. The 256 MB figure is therefore
no longer only a declared budget: it is **held, and measured as such**.

### Why it is not a permanent test

Honestly: because automating it would require an exception to a rule set
for an invariant.

- Reading the RSS requires `ps`, hence `std::process::Command::new`, which `clippy.toml`
  forbids in the name of [I-03](../CLAUDE.md#i-03) — a child process inherits
  the parent's environment, secrets included.
- The alternative, an instrumented allocator, requires `unsafe`, which the workspace
  refuses (`unsafe_code = "deny"`).

**Unsettled trade-off.** Is a narrow exception to one of the two worth it to
gain a permanent check of [I-06](../CLAUDE.md#i-06), or is the manual, dated
measurement enough? This document does not settle it: an exception to an
invariant rule is decided by an ADR, not in a table of measurements.

## The rule that prevents gratuitous optimization

**Clear code is not replaced by fast code without the measurement showing
it was worth it.** A benchmark before, a benchmark after, the
number in the commit message. Without that, the complexity is paid up front and
the gain is assumed.

The corollary also holds the other way: a `.clone()` on a path called
once per window opening is not a performance problem, and
turning it into a borrow that contaminates five signatures is a net loss.
