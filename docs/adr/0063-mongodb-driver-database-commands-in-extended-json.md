# ADR-0063 — MongoDB driver: database commands in Extended JSON, documents to a declared, lossless Arrow schema

**Status:** accepted · **Date:** 2026-10-23

**Refines:** [ADR-0003](0003-driver-capabilities.md), on the following point:
a document database gets a crate of its own (OP_MSG and BSON share nothing with
the drivers already written), and its query language is
`QueryLanguage::MongoQuery`, defined below as database commands, not as a
JavaScript shell.

## Context

MongoDB is the first document database Oxyn speaks to. Four properties of the
product make every SQL habit of the existing drivers wrong
([RESEARCH-NOTES](../RESEARCH-NOTES.md#mongodb-client-library-and-server--checked-on-2026-10-23),
checked on 2026-10-23):

* **There is no text query language on the wire.** The server receives BSON
  command documents (`{find: "users", filter: {…}}`) in OP_MSG messages. What
  users call "the MongoDB query language" is the JavaScript API of `mongosh`,
  a Node.js program that builds those commands. Running it means embedding a
  JavaScript engine and the shell's library — a second runtime whose every
  helper can write.
* **Documents declare no columns.** Two documents of one collection may share
  no field, or hold an `Int32` in one and a sub-document in the other under
  the same key. A `RecordBatch` has one schema
  ([ADR-0002](0002-arrow-result-model.md)).
* **Documents are large and nested.** A document may weigh 16 MiB; a `getMore`
  batch has no default limit but the 16 MiB message size. A batch counted in
  documents is unbounded in bytes ([I-06](../../CLAUDE.md#i-06)).
* **The official client retries.** `mongodb` (the Rust driver, Apache-2.0)
  retries a read and a write once by default; a write retried without the
  transaction number a retryable write carries may apply twice
  ([I-13](../../CLAUDE.md#i-13)).

The connection string `mongodb://user:password@host/` carries the password,
and a `mongodb+srv://` name needs DNS lookups before any server is reached.

## Decision

### Crate and client

`drivers/oxyn-driver-mongodb`, on `mongodb` `=3.9.1` and `bson` `3.1.0`, with
`default-features = false` and exactly `compat-3-3-0`, `bson-3`,
`rustls-tls-aws-lc` (rustls on aws-lc-rs, the workspace's provider) and
`dns-resolver` (SRV seed lists). No compression, no AWS, Kerberos or OIDC
authentication, no client-side encryption: none is tested. The client accepts
servers of wire version 9 to 29, MongoDB 4.4 and later; the tests and the CI
run `mongo:8.0.34`. DocumentDB, Cosmos DB for MongoDB and FerretDB are **not
claimed**: they would be this crate's capabilities once tested, never other
crates.

### Connection: fields, never a connection string

The form takes the parts — hosts (a seed list), port, `srv`, database, user,
password, `authSource`, `authMechanism` (`default`, `SCRAM-SHA-256`,
`SCRAM-SHA-1`), `replicaSet`, `directConnection`, `tls`, `tlsCAFile`,
`tlsCertificateKeyFile` — and the password goes to the keychain like every
driver's ([I-03](../../CLAUDE.md#i-03)). The client options are built
**programmatically**: no string holding the password is ever composed. The
one string composed is `mongodb+srv://<name>/`, from a name checked to be a
DNS name, so that nothing in it can open a user, path or option part. An
unknown parameter is refused rather than ignored. TLS follows
[ADR-0052](0052-verified-tls-outside-local.md): `verify-full` everywhere but on
a connection marked Local. `ConnectSpec` has a hand-written `Debug`.

`retry_reads` and `retry_writes` are **off**: the retry policy is the caller's
([DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md)). Connecting runs `buildInfo`
(version), a `killSessions` naming nobody (can this session stop its
commands?), and `connectionStatus` with privileges (can this account write?),
each racing the cancel token.

### The console: one database command, in Extended JSON

A `MongoQuery` request is **one JSON document, the command the server runs**,
in MongoDB Extended JSON v2, canonical or relaxed:
`{"find": "users", "filter": {"age": {"$gt": 30}}, "limit": 100}`,
`{"aggregate": "orders", "pipeline": […], "cursor": {}}`. A console holds
several, one after the other; Run takes the top-level object under the cursor
(`oxyn_query::current_request`). It is sent **as typed** — that is the
feature — with three rules:

* the name is the **first key**, so the text is read into an ordered tree;
  a key repeated in any object is refused, since two readers keeping different
  copies is how a classified read becomes a sent write;
* a type wrapper is an object whose keys are **exactly** the wrapper's
  (`{"$oid": …}`); any other object is a document, `$`-keys included. An
  integer literal beyond 64 bits is refused, never rounded into a double.
  DBPointer, deprecated, is not accepted back;
* `$db` is read as the target database; the envelope fields the driver owns —
  `lsid`, `txnNumber`, `autocommit`, `startTransaction`, `$clusterTime`,
  `$readPreference` — are refused, because they would move the command out
  of the session cancellation kills. Bound parameters are refused: a command
  carries its values in its own document.

Database and collection names are BSON string **values**, never text Oxyn
pastes into a command ([I-10](../../CLAUDE.md#i-10)): a composed preview is
built as BSON and serialized.

### Classification: a table of command names, read by the session

`oxyn_query::classify_mongo` reads the same tree against tables built from the
command reference: reads (`find`, `aggregate`, `count`, `distinct`,
`listCollections`, `listIndexes`, `dbStats`, `collStats`, `usersInfo`,
`currentOp`…), writes (`insert`, `update`, `delete`, `findAndModify`,
`bulkWrite`), DDL (`create`, `drop`, `createIndexes`, `collMod`,
`renameCollection`…), grants (user and role commands). An `aggregate` or an
`explain` holding `$out` or `$merge` at any depth writes. `$where`,
`$function` and `$accumulator` run JavaScript on the server: `Unknown`. A name
the tables do not hold is `Unknown`, hence mutating.

The executor reaches it through `Session::classify_intent`
([ADR-0064](0064-a-session-classifies-non-sql-requests.md)): `oxyn-query`'s
`classify_language` keeps `Unknown` for MongoDB, and the session answers with
the reading its `execute` enforces — `None` for `Unknown` and for the
transaction verbs it refuses. One reader decides production.

### Results: a declared sample, a lossless column

The first documents of a result — at most 1,000 and about 8 MiB — are read
before `execute` returns; the schema is inferred from them. Every field column
carries `oxyn:inferred` and its BSON type (`oxyn:bson_type`), the schema the
sample's size. A field the sample saw with two types or more is a **mixed**
text column of relaxed Extended JSON, never coerced. The **last column,
`$document`, holds each whole document in canonical Extended JSON**: a field
outside the sample, beyond the 1,024-column cap, or repeated inside a document
is never lost. A value met after the sample with a type its column does not
hold ends the stream with an error naming the field and both types, after the
rows before it.

| BSON | Arrow | Loss |
|---|---|---|
| Double | `Float64` | none; NaN and ±∞ kept |
| String | `Utf8` | none; invalid UTF-8 ends the stream |
| Int32 / Int64 | `Int32` / `Int64` | none; beyond 2^53 exact |
| Boolean | `Boolean` | none |
| Date | `Timestamp(ms, "UTC")` | none: UTC milliseconds by definition |
| ObjectId | `Utf8`, 24 hex digits | none |
| Decimal128 | `Utf8`, the decimal's text | none; never a double |
| Binary | `Binary`, subtype in `oxyn:bson_subtype` | none; a UUID is subtype 4 |
| Timestamp, Regex, JavaScript, Symbol, DBPointer, MinKey, MaxKey, Undefined, document, array | `Utf8`, relaxed Extended JSON | the type is in the wrapper and the canonical column; a Timestamp is never a date |
| unknown element type | — | the stream ends naming the type byte: BSON gives it no length |

### Streaming and bounds

The client holds one server batch (at most 16 MiB) and one Arrow batch of at
most 1 MiB or 8,192 rows; nothing collects a cursor. `maxTimeMS` and
`batchSize` are the user's to write; the server's 16 MiB document limit and
100 MB stage memory limit come back as their errors (`BSONObjectTooLarge`,
`QueryExceededMemoryLimitNoDiskUseAllowed`) with their remedy appended.

`bulkWrite` (MongoDB 8.0+) answers its per-operation results as a cursor on
`admin.$cmd.bulkWrite`: past `cursor.batchSize`, `firstBatch` is only their
start. It is read as a cursor like `find` — every `getMore` in the
execution's logical session, so a stop kills that cursor and no other — and
the reply's fields outside `cursor` (the `n…` counts, `nErrors`) are the
**first row**, ahead of the results: a row bound that stops the results, which
the result buffer marks truncated, never hides how many operations failed. A
`writeConcernError` in that reply makes the outcome unknown, as for any other
write. A batch that cannot be read ends the stream with an error after the
rows before it, never silently.

### Cancellation: one logical session per execution, `killSessions`

Every execution starts a logical session of its own, causal consistency off.
`Session::cancel` and the cancel token send `killSessions` with its id: the
server interrupts its operations and closes its cursors. The verdict waits for
the server's answer to the stopped command: a read is `Cancelled`; anything
else is `OutcomeUnknown`, since a multi-document write keeps what it changed. A
session whose server refuses `killSessions` (Atlas M0, Flex) does not declare
`SERVER_SIDE_CANCEL`. Proven in `$currentOp` by the integration tests.

### Errors

The server's `code`, `codeName` and labels are the data. Network codes,
step-downs, `WriteConflict`, `LockTimeout` and the `TransientTransactionError`
and `RetryableWriteError` labels are transient for a read and **ambiguous**
for anything else; `UnknownTransactionCommitResult` is always ambiguous; a
transport cut during a write is `OutcomeUnknown`. The library's own messages,
which quote hosts, are replaced by the driver's.

A command holding a `pwd` key at any depth — the password of `createUser` and
`updateUser` — gets errors without the server's text, and is persisted with
that value replaced by `"<redacted>"`: `oxyn_query::redact_password_literals`
reads the MongoDB language with a lexical scan of the JSON text, which keeps
every other byte, decodes an escaped key, skips string contents, and redacts
an unterminated value through the end
([ADR-0053](0053-redact-sql-passwords-before-persistence.md)).

### Session state, read-only, transactions

The target database is the session context (`set_context`), client-side
only; read preference `primary`, the server's read and write concerns, no
causal consistency — fixed and declared. `READ_ONLY_SESSION`, without `DML`,
`DDL` and `GRANT_REVOKE`, is declared only when `connectionStatus` shows every
privilege action among those of the built-in `read` roles: then the server
enforces it. Otherwise a bounded read-only execution is refused unless the
command classifies as a read: a client-side guard, which server-side
JavaScript would escape — and is therefore classified `Unknown`.
`readPreference` is not a write guard and is not used as one.

`TRANSACTIONS` is not declared: a multi-document transaction needs a replica
set and a logical session kept across executions, which the console does not
hold; `commitTransaction` and `abortTransaction` are refused.

### Catalog and preview

Databases are namespaces, collections, views and time-series collections are
relations: a session declares `SCHEMAS` and `TABLES`. The definition view
(`OBJECT_DEFINITION`) shows the `create` command rebuilt from the
`listCollections` options and one `createIndexes` with the secondary indexes,
each a console command, marked reconstructed: MongoDB stores no creation
command. The row count is `count` without a filter — metadata, never a scan
— on plain collections only. Fields are inferred from a sample of 1,000
documents and marked inferred; `_id` is the primary key of a plain collection.
A preview is a `find` command, written as BSON, with no imposed order
([ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)); a
requested sort is completed by `_id`, and a page past the first is refused on
a view or a time-series collection, which have no unique key. A field whose
name holds a dot or starts with `$` cannot be sorted or projected by `find`:
refused, never approximated. `PREVIEW_FILTER` is not declared: the preview
filter is SQL.

### Explain: the planner only

The console's Explain sends `{"explain": <the typed command>, "verbosity":
"queryPlanner"}`, its `$db` moved to the wrapper
(`oxyn_query::explain_request`, the per-language extension point beside
`current_request`). `queryPlanner` selects a plan without executing it;
`executionStats` and `allPlansExecution` run the winning plan, so the session
declares `EXPLAIN` and never `EXPLAIN_ANALYZE`. The wrapped command is
classified like the command it explains: the explain of a write is a write.

## Consequences

* **+** What runs is what the user wrote, byte for byte in meaning; no
  JavaScript engine ships, and no shell helper writes behind a read.
* **+** No field is ever lost or coerced: the grid shows the sample's columns,
  the `$document` column carries everything, a surprise ends the stream with
  its name.
* **+** The guard of production has one reader of a MongoDB command, tested
  against the command reference; a stop is proven in the server's
  `$currentOp`.
* **−** Users of `mongosh` must write commands, not `db.users.find(…)`; the
  convenience of the shell's helpers (`ObjectId(…)`, `ISODate(…)`) is gone.
  The agent writes commands for them.
* **−** The columns depend on the first documents: a field that appears only
  after the sample is visible in `$document` alone, and a type change ends
  the stream.
* **−** No console transaction, no bound value, and the read-only guard of a
  read-write account is client-side.
* **−** A new server command is `Unknown` until the table learns it: a
  confirmation on production for a read the table does not know yet.
* **−** A command's risk rests on the driver's reading
  ([ADR-0066](0066-a-session-classifies-the-risk-of-non-sql-requests.md)):
  `Session::classify_risk` answers the `MutationRisk` `classify_mongo` reads —
  a delete or an update whose filter is empty and whose effect is not bounded
  to one document, a `drop`, `dropDatabase`, `dropIndexes`, `dropSearchIndex`.
  A filter that selects everything without being empty
  (`{"_id": {"$exists": true}}`) is not recognized: it runs as a plain write.

**Exit cost:** medium. The console syntax is user-facing: saved queries and
history hold commands in this form. A shell syntax added later would be a
second language beside this one, not a replacement. The client crate is
confined to this crate.

**Reconsider if** users need transactions in the console (a session kept
across executions, `TRANSACTIONS` declared per replica set), if a
MongoDB-compatible product is tested and needs other capabilities, or if the
Rust client stops being maintained.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| A `mongosh`-compatible JavaScript console | Embeds a JavaScript engine and the shell's library; its helpers write, and a text that only calls `find` cannot be told from one that calls `deleteMany` without running it — the classification production rests on would be a guess |
| A small `db.coll.find({…})` syntax over the commands | A second grammar that looks like JavaScript and accepts only part of it: users would type the rest and meet errors the shell never gives; the command form is exact and documented by MongoDB itself |
| A connection-string field, password included | The string would reach the workspace file and any error that quotes it (I-03); parsing a pasted URI to split the password out is a follow-up of the form, not a reason to store it |
| The library's retryable reads and writes | A retried command is a second execution the caller did not decide; the driver contract forbids it, and a console command has no transaction number to make it idempotent |
| A schema from `$jsonSchema` validators only | Most collections have none, and a validator describes what may be written, not what is stored |
| One column per field with values coerced to a common type | An `Int32` in one document and a sub-document in another would lose one of them; coercion is the silent loss DRIVER-CONTRACT §3 forbids |
| `killOp` with a `comment` added to the user's command | Sends another command than the one typed, and overwrites a `comment` the user wrote; the logical session id is part of the envelope the driver owns |
