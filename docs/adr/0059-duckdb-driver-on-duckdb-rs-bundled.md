# ADR-0059 — The DuckDB driver embeds the engine through `duckdb-rs`, rebuilds its Arrow 58 as Arrow 59, and opens every file with external access off

**Status:** proposed · **Date:** 2026-10-23

## Context

DuckDB is the first analytical engine of the maintainer's daily databases
([IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md)). Like SQLite it runs in
Oxyn's process and a connection is a file; unlike SQLite its SQL reaches the
file system and the network, its type system has 128-bit integers, nested
types and unions, and it computes in parallel. Checked on 2026-10-23
([RESEARCH-NOTES](../RESEARCH-NOTES.md#duckdb-engine-and-client-library--checked-on-2026-10-23)):

* **The client.** `duckdb` (`duckdb-rs`, MIT) is the binding DuckDB's
  organization publishes. `1.10506.1` bundles DuckDB `v1.5.6`. Its `bundled`
  feature compiles the engine from C++ with `cc`; `parquet` and `json` link
  those two extensions statically and need nothing more; `icu` needs
  `bundled-cmake`, hence CMake on every build machine. `libduckdb-sys` declares
  `links = "duckdb"`, no conflict with `libsqlite3-sys`'s `sqlite3`
  ([ADR-0010](0010-contraintes-natives-sqlite.md)).
* **Its Arrow is not ours.** Every `duckdb` release up to `1.10506.1` depends on
  Arrow `^58`; `1.20000.0`, published on 2026-10-22, on `^60`. None on the
  workspace's Arrow 59 ([ADR-0002](0002-arrow-result-model.md)). Two Arrow
  versions are two sets of types the compiler keeps apart.
* **What streams.** `Statement::stream_arrow` executes in DuckDB's streaming
  mode; `Statement::step` then fetches one chunk at a time — up to 2,048 rows —
  through `duckdb_fetch_chunk` and returns it as an Arrow 58 `StructArray`, or
  an error. The iterator `stream_arrow` returns panics on a fetch error, and the
  release profile aborts on a panic.
* **What panics in the binding.** `column_logical_type` panics when DuckDB
  returns no type for an index; `schema` expects an executed statement;
  `LogicalTypeHandle::child` and `child_name` panic out of range;
  `get_alias` unwraps an alias as UTF-8; the row interface ends in
  `unreachable!` for an unexpected carrier.
* **What DuckDB exports by default.** `HUGEINT` and `UHUGEINT` as
  `Decimal128(38, 0)` — a `UHUGEINT` above `i128::MAX` arrives negative —,
  `TIMESTAMP WITH TIME ZONE` tagged with the session's zone, `TIME WITH TIME
  ZONE` without its offset, `BIT` as DuckDB's internal bytes, `JSON` as text
  without its alias.
* **What SQL reaches.** `read_csv`, `read_parquet` and `read_json` read paths
  and URLs, `COPY … TO` and `EXPORT DATABASE` write files, `ATTACH` opens other
  files, `INSTALL` and `LOAD` fetch and run extension code, and with
  `autoinstall_known_extensions` a statement downloads an extension on its own.
  `enable_external_access = false` stops all of it; DuckDB refuses to turn it
  back on while the database runs, refuses to change `allowed_directories` once
  it is off, and refuses to load any extension while it is off.
* **One file, one process.** DuckDB locks a database file against other
  processes; the lock is a POSIX record lock that a second engine of the same
  process does not see, and two engines writing one file corrupt it.
* **Cancellation.** `duckdb_interrupt` raises a flag on a connection that the
  running query reads between vectors; the next query lowers it.
* **The cost.** A clean build of the engine took 1,224 s of CPU; with debug
  info its archive weighs 1.80 GB per target directory, 73 MB without.

## Decision

1. **`oxyn-driver-duckdb` depends on `duckdb` `=1.10506.1`**, default features
   off, with `bundled`, `parquet` and `json`. The engine is linked into Oxyn:
   no system library, no download, the same engine on every machine. The dev
   profile builds `libduckdb-sys` with `debug = false`.

2. **Arrow 58 is a transitive duplicate that never crosses the crate.** No
   public item of the driver names an Arrow 58 type. Each chunk from `step` is
   rebuilt as Arrow 59 in safe code: every buffer is copied, the layout is
   rebuilt with `ArrayData::builder` and validated by `build()` — offsets,
   lengths, UTF-8 —, so a malformed buffer is a permanent decode error, never
   undefined behavior nor a panic. The column's DuckDB logical type, read once
   per column and owned (`DuckType`), decides at every nesting level:
   `HUGEINT` and `UHUGEINT` become `Decimal256(39, 0)` — the bits of a
   `UHUGEINT` read back as unsigned —; `TIMESTAMP WITH TIME ZONE` is tagged
   `UTC`, the instant DuckDB stores; `BIT` becomes the `'0101…'` text DuckDB
   prints. `TIME WITH TIME ZONE` and a type the binding does not know are
   refused by name before any row is read, with "cast it to VARCHAR": their
   default carrier loses the offset, and a guess would show a wrong value as a
   right one. Every other type keeps its carrier — integers of every width,
   `DECIMAL(w, s)` with its width, `TIMESTAMP` without a zone, `INTERVAL` as
   month-day-nano, `ENUM` as a dictionary, lists, arrays, maps, structs and
   unions with their children converted the same way — and the field metadata
   `oxyn.duckdb.type` carries DuckDB's name of the type. Each converted chunk is
   then split until every piece fits the byte bound; a single row larger than
   the bound travels alone.

3. **No panic is reachable from engine data.** Of the binding, the driver calls
   `Connection::open_with_flags`, `open_in_memory_with_flags`, `try_clone`,
   `interrupt_handle`, `execute_batch`, `prepare`, `Statement::stream_arrow`
   (whose iterator is dropped unread), `execute`, `step`, `schema` — only after
   a successful execution —, `column_logical_type` — only below the column count
   of that execution's schema —, `parameter_count`, and `LogicalTypeHandle`'s
   `try_id`, `raw_id`, `num_children`, `child` and `child_name` — only for an
   identifier that has children, below `num_children` —, `decimal_width` and
   `decimal_scale`. Never `get_alias`, never the row interface, never the
   Arrow iterators. Nesting deeper than 64 levels is refused. The driver's own
   catalog queries go through the same conversion and read cells with Arrow's
   checked accessors. A new `duckdb` version is re-audited against this list.

4. **One engine per file in the process, one carrier thread per session.**
   The first session of a file opens the engine; the others take a connection
   to it with `try_clone`, which is how DuckDB shares a database inside a
   process. An idle anchor connection keeps the engine while a session lives;
   the last session to close releases the file. A file already open in Oxyn
   under another access mode or folder is refused, not opened twice. `:memory:`
   is one private database per configured connection, shared by its sessions.
   Each session owns a thread that owns its connection, because a streamed
   statement borrows it across awaits; the thread computes at most one batch
   ahead of the cursor ([I-05](../../CLAUDE.md#i-05)).

5. **Every engine is opened with external access off, and one folder at most is
   granted.** At opening: `autoinstall_known_extensions` and
   `autoload_known_extensions` off; then, when the connection names a folder
   (`file_access`), `allowed_directories` set to it; then
   `enable_external_access = false`, before the session exists. SQL then reads
   and writes files only in that folder, reaches no network, and loads no
   extension. Parquet and JSON need none. A statement that needs an extension
   this build does not link fails with DuckDB's message, classified permanent;
   Oxyn does not offer to download one. The policy is the connection's, not the
   statement's: a typed `SET` cannot lift it.

6. **Read-only is the engine's.** A read-only connection opens the engine in
   `AccessMode::ReadOnly` and declares `READ_ONLY_SESSION` without `DDL`,
   `DML`, `TRUNCATE` and `TRANSACTIONAL_DDL`. A read-only execution
   (`ExecLimits::read_only`, which `oxyn-exec` sets for every production read)
   is held twice: each statement must classify as a read with the DuckDB
   profile of `oxyn-query` — so `COPY … TO`, which writes a file, is refused —,
   and the execution runs inside `BEGIN TRANSACTION READ ONLY`, rolled back
   after, which DuckDB enforces whatever a function in a `SELECT` tries.
   Inside a transaction the user opened, the wrapper would end their
   transaction: the execution is refused instead.

7. **The driver splits a batch with `oxyn-query` and runs it statement by
   statement**, so that an interruption is checked before each statement and a
   failure after an applied statement is never transient. A typed `USE` is
   refused: the context is `Session::set_context`'s
   ([ADR-0019](0019-contexte-de-session.md)). A data write without
   `RETURNING` reports its affected rows; everything else streams.

8. **Interruption targets one task.** Every task handed to the carrier thread
   has an identity; `duckdb_interrupt` reaches the engine only while that task
   runs, checked under a lock, and a flag of the task survives DuckDB lowering
   its own between two statements. A stopped read is `Cancelled`; anything else
   is `OutcomeUnknown`: DuckDB rolls the statement's transaction back, but not a
   file `COPY` began writing nor an `ATTACH`. The session does **not** declare
   `SERVER_SIDE_CANCEL`, like SQLite: there is no server, and the interface
   reads that flag as "the Cancel button reaches a server". The proof of the
   stop is the engine's: the same session answers at once, and once closed
   another process can lock the file.

9. **Errors are classified from DuckDB's exception names.** The C API hands
   back only the rendered message, whose first words are the exception type
   from a fixed table (`EXCEPTION_MAP`). The driver reads that prefix once and
   carries the class as data: refusals permanent; a transaction conflict
   transient for a read and ambiguous for a write; I/O, memory and fatal errors
   ambiguous for a write; a file held by another process permanent, with what
   to do. No path appears in an error ([I-03](../../CLAUDE.md#i-03)).

## Consequences

* **+** Every Oxyn user gets the same engine and the same SQL, with Parquet and
  JSON, without installing anything.
* **+** A query that reads a CSV can do nothing else to the machine: the policy
  is set before the first statement and the engine refuses to lift it.
* **+** Values survive: 128-bit integers, `UBIGINT` beyond 2^53, decimals with
  their width, nested types, zone-less timestamps without an invented zone.
* **−** **The build is heavier.** The engine is about 1,200 s of CPU from
  clean, and adds a C++ toolchain requirement every platform already met for
  SQLite. CI caches it; a cold cache pays it in each Rust job.
* **−** **Two Arrow versions are compiled**, and every chunk is copied once
  more than with a binding on Arrow 59. The copy is linear in the batch, under
  the byte bound; it was not measured apart from the end-to-end streaming test
  (2,964 MB streamed, resident memory up 164 MB).
* **−** **No extension beyond Parquet and JSON**: no `httpfs`, `spatial`,
  `icu` (time-zone functions such as `AT TIME ZONE 'Europe/Paris'`), `iceberg`,
  `postgres_scanner`. Each is a linking decision, and `icu` needs CMake.
* **−** **`TIME WITH TIME ZONE` is refused**, cast to text by the user, and a
  `JSON` result column is labeled `VARCHAR`: DuckDB's default export loses the
  offset and the alias, and the binding's alias accessor can panic.
* **−** **A file must be opened under one setting at a time in Oxyn**: a
  read-only and a writable connection on one file cannot be open together.
* **−** **Granting a folder is all or nothing within it**: SQL may read and
  write anything under it, and the folder is typed — the connection form's
  picker chooses files.

**Exit cost:** moderate. The binding is behind the crate's own types — no
`duckdb` type crosses its public API —, so another binding replaces `open`,
`worker`, `stream`, `query` and `logical` (about 1,200 lines) and keeps the
conversion's tests. Moving to a binding on Arrow 59 or later removes the copy of
decision 2 and keeps the logical-type fixups.

**Reconsider if** a `duckdb` release depends on the workspace's Arrow major
(re-checked by `/versions`), at which point the copy goes; if DuckDB's C API
exposes the exception type, at which point the message prefix is no longer
read; if a user need for `icu`, `spatial` or `httpfs` outweighs CMake in every
build; or if the shipped binary grows past what the release budget accepts
once it is measured.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| `duckdb` `1.20000.0` | Published the day before this decision, a new engine major line, and its Arrow is `^60`: still a duplicate, with the least run time behind it |
| The system's `libduckdb` (`duckdb` without `bundled`) | Every user would install DuckDB, at a version Oxyn does not choose: the SQL and the file format would differ from machine to machine |
| The DuckDB CLI as a sidecar ([ADR-0007](0007-driver-sidecar.md)) | Text or JSON on a pipe instead of Arrow, a second process per session, and no interruption of one statement |
| Writing the FFI to DuckDB's C API | `unsafe` throughout a crate, refused by the workspace ([SECURITY](../SECURITY.md#unsafe-policy)), for what the binding already does |
| The binding's row interface (`query_map`, `Row::get`) | Value decoding ends in `unreachable!` for a carrier it does not expect, and a `SET produce_arrow_string_view = true` typed in a console makes every `VARCHAR` such a carrier |
| `arrow_lossless_conversion = true` at opening | It changes how a `UUID` and a `BOOLEAN` travel for every session of the file, and Oxyn's cell rendering does not read DuckDB's Arrow extension types yet |
| Declaring `SERVER_SIDE_CANCEL` | The interface reads it as "the Cancel button reaches a server"; SQLite, which interrupts the same way, does not declare it either |
| One engine per session on a shared file | Two engines writing one file in one process corrupt it, and DuckDB's lock does not see it |
