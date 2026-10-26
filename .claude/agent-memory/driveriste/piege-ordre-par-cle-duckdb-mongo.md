---
name: piege-ordre-par-cle-duckdb-mongo
description: DuckDB ORDER BY pk LIMIT is a TOP_N over a seq scan (measured cost), and MongoDB listCollections idIndex is documented ambiguously — do not key logic on it without a live server
metadata:
  type: reference
---

**DuckDB** has no ordered index scan: `EXPLAIN SELECT * FROM t ORDER BY id LIMIT 200`
shows `TOP_N` over `SEQ_SCAN` with an "optional: Dynamic Filter (id)" that prunes
by zone maps. Measured 2026-10-26, 10M rows, in-memory, M1 Max, dev profile
(libduckdb-sys at opt-level 2): 2 ms in insertion order, 25 ms when rows are
shuffled, 0.13–0.5 s at `OFFSET 5000000`. Quick way to measure: an `#[ignore]`d
test using `duckdb::Connection::open_in_memory()` directly, run with
`-- --ignored --nocapture`, then deleted.

**MongoDB `listCollections`**: the manual's output table puts `idIndex` under
`info`, while its example shows it as a sibling of `info`; nothing says whether
views or time-series collections return it, and `nameOnly: true` drops it. Without a
live server, "plain collection" (`type == "collection"`) is the condition the
driver relies on to treat `_id` as unique. Known gap: `system.profile` is
`type: collection` but has no `_id` index.
