# ADR-0054 — Bundle sqlite-vec in the SQLite driver, opt-in per connection

**Status:** accepted (2026-10-07) · **Date:** 2026-10-06

## Context

Vector tools store embeddings in SQLite through the sqlite-vec extension: a
user's database created by `semantiq` holds `chunks_vec`, a
`CREATE VIRTUAL TABLE … USING vec0(…)`. Opening it in Oxyn fails with
`no such module: vec0`. The engine needs the module to read a virtual table,
not only to write it: the table is visible in the catalog and nothing can be
read from it. [VISION](../VISION.md) puts vector databases in scope.

SQLite extensions come in two forms: a shared library loaded at runtime
(`load_extension`), or C code compiled into the binary and registered on the
connection. The facts below were checked on 2026-10-06 and are recorded in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#sqlite-vec--checked-on-2026-10-06):

- the `sqlite-vec` crate, **0.1.9** (latest stable, 2026-03-31), dual
  MIT/Apache-2.0, accepted by `deny.toml`; it ships the C amalgamation and a
  `build.rs` that compiles it with `cc` and `SQLITE_CORE`, so it calls the
  SQLite 3.50.2 that `rusqlite` 0.37's `bundled` already links — one engine,
  not two;
- 96 KiB of code and 1 KiB of data in the static archive, 84 480 bytes once
  linked into the stripped release binary, 8 s for the crate's release build on
  an arm64 laptop, no SIMD flag (portable scalar code);
- its 42 `assert()`s stay live in release: `build.rs` does not define
  `NDEBUG`, and some assert sizes read from the file's shadow tables. A hostile
  file that reaches one **aborts the process**, without a panic and with
  unsaved work lost;
