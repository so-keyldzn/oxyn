# ADR-0048 — A statement sqlx cannot prepare, or whose result has no binary form, runs in the simple protocol, as text

**Status:** accepted · **Date:** 2026-09-28 · **Deciders:** Nicolas Boromée

## Context

The PostgreSQL driver runs every user statement through `sqlx` 0.9.0 (the
latest release on 2026-09-28) in the **extended** protocol: Parse, Describe,
Bind with every result column in binary, Execute. Three families of types made
the **whole query** fail, checked on PostgreSQL 17.11 on 2026-09-28:

* **Multiranges** (`int4multirange`… — 6 types, PostgreSQL 14+) and the
  internal category `Z` (`pg_node_tree`, `pg_ndistinct`, `pg_dependencies`,
  `pg_mcv_list`, the BRIN summaries): after Describe, `sqlx` queries
  `pg_type` for every OID it does not know, and refuses `typtype = 'm'` and
  `typcategory = 'Z'` (`connection/resolve.rs`, `unknown type code 109`,
  `invalid category code 90`). Neither 0.9.0 nor `sqlx`'s `main` branch handles
  them.
* **Types without a binary output function** (`typsend = 0`): `aclitem`,
  `gtsvector` and their arrays. `sqlx` hard-codes the binary result format for
  every column (`connection/executor.rs`), and the server refuses the Bind:
  `no binary output function available for type aclitem`.

These are not exotic: `SELECT * FROM pg_class` carries `relacl`
(`aclitem[]`) and `relpartbound` (`pg_node_tree`), and failed.

## Decision

1. When `prepare` fails on `sqlx`'s type resolution, or when the described
   result has a column whose type has no binary output (`aclitem`,
   `gtsvector`, their arrays, a domain over them), **and the request has no
   bound parameter**, the driver runs the same text through `sqlx::raw_sql`,
   the **simple** protocol, where the server sends every value as text.
2. The result's columns are then all `Utf8`: the text the server printed,
   marked `oxyn:fallback = text` with the PostgreSQL type name (or its OID when
   `sqlx` could not resolve it) in `oxyn:pg_type`. The schema is known at the
   first row.
3. With bound parameters, the simple protocol cannot carry them: the driver
   refuses with a permanent error that names the cause and the remedy (cast
   the column to `text`).
4. Everything else in the execution is unchanged: the same connection, the
   same `BEGIN READ ONLY` and session context, the same row bound, deadline,
   server-side cancellation and connection reset.

Two facts make this safe:

* **Nothing ran.** `prepare` sends only Parse and Describe; the
  failure happens before Bind. It is not the replay of an ambiguous error
  ([I-13](../../CLAUDE.md#i-13)).
* **It is one statement.** The server's Parse accepted the text, and Parse
  refuses more than one command. The simple protocol, which would run several,
  receives a text already proven to hold one.

## Consequences

**Positive.** Every type the server can send reaches the grid: multiranges,
catalog queries on `pg_class`, `pg_rewrite`, `pg_statistic_ext_data`.

**Negative.**

* In the fallback, every column is text, `int4` included: no typed sort, no
  typed export for that result. The marking says so.
* A fallback result **without rows** has no columns: the simple protocol gives
  the column list with the rows, and `sqlx` does not expose it otherwise.
* With bound parameters these types still fail — with a clear message now.
* `sqlx`'s `PgTypeInfo::kind()` **panics** on a type it did not resolve,
  which is what the simple protocol yields: the fallback path must never call
  the typed decoding (`decoding_for`). A test holds it.

**Exit cost.** Low: one branch in `session.rs` and one source variant in
`cursor.rs`. Removing it brings the failures back.

**Reconsider if** `sqlx` resolves `typtype = 'm'` and `typcategory = 'Z'`
and lets a caller choose the result format per column: the extended protocol
could then handle all three cases with typed columns.

## Rejected alternatives

* **Fork `sqlx`** (`[patch.crates-io]`) to accept the two codes and choose
  text per column. Fixes the cause, but maintaining a fork of the driver's
  whole client library for three families of types costs more than the
  fallback; an upstream contribution stays desirable.
* **Retry after the server's Bind error** for `aclitem`. Inside a user
  transaction, the failed Bind aborts it: the retry would fail in turn, and the
  user would lose the transaction. Detecting before Bind costs nothing.
* **Rewrite the user's SQL** (`::text` casts). The SQL the user writes is sent
  as is: that is the feature ([I-10](../../CLAUDE.md#i-10)).
