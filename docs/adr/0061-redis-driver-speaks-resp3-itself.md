# ADR-0061 — The Redis driver speaks RESP3 itself, proves a read by its own list and the server's table, and stops only what Redis can stop

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
  `readonly` and `blocking`; every `ACL` subcommand lacks `readonly`. The
  flag is the server's word, not a proof: on Redis 8.10.2 the search module
  flags `FT.CURSOR DEL` and the `_FT.DEBUG` subcommands that force a garbage
  collection or change a limit `readonly` (2026-10-24), and a fork, a proxy
  or a hostile endpoint answers whatever table it wants.
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
3. **A read is proven by the driver's own list and the server's `COMMAND`
   table together; the table only ever raises.** The driver compiles in the
   commands proven read-only (`reads.rs`, 157 of them, dated in
   RESEARCH-NOTES): those Redis 8.10.2 and Valkey 9.1.2 both flag `readonly`
   outside `@dangerous`, and the read commands of the modules Redis loads.
   In order: any `ACL` command is `Grant`; a write whose reach the driver
   knows (point 15) is a `Write`, whatever the table says or omits; `EVAL`,
   `EVALSHA` and `FCALL` are `Unknown` even when the table flags them
   `write`, a script's body being unread; a command in `@dangerous` per the
   table is `Unknown`; one the table flags `write` is `Write`; one on the
   list **and** flagged `readonly` by the table is `Read`; anything else — no
   flag, `readonly` but off the list, a name the server does not know, a
   container named without a known subcommand — proves nothing and is
   `Unknown`. A table that flags `SET`, `CONFIG SET` or `FLUSHALL` `readonly`
   therefore proves no read, and one that leaves `FLUSHALL` out does not lower
   it. The driver answers this through `Session::classify_intent`
   (ADR-0064); the table is read once per session.
