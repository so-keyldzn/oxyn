# ADR-0050 — The MySQL driver runs on `mysql_async`, prepares every statement first, and decodes every type the server sends

**Status:** accepted · **Date:** 2026-09-30 · **Amended:** 2026-09-30 (point 7, before
acceptance), 2026-10-02 (point 7, issue #139)

## Context

`oxyn-driver-mysql` is the third driver of phase 2's exit gate, postponed until
after phase 3 ([IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md#phase-2--the-protocols-that-matter)).
[ADR-0003](0003-driver-capabilities.md) already settles that it serves MySQL
and MariaDB, one protocol, one crate. What is left open, and expensive to undo
once the driver is written, is the client library, how user SQL crosses the
wire, and how values become Arrow. Checked on 2026-09-30
([RESEARCH-NOTES](../RESEARCH-NOTES.md#mysql-protocol-and-client-libraries--checked-on-2026-09-30)):

* **Servers.** MySQL has two LTS lines, 8.4 (8.4.11, supported to 2032-04-30)
  and 9.7 (9.7.2, to 2034-04-30); 8.0 reached its end of life on 2026-04-30.
  MariaDB's LTS lines are 11.8 (to 2028-06-04) and 12.3 (to 2029-06-12).
* **`sqlx-mysql` 0.9.0 cannot read a `VECTOR` column.** MySQL 9 sends it as
  type code 242 (`MYSQL_TYPE_VECTOR`); `sqlx-mysql` maps codes to a closed enum
  and fails the whole result with `unknown column type 0xf2` — on `main` too. A
  `SELECT *` on a table holding embeddings fails outright. It also drops the
  column's `decimals` (the scale of a `DECIMAL`), keeps raw bytes and the
  transaction status `pub(crate)`, supports neither of MariaDB's `client_ed25519`
  and `parsec` plugins, and needs, for `caching_sha2_password` without TLS, the
  `rsa` crate 0.10.0-rc.18 — a release candidate under RUSTSEC-2023-0071,
  which lists no patched version.
* **`mysql_async` 0.37.1** (MIT OR Apache-2.0) sits on `mysql_common`, which
  knows `MYSQL_TYPE_VECTOR` (0.37.x), exposes each column's type, flags,
  length, `decimals` and character set, implements MariaDB's `client_ed25519`
  and `parsec`, and does its RSA exchange with its own code on `num-bigint`,
  not the `rsa` crate. `Conn::id()` gives the connection id, and
  `Conn::last_ok_packet()` the server's status flags, `SERVER_STATUS_IN_TRANS`
  included. TLS goes through `rustls` with `aws-lc-rs`, the provider the
  workspace already ships.
* **Both libraries announce `CLIENT_MULTI_STATEMENTS` unconditionally** and
  offer no public way to send `COM_SET_OPTION`. A text sent through
  `COM_QUERY` can therefore run several statements. `mysql_async` also
  announces `CLIENT_LOCAL_FILES`: the server may then ask the client for a
  local file by name, and only the absence of a handler refuses it.
* **A prepared statement is one statement, proven by the server.** In MySQL
  8.4's `Prepared_statement::prepare`, the lexer runs with
  `multi_statements = false`, so anything after a `;` is a parse error
  (`ER_PARSE_ERROR`, 1064); only then does `prepare_query` refuse what cannot
  be prepared (`ER_UNSUPPORTED_PS`, 1295). What cannot be prepared, from the
  source: `START TRANSACTION`/`BEGIN`, `SAVEPOINT`, `USE`, `LOCK TABLES`,
  `ALTER VIEW`, `CREATE PROCEDURE`/`FUNCTION`/`TRIGGER`/`EVENT`, `LOAD DATA`,
  `XA`. `SET`, `CREATE VIEW`, `COMMIT`, `ROLLBACK` and `EXPLAIN` can, although
  the manual's list omits some of them. **Observed on 2026-09-30** with
  `mysql_async` 0.37.1 against MySQL 8.4.11 and 9.7.2 and MariaDB 11.8.9 and
  12.3.3: all four refuse a multi-statement text at prepare with 1064 — also
  when its first statement cannot be prepared, and when the `;` sits inside
  `/*! … */` —, while `COM_QUERY` runs it. MariaDB prepares every statement
  tried, `USE`, `START TRANSACTION`, `LOCK TABLES`, `CREATE PROCEDURE` and
  `LOAD DATA` included; MySQL refuses those with 1295.
* **`oxyn-query` splits MySQL wrongly on compound bodies.** Measured on
  2026-09-30: `CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END` becomes
  three fragments. The scanner knows SQLite trigger bodies, not MySQL's
  `BEGIN … END`, and it does not know the `mysql` client's `DELIMITER`.
* **What the servers send, observed the same day.** A MySQL 9.7 `VECTOR(3)`
  arrives as type 242, length 12, packed little-endian `f32`. A MariaDB
  `VECTOR(3)` arrives as `VAR_STRING` with the `binary` character set — the
  same as a `VARBINARY(12)`; only MariaDB's extended metadata, which
  `mysql_async` does not request, would tell them apart. MySQL sends `JSON` as
  type 245 with the `binary` character set (63); MariaDB sends it as `BLOB` in
  `utf8mb4`. A `DECIMAL(p, s)` column's length is `p`, plus one for the sign
  unless `UNSIGNED`, plus one for the point when `s > 0` — checked on `(10,2)`,
  `(65,30)`, `(10,0) UNSIGNED`, `(5,5)`. `mysql_async` decodes a zero date
  (`0000-00-00`, `2024-00-15`) without error, and a `TIME` of `-838:59:59`.
* **A stalled reader stalls the server.** When the client stops reading, the
  server waits `net_write_timeout` (60 s by default) and then aborts the
  statement, holding its locks meanwhile.
* **Cancellation.** `KILL QUERY <id>` stops the statement a connection runs and
  keeps the connection; the flag is read between blocks of rows. Without
  `CONNECTION_ADMIN`, an account can kill only its own threads — the case of a
  second connection of the same account. Observed on the four servers: an
  account with `SELECT` only kills its own query from its second connection,
  the query fails with 1317, and the connection serves the next statement.
* **Read-only mode, observed on the four servers.** Under
  `SET SESSION TRANSACTION READ ONLY`, `INSERT`, `UPDATE`, `DELETE`,
  `CREATE TABLE`, `DROP TABLE`, `TRUNCATE`, `ALTER TABLE`, `RENAME TABLE`,
  `CREATE INDEX` and `CREATE TEMPORARY TABLE` all fail with 1792, in the text
  and the binary protocol alike; `SELECT` runs. The manual allows DML on
  temporary tables, but none can be created in that mode.

## Decision

1. **`oxyn-driver-mysql` depends on `mysql_async`**, without its default
   features, with `default-rustls` (`rustls` on `aws-lc-rs`), `client_ed25519`
   and `client_parsec`. One session holds one `Conn`, opened with
   `stmt_cache_size(0)` and without a `local_infile_handler`: a
   `LOAD DATA LOCAL INFILE` request from the server is then refused, no row is
   loaded, and the connection is closed — observed on the four servers; the
   driver reports it as a lost connection, and a driver test holds it. One crate serves MySQL and MariaDB;
   differences are capabilities read at connection time from the server
   version, not a second crate.
2. **Every user statement is prepared first**: `Conn::prep`, then
   `exec_iter` with no parameters, then `Conn::close` on the statement — with
   the cache disabled, closing is the caller's job, and a statement left open
   counts against `max_prepared_stmt_count`. The server proves the text is one
   statement, and values arrive in the binary protocol.
3. **The text protocol is a fallback on one error only.** When the prepare
   fails with 1295, **and** the request has no bound parameter, **and**
   `oxyn-query` splits the text into exactly one statement, the driver sends the
   same text through `query_iter`. Every other prepare error is the result shown
   to the user. On MariaDB, which prepares everything tried, the fallback does
   not occur in practice. A driver integration test keeps the observation of
   the context on each tested server:
   `CREATE TRIGGER x BEFORE INSERT ON t FOR EACH ROW SET @a = 1; DROP TABLE t`
   fails the prepare with 1064, not 1295.
4. **`oxyn-query` learns MySQL before the driver ships.** Its MySQL scanner keeps
   a `BEGIN … END` body inside `CREATE PROCEDURE`, `FUNCTION`, `TRIGGER` and
   `EVENT` as one fragment, and honors a `DELIMITER` line as the `mysql` client
   does: a directive of the batch, never sent to the server.
5. **The driver never declares `MULTIPLE_STATEMENTS`.** A `CALL` legitimately
   returns several result sets: the first is the result, the next ones are
   counted and reported, never dropped in silence. Any other statement that
   yields a second result set means the server ran more than Oxyn sent: the
   driver drains it, reports it as an error naming that fact, and closes the
   connection.
6. **Session setup, on every connection:** `SET time_zone = '+00:00'`, so a
   `TIMESTAMP` arrives in UTC, and `SET NAMES utf8mb4`. TLS is required
   (`SslOpts` present, certificate verified) unless the connection's
   environment is `Environment::Local`.
7. **Cancellation is `KILL QUERY <id>` from a second connection of the same
   account**, `id` being the connection's `CONNECTION_ID()` read at opening,
   sent by the stream task while it holds the targeted `Conn` — the rule of the
   PostgreSQL driver's `cancel.rs`, for the same reason: the id names a
   connection, not a statement. The task then **keeps reading the targeted
   statement**, and keeps the connection only when the server answers
   `ER_QUERY_INTERRUPTED` (1317) on that very statement: the kill is then
   spent, and the connection — with the user's open transaction, which
   `KILL QUERY` leaves open — serves the next statement. A natural end,
   another error, or no answer within five seconds means the kill cannot be
   proven spent: the task sends `KILL CONNECTION <id>`, which ends a statement
   the query kill missed, and the connection is closed; the session then
   reports its transaction state as unknown and the cursor says the open
   transaction was rolled back. When the cursor stops reading for good — row
   bound reached, result abandoned —, the driver cancels the same way instead
   of leaving the server blocked until `net_write_timeout`; at the row bound it
   first reads ahead a bounded number of rows, so a result the server has
   already finished sending needs no kill. `SERVER_SIDE_CANCEL` is declared per
   session only if the second connection opens.

   *Amended before acceptance:* the first text closed the connection after
   every kill. With one connection per session (point 1), that rolled back the
   user's open transaction each time a console result was truncated by the row
   bound or a tab was closed — uncommitted writes discarded without a word.

   *Amended on 2026-10-02 (issue #139):* the read-ahead and the kill at the
   row bound apply to a read only. A write returning rows — MariaDB's
   `RETURNING` — is drained to its end under the same token and deadline: the
   kill rolled back an autocommit statement whose first rows were on screen,
   and reported nothing. A write stopped by a kill the server did not answer on
   that very statement ends on `OutcomeUnknown`, not `Cancelled`: it may have
   run to its end and committed.
8. **Transaction state** ([ADR-0039](0039-etat-de-transaction-d-une-session.md))
   is read from `SERVER_STATUS_IN_TRANS` after each statement — the server says
   it, the driver does not infer it from the text.
9. **The read-only session is `SET SESSION TRANSACTION READ ONLY`.**
   `READ_ONLY_SESSION` is declared: the four servers refuse DDL and DML in
   that mode (context). A driver integration test keeps that observation, as
   [ADR-0042](0042-revue-sur-place-des-operations-destructrices.md) requires
   for each flag.
10. **Every type the server sends is decoded; none fails the result.**

    | MySQL / MariaDB | Arrow |
    |---|---|
    | `TINYINT` … `BIGINT` | `Int8` … `Int64`; `UInt8` … `UInt64` when `UNSIGNED`. `TINYINT(1)` stays `Int8`: it holds up to 127, not a boolean |
    | `MEDIUMINT` | `Int32` / `UInt32` |
    | `FLOAT`, `DOUBLE` | `Float32`, `Float64` |
    | `DECIMAL(p, s)` | `Decimal128(p, s)` up to `p = 38`, `Decimal256(p, s)` beyond (MySQL allows 65); `s` is the column's `decimals`, `p` its length minus the sign and point positions (context) |
    | `DATE` | `Date32` |
    | `DATETIME` | `Timestamp(Microsecond, None)` |
    | `TIMESTAMP` | `Timestamp(Microsecond, "UTC")` |
    | `TIME` | `Duration(Microsecond)` — its range is ±838:59:59, not a time of day |
    | `YEAR` | `UInt16` |
    | `BIT(n)` | `UInt64` |
    | `CHAR`, `VARCHAR`, `TEXT`, `ENUM`, `SET` | `Utf8`; `Binary` when the character set is `binary` (63). The `BINARY` flag is not the criterion: MariaDB sets it on `utf8mb4` text |
    | `JSON` | `Utf8`, although MySQL announces it in the `binary` character set; on MariaDB it is a `utf8mb4` `BLOB` and falls under the line above |
    | `BINARY`, `VARBINARY`, `BLOB` | `Binary` |
    | `GEOMETRY` | `Binary`: the server's 4-byte SRID followed by WKB, marked `oxyn:mysql_type` |
    | `VECTOR` | MySQL: `Binary` (packed little-endian `f32`), marked `oxyn:mysql_type`. MariaDB: `Binary`, indistinguishable from `VARBINARY` without extended metadata |
    | a type code the driver does not know | `Binary`, with the code in `oxyn:mysql_type` |

    A zero or partial date (`0000-00-00`, `2024-00-15`), which `Date32` and
    `Timestamp` cannot hold, fails the result with a permanent error naming
    the column and the remedy (`CAST(… AS CHAR)`) — the rule the PostgreSQL
    driver applies to `infinity`: neither `NULL` nor an invented date.
    `mysql_async` reads such a date without error: the refusal is the driver's,
    at the conversion to Arrow.

## Consequences

* **+** A `SELECT *` reaches the grid whatever the server holds: vectors,
  geometries, and type codes that do not exist yet.
* **+** `DECIMAL` is typed and exact, where the PostgreSQL driver has to fall
  back to text for `numeric`.
* **+** MariaDB accounts on `client_ed25519` or `parsec` connect, and no
  dependency sits under an advisory without a patch.
* **+** Most user SQL — `SELECT`, DML, `SET`, `CREATE VIEW`, `SHOW` — is proven
  one statement by the server itself; on MariaDB, all of it.
* **−** A second SQL library next to `sqlx`: its own error type, its own TLS
  wiring, its own upgrade path, and a second place where a protocol gap becomes
  ours.
* **−** On MySQL, for non-preparable statements — routines, `USE`,
  `START TRANSACTION`, `LOCK TABLES` — only `oxyn-query`'s split protects against a hidden second
  statement. Point 4 adds scanner work the driver waits on, and a split that
  disagrees with the server stays possible in general.
* **−** One extra round trip per statement (prepare, execute, close): on a
  server 50 ms away, about 100 ms before the first row, to measure against the
  [PERFORMANCE](../PERFORMANCE.md) budget rather than assume.
* **−** A legacy table with zero dates cannot be read with `SELECT *`; the
  user casts the column. It is honest and it is friction.
* **−** `mysql_async` announces `CLIENT_LOCAL_FILES` whatever the options: the
  protection is the missing handler, held by a test, not a flag the server
  sees; and a server that asks costs the session its connection.
* **−** A MariaDB `VECTOR` is shown as raw bytes, without the marking a MySQL
  one gets.
* **−** `mysql_async` rewrites `:name` outside strings and comments into a
  placeholder before preparing, so the server would prepare another statement
  than the one written — `lbl:begin` in a routine is one. The driver refuses
  such a text instead of sending it changed; the user adds a space after the
  colon.
* **−** `mysql_common` 0.37.3 panics decoding the server's internal type codes
  (`TIMESTAMP2`, `DATETIME2`, `TIME2`, `TYPED_ARRAY`, `UNKNOWN`) in the binary
  protocol. A well-behaved server never sends them; the driver checks each
  result set's columns before its first row and refuses such a result. What it
  cannot guard is recorded in [RESEARCH-NOTES](../RESEARCH-NOTES.md#mysql-protocol-and-client-libraries--checked-on-2026-09-30).

**Exit cost:** moderate. Replacing `mysql_async` rewrites the session, the
cursor and the decoding — about the size of the PostgreSQL driver's
`session.rs`, `cursor.rs` and `decode.rs` — but not the catalog queries, the
capabilities or the tests, which speak SQL and Arrow. The type table (point 10)
is the costly part to change: it shapes exports and saved results already
written ([I-11](../../CLAUDE.md#i-11)).

**Reconsider if** `sqlx-mysql` reads unknown type codes without failing and
exposes `decimals` and the raw bytes (a single SQL library would come back);
if either library lets a caller withhold `CLIENT_MULTI_STATEMENTS` or send
`COM_SET_OPTION` (the text protocol would then be safe alone); or if the extra
round trip exceeds the first-batch budget on a measured remote server.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| `sqlx-mysql` 0.9.0, already in the workspace | fails the whole result on a `VECTOR` column, loses the `DECIMAL` scale, cannot authenticate MariaDB's `client_ed25519`/`parsec`, and needs a release-candidate `rsa` under an unpatched advisory without TLS. One library for three drivers does not outweigh a `SELECT *` that fails |
| Text protocol for everything, guarded by `oxyn-query`'s split only | the only multi-statement defense would be client-side, on the path the AI takes ([I-07](../../CLAUDE.md#i-07)); the prepare gives a server-side proof for most statements |
| Prepare only, refusing what MySQL cannot prepare | a client that cannot run `START TRANSACTION`, `USE` or `CREATE PROCEDURE` is not a MySQL client |
| Map `DECIMAL` to `Utf8`, as the PostgreSQL driver does for `numeric` | MySQL sends precision and scale with every column; text would give up a typed sort and export for nothing |
| Map zero dates to `NULL` or to the epoch | invents data; the PostgreSQL driver refuses `infinity` for the same reason |
| Write our own MySQL wire implementation | a protocol, several authentication plugins and a TLS integration to maintain alone |
| A separate MariaDB driver | contradicts [ADR-0003](0003-driver-capabilities.md): the protocol is the same, differences are capabilities |
