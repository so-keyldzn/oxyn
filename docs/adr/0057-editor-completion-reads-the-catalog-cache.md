# ADR-0057 — The SQL editor completes from the catalog cache, reads what is missing within bounds, and takes its insertion text from Rust

**Status:** proposed · **Date:** 2026-10-21

**Refines:** [ADR-0036](0036-l-assistant-complete-le-catalogue.md), on the
following point: the editor is a second reader of the catalog cache that
completes it itself. The mechanism — the bus, `Actor::Human`, sequential reads
on the session reserved for the catalog, nothing read twice — is ADR-0036's; the
selection of what to read is the editor's.

**Builds on:** [ADR-0022](0022-rafraichissement-automatique.md), for the
per-connection signals the editor listens to, which gain one variant.

## Context

[Issue #275](https://github.com/so-keyldzn/oxyn/issues/275) asks that the
console suggest the tables and columns of the connected database. Today the
editor calls `sql({ dialect, upperCaseKeywords })` without a `schema`
(`apps/desktop/src/components/oxyn/sql-editor.tsx`): it suggests the keywords
of the dialect and nothing else, as
[IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md) (Phase 1) already records.
Five facts decide the design.

**The cache is empty where the user needs it first.** The catalog loads on
demand ([ARCHITECTURE](../ARCHITECTURE.md#6-the-catalog)): a schema's relations
when it is expanded, a table's fields when it is opened. A completion that
reads only the cache suggests nothing on a connection whose tree was never
expanded — the failure [ADR-0036](0036-l-assistant-complete-le-catalogue.md)
observed for the assistant ("0 of 0 known relations" on eleven tables).
`catalog_tree` returns nodes without fields, and `relation_detail` returns
`None` until a relation is loaded: there is no bulk read of what the cache
knows.

**Catalog reads are serial and shared.** The executor reads a connection's
catalog on one session reserved for it, under one lock
(`crates/oxyn-desktop/src/backend/ai/catalog_fill.rs`): the tree's expansion,
the assistant's completion and the editor's would queue on the same lock. The
cache itself is bounded — 1,024 scopes and 50,000 metadata objects
(`crates/oxyn-exec/src/catalog/budget.rs`) — and evicts under that budget, so a
name known a minute ago can be gone.

**The webview cannot tell which connection gained names.**
`RefreshSignal::of` (`crates/oxyn-desktop/src/ipc/metadata.rs`) emits
`catalogInvalidated { connection }` for a finished DDL only. The
`CatalogUpdated` event a *load* publishes reaches the webview as `catalogUpdated`
on the execution channel, which bumps `catalogVersion`
(`apps/desktop/src/lib/ipc/events.ts`): a counter without a connection.

**The library composes SQL from received names, and does it wrongly.**
`@codemirror/lang-sql` 6.10.0 (read in the installed sources on 2026-10-21)
inserts a name bare when it matches `^[a-z_][a-z_\d]*$`, and otherwise wraps it
in the dialect's first `identifierQuotes` character **without doubling an
embedded quote** (`nameCompletion`). A relation named `a"b` is inserted as
`"a"b"`; a relation named `order` or `user`, reserved words on PostgreSQL 18
([Appendix C](https://www.postgresql.org/docs/18/sql-keywords-appendix.html):
"tokens that are not allowed as column or table names"), is inserted bare. A
name is hostile input (`"users"; DROP TABLE audit; --` is legal on
PostgreSQL). `oxyn-catalog::path::quote_identifier` doubles the closing
character in each style and is the project's one quoting
([DRIVER-CONTRACT §6](../DRIVER-CONTRACT.md#6-it-escapes-every-identifier-it-composes),
[I-10](../../CLAUDE.md#i-10)).

**PostgreSQL resolves an unqualified name through a state the front does not
know.** `search_path` is per connection. The driver sets `SET search_path TO
"<schema>"` before an execution when the console declared a context, `SET
search_path TO DEFAULT` otherwise, and resets it before the connection returns
to the pool, so that introspection never carries a console's context
(`drivers/oxyn-driver-postgres/src/session.rs`). A console with no declared
context is `serverDefault` (`console-panel.tsx`), and nothing in the front or
in the driver reports what that default is. The server's documentation
(PostgreSQL 18.6, read on 2026-10-21): the default is `"$user", public`; a
`$user` entry with no such schema is ignored; `pg_catalog` is always effectively
searched; `current_schemas(include_implicit)` returns the schemas of the
effective path "in their priority order", omitting those that do not exist or
are not searchable; and a name matching nothing in the path is an error "even if
matching table names exist in other schemas".
[RESEARCH-NOTES](../RESEARCH-NOTES.md#editor-completion--check-of-2026-10-21)
records these sources.

[PERFORMANCE](../PERFORMANCE.md#interaction-budgets) sets two numbers that
apply and none specific to completion: **100 ms** of visible feedback after a
keystroke, and an interface frame under **8 ms** p99 while typing.

## Decision

**The editor completes from the cache, and Oxyn itself reads what the cache
lacks, within bounds, never at the pace of the keystrokes.**

1. **A bulk cache read.** One read-only IPC command, `completion_catalog`
   (name fixed by the first pull request), of the family of `catalog_tree`:
   built in `crates/oxyn-desktop/src/completion.rs` from the cache on the
   blocking pool, calling no driver. For a connection it returns the namespaces
   and, for each, the relations the cache holds — kind, `system`, `stale`,
   virtual and shadow marks as `CatalogNode` carries them — with the fields of
   those that are loaded, and a state that says `notLoaded` or `omitted` for the
   others, never an empty list that would pass for "no columns". It also returns the resolution path (4) and, if
   a bound cut it, `truncated` with counts (6). The editor reads it on mount and
   when a signal (5) says the cache changed, keeps the answer in memory, and the
   completion source reads that copy: **no IPC is sent on a keystroke.**
2. **Bounded loads, through the bus, driven by the editor.** After 300 ms
   without typing or moving the caret, and once when a console opens — never on
   a keystroke —, the editor sends the host the names the statement under the
   cursor holds, through `completion_load`: the relations it names after
   `FROM`, `JOIN`, `INTO`, `UPDATE` and `TABLE`, and the qualified name being
   typed (`public.us|` is `public`, then the unfinished `us`). A name is one or
   two parts, as lexed, quoted or not; at most 64 are accepted, none longer
   than a name of the snapshot. The host **resolves those names in Rust**
   (`completion::plan`) — dialect and resolution path decide, not a second rule
   in TypeScript; the editor's own matching only chooses what to *suggest* —
   and only against relations the cache has *listed*, so a typo or a name
   nobody listed costs no query and cannot steer a read toward an arbitrary
   object: a name is compared with the cache's, and the path of every read is
   the cache's, never composed from what the webview sent
   ([I-10](../../CLAUDE.md#i-10)). A pass reads, in this order and only what
   the cache lacks or has invalidated:
   1. the server level, then the resolution path (4), each once;
   2. the relations of each namespace of that path never listed — and of each
      namespace the statement qualifies with, system ones included, which the
      tree folds — at most `MAX_LISTINGS`;
   3. the fields of the resolved relations not loaded — at most
      `MAX_DESCRIPTIONS`;

   all within `DEADLINE`. A bare name resolves in the first namespace of the
   path that holds it, and only once every earlier namespace of the path was
   read: a namespace nobody read may hold the same name and shadow it, and
   describing the wrong relation would be worse than describing none. Each read
   is a `Command::RefreshCatalogScope` through `ExecutorSink`, the commands of a
   tree expansion: `StatementIntent::Read`, allowed everywhere, `production`
   included ([I-01](../../CLAUDE.md#i-01)); the actor is `Actor::Human`, since
   typing is the user's gesture. They are submitted by the same function as the
   assistant's fills (`CatalogFill::read`), so the deadline, the cancellation
   and the withdrawal of a read held for approval are one implementation. Reads
   are submitted **one at a time**, never queued ahead, so a tree expansion
   issued meanwhile waits behind at most the read in progress; **a newer pass
   of the same connection cancels the older** through its `CancelToken`, since
   the older one works for a statement the user has already changed. A
   `Fetched` level is not re-read, an `Invalidated` one is, and a read that
   failed is not retried within the pass. The front sends a statement's names
   once; it sends them again after a DDL (`catalogInvalidated`), when a pass
   ended before it was done — out of time, or replaced by another console of the
   connection —, and ten seconds after a read failed (not sooner: a table the
   role cannot describe would be asked at every pause). Only the console on
   screen sends. Nothing runs on the UI thread
   ([I-05](../../CLAUDE.md#i-05)). **No new `Command` variant is needed for
   this part**; the IPC command that takes the names is a wrapper over the
   existing commands, and answers with counts (`attempted`, `failed`, `stopped`)
   — what a pass read reaches the editor through the signals (5), not through
   that answer.
3. **The insertion text is composed in Rust, once.** Each name in the snapshot
   carries its `insert` form, produced by `oxyn-catalog::insertion_form`, which
   builds on `quote_identifier`: bare only when the name is a simple lowercase
   identifier **and** not a reserved word of the dialect, quoted with
   `quote_identifier` otherwise; a qualified name is the forms of its levels
   joined by dots (`CatalogPath::insertion_sql`), while `qualify_sql`, which
   quotes every level, stays the form Oxyn itself sends. The reserved-word table
   of each dialect is external data: it is sourced and dated in
   [RESEARCH-NOTES](../RESEARCH-NOTES.md) by the pull request that adds it, and
   **a name not known to be safe is quoted**. The front gives every
   `Completion` an explicit `apply` equal to `insert`, so `nameCompletion` never
   composes SQL. If `lang-sql`'s schema model cannot carry that, the front
   writes its own completion source over the snapshot — decided by a spike at the
   start of the second pull request, not by this ADR. A front test inserts
   `"users"; DROP TABLE audit; --`, `a"b`, `order` and a name containing
   `<img onerror=…>` on each dialect. **A name that hides text is not suggested
   at all**: quoting keeps the SQL intact, but an editor that shows a direction
   control or a line separator shows text other than the one that runs. The
   predicate — Unicode `Cc`, `Cf`, `Zl` and `Zp` plus four blank fillers —
   lives in Rust beside `insertion_form` (`hides_text`), is the one the review
   of destructive operations applies too, and what it refuses is counted in
   `truncated`, not dropped silently: a name, a column's type shown beside it,
   a namespace or an entry of the resolution path. **The dialect is the
   server's, not only the driver's**: the `postgres` driver serves Redshift,
   which reserves words PostgreSQL does not, so the snapshot is composed for
   `Redshift` when the server announces itself as one, and `Redshift` quotes
   every name (its reserved words are not recorded).
4. **The resolution path is read from the server, never assumed.**
   `CatalogCache` holds, for the connection, an ordered list of the namespaces
   an unqualified name resolves in, or "unknown". A new `CatalogRefreshScope`
   variant, `ResolutionPath`, reads it on the catalog's reserved session, which
   no console's context ever reaches: the executor refuses it a context, and
   PostgreSQL resets `search_path` before the connection returns to the pool.
   Each driver answers in its own terms, none of them assumed:

   * **PostgreSQL** reads the server's effective path with
     `current_schemas(true)`, which keeps the implicitly searched `pg_catalog`:
     the server looks in it first unless the path names it, and a relation of
     `public` called like one of its own is not what an unqualified name
     reaches (a review of the first version found it with `false`). The
     session's own temporary schema is left out. Where the path names a system
     namespace, the pass lists it all the same — the tree folds it, and the
     shadowing can only be told from its relations. A name written without
     quotes is folded **by the dialect**: PostgreSQL and Redshift fold it to
     lowercase, ASCII letters only, and compare exactly, so `users` is never
     `"Users"`; the others compare without case, the name as typed first. The
     snapshot carries the rule (`folding`), because the host is the one that
     knows the dialect;
     **Amazon Redshift refuses**, the query leaning on `unnest … WITH
     ORDINALITY`, which Redshift (announcing PostgreSQL 8.0.2) is not known to
     accept;
   * **MySQL and MariaDB** read `DATABASE()`: the server says which database
     the session selected, which the connection form cannot (`lower_case_table_names`
     spells it another way, `init_connect` or a proxy selects another). `NULL`
     is an empty path, which is an answer; an unreadable name is an error,
     never an empty path;
   * **SQLite** answers `temp`, then `main`, then the attached databases in the
     order they were attached (`sqlite3FindTable`, SQLite 3.50.2).

   A provider that cannot answer **refuses**, and the cache keeps "unknown",
   which is not an empty list
   ([DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-it-declares-its-capabilities-per-session-and-simulates-nothing)
   specifies each driver's answer). A console that changed context after
   opening resolves in its own context, and following it is the editor's
   session-context work, not this read's. The
   path the completion uses is the console's declared namespace alone when it
   declared one — `SET search_path TO "<schema>"` replaces the path — and the
   server default otherwise. A relation is suggested **unqualified** only if its
   namespace is in that path and no earlier namespace of the path holds the same
   name — which can only be told if every earlier namespace of the path is
   `listed` and not `stale` and the snapshot is not `truncated`, otherwise a
   namespace nobody read may shadow the name and the suggestion does not resolve
   where it claims to. The path's entries are opaque match keys: never displayed
   as typed text, never composed into SQL. Every relation is suggested qualified
   after its qualifier is typed.
   System namespaces are suggested only after their qualifier. When the path is
   unknown, nothing is suggested unqualified: qualified names only, and
   **`public` is never assumed.** A path the snapshot cannot carry whole — longer
   than its bound (a path is a handful of schemas), or with an entry that hides
   text or is too long — crosses as
   unknown rather than with an entry dropped: the first match wins, so a path
   with one missing would make a later namespace pass for it. A namespace left
   out of the snapshot is counted in `truncated` even when the cache holds
   nothing of it, so the front never has to infer it from an absence. A `SET search_path` typed inside a script is
   not followed.
5. **One signal more.** `RefreshSignal::of` maps `Event::CatalogUpdated`
   to a new `catalogChanged { connection }` — "a catalog read of this
   connection finished", which does not say that anything changed, only that
   the cache may hold other content — delivered like the others to the windows
   that hold the connection ([ADR-0043](0043-multi-fenetre.md)). The editor
   re-reads the snapshot on `catalogChanged` and `catalogInvalidated`, coalesced.
   **A signal starts no load**: it only prompts a re-read of the cache; loads
   start from the editor's pass (2), whose reads are what produce the next
   signal, so a signal cannot feed itself. The global `catalogVersion` is not
   used, and the cache gets no revision counter: the signals already say what
   changed. After a DDL, `stale` names stay
   suggested, flagged, while the editor's next pass re-reads them: the executor
   invalidates the whole catalog after any DDL, and emptying the completion
   after a `CREATE INDEX` would be worse than suggesting a name for a moment
   longer. A name leaves when a re-read no longer lists it, or when the cache
   evicts it.
6. **Bounds measured before they are written.** This ADR writes no number for
   the snapshot's caps — relations, columns, bytes of
   names (paid by namespaces too: a schema per tenant is thousands of them), and
   the length of one name, because the cache counts objects and not bytes and a
   name can be long. The first pull request measures
   them with [`/benchmark`](../../.claude/commands/benchmark.md) on a catalog at
   the cache budget (50,000 objects), against the two existing budgets
   (100 ms, 8 ms p99), records the result in PERFORMANCE, and writes the
   constants with their argument. The loads' constants are another kind of
   figure: `MAX_LISTINGS` (32) and `DEADLINE` (5 s) are the assistant's own
   (`catalog_fill`), and `MAX_DESCRIPTIONS` is 16 against its 24, since a
   statement names a few relations and the pass is repeated at each pause. They
   are argued from that module's arithmetic and **not re-measured**: it takes a
   server at a known round trip, and a pass is bounded by construction — one
   read at a time, under a deadline, replaced by the next. The message is parsed on the webview's main
   thread, so its size is the cost that decides. A snapshot cut by a cap says so
   (`truncated`, with the counts of what was left out), and the completion list
   says so too —
   the wording goes into UX-SPEC in the second pull request. **A partial answer
   is never presented as complete.**
7. **What it does not promise.** Columns are proposed for relations the
   statement names by the lexical rule above, with their aliases; the names a
   leading `WITH` declares are told apart from the relations of the catalog they
   may shadow, and their bodies are not read; subqueries, `RETURNING`, table
   functions and dynamic SQL are not inferred, and what the rule cannot resolve
   gets relations and keywords, never a guessed column. The snapshot goes to the webview and nowhere else — not to
   `oxyn-ai`, which keeps its single gateway
   ([I-04](../../CLAUDE.md#i-04)) —, holds names and kinds, no row, and is not
   persisted ([I-11](../../CLAUDE.md#i-11)).

## Consequences

* **+** The first keystroke on a connection never expanded already suggests the
  schema's tables, and `alias.` suggests the columns of a table nobody opened.
* **+** An inserted name is correct on each dialect: quotes doubled, reserved
  words quoted, a hostile name inert. One quoting, in Rust, tested once.
* **+** PostgreSQL suggests what resolves: the unqualified names of the real
  path, and nothing invented when the path is unknown.
* **+** No IPC on a keystroke; the loads reuse the commands, the policy and the
  journal of the tree's expansion, and no `Command` variant is added for them.
* **−** **Oxyn reads the server where nobody clicked**, including a production
  database: bounded per pass, but a console left open and edited produces
  catalog queries all day. That is what makes the decision expensive to undo for
  whoever audits a server.
* **−** The audit journal gains one entry per level read, as in ADR-0036.
* **−** Editor, tree and assistant share one session and one lock: the editor's
  reads can delay a tree expansion by one read, and fill the cache toward its
  50,000-object budget faster, hence evict sooner.
* **−** A reserved-word table per dialect to keep current, sourced and dated: a
  word reserved by a later server version and missing from the table is inserted
  bare, and the server rejects the statement naming that word — a visible
  failure, not a silent one. The MySQL and MariaDB lists are read from the
  grammar of MySQL 8.4.0 and MariaDB 11.8, not from the server: a 9.x server
  is not checked, and the lists are re-extracted by `/versions`.
* **−** Where `temp` and `main` rank among SQLite's databases is read from
  `build.c` of 3.50.2, not from a documented guarantee: the ATTACH page says the
  least recently attached database wins among the attached ones and is silent
  on those two. A later release may change the order; the source is re-read by
  `/versions`.
* **−** Changes the contract surface: a `CatalogRefreshScope` variant in
  `oxyn-core`, an optional provider method in `oxyn-catalog`, an answer from each
  driver, a signal variant, the sections of DRIVER-CONTRACT and ARCHITECTURE that
  list them.
* **−** A provider that refuses the resolution path (Redshift today) leaves the
  cache on "unknown", which no read can change, and the refusal still completes
  as a read: the pass asks the path once, and each pass on such a connection
  costs one read and one `catalogChanged` for nothing. The front sends a
  statement's names once, which bounds it to one per distinct statement;
  remembering the refusal would be state to keep in step with the root's
  refresh, and is not worth it for one cheap read.
* **−** The default path is read once per connection: an `ALTER ROLE … SET
  search_path` made meanwhile is seen at the next root refresh.
* **−** A copy of the snapshot lives in the webview's memory, bounded by the
  caps of (6).
* **−** If `lang-sql` cannot carry the explicit `apply`, the front owns a
  completion source: more code than a configuration option.

**Exit cost:** low on the front — the snapshot command, the adapter and the
loads are isolated, and removing them returns the editor to keyword completion.
The resolution path, the `catalogChanged` signal and the insertion function stay
useful without it (the tree, "Copy as…"). The real cost is elsewhere: users will
have stopped expanding the tree to get their names.

**Reconsider if** a connection must forbid every server read nobody clicked (an
audited server): ADR-0036 already names that condition, and a connection marking
read by the `PolicyGate` would then switch the editor to cache-only — which the
design allows, since loading is a separate layer; if the measured completion
exceeds the 100 ms or 8 ms budgets on a catalog at the cache budget with no cap
that keeps it under; or if `@codemirror/lang-sql` stops being the editor's
language package.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Complete from the cache only | Empty on a connection whose tree was never expanded — the observed failure of ADR-0036, for the editor this time |
| Introspect the whole catalog on connect | Costs everyone, and a database of 10,000 tables exceeds the cache budget (ADR-0036 rejected the same) |
| Ask the server on each keystroke, or from the webview | Breaks [I-01](../../CLAUDE.md#i-01) and [I-05](../../CLAUDE.md#i-05) and the 100 ms budget; a typo would cost a query |
| Let `lang-sql` quote what it inserts | Does not double an embedded quote and inserts reserved words bare (read in 6.10.0): SQL composed from a received name, [I-10](../../CLAUDE.md#i-10) |
| Quote in TypeScript | A second implementation of DRIVER-CONTRACT §6, which diverges |
| Quote every inserted name | Safe, and it avoids the reserved-word table, but `FROM "users"` for every table makes users stop accepting suggestions |
| Assume `public` on PostgreSQL | The path can differ, and a name that exists only in a schema outside it is an error for the server: the completion would suggest names that do not resolve |
| Read the path on the console's session | That session may be running or in a transaction, and `search_path` is per connection; the catalog's reserved session is on the default by construction |
| Resolve the statement's names in TypeScript | Case folding and quoting depend on the dialect and the server; a second rule next to the Rust one |
| A revision counter in the cache | The signals already say what changed ([ADR-0022](0022-rafraichissement-automatique.md)); a counter is state to keep in step |
| Keep using the global `catalogVersion` | No connection: every editor would re-read on any connection's change |
| Empty the completion on `stale` | The executor invalidates the whole catalog after any DDL: the completion would go blank after each `CREATE INDEX` |