- `rusqlite` 0.37 has no safe API to register a statically linked extension,
  and the workspace refuses `unsafe_code`
  ([SECURITY](../SECURITY.md#unsafe-policy)): lifting that refusal for one
  module takes this ADR.

## Decision

**sqlite-vec is compiled into `oxyn-driver-sqlite`, and runs only on a
connection where the user turned it on.** The workspace pins
`sqlite-vec = "=0.1.9"` exactly — the `unsafe` below relies on its C signature
and its `build.rs` — and `Cargo.lock` holds the checksum.

**The switch is a connection parameter, off by default.** The SQLite driver
declares a second field, `sqlite_vec` (`SqliteDriver::VECTOR_EXTENSION`), of
kind `Bool`, default `"false"`, labelled "Enable sqlite-vec (vec0 vector
tables)". Its description says what it runs and when it applies: "Runs the
bundled sqlite-vec C extension on this file. Enable it only for files you
trust. A change applies to sessions opened afterwards: disconnect to apply it
to open ones." It is persisted with the
other non-secret parameters, as readable JSON in the workspace
([I-11](../../CLAUDE.md#i-11)); a connection saved before the field existed has
no such key and reads as off. Only `"true"` turns it on; another non-empty value
is a configuration error rather than a silent off. A `Duplicate` does not copy
it: a copy is a new connection, often to another file.

**Registered at opening, or not at all.** When the switch is on,
`worker::open` calls `vector_extension::register` right after
`sqlite3_open_v2`, before the `query_only` pragma, for every session of the
connection: read-write, read-only, previews, file and shared in-memory alike.
`SELECT name FROM pragma_module_list` then lists `vec0` and `vec_each`. When it
is off, nothing of sqlite-vec is registered: a `vec0` table answers
`no such module: vec0`, and no statement on a session opened that way can
reach the extension's C code. A change applies, like any other connection
parameter, to the sessions opened after it (`Backend::update_connection`
closes none: a console may hold an open transaction). Sessions already open
keep what they were opened with until `Disconnect`, which the description
says — turning the switch off on a file one no longer trusts therefore takes a
`Disconnect` to reach the catalog session and the open consoles. Registration
failing fails the opening with a **permanent** driver error — it fails the same
way at every attempt — carrying sqlite-vec's message, which names a function or
a module, never a path nor a value.

**Only the human changes it.** The switch decides whether bundled C code parses
a file: it is connection configuration, edited in the connection form. No AI
tool produces `CreateConnection` or `UpdateConnection`, and the `PolicyGate`
refuses both to an `Actor::Agent` in every environment — a refusal, not an
approval ([I-07](../../CLAUDE.md#i-07), [I-02](../../CLAUDE.md#i-02)).

**Only `sqlite3_vec_init` is called.** It registers the `vec_*` scalar
functions and the `vec0` and `vec_each` modules. `sqlite3_vec_numpy_init`,
whose `vec_npy_file` reads any file by path, is not registered: a database or
an agent could otherwise make the driver read a file of their choice.

**The registration is per connection, not process-wide.** `sqlite3_auto_extension`
would also equip the connections of `oxyn-store` and `oxyn-desktop`, which have
no use for it, and could not honour a per-connection switch.

**`unsafe` is allowed in one function and nowhere else.**
`drivers/oxyn-driver-sqlite/src/vector_extension.rs` carries the crate's only
`#[allow(unsafe_code)]`, on `register`, with three blocks, each preceded by its
`// SAFETY:`:

1. the upstream crate declares the entry point as `fn()`; it is transmuted to
   its C signature, `int (*)(sqlite3*, char**, const sqlite3_api_routines*)`.
   The pinned version keeps that true; a bump re-reads it;
2. the call receives the live handle of the borrowed connection, on the
   worker thread that owns it, a valid `pzErrMsg` (sqlite-vec writes it
   without checking for null) and a null `pApi`, ignored under `SQLITE_CORE`;
3. the error message, allocated by `sqlite3_mprintf`, is copied then released
   once with `sqlite3_free`.

**What a read returns.** A vector column is a BLOB for SQLite — `float[N]` is N
little-endian `f32`, `int8[N]` N bytes, `bit[N]` N/8 bytes — and the existing
type decision of [`convert`](../../drivers/oxyn-driver-sqlite/src/convert.rs)
makes it an Arrow `Binary` column ([ADR-0002](0002-arrow-result-model.md)).
The driver does not reinterpret the bytes as a float list: the blob carries no
element type of its own, and the user who wants text writes
`vec_to_json(embedding)`. Nothing is decoded in Rust, so a malformed vector
cannot reach a slice index ([I-09](../../CLAUDE.md#i-09)); the extension's own
checks return an engine error, which the driver classifies as for any
statement. The tests prove it for a truncated blob, the wrong dimension, text
that is not a vector, a malformed KNN query vector, and a truncated chunk in a
shadow table.

**A KNN query stays user SQL.** `WHERE embedding MATCH ? AND k = 10` is sent as
written; the read/write decision stays the engine's
(`sqlite3_stmt_readonly`), which classifies it as a read, and an `INSERT` into
a `vec0` table as a write. No new path bypasses the command bus
([I-01](../../CLAUDE.md#i-01)).

**A stopped write is ambiguous whatever its code.** sqlite-vec turns the
`SQLITE_INTERRUPT` of its inner statements into a plain `SQLITE_ERROR`
("Could not find latest chunk"), so the result code no longer tells a stopped
write from a refused one. `WorkerHandle::await_verdict` therefore classifies
any driver error of an interrupted request not declared read-only as
`Ambiguous` ([I-13](../../CLAUDE.md#i-13)) — a KNN read sent with write limits
included, which is imprecise but never retried. The worker test
`an_interrupted_write_failing_under_another_code_is_ambiguous` proves the rule
deterministically; `a_stopped_write_into_vec0_is_ambiguous` reproduces the case
on the extension.

Displaying virtual tables and the availability of their module in the catalog
is a separate change; this decision only makes `vec0` available where the user
asked for it.

## Consequences

* **+** A file the user did not opt in for can never reach sqlite-vec's C code:
  the abort and memory-safety risks below are confined to files the user chose
  to trust, connection by connection.
* **+** On such a connection, a `vec0` table opens, previews and answers KNN
  queries like any table, with nothing to install.
* **+** No native code enters at runtime: what runs is what was built, signed
  and shipped.
* **+** The `unsafe` surface is one function of about twenty lines, without
  pointer arithmetic; the rest of the crate keeps calling safe code.
* **−** One more step for the user who opens a vector database: the first
  attempt fails with `no such module: vec0` — explaining that error is the
  separate catalog change — and reading needs the switch, then a `Disconnect`
  and a new `Connect`.
* **−** The switch trusts a connection, not strictly one file: the module is
  registered on the database handle, so a file brought in with `ATTACH` on
  that connection reaches sqlite-vec too. `ATTACH` is user SQL, and DDL for
  the `PolicyGate` when an agent proposes it (refused in production, subject to
  approval elsewhere).
* **−** Turning the switch off does not reach sessions already open: until
  `Disconnect`, they keep sqlite-vec registered. Closing them on save was
  rejected below.
* **−** 320 KB of third-party C source runs in Oxyn's process on data that
  comes from the opened file, once the switch is on: `vec0` reads its shadow
  tables. A memory error in it is not caught by Rust; the threat model of
  [SECURITY](../SECURITY.md) — the attacker is the data the user opens —
  applies to it. The corrupted-chunk tests cover the size checks of the
  `vectors` and `rowids` blobs, not the extension.
* **−** On a connection with the switch on, a hostile file that reaches one of
  the live `assert()`s **aborts the process**. One path is plausible and was not
  reproduced: `vec0_metadata_filter_text` receives a chunk id truncated to `int`
  and only asserts the size of the `rowids` blob it then opens. Defining
  `NDEBUG` would turn that abort into an unchecked read: it is not a fix.
* **−** The workspace's "no `unsafe`" statement now has one exception, which
  [SECURITY](../SECURITY.md#unsafe-policy) names.
* **−** sqlite-vec is pre-1.0 and maintained by one person; the on-disk format
  of `vec0` may change between versions, and a database written by a newer
  sqlite-vec may not open with the bundled one.
* **−** Writes into a `vec0` table become possible too, through the same
  confirmations as any write ([I-02](../../CLAUDE.md#i-02)).
* **−** 84 480 bytes more in the stripped release `oxyn-desktop` (macOS arm64,
  measured against the same tree without the extension), and a C compilation
  in each cold build (8 s, in parallel with the rest).

**Exit cost:** low. Removing the dependency, `vector_extension.rs`, the
`sqlite_vec` field and one call in `worker::open` returns to the previous
state; a saved `sqlite_vec` parameter would then be an unknown key, and users
lose the ability to read `vec0` tables. Nothing else they saved depends on it.

**Reconsider if** a memory-safety advisory hits sqlite-vec without a timely
fix, or an `assert()` is shown reachable from file data; if upstream stops
publishing stable releases for a year; if `rusqlite` gains a safe registration
API (the `unsafe` then goes); if sqlite-vec's asserts become real checks
upstream, which would make on-by-default worth asking again; or if a second
extension is requested — the question then becomes a list of vetted
extensions, not this one-off.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Registering sqlite-vec on **every** connection the driver opens (this ADR's first draft) | Any SQLite file opened in Oxyn — a download, a file dropped on the window, a shared database — would reach sqlite-vec's C code on its first `vec0` read, and its live `assert()`s can abort the process on a hostile file. The cost of the switch is one checkbox; the cost of its absence is a crash with unsaved work lost, on a file the user never chose to trust |
| Closing the connection's open sessions when the switch is saved | A console may hold an open transaction and unsaved drafts: closing it rolls back the user's work on a settings change, the very loss the switch exists to avoid. No other connection parameter does it; `Disconnect` is the explicit act that writes the drafts first, then closes |
| Turning it on automatically when the file contains a `vec0` table | The file itself would decide that the C code runs on it: exactly the hostile case, a file built to reach an assert |
| A global preference instead of a per-connection switch | Trust is about a file, not about the machine: the setting chosen for one's own vector index would apply to the next database downloaded |
| Loading a `.dylib`/`.so` the user designates (`load_extension`) | Arbitrary native code run with the user's rights, from a path that a shared database, a workspace file or an agent could suggest. No signature, no review, nothing Oxyn can vouch for — and the `load_extension` feature would stay switched on for every connection |
| Loading the extension from a file Oxyn ships next to the binary | Same code as bundling, plus a file to sign per platform and a path to protect from replacement; nothing gained |
| Building the upstream C amalgamation with our own `build.rs` | Same code, but the vetting, the version pin and the license tracking move into the repository instead of `Cargo.lock` and `cargo-deny` |
| `PRAGMA trusted_schema = OFF` on every connection, so that the file's own views and triggers cannot drive `vec0` | sqlite-vec marks its module neither innocuous nor direct-only: a view over a `vec0` table — what a vector tool writes to join chunks to their text — would stop working. The C code reached through a view is the one a direct preview reaches anyway; the switch already confines both to trusted files |
| Process-wide `sqlite3_auto_extension` | Equips connections that do not need it (`oxyn-store`, `oxyn-desktop`), cannot follow a per-connection switch, and makes the registration depend on the order of first opening |
| Reading `vec0` tables without the module, through their shadow tables | Duplicates the extension's on-disk format in Rust, breaks at its next change, and still cannot answer a KNN query |
| Decoding vectors into an Arrow `List<Float32>` | The blob does not say its element type (`float`, `int8`, `bit`) outside the table declaration; a guess would misrender two of the three, and decoding would put slice arithmetic on file-controlled bytes |
| The 0.1.10 alpha line | Pre-release; nothing in it is needed to read existing tables |