4. **What changes the connection is refused in a console**, by the driver's
   own list — no flag says it: `UNSUBSCRIBE` and its family, `SYNC`, `PSYNC`, `REPLCONF`, `HELLO`, `AUTH`, `SELECT` (the session context selects
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
   back to the exact bytes. The grammar lives in `oxyn_query::redis::argv`:
   the driver builds the argv it sends with it, and the redaction of point 12
   reads a console text with the same token function, so that what is kept
   and what runs cannot be read two ways.
6. **A read-only execution sends only a proven read** (point 3), and checks
   the driver's list again from the argv alone, on top of the intent: a table
   that lies cannot open it by itself. Redis has no read-only session, so the
   session does not declare `READ_ONLY_SESSION`; `EVAL_RO`, `EVALSHA_RO` and
   `FCALL_RO` qualify, and the server refuses a write inside them.
7. **Stopping:** a `blocking` command is stopped with `CLIENT UNBLOCK <id>
   ERROR` from a second connection, proven by the stopped command's own
   `-UNBLOCKED` (a module's blocking command answers its unblocking with an
   error of its choosing, which proves nothing); an `EVAL`-family or `FCALL`-family script
   with `SCRIPT KILL` or `FUNCTION KILL`, which stop whatever script runs.
   The driver narrows that down: `FUNCTION STATS` must show this session's
   command line for a function, and answers `BUSY` while an `EVAL` runs;
   then the stopped connection is given 250 ms to show a reply of its own —
   this session's command refused with `BUSY` behind another client's
   script — and a reply waiting means nothing is killed. What stays is a race
   of one round trip, in which another client's script that has not written
   is killed; this decision accepts it rather than leave a runaway script of this
   session holding the server. Only the stopped command's own reply proves
   the stop: `Script killed by user`, or `BUSY`. Any
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
   after a command that may write was sent is `OutcomeUnknown`, and so is a
   failure **while sending** it: `write_all` does not say how many bytes left
   before it failed, and a TLS stream accepts the plaintext before its socket
   write fails, so no I/O result proves that no byte reached the server. Only
   a read stays transient.
10. **Replies as Arrow.** The reply's first byte decides the columns: a
    scalar is one typed row (`Int64`, `Float64`, `Boolean`; a big number is
    `Utf8` marked `oxyn:redis_type`); an array, set or push is one row per
    element (`index`, `type`, `value`, `value_bytes`, `truncated`); a map is
    one row per entry. A string is `value` (`Utf8`, verbatim) when its bytes
    are UTF-8 and `value_bytes` (`Binary`) when not — never escaped, never
    guessed. A nested aggregate is one cell in a compact JSON-like text. A
    batch closes at 1 MiB, a value is cut at 16 MiB and its row says so. The
    command chooses a richer shape for the `SCAN` family, `ZRANGE … WITHSCORES`,
    `XRANGE`/`XREVRANGE`, `TS.RANGE`, `INFO`, `SLOWLOG GET` (one row per
    entry, its duration in microseconds) and `CLIENT LIST`/`CLIENT INFO`
    (one row per client: its usual fields as columns, and its whole line, so
    that a field a later server adds is not lost). An array of maps —
    `XINFO GROUPS`, `XINFO CONSUMERS`, `ACL LOG`, `MODULE LIST`,
    `FUNCTION LIST` — is one row per map, its columns the first map's keys,
    read before the first batch; a key the first map lacked goes to
    `other_fields`. A reply abandoned before its end costs
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
    `masteruser`, `tls-key-file-pass` and `tls-client-key-file-pass`, and
    `MIGRATE`'s `AUTH`/`AUTH2` passwords
    (`oxyn_query::redis::redact_secrets`). The decision is taken on the
    **decoded argv**, read by the execution's own grammar (point 5), and the
    replacement covers the argument as written: `"A\x55TH" hunter2` is `AUTH`,
    `"\x3ehunter2"` a `>` rule. What the driver refuses — an open or glued
    quote, a NUL — is still read, keeping the most bytes in one argument.
    A sink that knows the language redacts by it — the history, the journal
    entry built from an `Execute`. One that does not — a tool-call record and
    its re-encoding, an audit entry written back, a saved query mentioned to
    an agent — applies every grammar (`oxyn_query::redact_any_language`):
    forcing SQL there kept `CONFIG SET requirepass` whole, in the workspace
    and in the next prompt. The cost is a SQL line starting with a Redis
    credential keyword losing an argument in the kept copy, never in what
    runs.
13. **Pub/Sub and `MONITOR` are bounded captures.** `SUBSCRIBE`,
    `PSUBSCRIBE`, `SSUBSCRIBE` and `MONITOR` run on a connection of their
    own, never the console's; what arrives becomes rows until the row limit,
    a window that closes one second before the execution's timeout, or Stop.
    Closing that connection ends the subscription or the monitor on the
    server, and a stop is `Cancelled`: a capture applies nothing. Their
    intent stays the server's — neither command is flagged `readonly`, and
    `MONITOR` is `@dangerous` —, so a capture is confirmed on production and
    refused to an agent there. A message is dated by Oxyn's clock on arrival,
    a `MONITOR` line by the server's.
14. **The assistant has an agent of its own on Redis**,
    `crates/oxyn-ai/agents/redis.md` (`languages: [redis]`, ADR-0065): never
    `KEYS`, a key sized before it is read whole, the scope of a write shown
    before the write, no transaction to lean on. What it sends is classified
    by the session as point 3 says, as any console request.
15. **A write that reaches everything in its target carries a risk**
    (`Session::classify_risk`, ADR-0066), from the driver's own list — no
    flag says it: `FLUSHALL` and `FLUSHDB` are `Truncate`, `SWAPDB`
    `UnboundedUpdate`, and `FUNCTION FLUSH`, `FUNCTION DELETE`,
    `FUNCTION RESTORE … FLUSH`, `XGROUP DESTROY`, `ACL DELUSER`,
    `FT.DROPINDEX` and `FT.DROP` are `DropObject`. A risk is answered only for
    a request proven mutating, never for a read. `FLUSHALL`, `FLUSHDB`,
    `SWAPDB` and `FT.DROPINDEX` are `write` and `@dangerous`: left `Unknown`,
    a human's `FLUSHALL` on staging would run at one click, so a command on
    this list is a `Write` before the table is read — a table that flags it
    otherwise or leaves it out changes nothing —, and its risk asks for an
    approval on every environment. A key named explicitly is bounded, however
    large: `DEL`, `UNLINK` and `MIGRATE` carry no risk. A script carries none
    either — `EVAL` proves no intent, stays `Unknown` and is never asked for
    its risk, so a `redis.call('flushall')` inside it is confirmed as a plain
    write on production and runs on staging; this decision accepts it, a
    script's body being code the driver does not read.

## Consequences

* **+** A `HGETALL`, `LRANGE 0 -1` or `SMEMBERS` of any size reaches the grid
  in bounded batches; a 512 MB value costs 16 MB.
* **+** Redis 7, Redis 8 and Valkey, with or without modules, flag their own
  writes; a server whose table lies cannot make a write pass for a read, on
  the gate or on a read-only execution.
* **+** Nothing is sent that the user did not type or the form did not
  declare: no `CLIENT SETINFO`, no automatic reconnection inside a command, no
  retry.
* **−** A protocol decoder to maintain: about 500 lines, and the RESP3 corner
  cases a library would have met first.
* **−** `PING`, `INFO`, `KEYS`, `CONFIG GET` and every script are `Unknown`:
  confirmed on production, refused to an agent there. The server's flags say
  nothing safer, and guessing would be the mistake I-02 forbids.
* **−** A read the list does not hold — a command of the next release, of a
  module it does not name — is `Unknown` until the list is updated:
  confirmed on production, refused to an agent there, refused on a read-only
  execution. The list is dated, and re-observed with each server version.
* **−** No read-only session: a read-only execution rests on the driver
  refusing what it cannot prove a read.
* **−** A slow non-blocking command cannot be stopped: Stop closes the
  connection, the server finishes the command. A production `KEYS *` on fifty
  million keys stalls the server whatever Oxyn does — which is why it needs a
  confirmation there.
* **−** Cluster, Sentinel and `MULTI/EXEC` are refused in this stage.
* **−** A capture is a window, not a subscription that lives on: a message
  published after it closes is not seen. `MONITOR` slows a busy server for
  as long as the window lasts.
* **−** **Known limit, open decision.** The catalog of decision 11 lists
  *exact* keys, and the AI context writes every relation it keeps by its name,
  whatever its kind ([AI-PROVIDERS](../AI-PROVIDERS.md)): under a tier that
  carries names, the exact names of the keys selected for a question reach the
  prompt, and a key name can embed data itself. This ADR does not decide
  whether to keep sending them or to send key patterns instead: that choice is
  the maintainer's, and issue
  [#339](https://github.com/so-keyldzn/oxyn/issues/339) (opened on 2026-10-25)
  poses it. Until it is taken, AI-PROVIDERS states what goes out.

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
| Classifying writes from a list written in the driver | wrong for every module command and for the next Redis or Valkey release; the server describes its own writes, and a write it flags is a write |
| Trusting the server's `readonly` flag alone | the flag is the server's word: a hostile or odd table makes `SET` or `CONFIG SET` a read on the gate and on the read-only guard, and Redis 8.10.2's own search module flags `FT.CURSOR DEL` `readonly` |
| Proving "nothing was sent" from a failed write | `write_all` hides the count, and TLS accepts plaintext before the socket fails: a retried write could apply twice (I-13) |
| A second, lighter tokenizer for the redaction | it disagreed with the execution on escapes: `"\x3ehunter2"` ran as a password rule and was kept as written |
| Treating `@dangerous` reads (`KEYS`) as reads | a read the server itself files as dangerous stalls a production instance; a confirmation is the least |
| Following `SCAN` cursors automatically in the console | sends commands the user did not type; the page's cursor is shown on every row instead |
| `SCRIPT KILL` on every Stop of a script | it stops whatever script runs, possibly another application's |
| Escaping non-UTF-8 bytes into the text column | a value copied from the grid into a `SET` would write different bytes |
| One crate per product (Redis, Valkey, KeyDB) | contradicts ADR-0003: one protocol |
