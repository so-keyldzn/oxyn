---
paths:
  - "drivers/oxyn-driver-*/**"
  - "crates/oxyn-driver/**"
---

# Drivers — conventions

The contract is authoritative: [DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md).
It is not summarized here. This rule carries what gets missed while writing.

## Before writing a line

Go through [`/driver`](../commands/driver.md). The command loads the contract,
the checklist and the question that comes before all the others: **is it a new
protocol, or a product that speaks an already implemented protocol?**
Redshift ≡ PostgreSQL, MariaDB ≡ MySQL, OpenSearch ≡ Elasticsearch
([ADR-0003](../../docs/adr/0003-driver-capabilities.md)). One crate too many
means two protocol decoders to maintain for one dialect.

## The four traps

**A batch is sized in bytes, not in rows.** A thousand rows each carrying a
one-megabyte BLOB make a gigabyte. A `batch_size` counted in rows works on demo
tables and triggers the OOM on real ones.

**Cancellation must reach the server.** `pg_cancel_backend`, `KILL QUERY`,
`sqlite3_interrupt`. Dropping the future frees neither the connection nor the
lock: by the tenth closed tab, the database refuses connections and the user
concludes that Oxyn broke their production. A driver that cannot cancel
server-side **declares** it in its capabilities.

**Capabilities are evaluated per session, not per driver.** The server version,
its extensions and the account's privileges change what is available. The same
PostgreSQL driver talks to a version 12 database and to a version 17 one.

**An ambiguous error is not retried** ([I-13](../../CLAUDE.md#i-13)). A timeout
during a write is not transient. It is the case most tempting to handle with a
retry loop, and the one that creates invisible duplicates.

## Types

The mapping table goes **both ways** and documents its losses. What gets lost
most often, and most silently:

- a PostgreSQL `NUMERIC` without precision fits in no floating-point type;
  converting it to `f64` corrupts amounts;
- a `timestamp` without time zone is **never** assigned a time zone on read
  ([DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md#7-it-treats-time-zones-and-temporal-types-as-data-not-as-text));
- a MySQL `u64` beyond 2^53 does not survive a trip through a float;
- an unknown type is returned as raw bytes **with its type identifier**, never
  as a "best effort" string.

## Forbidden

| Forbidden | Why |
|---|---|
| Depending on `oxyn-desktop`, `oxyn-ai`, or another driver | reverses the direction of dependencies. **`oxyn-core`, on the contrary, is the expected dependency** — it is the shared vocabulary, and both shipped drivers depend on it ([DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md)) |
| Reading an environment variable, writing a file | a driver receives its configuration |
| Retrying on its own | the retry policy belongs to the caller, the only one who knows whether the operation can be replayed |
| Undeclared `SET`/`USE` | silently changes the meaning of the user's next queries |
| Logging a bound value | [I-03](../../CLAUDE.md#i-03) |

## Verify

[Checklist](../checklists/revue-driver.md), then `make qualite`. The two tests
that cannot be skipped: **cancellation that proves the server-side stop**, and
**streaming over a volume that would not fit in memory**.
