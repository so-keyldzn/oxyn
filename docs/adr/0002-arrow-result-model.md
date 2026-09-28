# ADR-0002 — Apache Arrow as the universal representation of results

**Status:** accepted · **Date:** 2026-09-05

## Context
A result can reach hundreds of millions of rows. A row representation
(`Vec<Vec<String>>`) saturates memory, slows rendering and forces a conversion on
every export.

## Decision
Every driver produces `arrow::RecordBatch`es. No reconversion happens between the
driver and the screen, the export or the sidecar process.

## Consequences
* **+** Columnar memory footprint; O(1) access for the virtualized grid.
* **+** CSV/Parquet/JSON/IPC export provided by the ecosystem.
* **+** DataFusion plugs in directly: filter, sort, aggregation on the client side.
* **+** Zero-copy with DuckDB, ClickHouse and the sidecar (Arrow IPC).
* **−** Row-by-row drivers (`sqlx`) need a batch conversion layer.
* **−** Schemaless data (Mongo) requires inference by sampling, shown as such in the
  UI.

## Detail: spilling to disk
`ResultBuffer` keeps a configurable memory budget (default 256 MB) and writes the rest
to a temporary Arrow IPC file. Scrolling far reads a disk page; the query is never
re-run.

> **Corrected on 2026-09-14.** This sentence said "memory-mapped". That was
> wrong, and the repository forbids itself from making it true: the `memmap2` API
> is `unsafe`, and `unsafe_code = "deny"` applies to the whole workspace.
> `spill.rs` therefore allocates a buffer and reads the file, as its own `///`
> explains. The `memmap2` dependency, declared but used nowhere, was removed at the
> same time. What remains true is the essential part: **a read, never a
> re-execution** — measured at 4.5 µs for a batch of 512 rows
> ([PERFORMANCE](../PERFORMANCE.md)).
