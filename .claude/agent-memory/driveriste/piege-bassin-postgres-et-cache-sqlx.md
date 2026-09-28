---
name: piege-bassin-postgres-et-cache-sqlx
description: How to force and observe several connections of the PostgreSQL pool in a test, and the sqlx prepared-statement cache that moves the error from preparation to the stream
metadata:
  type: reference
---

Traps found on 2026-09-10 while testing the session context on PostgreSQL
17.11. They concern `sqlx` and the lifecycle of a connection, not the product —
the contract lives in `docs/`.

## Forcing several connections of the pool

`execute` holds its connection from start to finish and the cursor's channel is
bounded to **one** batch. As long as you do not drain, the streaming task stays
blocked on its connection. So: open `MAX_CONNECTIONS` cursors **before** draining
a single one, on a query that exceeds `BATCH_ROW_CEILING` (8,192 rows) — a
`CROSS JOIN generate_series(1, 50000)` is enough and costs a few milliseconds.
Read `pg_backend_pid()` **inside the query itself** to prove it.

## An abandoned connection does not return to the pool

Two paths close it (`close_on_drop`), and then two successive executions
**never** land on the same backend:

- a cursor destroyed before `next_batch()` returned `None` — it may have left
  unread bytes;
- a `prepare` that fails while `limits.read_only` is true.

Consequence for a test that compares pids from one execution to the next: drain
until `None`, and put no failing query in it. To observe a resolution *without*
failing, `pg_catalog.to_regclass('name')` uses the same `search_path` and
returns `NULL` instead of raising.

Corollary: comparing **one** pid with **one** other stays fragile — the pool
keeps several idle connections and does not hand them out FIFO. What holds is to
saturate the pool, and compare the **set** of pids from one phase to the next.

## The rendering of a definition depends on `search_path`

Found on 17.11: `format_type`, `pg_get_constraintdef`, `pg_get_indexdef` and
`pg_get_expr` **qualify** their output when the schema is not on the path and
**omit** it when it is — `oxyn_ctx_a.ctx_amount` versus `ctx_amount`.

It is the only channel to observe the `search_path` of a pool connection
**without** going through an execution (which re-applies its `SET`): a fixture
with a domain and a function in the schema, read by the catalog, reveals the
state of the connection that served.

To make the observation deterministic: the pool is capped at four, so holding
three live cursors forces the catalog onto the fourth connection.

## The sqlx prepared-statement cache

`sqlx` keeps a cache **per connection**, keyed on the SQL text. After a
`SET search_path`, re-running the same text does not go through a client-side
preparation again: `execute` returns `Ok` even if the relation can no longer be
resolved, and the refusal arrives **through the stream**. In a test, go through
`echouer` and not through `refus`.

Checked on 17.11: the server does redo its analysis at execution when
`search_path` has changed. A same-name table in two schemas returns the rows of
the **new** schema, not those of the preparation's schema. It is the behavior to
re-check before trusting a statement cache on another engine.

See [[outil-cluster-postgres-jetable]] to start the test server.
