# ADR-0060 — The ClickHouse driver speaks the HTTP interface, reads `RowBinaryWithNamesAndTypes` itself, and keeps no server session

**Status:** proposed · **Date:** 2026-10-23

## Context

ClickHouse is a column-oriented analytical server. It is a protocol of its own,
not a product speaking one Oxyn already implements
([ADR-0003](0003-driver-capabilities.md)): `SqlDialect::ClickHouse` exists in
`oxyn-core` since phase 1, but no driver serves it. What is expensive to undo
once the driver is written is the wire protocol, the client library, how
values become Arrow, and what a "session" is. Checked on 2026-10-23
([RESEARCH-NOTES](../RESEARCH-NOTES.md#clickhouse-protocol-and-client-libraries--checked-on-2026-10-23)):

* **Two protocols.** An HTTP interface (8123, 8443 over TLS) and a native TCP
  protocol (9000, 9440). Both carry SQL; the HTTP one is the one every
  managed offering exposes and every proxy understands.
* **Server lines.** Docker Hub publishes 26.9.25.15 (latest), 26.3.59.10 and
  25.8.33.6 — the `.3` and `.8` lines being the long-term ones, still patched.
  The driver is tested on 26.3.
* **`clickhouse` 0.15.3** (the official crate, HTTP) decodes rows into Rust
  types known at compile time through `serde`; it has no untyped row, no
  `Dynamic`, and no raw byte stream of a chosen format. Oxyn runs arbitrary
  user SQL whose schema is known only when the answer arrives.
* **`clickhouse-arrow` 0.2.1** (native protocol to Arrow) depends on
  `arrow ^57` while the workspace is on 59, and on `webpki-roots` rather than
  the platform verifier. **`klickhouse` 0.16.0** (native) decodes into its own
  value model. **`clickhouse-rs` 0.1.21** was last published in 2023-09.
* **`FORMAT ArrowStream`** looks like the shortest path, and is not: on output
  ClickHouse writes `DateTime` as `UINT32` (its zone lost), `Date32` as
  `UINT16`, `Int128`/`UInt128`/`Int256`/`UInt256` and `IPv6` as
  `FIXED_SIZE_BINARY`, `Enum` as its integer (the label lost), and `JSON`,
  `Dynamic` or an aggregate state as opaque binary by default
  (`output_format_arrow_unsupported_types = binary`). Every one of those needs
  a per-type rewrite after decoding, from a type name Arrow does not carry.
* **An exception raised after the first byte travels in the body.** Once rows
  have left, the status is already 200; ClickHouse 26.3 then appends
  `\r\n__exception__\r\n<tag>\r\n<message>…` to the body, `<tag>` being the
  random text of the `X-ClickHouse-Exception-Tag` header (observed). An
  Arrow IPC reader, like any decoder, reads it as data unless something cuts
  it out first.
* **HTTP is stateless** unless a `session_id` is sent; `SET`, `USE` and
  temporary tables live in that session. The server refuses two statements in
  one request (`Multi-statements are not allowed`, observed).
* **No transaction** in the general case. An `INSERT … SELECT` writes block by
  block and keeps the blocks before a failure; a mutation
  (`ALTER TABLE … UPDATE/DELETE`) is queued and keeps running after the
  statement that created it returned, and after `KILL QUERY`.
* **Read-only is the server's**: `readonly = 1` refuses writes, DDL, table
  functions that reach out (`url()`), and any setting the statement itself
  carries — a `SELECT … SETTINGS max_threads = 2` included (observed, code
  164). `readonly = 2` lets settings and `url()` through. Neither can be
  lowered by a request.
* **`KILL QUERY WHERE query_id = …`** stops a statement from another request;
  the account needs the right even for its own statement. `KILL … TEST` checks
  the right without stopping anything, and answers 200 with an empty body and
  no result format when nothing matches (observed).

## Decision

1. **The HTTP interface, through the workspace's `reqwest`** (rustls on
   `aws-lc-rs`, the platform verifier), with no proxy: a system proxy would
   carry the password and the data through a host nobody configured. TLS
   follows [ADR-0052](0052-verified-tls-outside-local.md): `verify-full` is
   the only mode outside a connection marked Local. The password travels in
   the `X-ClickHouse-Key` header, marked sensitive, never in the URL.
2. **One statement, one request, no `session_id`.** Each request carries what
   decides its meaning: the database (the context selector's, through the
   `database` parameter — [ADR-0019](0019-contexte-de-session.md)), the output
   format, the read-only bound, a `query_id` the driver chose. A typed `USE`,
   `SET` or `CREATE TEMPORARY TABLE` is refused before sending, with a fixed
   text naming the way out: the context selector, the statement's own
   `SETTINGS` clause, a `Memory` table. `SESSION_CONTEXT` is declared;
   `TRANSACTIONS` and `MULTIPLE_STATEMENTS` are not.
3. **Results arrive as `RowBinaryWithNamesAndTypes`, decoded by the driver.**
   The header gives every column's exact ClickHouse type as text; the driver
   parses it into `ChType` (bounded nesting, no recursion on hostile depth),
   measures each row whole before decoding it, and appends it to Arrow
   builders. The mapping table and its losses live in the crate's `schema`
   module and are tested against the server's own `toString` of the same
   values. No integer becomes a float: `UInt64` stays `UInt64`, `Int128`
   becomes `Decimal256(39, 0)`, the 256-bit integers their exact digits,
   `Decimal(P, S)` `Decimal128`/`Decimal256`. An instant carries the zone its
   type declares, else the query's zone the response names in
   `X-ClickHouse-Timezone` — the server's, never the workstation's.
4. **A type whose wire layout the driver does not know is refused naming its
   column**, before the first row: `RowBinary` gives no length to skip it, so
   it cannot be returned as raw bytes. `Variant`, `Dynamic`,
   `AggregateFunction` and `Object` are such types today. The table preview
   reads them through `toString(…)` (an aggregate state through
   `hex(toString(…))`), so a click on a table never fails on one column. A
   `String` that is not UTF-8 fails the result naming the column and pointing
   to `hex()`.
5. **Batches close at 1 MiB of decoded bytes** (or 8,192 rows, so narrow rows
   reach the grid early), a row larger than 64 MiB — on the wire, or once
   decoded, counted before anything is appended — is refused, and the task
   that reads the body hands batches to the cursor through a one-slot
   channel: the network is read only as fast as the caller drains
   ([I-06](../../CLAUDE.md#i-06)). `ExecLimits::max_rows` is the driver's to
   hold: a read past it is stopped on the server and marked truncated; any
   other statement is read to its end, its rows dropped, so that what it
   wrote is known.
6. **The exception marker is cut out of the body** with the response's tag,
   holding back the marker's length of every chunk until more arrives, so a
   marker split across chunks is never decoded as a value; the message is
   read to its closing line. A server whose 200 carries no tag — checked at
   connection, not inferred from a version — is asked `wait_end_of_query = 1`
   on every request, so an error keeps its own status; an account that may
   carry no setting is then refused rather than shown an error as rows.
7. **A stop is `KILL QUERY … ASYNC` on the driver's own `query_id`**, from a
   separate request, with the id bound as a query parameter. The stop is
   reported from the statement's intent and the server's own end: a read is
   `Cancelled`; anything else is `OutcomeUnknown`, since nothing can be rolled
   back ([DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).
   `SERVER_SIDE_CANCEL` is declared only when `KILL … TEST` succeeds for the
   account at connection. Every request also carries
   `cancel_http_readonly_queries_on_client_close = 1`, which acts only on a
   request at `readonly` above 0 (observed on 26.3): a read not bounded
   read-only therefore carries `readonly = 2`, which keeps its own
   `SETTINGS` and its table functions, so that a read whose cursor is
   dropped stops on the server even for an account that may not kill, or
   when a kill came before the server registered the statement — a kill
   that finds it still running is sent again. A statement's own `SETTINGS`
   clause applies after the request's, so one that assigns a setting the
   driver pins — the framing ones, `wait_end_of_query`,
   `cancel_http_readonly_queries_on_client_close`, `readonly` — is refused
   before it is sent.
8. **Errors are classified from the server's code, the statement's intent and
   the response's `written_rows` summary.** A read's passing conditions
   (memory, timeout, load, network between servers) are transient; a write is
   permanent only when the server refused it before running anything (syntax,
   unknown object, privilege) **and** said it wrote nothing; every other
   failure of a write is ambiguous and never replayed
   ([I-13](../../CLAUDE.md#i-13)). The driver retries nothing. The server's
   message is shown as it came unless the statement gave the server a secret
   to quote — a bound value, or a password literal the history's redaction
   recognizes ([ADR-0053](0053-redact-sql-passwords-before-persistence.md)):
   it is then withheld, its code kept ([I-03](../../CLAUDE.md#i-03)).
9. **A read-only execution carries `readonly = 1`**, the strongest bound the
   server enforces. The price is accepted and stated: a read bounded
   read-only — every read of a production connection — cannot carry its own
   `SETTINGS` clause; the server refuses it with code 164. An account the
   server already holds read-only is not bound further, and declares no
   `DML`/`DDL`.
10. **No page past the first in a preview.** A ClickHouse primary key is the
    sorting key's prefix and is not unique, so no order is total and an
    `OFFSET` would repeat or skip rows ([ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)).
    No column is marked a primary key in the catalog; the primary key is
    listed as a non-unique index, and the table's own `CREATE` statement is
    its definition.
11. **Bound values go to the server as query parameters.** The n-th value of
    the editor's parameter panel binds the n-th distinct `{name:Type}`
    placeholder of the statement, in order of first appearance — quotes,
    comments and heredocs skipped; a count that differs is refused before
    the request. The text reaches the server unchanged, the values as
    `param_<name>` with their tab-separated escapes, `\N` for a `NULL`: the
    server types them, the driver never writes them into the text
    ([I-10](../../CLAUDE.md#i-10)). A boolean binds as `1`/`0`, an instant as
    seconds since the epoch with its nanoseconds; bytes, intervals, arrays and
    instants before 1970 are refused naming their rank.

## Consequences

* **+** Every ClickHouse type the driver knows reaches Arrow exactly, its
  server type in the field metadata (`oxyn:clickhouse_type`), with no
  dependency besides the HTTP client the workspace already ships.
* **+** Nothing a statement does outlives its request: a reconnect, a retry by
  the caller or a second console cannot meet a setting the user forgot.
* **+** Cancellation is proven in `system.processes`, for a read, a dropped
  cursor, a closed session and a stopped `INSERT`.
* **−** The driver owns a `RowBinary` decoder: about 1,500 lines that must
  follow every new ClickHouse type. A type added by the server is refused
  until the decoder learns it.
* **−** `Variant`, `Dynamic`, `AggregateFunction` and `Object` cannot be read
  by a console query without a cast.
* **−** Production reads cannot use a `SETTINGS` clause; a professional tuning
  `max_threads` on production must do it on a connection that is not bounded,
  or wait for a per-connection `readonly = 2` option.
* **−** Temporary tables and `SET` are not available in a console.
* **−** An account without the `KILL QUERY` right cannot stop a write on the
  server: the driver declares it, and the write goes on.

**Exit cost:** the decoder (`types`, `measure`, `decode`, `body`, `schema`)
is replaceable by another wire format without touching the session, the
catalog or the desktop, which see only `RecordBatch`es. Moving to a native
protocol client would rewrite `http` and `stream`, about a third of the crate.
A server session (`session_id`) would be additive: a declared option, the
statements refused today then accepted.

**Reconsider if** ClickHouse's Arrow output carries the zone of a `DateTime`,
the label of an `Enum` and the 128- and 256-bit integers as numbers — the
decoder would then reduce to the exception cut; if a maintained native-protocol
crate decodes untyped results onto the workspace's Arrow; or if users need
temporary tables or `SET` in a console, which would call for a declared
`session_id`.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| The `clickhouse` crate | rows must be Rust types known at compile time; no untyped result, no `Dynamic`, no raw stream of a chosen format |
| `clickhouse-arrow` (native protocol) | a second Arrow (^57) in the graph next to the workspace's 59, `webpki-roots` instead of the platform verifier, and a 0.2 crate on the widest external boundary |
| `klickhouse` (native protocol) | its own value model to convert again, and the native port, which managed offerings and proxies expose less than HTTP |
| `FORMAT ArrowStream` | a `DateTime` loses its zone, an `Enum` its label, 128/256-bit integers and `IPv6` become bytes; the per-type fixups would need the ClickHouse type names Arrow does not carry, and the exception marker must still be cut out first |
| A `session_id` per console | state the user cannot see, lost by any reconnect, and a session the server expires on its own timeout: the next statement would run under different settings without a word |
| `readonly = 2` for read-only executions | lets `url()` and the other table functions reach out from a production read |
| `wait_end_of_query = 1` (the server buffers the whole result, so an exception comes as a status) | the first row waits for the last, and the server holds the whole result in memory or on disk |
