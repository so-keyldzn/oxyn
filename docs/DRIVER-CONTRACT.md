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
| Transient | network drop, `too many connections`, lock timeout | may retry, with exponential backoff |
| Permanent | syntax error, missing table or column, insufficient privileges | **never** retries, displays |
| Ambiguous | client-side timeout during a write | **never** retries, reports the uncertainty |

**Concrete failure:** an `INSERT` times out on the client side while the server
applied it. Classified as "transient" and replayed, it creates a duplicate in the
user's data, without any error message anywhere. It is the most expensive case,
and the most tempting to handle with a simple retry loop:
ambiguity is not retried.

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

Corollary for a non-SQL driver: a query carries an explicit `QueryLanguage`.
SQL is one case among others, not the default the others reduce to.

### 6. It escapes every identifier it composes

The SQL **the user writes** is sent as is: it is a professional tool,
and arbitrary SQL is the feature. The SQL **Oxyn composes**
— introspection, table preview, sort by column, sidebar filter,
AI suggestion — never concatenates a received name: it goes through the driver's
identifier-quoting function, and values are bound.

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
test that proves the stop on the server side, and a streaming test on a volume
that would not fit in memory.
