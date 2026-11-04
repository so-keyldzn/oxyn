# ADR-0069 — The Turso Cloud driver speaks Hrana over HTTP itself, keeps no stream between statements, and reads a whole answer under a budget

**Status:** proposed · **Date:** 2026-11-04

## Context

Turso Cloud hosts libSQL databases, a fork of SQLite, behind an HTTP API. The
maintainer's decisions on the proposal (issue #350) are that it is queried
over HTTP (`/v2/pipeline`) rather than opened as a file, so it needs a driver
of its own; that the SQL dialect stays SQLite's; and that a connection is the
database URL plus a **database** token (`turso db tokens create <db>`), not
the Platform API token. What is expensive to undo is the wire protocol, whether
a library is linked, what a "session" is on a protocol with no connection, and
what happens to a result the server sends whole. Checked on 2026-11-04
([RESEARCH-NOTES](../RESEARCH-NOTES.md#turso-cloud-and-hrana-over-http--checked-on-2026-11-04)):

* **Hrana over HTTP.** `POST <database URL, https>/v2/pipeline`, header
  `Authorization: Bearer <token>`, a JSON body `{"requests": [...]}` where each
  request is `execute` (a statement and typed `args`) or `close`, and a
  `baton` that ties requests to one *stream*. Values are typed:
  `null`, `integer` (a **string**, for 64-bit precision), `float`, `text`,
  `blob` (base64). An `execute` takes **one** statement. A statement error is
  an `error` entry of `results`, with a `message` and a `code` the
  specification says are "not yet stabilized"; an HTTP 4xx/5xx body may be
  anything.
* **A stream is a connection.** It carries the transaction state. Turso closes
  one after 10 seconds without a request, and gives a transaction 5 seconds
  to complete. The specification has no call that interrupts a running
  statement.
* **The answer is one document.** `execute` returns `cols`, **all** `rows`,
  `affected_row_count`, `query_duration_ms`. The specification also defines a
  streamed `cursor` endpoint (`/v3/cursor`); Turso's reference documents only
  `/v2/pipeline`.
* **The libraries.** `libsql` links a C engine, `libsqlite3-sys`, of which the
  graph holds one version ([ADR-0010](0010-contraintes-natives-sqlite.md));
  the HTTP client would be the workspace's `reqwest`.
* **Tokens.** A database token can be created `--read-only` and with an
  expiration (`turso db tokens create`).

## Decision

1. **A crate of its own, `oxyn-driver-turso`, id `turso`, on SQLite's
   dialect.** The protocol is a server's, not a file's
   ([ADR-0003](0003-driver-capabilities.md): one driver per protocol), so it
   cannot be a capability of `oxyn-driver-sqlite`. It speaks Hrana itself and
   depends on **neither `libsql` nor `libsqlite3-sys`**; `dialect_for` maps
   `turso` to `SqlDialect::Sqlite`, so classification, splitting, quoting and
   the AI's dialect prompt are SQLite's. Nothing is shared with the SQLite
   driver's code: the logical-type table and the preview composition are
   copied, with a comment saying so, since a driver may not depend on another.
2. **The workspace's `reqwest`, JSON, no new dependency.** No proxy, no
   redirect (the `Authorization` header would follow it), `https` only, TLS
   verified always ([ADR-0052](0052-verified-tls-outside-local.md)): there is
   no `sslmode` and no way to turn verification off. Base64 is forty lines in
   the crate. The pipeline endpoint is `/v2/pipeline`, the one Turso documents.
3. **Two form fields.** `url`, persisted with the connection, accepts exactly
   `https://<host>[:port]` and `libsql://<host>[:port]` (read as `https://`);
   a user, password, path, query, fragment, any other scheme (`http://`,
   `ws://`, `file:`) is refused with **one fixed text that never echoes the
   input**, since a pasted `?authToken=…` is a secret. `token` is a
   `FieldKind::Password`, so it lives in the keyring's token slot and never in
   the workspace file ([I-03](../../CLAUDE.md#i-03)); the driver reads it from
   `Credentials::token`, sends it as a sensitive `Authorization` header,
   removes it from every server message, and has no `Debug` that shows it. The
   form's help says it is the database token and not the Platform API token.
4. **One statement, one stream, closed in the same request.** Each statement
   is a pipeline of `execute` then `close` with no `baton`. The session keeps
   nothing between statements, so nothing a statement does to a connection
   outlives it, and the statements that would are **refused before sending,
   with a fixed text naming the way out**: `BEGIN`, `COMMIT`, `END`,
   `ROLLBACK`, `SAVEPOINT`, `RELEASE`; `PRAGMA name = value`; `ATTACH`,
   `DETACH`; and what the executor flags as a context change. A `PRAGMA` that
   reads (`PRAGMA table_info(t)`) goes through. `TRANSACTIONS`,
   `SAVEPOINTS`, `MULTIPLE_STATEMENTS` and `SESSION_CONTEXT` are not declared.
   Holding a stream open is not an option the protocol offers a console: 10
   idle seconds would end it under the user's feet.
5. **A whole answer is read under a budget of 32 MiB** of JSON
   (`MAX_RESPONSE_BYTES`): a declared length past it is refused before the
   first byte is read, the body is read chunk by chunk, and the request is
   dropped the moment the budget is crossed ([I-06](../../CLAUDE.md#i-06)). The
   refusal is a permanent `Query` error pointing to `LIMIT`. `max_rows` is held
   by the driver after receipt (the server still sends every row), and marks
   the result truncated. Arrow batches are built from the parsed rows as the
   cursor is drained, closed at 1 MiB of values or 8,192 rows.
6. **Column types come from every value of the result.** Because the answer is
   whole, the type of a column is decided before the first batch from all of
   its values, not from the first batch: integers → `Int64`, floats →
   `Float64`, integers mixed with floats → `Utf8` (no `f64` holds every
   `i64`), text → `Utf8`, a blob with anything → `Binary`; a column with no
   value follows its declared affinity. The declared type, the inference and
   the classes found are kept in the field's metadata. A `DATETIME` is neither
   converted nor given a time zone.
7. **A stop drops the request and promises a verdict, not an interruption.**
   Hrana over HTTP cannot interrupt a statement, so `SERVER_SIDE_CANCEL` is
   **not declared** and the executor does not call `Session::cancel` on this
   driver; the execution's token drops the request. A stopped read is
   `Cancelled`; a statement that may write — an unknown intent counts — is
   `OutcomeUnknown`, never `Cancelled`
   ([DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).
   A request that was never sent is `Cancelled` for both. A statement's
   deadline that elapses is `Timeout`, which is ambiguous.
8. **Errors are classified from where they happened, the statement's intent
   and the server's words.** The table is in the `error` module and tested.
   In short: a connection never made is transient for any statement; a broken
   exchange, an HTTP 5xx/408/429 or an answer that is not Hrana is transient or
   permanent for a read and `OutcomeUnknown` for anything that may write; 401
   and 403 are `Authentication` naming the database token; 404 is `Config`; a
   statement error is permanent (SQLite undid the statement whole), except a
   lock, which is transient for a read only (behind a primary, nothing proves a
   locked write was undone, so it is ambiguous). A body that breaks in flight is
   a failed exchange, not a foreign answer. The codes are the server's
   and "not yet stabilized", so a code decides only where it exists and a
   message only by whole phrases (a column named `locked` is not a lock). The
   driver retries nothing ([I-13](../../CLAUDE.md#i-13)).
9. **Read-only is classification alone.** The server is not asked to enforce
   it, so `READ_ONLY_SESSION` is not declared, and the driver refuses a
   mutating request carrying a read-only bound. The way to have Turso enforce
   it is a token created with `--read-only`; the form does not detect it.
10. **The catalog reads `sqlite_master` and the `pragma_*` table-valued
    functions.** Every statement is a constant; a relation's name is a typed
    argument (`pragma_table_info(?1)`), never in the text
    ([I-10](../../CLAUDE.md#i-10)). One namespace, `main`; tables and views;
    columns with their key rank; indexes (one join, no round trip per index);
    foreign keys with an implicit target resolved to the primary key. A
    preview comes in key order for a rowid table
    ([ADR-0068](0068-a-preview-comes-in-key-order.md)) and not for a virtual
    one. Not read yet, hence not declared: constraints, incoming foreign keys,
    object definitions, a partial index's predicate, `TRUNCATE`.
11. **Bound values are typed `args`.** The n-th value binds the n-th
    positional placeholder; conversions are SQLite's (a decimal stays text, an
    instant is RFC 3339 in UTC, an interval or an array is refused naming its
    rank, never its value). `PREPARED_STATEMENTS` is declared for that promise.

## Consequences

* **+** No second SQLite engine and no dependency: the graph is what it was,
  plus one crate that uses the workspace's `reqwest`, `arrow` and `serde_json`.
* **+** The token has one path: keyring, header, nowhere else, and the tests
  check it against a server that echoes the header back.
* **+** Nothing a statement does outlives its request, so a reconnect, a retry
  by the caller or a second console cannot meet a setting or a transaction
  the user forgot.
* **+** The suite runs against a local server that speaks Hrana: no account,
  no secret, no wall-clock wait.
* **−** **No transaction, no `PRAGMA` setting, no `ATTACH`** in a console. A
  user who needs several statements to be atomic cannot get that from Oxyn
  today.
* **−** **A stop does not stop the statement.** The request is dropped; Turso
  may finish it. The verdict is honest, the effect is not a cancellation, and
  a write that was stopped must be checked by the user.
* **−** **A big result is refused, not truncated.** `SELECT * FROM events`
  without a `LIMIT` on a large table fails with a pointer to `LIMIT` where the
  SQLite driver would show 10,000 rows, because the answer is one document.
  Under the budget, memory is 100 to 300 MiB at the limit (the parsed rows
  weigh 3 to 9 times the JSON), outside the `ResultBuffer` accounting.
* **−** The driver does not know the server's real error codes: they are
  unstable and could not be checked without an account. An unrecognized code is
  a permanent `Query` error, for a write too, resting on the assumption that a
  statement that failed in autocommit applied nothing. That assumption is not
  verified, notably behind write forwarding to a primary; where it is wrong, the
  user reads "refused" for a statement that took effect.
* **−** The catalog depends on `pragma_*` functions being callable on Turso
  Cloud, which was **not checked** (no account). If they are not, every
  introspection fails with a visible error and the tree stays empty.
* **−** Two copies of the logical-type table and of the preview composition
  (here and in the SQLite driver).

**Exit cost:** the crate is self-contained: `wire`, `http` and `convert` are
the protocol and the types, `catalog` and `preview` the SQL. Moving to the
streamed `cursor` endpoint rewrites `http` and the head of `convert`, not the
session, the catalog or the desktop, which see only `RecordBatch`es. Holding a
stream (transactions) would be additive: a declared capability, the statements
refused today then accepted.

**Reconsider if** Turso documents a way to interrupt a statement (then declare
`SERVER_SIDE_CANCEL`), documents `/v3/cursor` as supported on Cloud (then
stream, drop the 32 MiB refusal and make `max_rows` a read stopped early), lets
a stream live long enough for an interactive transaction, or if a verified run
against a real database shows the introspection or the error codes behave
differently from what is assumed here.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| The `libsql` crate (remote mode) | links a C SQLite and its `libsqlite3-sys`, of which the graph holds one version ([ADR-0010](0010-contraintes-natives-sqlite.md)); one more client with its own TLS stack on the widest boundary |
| A capability of the SQLite driver | the protocol is a server's: no file, no `sqlite3_interrupt`, no `rusqlite`; the SQLite driver's worker thread has nothing to offer a request/response protocol |
| Holding a `baton` per console | Turso ends a stream after 10 idle seconds and a transaction after 5; the next statement would run on another stream with a transaction silently gone — the failure the driver contract calls an invisible `SET` |
| Accepting `http://` for a local database | the token would travel in clear; `sqld` on a laptop is a different product from Turso Cloud, and can come back as an explicit option |
| Wrapping the user's `SELECT` in a `LIMIT` to bound the answer | "sent as is, or refused — never rewritten" ([DRIVER-CONTRACT §6](../DRIVER-CONTRACT.md#6-it-escapes-every-identifier-it-composes)) |
| `PRAGMA query_only = 1` before each read, to have the server enforce read-only | works only if Turso allows the pragma, which could not be checked; a read-only session that fails every production read is worse than classification alone |
| The Platform API token | administers the organization and opens no database; the maintainer's decision |
| Declaring `STREAMING` | the answer is one document read whole; the flag would say the opposite of the budget |
