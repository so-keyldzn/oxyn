# ADR-0061 — The Redis driver speaks RESP3 itself, classifies commands from the server's own table, and stops only what Redis can stop

**Status:** proposed · **Date:** 2026-10-22

## Context

`oxyn-driver-redis` is the first driver whose server has no schema, no tables
and no SQL. [ADR-0003](0003-driver-capabilities.md) settles that Redis and
Valkey are one driver — both speak RESP —, and that the session declares what
it can do. What is left open, and expensive to undo, is the client, the shape
of a reply as Arrow, how a command's intent is known, and what a stop does.
Checked on 2026-10-22 ([RESEARCH-NOTES](../RESEARCH-NOTES.md#redis-protocol-and-client--checked-on-2026-10-22)):

* **Servers.** Redis 8.10.2 (`redis:8.10.2`, Docker Hub, 2026-10-22) and Valkey
  9.1.2 (`valkey/valkey:9.1.2`, 2026-09-21). Both answer `HELLO 3` with a map
  holding `server`, `version`, `proto`, `id`, `mode`, `role` and `modules`.
  Redis 8.10.2 knows 447 commands (`COMMAND COUNT`), modules included.
* **The protocol.** RESP3 adds nulls, booleans, doubles (`inf`, `-inf`,
  `nan`), big numbers, verbatim strings, maps, sets, attributes and pushes to
  RESP2. Redis emits neither streamed strings nor streamed aggregates. A bulk
  string is at most 512 MB by default (`proto-max-bulk-len`). An error reply
  starts with an uppercase prefix; the documentation calls it a convention.
* **`redis` (redis-rs) 1.7.1** (BSD-3-Clause, MSRV 1.88, 2026-09-25):
  `MultiplexedConnection::send_packed_command` returns a whole
  `RedisResult<Value>` — the reply is parsed into a tree before the caller sees
  any of it; its `Value::Array` is a `Vec`. Connection setup pipelines
  `CLIENT SETINFO LIB-NAME redis-rs` and `LIB-VER` unless told not to;
  `ConnectionManager` reconnects on its own.
* **Command metadata.** `COMMAND` returns, per command, its flags (`readonly`,
  `write`, `blocking`, `admin`, `pubsub`, `noscript`…), its ACL categories
  (`@read`, `@write`, `@dangerous`, `@scripting`, `@blocking`…) and its
  subcommands (`config|set`). Observed on Redis 8.10.2: `KEYS` is `readonly`
  and `@dangerous`; `FLUSHALL` is `write` and `@dangerous`; `EVAL` carries
  neither `readonly` nor `write`; `EVAL_RO` is `readonly`; `PING` carries only
  `fast`; `INFO` is `@dangerous`; `BLPOP` is `write` and `blocking`; `XREAD` is
  `readonly` and `blocking`; every `ACL` subcommand lacks `readonly`.
* **Stopping.** `CLIENT UNBLOCK <id> ERROR` (`@admin`, `@dangerous`) unblocks a
  client blocked by a blocking command; it then reads `-UNBLOCKED`.
  `SCRIPT KILL` and `FUNCTION KILL` stop the running script — whichever it is —
  unless it already wrote, in which case they answer `-UNKILLABLE`
  (`src/script.c`, `scriptKill`, Redis 8.10.2); only `SHUTDOWN NOSAVE` stops
  such a script. Any other command runs to its end: Redis is single-threaded
  and has no interruption point.
* **Errors.** The prefixes Redis 8.10.2 writes (`src/*.c`): `ERR`, `WRONGTYPE`,
  `NOAUTH`, `WRONGPASS`, `NOPERM`, `NOPROTO`, `NOSCRIPT`, `NOGROUP`,
  `BUSYGROUP`, `BUSYKEY`, `EXECABORT`, `LOADING`, `BUSY`, `OOM`, `MISCONF`,
  `MASTERDOWN`, `NOREPLICAS`, `READONLY`, `TRYAGAIN`, `CLUSTERDOWN`,
  `CROSSSLOT`, `MOVED`, `ASK`, `UNBLOCKED`, `UNKILLABLE`, `NOTBUSY`, `DENIED`,
  `IOERR`. The transient ones are refused in `processCommand` before the
  command runs.
* **Session classification.** `oxyn-exec` reclassifies every request from its
  text with `oxyn-query`, which answers `Unknown` for every language that is not
  SQL: without a source of truth for Redis, `GET` would need the production
  confirmation and be refused to an agent. A `Session::classify_intent` hook,
  answered by the driver without I/O, is decided separately in ADR-0064, for
  every non-SQL driver.

## Decision

1. **The driver reads and writes RESP3 itself**, over `tokio` (TCP or Unix
   socket) and `tokio-rustls` 0.26 on `aws-lc-rs`, with `webpki-roots` for the
   public roots. No Redis client library is a dependency. The reader hands out
   one *head* at a time, so an aggregate becomes rows element by element;
   lengths from the server never size an allocation beyond 1,024 elements; a
   line is bounded at 1 MiB; nesting deeper than 32 levels is skipped
   iteratively and marked; streamed RESP3 types are refused as malformed.
   A connection counts the replies it sent for and has not read to their
   end; one that is not in step — its reader dropped by a deadline or a
   closed tab — is closed and never written on again.
2. **Handshake and declared state:** `HELLO 3 [AUTH user password] SETNAME
   oxyn`, then `SELECT n` when the form names a database other than 0. A server
   without `HELLO` (Redis before 6) is refused. A server whose `HELLO` says
   `cluster` or `sentinel` is refused with a message naming the mode: a cluster
   would answer `MOVED`, which the driver never follows silently.
3. **The intent comes from the server's `COMMAND` table, read once per
   session**: a subcommand of `ACL` without `readonly` is `Grant`; `readonly`,
   not `write`, outside `@dangerous` is `Read`; `write` outside `@dangerous` is
   `Write`; anything else — `@dangerous`, no flag, a script, a name the server
   does not know, a container named without a known subcommand — proves
   nothing and is `Unknown`. The driver answers this through
   `Session::classify_intent` (ADR-0064).
4. **What changes the connection is refused in a console**, by the driver's
   own list — no flag says it: `SUBSCRIBE` and its family, `MONITOR`, `SYNC`,
   `PSYNC`, `REPLCONF`, `HELLO`, `AUTH`, `SELECT` (the session context selects
   the database), `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH`, `RESET`, `QUIT`,
   `READONLY`/`READWRITE`/`ASKING`, and `CLIENT TRACKING`, `CACHING`, `REPLY`,
   `SETNAME`, `SETINFO`, `NO-EVICT`, `NO-TOUCH`. Blocking commands are sent;
   the execution's timeout and the Stop bound them.
5. **An execution is one command**, read as `redis-cli`'s `sdssplitargs`
   reads it; a second line outside quotes and a typed NUL are refused. A
   console holds one command per line, and Run without a selection sends the
   line under the cursor, a line break inside quotes kept in its argument
   (`oxyn_query::redis::current_command`); a quote left open makes the
   boundary unknown, and a selection is asked for. Keys are
   shown, previewed and composed through the inverse rendering, which parses
   back to the exact bytes.
6. **A read-only execution sends only what the server flags `readonly`**
   outside `@dangerous`. Redis has no read-only session, so the session does
   not declare `READ_ONLY_SESSION`; `EVAL_RO` and `FCALL_RO` qualify, and the
   server refuses a write inside them.
7. **Stopping:** a `blocking` command is stopped with `CLIENT UNBLOCK <id>
   ERROR` from a second connection, proven by the stopped command's own
   `-UNBLOCKED` (a module's blocking command answers its unblocking with an
   error of its choosing, which proves nothing); an `EVAL`-family or `FCALL`-family script
   with `SCRIPT KILL` or `FUNCTION KILL`, sent only when the running script is
   this session's: `FUNCTION STATS` shows a function's command line, and
   answers `BUSY` while an `EVAL` runs — that one is then the `EVAL` this
   session still waits on, since the server refuses every other command
   meanwhile. Only the stopped command's `Script killed by user` error proves
   the kill. Any
   other command, and a script that already wrote, cannot be stopped: the
   driver closes the connection, which frees the reply the server writes, and
   says the command runs on. `SERVER_SIDE_CANCEL` is declared per session when
   the account may send `CLIENT UNBLOCK`.
8. **Verdicts.** A stop ends on `Cancelled` (or `Timeout`) only when the
   command was never sent, when the server answered the stopped command with
   its own error, or when the server flags the command `readonly`. Otherwise
   it ends on `OutcomeUnknown`, naming what may have happened.
9. **Error classes from the prefix:** `LOADING`, `BUSY`, `OOM`, `MISCONF`,
   `NOREPLICAS`, `MASTERDOWN`, `TRYAGAIN`, `CLUSTERDOWN` are transient; every
   other prefix is permanent. Any error of a script (`@scripting`, or an
   `EVAL`/`FCALL` name when the table is unreadable) but `NOSCRIPT` is
   ambiguous, whatever its prefix: Redis keeps what a failing script wrote,
   and the script chooses the prefix — a `TRYAGAIN` it returns after a
   `DECRBY` is not the server's refusal. `IOERR` is ambiguous: `MIGRATE`'s
   transfer failed and the key may be on both instances. A connection lost
   after a command that may write was sent is `OutcomeUnknown`.
10. **Replies as Arrow.** The reply's first byte decides the columns: a
    scalar is one typed row (`Int64`, `Float64`, `Boolean`; a big number is
    `Utf8` marked `oxyn:redis_type`); an array, set or push is one row per
    element (`index`, `type`, `value`, `value_bytes`, `truncated`); a map is
    one row per entry. A string is `value` (`Utf8`, verbatim) when its bytes
    are UTF-8 and `value_bytes` (`Binary`) when not — never escaped, never
    guessed. A nested aggregate is one cell in a compact JSON-like text. A
    batch closes at 1 MiB, a value is cut at 16 MiB and its row says so. The
    command chooses a richer shape for the `SCAN` family, `ZRANGE … WITHSCORES`,
    `XRANGE`/`XREVRANGE` and `INFO`. A reply abandoned before its end costs
    the connection; the session reopens it with the same declared state.
11. **The catalog:** databases are catalogs named `db0`, `db1`… (from
    `CONFIG GET databases`, or `INFO keyspace` when the account may not read
    the configuration), at most 256 of them besides the non-empty ones and
    the form's; a database lists at most 1,000 keys and 1 MiB of key names,
    found by at most 50 `SCAN … COUNT 1000` calls of at most 4 MiB each, each
    key with its type, as `KeyPattern` relations; `KEYS` is never sent. A key
    name is written in the quoted form the console reads back to the same
    bytes, invisible format characters escaped. The catalog and each preview
    use their own connections, so browsing never moves the console's
    database.
12. **Credentials typed in a console are redacted** before the history, the
    journal or a conversation keeps the text, as ADR-0053 does for SQL: the
    password of `AUTH` and `HELLO … AUTH`, the `>`/`<`/`#`/`!` rules of
    `ACL SETUSER`, the value of `CONFIG SET` for `requirepass`, `masterauth`,
    `tls-key-file-pass` and `tls-client-key-file-pass`, and `MIGRATE`'s
    `AUTH`/`AUTH2` passwords (`oxyn_query::redis::redact_secrets`).

## Consequences

* **+** A `HGETALL`, `LRANGE 0 -1` or `SMEMBERS` of any size reaches the grid
  in bounded batches; a 512 MB value costs 16 MB.
* **+** Redis 7, Redis 8 and Valkey, with or without modules, classify their
  own commands; a module command is `Unknown` until its module flags it.
* **+** Nothing is sent that the user did not type or the form did not
  declare: no `CLIENT SETINFO`, no automatic reconnection inside a command, no
  retry.
* **−** A protocol decoder to maintain: about 500 lines, and the RESP3 corner
  cases a library would have met first.
* **−** `PING`, `INFO`, `KEYS`, `CONFIG GET` and every script are `Unknown`:
  confirmed on production, refused to an agent there. The server's flags say
  nothing safer, and guessing would be the mistake I-02 forbids.
* **−** No read-only session: a read-only execution rests on the driver
  refusing what the server does not flag `readonly`.
* **−** A slow non-blocking command cannot be stopped: Stop closes the
  connection, the server finishes the command. A production `KEYS *` on fifty
  million keys stalls the server whatever Oxyn does — which is why it needs a
  confirmation there.
* **−** Cluster, Sentinel, `MULTI/EXEC`, Pub/Sub and `MONITOR` are refused in
  this stage.

**Exit cost:** moderate. Replacing the hand-written RESP layer with a library
rewrites `resp.rs`, `connection.rs` and the cursor's reading loop, but not the
classification, the shapes or the tests, which speak RESP values and Arrow. The
shapes (decision 10) are the costly part to change: they reach exports and
saved results ([I-11](../../CLAUDE.md#i-11)).

**Reconsider if** a maintained Rust Redis client exposes a reply element by
element with bounded buffering; if Redis gains a read-only session or an
interruption point for running commands; or when Cluster support is decided,
since slot routing would change the connection model.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| `redis` 1.7.1 | parses a whole reply into a `Value` tree before returning it: a large collection is materialized in full (I-06). It also sends `CLIENT SETINFO` and reconnects on its own unless configured not to |
| Classifying commands from a list written in the driver | wrong for every module command and for the next Redis or Valkey release; the server describes its own commands |
| Treating `@dangerous` reads (`KEYS`) as reads | a read the server itself files as dangerous stalls a production instance; a confirmation is the least |
| Following `SCAN` cursors automatically in the console | sends commands the user did not type; the page's cursor is shown on every row instead |
| `SCRIPT KILL` on every Stop of a script | it stops whatever script runs, possibly another application's |
| Escaping non-UTF-8 bytes into the text column | a value copied from the grid into a `SET` would write different bytes |
| One crate per product (Redis, Valkey, KeyDB) | contradicts ADR-0003: one protocol |
