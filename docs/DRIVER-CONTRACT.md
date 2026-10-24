# Driver contract

> **Authority**: what every database driver must guarantee, and what it is
> forbidden to do. It is Oxyn's widest external boundary — a DBMS is a hostile
> system by default: it can be slow, lie about its types, close a connection in
> the middle of a response.

Derives from [ADR-0002](adr/0002-arrow-result-model.md) (Arrow),
[ADR-0003](adr/0003-driver-capabilities.md) (capabilities) and
[ADR-0007](adr/0007-driver-sidecar.md) (sidecar). When this document seems to
contradict an ADR, this document is the one that is wrong.

Invariants involved: [I-02](../CLAUDE.md#i-02), [I-06](../CLAUDE.md#i-06),
[I-09](../CLAUDE.md#i-09), [I-10](../CLAUDE.md#i-10).

## What a driver guarantees

### 1. It never panics on input coming from the server

A driver translates; it does not assume. Everything that arrives from the
network is untrusted data: an unknown type, a `NULL` where the schema says
`NOT NULL`, an out-of-range integer, an invalid encoding.

**Concrete failure:** a MySQL server configured with a spatial type returns a
BLOB that an `unwrap()` on decoding turns into a panic. The `release` profile
compiles with `panic = "abort"`: nothing catches the panic, the application dies, and the user loses their
tabs and unsaved queries.

Forbidden on a path reachable from a server response: `unwrap()`,
`expect()`, `panic!()`, `unreachable!()`, `todo!()`, slice indexing by
range, `as` on an integer that can overflow.

### 2. It exposes cancellation, and cancellation really cuts

Every method that can take time accepts being cancelled, and cancellation reaches
the query **on the server side**, not just the future on the client side.

**Concrete failure:** the user closes the tab of a 4-minute aggregation.
The future is dropped, but the query keeps running on the server, holding a
pool connection and a lock. By the tenth closed tab, the database refuses
connections and the user concludes that Oxyn broke their production.

In practice: PostgreSQL has `pg_cancel_backend`, MySQL has `KILL QUERY`, SQLite has
`sqlite3_interrupt`. A driver that cannot cancel on the server side **declares** it
in its capabilities; it does not pretend.

### 3. It produces Arrow `RecordBatch`es, streamed

The result interface is a stream of `arrow::RecordBatch`
([ADR-0002](adr/0002-arrow-result-model.md)). No driver builds the
whole result before returning control, and **no driver returns a
row representation**: the conversion belongs to the driver, not to the caller.

**Concrete failure:** `SELECT * FROM events` on a table of 50 million
rows. A driver that materializes makes the RSS climb up to the OOM killer; on
macOS the process is killed without a trace. The user did nothing unusual:
they clicked on a table in the sidebar.

Two consequences drivers miss most often:

* **A row-by-row driver (`sqlx`, most SQL clients) must accumulate
  into batches**, and the batch has a size bounded in bytes, not in number of rows:
  a thousand rows each carrying a one-megabyte BLOB is a gigabyte.
* **A schemaless source (MongoDB) infers its schema by sampling**, and
  that inference is declared as such all the way to the interface. A field absent
  from the sample but present further on must produce an explicit error or
  a schema widening — never a silently lost value.

### 4. It distinguishes three families of errors, and classifies them

| Family | Examples | What the caller does |
|---|---|---|
| Transient | network drop during a read, too many connections, a lock timeout / deadlock / resource error on a read or a plain data write after a whole-statement rollback | may retry, with exponential backoff |
| Permanent | syntax error, missing table or column, insufficient privileges | **never** retries, displays |
| Ambiguous | client-side timeout during a write; a concurrency or resource error in a procedure, non-transactional command, or unknown intent | **never** retries, reports the uncertainty |

**Concrete failure:** an `INSERT` times out on the client side while the server
applied it. Classified as "transient" and replayed, it creates a duplicate in the
user's data, without any error message anywhere. It is the most expensive case,
and the most tempting to handle with a simple retry loop:
ambiguity is not retried.

**A stopped write is not a cancelled write until the server proves it.** A Stop,
a closed tab or a kill that reaches a write does not say what the statement left
behind: it may have committed an instant before, a procedure keeps what it
already committed, a non-transactional table keeps the rows it changed, and a
batch keeps the statements that ran before the one interrupted. `Cancelled`
means that **nothing was applied**, and the interface reads it that way, so a
driver answers it with proof — the server's own interruption error on a
statement it undoes as a whole, or a write that was never sent — and answers
`OutcomeUnknown` otherwise. A failure after an earlier statement of a batch
applied is never transient: replaying the batch would apply that statement
twice. What counts as proof for each protocol is written in
[ARCHITECTURE](ARCHITECTURE.md), where the executor's verdict is.

**An unknown column is a permanent refusal, even when pronounced before sending.**
A preview column that the relation does not declare — in projection as in
sort, see [§6](#6-it-escapes-every-identifier-it-composes) — is returned as
`OxynError::Query`, as the server would have classified it: retrying will not make
the column appear. `OxynError::CatalogUnavailable`, transient, means a catalog that
may come back — introspection in progress, empty cache. Confusing the two would
offer the user "retry" for a request that will always fail
the same way.

### 5. It declares its capabilities per session, and simulates nothing

A driver and each `Session` expose a `Capabilities`
([ADR-0003](adr/0003-driver-capabilities.md)): transactions, server-side
cancellation, named cursors, prepared statements, index introspection, accepted
query languages.

**Capabilities are evaluated per session, not per driver.** The server version,
its extensions and the privileges of the connected account change what is available:
the same PostgreSQL driver talks to a version 12 database without `MERGE` and to a version 17 database that
has it, to a database with `pg_stat_statements` and to another without.

**Concrete failure:** a driver that emulates transactions by simply
chaining queries lets the user believe that a `ROLLBACK` undid
their write. Not knowing how is an acceptable answer; letting someone believe is
not.

**A session that declares `TRANSACTIONS` says whether a transaction is open**
([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)). It overrides
`Session::transaction_state`, which returns `Idle`, `Open` or `Unknown`:

* **after** the end of every operation already submitted to the session — execution,
  `begin`, `commit`, `rollback`, `set_context` —, whether it succeeded, failed or
  was interrupted. A value simply stored by the driver and read without
  waiting would be the one from before the automatic rollback SQLite performs on a
  Stop;
* **without a round trip** to the server: it waits, it does not query;
* **never inferred from the submitted text**: only the engine knows that an error or an
  interruption closed the transaction;
* `Unknown` if the token fires or if the session can no longer answer —
  never an error, never `Idle` by default.

The caller drains or drops the cursor before calling it. The trait default
returns `Unknown`: it is the honest answer of a session that does not know.

**Concrete failure:** a display that says "no transaction" after a Stop
while the transaction is still open — or the reverse — and the console
closes without warning, discarding uncommitted writes.

**A session that declares `TRANSACTIONS` holds `limits.read_only` without touching
the open transaction.** `oxyn-exec` bounds every production read to
read-only ([SECURITY](SECURITY.md#connection-marking)). Outside a
transaction, a `READ ONLY` transaction opened and closed by the driver
is enough. Inside a user transaction, it is worthless: PostgreSQL
answers a nested `BEGIN` with a mere warning, and the closing `ROLLBACK`
would undo the user's writes. The bound must then go
through a mechanism that leaves the transaction intact — a savepoint and
`SET TRANSACTION READ ONLY` are the lead, to be tested against the server before
relying on it — or through refusing the execution. SQLite
holds it through `sqlite3_stmt_readonly`, which does not touch the transaction.

**Concrete failure:** a `SELECT` in a production console where a
transaction is open, and the `INSERT`s confirmed just before vanish
without a message.

**A session says where an unqualified name resolves, or says it cannot.**
`CatalogProvider::resolution_path` returns the namespaces, first match first,
([ADR-0057](adr/0057-editor-completion-reads-the-catalog-cache.md)) — read on
the session reserved for the catalog, in its default state, never on a console's
session, whose context a user may have changed:

* PostgreSQL answers with the server's effective `search_path`
  (`current_schemas(true)`: the entries that do not exist or cannot be searched
  are omitted, and the implicitly searched `pg_catalog` is **kept** — the server
  looks in it before the path unless the path names it, so a relation of
  `public` called like one of its own is not what an unqualified name reaches).
  The session's own temporary schema is left out: the catalog's reserved session
  holds no temporary object, and a console's lives on its own session. Amazon
  Redshift refuses with `NotSupported`: the query leans on `unnest … WITH
  ORDINALITY`, and a query the server rejects would fail the same read at every
  pass;
* MySQL and MariaDB answer with `DATABASE()`, the database the server says the
  session selected — not the one the connection form names, which the server may
  spell another way (`lower_case_table_names`) or replace (`init_connect`, a
  proxy). `NULL` is an empty path: nothing resolves then, which is an answer; a
  name that is not text is an error, never an empty path. A console that
  switched database with `USE` resolves in its own context, which this answer
  does not see;
* SQLite answers `temp`, then `main`, then the attached databases in the order
  they were attached — the order `sqlite3FindTable` searches them in (SQLite
  3.50.2, `src/build.c`). The ATTACH page says the least recently attached
  database wins among the attached ones and is silent on `main` and `temp`, so
  their rank is the source's and not a documented guarantee
  ([RESEARCH-NOTES](RESEARCH-NOTES.md#editor-completion--check-of-2026-10-21));
* any other driver keeps the default, `NotSupported`: an empty list would assert
  that no namespace resolves anything, and a driver that cannot tell cannot
  assert it.

**Concrete failure:** a completion that supposes `public` suggests a name the
server then refuses — PostgreSQL reports an error "even if matching table names
exist in other schemas".

Corollary for a non-SQL driver: a query carries an explicit `QueryLanguage`.
SQL is one case among others, not the default the others reduce to.

**A non-SQL session classifies only what it can prove**
([ADR-0064](adr/0064-a-session-classifies-non-sql-requests.md)). `oxyn-query`
reads SQL alone; any other language comes out `Unknown`, which counts as
mutating. `oxyn-exec` therefore asks `Session::classify_intent`, and **only**
for a request whose language has no SQL dialect — a SQL request never reaches
it:

* `Some(Read)` for a request **proven** read-only, every command in it known to
  the session and known not to write; `None` whenever unsure, which keeps
  `Unknown`. A wrong `Read` bypasses the production guard: no named
  confirmation for a human, no refusal for an agent, a preview that runs it;
* no I/O and no lock that can block: the answer comes from what the session
  learned when connecting, such as the server's own command table;
* a request answered `Read` on production, or on a connection marked read-only,
  reaches `execute` with `limits.read_only` set: the session holds that bound or
  refuses the execution;
* the answer carries no transaction control and no context change. A language
  that has such statements refuses them in its console, or carries a context
  change through `set_context`.

The default answers `None`: a session that does not override it behaves as if
the method did not exist.

**Concrete failure:** a driver that answers `Read` for every command it does
not recognize runs a `FLUSHALL` on a production cache with no confirmation, and
lets an agent run it too.

**It tells the risk of a write it classified**
([ADR-0066](adr/0066-a-session-classifies-the-risk-of-non-sql-requests.md)).
For a request `classify_intent` answered `Write`, `Ddl` or `Grant` — never for
SQL, never for a read, never for an unproven request — `oxyn-exec` asks
`Session::classify_risk`, and the `MutationRisk` it answers is the one the gate
sees. A risk makes the gate ask a human for an approval whatever the
environment, exactly as SQL's `DELETE FROM users` does on staging:

* conservative **in the other direction**: answer the risk whenever the request
  may touch every document, key or index of its target — an empty delete
  filter, a drop, `FLUSHALL`, `FLUSHDB`, `_delete_by_query` with `match_all`,
  `DELETE /index` — not only when that is proven. A risk missed, or answered
  lower than the request deserves, bypasses an approval and nothing notices;
* the same rules as `classify_intent`: synchronous, no I/O, no lock that can
  block;
* the answer can only raise the risk: `None` leaves the request with none, the
  behavior of a session that does not override the method.

**Concrete failure:** a MongoDB driver that classifies
`{"delete":"users","deletes":[{"q":{},"limit":0}]}` as a write but not as an
unbounded delete empties a staging collection on one click, where
`DELETE FROM users` asked for an approval.

### 6. It escapes every identifier it composes

The SQL **the user writes** is sent as is: it is a professional tool,
and arbitrary SQL is the feature. The SQL **Oxyn composes**
— introspection, table preview, sort by column, sidebar filter,
AI suggestion — never concatenates a received name: it goes through the driver's
identifier-quoting function, and values are bound.

**Sent as is, or refused — never rewritten.** A driver refuses a statement the
user wrote, before sending it, only when running it would make the driver's own
reading of what follows false: a typed `USE`, since the context is the
session's, and on MySQL and MariaDB a typed `SET` of a setting the decoders rest
on ([§7](#7-it-treats-time-zones-and-temporal-types-as-data-not-as-text)). It
never edits the text to make it acceptable.

**The projection of a preview follows the same rule.** `PreviewShape::columns`
names the only columns to read: the driver takes the list through
`PreviewShape::projection`, which deduplicates it and bounds it to
`MAX_PROJECTED_COLUMNS` names, checks each name against the description of the
relation, then quotes it like the relation itself. A name the relation does not
declare is refused with `OxynError::Query`, a permanent error
([§4](#4-it-distinguishes-three-families-of-errors-and-classifies-them)), before
reaching the server, like an unknown sort column; an empty list is refused with
`OxynError::Config` and never means `SELECT *`, which would read precisely what
nobody approved. An ignored projection is not an acceptable
degradation: it is what bounds a sample to the checked columns
([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)), and a driver that
cannot compose it refuses the preview.

**One exception, and only one**: the preview predicate. The user writes there a
`WHERE` fragment that the driver inserts into a `SELECT` composed by Oxyn —
hence free text in composed SQL. This is deliberate and argued in
[ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md): this field **is** SQL
that the user writes, and Figma frame `190:1618` shows it as such.
Four barriers bound it — reclassification of the text before any decision,
read-only server session, row bound, and the parenthesization
`WHERE (…\n)` that turns an unterminated comment into a syntax error
rather than a swallowed `LIMIT`. This exception is named here because
[I-10](../CLAUDE.md#i-10) points to this paragraph: without it, a review of the
preview code would conclude an invariant is violated.

**Concrete failure:** a table named `"users"; DROP TABLE audit; --` exists
legally in PostgreSQL. A preview built by concatenation runs the
drop on a simple click on that table in the tree. The distinction
between "the user's SQL" and "Oxyn's SQL" is not a matter of style:
it is the line that separates a tool from a weapon.

### 7. It treats time zones and temporal types as data, not as text

No implicit conversion to the local time zone on read. A `timestamptz`
is carried in UTC and returned in the time zone declared by the Arrow schema of the
column; a `timestamp` without a time zone is carried **without** inventing one.
**No driver converts for display**: there is no display time zone
preference — the time zone Oxyn announces for a result is derived
from the schema (`oxyn_data::timestamp_display`), it is not a setting. If such a
preference ever came to exist, it would only affect rendering (`oxyn-data`,
`cell.rs`), never the driver.

**A session setting a decoder rests on is pinned, and a typed change of it is
refused.** The MySQL driver opens every connection at `time_zone = '+00:00'` and
in `utf8mb4`, and tags a `TIMESTAMP` UTC on that basis. A `SET time_zone`,
`SET NAMES`, `SET CHARACTER SET` or `SET character_set_*` the user types is
refused with `OxynError::NotSupported` before anything is sent — whatever its
scope, its quoting (`` @@session.`time_zone` ``), and inside an executable
comment too, a dump's `/*!40103 SET TIME_ZONE=… */` lines included — so a value
reads the same before and after a reconnect. Reading the setting is not
refused, on the right-hand side of a `SET` included. **What a lexical check
cannot see stays possible:** the body of a `CALL`ed procedure, a statement built
at run time (`PREPARE … FROM @sql`, MariaDB's `EXECUTE IMMEDIATE`), and a text
lexed under `NO_BACKSLASH_ESCAPES`, where a string ends earlier than the check
reads it. A zone changed that way is shown under the UTC tag, wrong, until the
next reconnect puts `+00:00` back.

**Concrete failure:** Oxyn displays a value converted into the machine's time zone,
the user copies it into an `UPDATE`, and shifts the data by two hours in
the database. The corruption is invisible and permanent.

## What a driver is not allowed to do

| Forbidden | Why |
|---|---|
| Depend on `oxyn-exec`, `oxyn-store`, `oxyn-desktop`, `oxyn-ai` or another driver | inverts the direction of dependencies. `oxyn-core` **is**, on the contrary, the expected dependency: it is the shared vocabulary — `ExecRequest`, `OxynError`, `PreviewShape` —, and both shipped drivers depend on it ([ARCHITECTURE](ARCHITECTURE.md#le-sens-des-dépendances)) |
| Exist twice for two products speaking the same protocol | [ADR-0003](adr/0003-driver-capabilities.md): Redshift ≡ PostgreSQL, MariaDB ≡ MySQL. The difference is a capability, not a crate |
| Write to a file, open a window, read an environment variable | a driver receives its configuration, it does not go and fetch it |
| Log a parameter value or a connection identifier | [I-03](../CLAUDE.md#i-03) |
| Retry on its own | the retry policy belongs to the caller, who alone knows whether the operation is replayable |
| Execute a write because the call "looked like" one | [I-02](../CLAUDE.md#i-02) |
| Change the server's session state without declaring it | an invisible `SET search_path` changes the meaning of the user's next queries |

## What a new driver must provide to be accepted

The procedure is in [`/driver`](../.claude/commands/driver.md) and the review in
[the checklist](../.claude/checklists/revue-driver.md). In short: the
capability declaration, the type mapping table **in both
directions** with the documented loss cases, the error classification, a cancellation
test that proves the stop on the server side, a test of the verdict a stopped
write gets (`Cancelled` only with proof), and a streaming test on a volume
that would not fit in memory.
