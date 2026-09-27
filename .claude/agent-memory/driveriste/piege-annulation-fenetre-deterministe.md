---
name: piege-annulation-fenetre-deterministe
description: sqlite3_interrupt targets the connection and its flag is cleared at statement start; pg_cancel_backend targets a process reused by the pool — and how to make these windows reproducible in tests
metadata:
  type: reference
---

Found on 2026-09-16 while fixing two mistargeted cancellations.

## SQLite (source 3.50.2 embedded by libsqlite3-sys 0.35.0)

`sqlite3_interrupt` sets `db->u1.isInterrupted` on the **connection**. It is
reset to 0 in `sqlite3Step` and `sqlite3RunParser` only if `nVdbeActive == 0`.
So: an interruption arriving during the `step` of the **next** query kills it;
an interruption set between two statements or during a preparation **is lost**.
You have to check under lock that the targeted task is indeed the running one,
and keep a flag specific to the task.

Deterministic test: a task that reads **one** row (active statement), signals,
waits for a go-ahead on a std channel, then reads the rest. The "active
statement" state is then guaranteed at the moment the test interrupts.

## PostgreSQL

`pg_cancel_backend(pid)` targets a process; the sqlx pool hands out the same
connection — same pid — on the next borrow. Only whoever **holds** the
connection can cancel safely. `PoolConnection` returns to the pool
asynchronously (task spawned on drop): a few ms.

Test tools that worked:
* **advisory locks** held by a control connection
  (`pg_advisory_xact_lock($1)` in the tested query), waiting on
  `pg_locks WHERE locktype='advisory' AND NOT granted AND classid=0 AND objid=($1::bigint)::oid`;
* a pool of **two** connections: a single idle one ⇒ sqlx hands it out for sure;
* `pg_stat_activity.wait_event = 'ClientWrite'` proves the client no longer
  reads its socket (back-pressure on the streaming task side);
* an assertion "the process is gone" is specific to a design that closes the
  connection: for a proof valid in both directions of a mutation, prefer
  "`state <> 'active'`".

## Blocking `execute` between the `SET` and the preparation (2026-09-17)

Preparing (Parse) takes `ACCESS SHARE` on the cited tables: a control connection
in `BEGIN; LOCK TABLE t IN ACCESS EXCLUSIVE MODE` blocks `execute` **after**
`SET search_path` and `BEGIN READ ONLY`, visible in `pg_locks`
(`NOT granted`, join on `pg_class`). `SET` and `BEGIN` themselves never block:
no server barrier for their own round trip.

sqlx-core 0.9.0: `PoolConnection::close_on_drop` cannot be reverted; `Drop`
without this flag returns the connection as is. For "close unless reset to
default", a guard that only calls it in its own `Drop`.

Protocol: `ROLLBACK`/`COMMIT` outside a transaction are **not** errors —
`WARNING: there is no transaction in progress`, invisible on the sqlx side. On a
pool, `BEGIN; INSERT; ROLLBACK` executed separately "succeed" and the `INSERT`
stays committed (checked 17.11, 2026-09-17). A text opens a transaction block
only with `BEGIN` or `START TRANSACTION` at its head; sqlx-postgres 0.9.0 keeps
`in_transaction` as `pub(crate)`.

Mutation trap: a corrective `SET` issued by the cursor after each execution
**masks** the absence of a setting applied at opening. Test the opening setting
on a fresh session, before any writable execution.

See [[outil-cluster-postgres-jetable]].
