---
description: Implement or modify a database driver
argument-hint: "<protocol or crate, e.g. postgres>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, Skill, WebFetch
---

Purpose: implement or modify the **$ARGUMENTS** driver.

This command exists because `crates/` may be empty: the
`.claude/rules/drivers.md` rule only loads when Claude *reads* an existing
file, hence never for the first file of a new driver.

## Before writing

1. Read `docs/DRIVER-CONTRACT.md` **in full**. It is not a formality: each of
   the seven guarantees is an incident already anticipated.
2. Read `docs/adr/0003-driver-capabilities.md` and
   `docs/adr/0002-arrow-result-model.md`.
3. Read `.claude/rules/drivers.md`.

## The question that comes before all the others

**Is it a new protocol, or a product that speaks an already implemented
protocol?**

Redshift speaks PostgreSQL. MariaDB speaks MySQL. OpenSearch speaks
Elasticsearch. Memgraph speaks Bolt. pgvector and TimescaleDB are PostgreSQL
extensions.

If the protocol already exists, **there is no crate to create**: the difference
is declared as capabilities. Creating the crate anyway condemns you to maintain
two protocol decoders for one dialect — and to fix every bug twice, forgetting
one time out of two.

## Tools rather than memory

| Need | Tool |
|---|---|
| Version of a client crate | `/versions`, never memory ([I-12](../../CLAUDE.md#i-12)) |
| Exact semantics of a server type | the DBMS's official documentation, allowed in `permissions.allow` |
| Signature of a client API | docs.rs, allowed |

## Constructs to use / never

| Use | Never | Why |
|---|---|---|
| Stream of `RecordBatch` | a full `Vec<Row>` | [I-06](../../CLAUDE.md#i-06): OOM on a click in the sidebar |
| Batch bounded **in bytes** | batch bounded in number of rows | a thousand rows × 1 MB of BLOB = 1 GB |
| Cancellation reaching the server | dropping the future | the query is still running and holds a connection |
| `Capabilities` evaluated per session | per driver | the server version changes what exists |
| Error classified transient / permanent / **ambiguous** | an `is_retryable` boolean | the ambiguous one is not retried ([I-13](../../CLAUDE.md#i-13)) |
| Identifier quoting by the driver | `format!("SELECT * FROM {}", name)` | [I-10](../../CLAUDE.md#i-10) |

## The traps, with their failure scenario

**The batch counted in rows.** Works on demo tables, triggers the OOM on real
ones. The symptom is a process killed without a trace on macOS.

**The cancellation that only cancels the future.** The user closes ten tabs;
ten queries are still running server-side, holding ten connections. The
database refuses new connections, and the user concludes that Oxyn broke their
production.

**The replayed ambiguous error.** An `INSERT` times out client-side while the
server applied it. Classified transient and replayed: a duplicate in the data,
no error anywhere. It is the case most tempting to handle with a retry loop,
and the most expensive.

**`NUMERIC` converted to `f64`.** PostgreSQL accepts arbitrary precision; `f64`
does not. Amounts are corrupted, silently, and the corruption is permanent once
copied into an `UPDATE`.

**The time zone invented on read.** A `timestamp` without time zone given the
workstation's: the user copies the displayed value into an `UPDATE` and shifts
the data by two hours in the database.

**The silent `SET search_path`.** Changes the meaning of all the user's
subsequent queries, without them having asked for it or being able to see it.

## Verify

```bash
make qualite
```

Then `.claude/checklists/revue-driver.md`, in full. The two tests that cannot be
worked around:

1. **cancellation proven server-side** — checked in the DBMS's process view,
   not on the function's return;
2. **streaming over a volume that would not fit in memory** — with a bound on
   the process's memory, otherwise the test passes by accident.

Finally, run the `relecteur-invariants` agent on the result.

## Reminders

- a driver depends only on `oxyn-core`, `oxyn-driver`, `oxyn-data` and
  `oxyn-catalog`;
- a driver never retries on its own: the retry policy belongs to the caller,
  the only one that knows whether the operation can be replayed;
- not knowing how is an acceptable answer, declared as a capability; pretending
  is not.
