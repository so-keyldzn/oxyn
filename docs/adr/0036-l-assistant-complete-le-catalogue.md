# ADR-0036 — The assistant completes the catalog itself, through the bus and within bounds

**Status:** proposed · **Date:** 2026-09-24

**Clarifies:** [ADR-0006](0006-ai-privacy-tiers.md), on the following point: what
`Metadata` lets out is no longer "what the cache contains", but "what the
question retains", read from the server if need be.

**Clarifies:** [ADR-0030 § 4 bis](0030-outils-oxyn-exposes-a-un-agent-externe.md#4-bis-the-database-structure-one-tool-for-every-destination),
on the following point: `Command::DescribeCatalog` remains a cache read that
contacts no server, but the `describe_schema` call is now preceded by the
missing metadata reads, each its own command.

## Context

The catalog loads **on demand**, and that is intended: a database of 50,000
relations is not introspected at connection time
([ARCHITECTURE](../ARCHITECTURE.md#6-the-catalog)). On opening, the executor
reads at most the server and its first level; the list of tables of a schema
is read when the user **expands** that schema
(`CatalogRefreshScope::Relations`), the fields of a table when they **open** it.

The AI reads the same cache, and nothing else: that is what makes the single
gateway verifiable ([I-04](../../CLAUDE.md#i-04)). Consequence observed by the
user on 2026-09-24: on a SQLite database of eleven tables never expanded, the
agent receives "Describing 0 of 0 known relations", proposes
`SELECT * FROM your_table`, and `describe_schema` returns the same emptiness.
The `erd` diagram and the list of `@` mentions say "not found" for a table not
expanded. `refresh_catalog` only re-reads the root: it repairs nothing.

What the AI knows about a database therefore depends on what the user has
clicked, for every destination — provider, external agent — and every database.

The order of magnitude that sets the bounds: a remote PostgreSQL at 50 ms round
trip describes a table in three queries (description, indexes, foreign keys),
i.e. about 150 ms; twenty-four tables, 3.6 s. This figure is a sizing
assumption, not a measurement.

## Decision

**Before building a question's context, and when `describe_schema` is called,
Oxyn itself reads what is missing from the cache — without a model.**

1. **What is read.** The server if it has never been read; the list of
   relations of each schema never listed; then the fields, indexes and foreign
   keys of the relations the gateway **will retain** — `@` mentions first, then
   the search guided by the question. A question that follows an already
   informed session only reads its mentions. `refresh_catalog` re-reads the
   server then **re-lists** each schema, fresh or not; it describes nothing.
   The panel's first `@` on a connection (`ai_list_mentionable`) reads the
   server and lists the schemas never listed, without describing anything: the
   mention list only offers what the cache names, and waiting for the first
   question to fill it amounted to asking the user to expand the tree.
2. **The selection is the gateway's.** `oxyn_ai::context::wanted_relations` is
   the function `ContextBuilder::build` calls to choose; the host calls it to
   load. A single selection: two would diverge.
   `CatalogCache::listing_scopes` (`oxyn-catalog`) says which levels name the
   relations — the session's catalog only, without the system schemas.
3. **Through the bus.** Each read is a `Command::RefreshCatalogScope` (or
   `RefreshCatalog` for the server), submitted by `ExecutorSink`: the
   `PolicyGate` decides, the journal records ([I-01](../../CLAUDE.md#i-01),
   [ADR-0004](0004-command-bus.md)). These are the commands of a tree
   expansion: metadata reads, `StatementIntent::Read`, allowed everywhere,
   `production` included. An approval request, if a future policy raised one,
   is withdrawn and the level stays unread.
4. **The actor is the one at the origin.** For a question and for the first
   `@`, `Actor::Human`: asking the question, typing `@`, are user gestures, and
   what it causes to be read is decided by Oxyn from their words and
   mentions — no model asked for anything. For `describe_schema` and
   `refresh_catalog`, the agent's `Actor::Agent`, through its bound sink
   (`ExecutorSink::for_agent`): it is the model's tool call that causes the
   read ([I-07](../../CLAUDE.md#i-07)).
5. **Bounds, written as argued constants** in
   `crates/oxyn-desktop/src/backend/ai/catalog_fill.rs`: 32 schemas listed,
   24 relations described — `ContextPolicy::max_relations`, kept equal by a
   test —, 5 seconds in total. Reads are **sequential**: the executor reads a
   connection's catalog on its reserved session, under a lock; parallelizing
   them would make them wait for that lock.
6. **Exceeding does not fail.** The context goes out with what is loaded. The
   box says what is missing, counted from the cache by the gateway itself,
   whatever the reason: "N relations not loaded yet", "N schemas not listed
   yet", "the catalog of this connection has not been read yet". Counts
   without names, identical under every tier.
7. **Nothing is re-read.** A `Fetched` level is not re-read; `Invalidated`
   (after a DDL issued by Oxyn) is re-read once. A failed read is not retried
   within the same completion.
8. **Cancellable and off the UI thread.** The question's token is the parent
   of each read's token; the timeout cancels the current read and waits two
   seconds for it to finish cleanly ([I-05](../../CLAUDE.md#i-05)).
9. **The panel shows the step**: `AiEvent::CatalogReading` before the first
   read, `AiEvent::CatalogRead` and its counts after
   ([UX-SPEC](../UX-SPEC.md#the-panel-shows-what-is-happening-including-when-nothing-arrives)).

## Consequences

* **+** The AI sees the database, not the clicks: a question on a connection
  never expanded receives its tables and their columns, for every destination.
* **+** The other cache readers benefit without code: the tree through
  `CatalogUpdated`, the `@` mentions, the `erd` diagram.
* **+** A partial schema says so: the model no longer invents the table it does
  not see, and the user reads what is missing in the panel.
* **−** **The AI triggers reads on the server that nobody clicked**, including
  on a production database: up to about sixty catalog queries per question on
  PostgreSQL. That is what makes the decision expensive to undo from the point
  of view of whoever audits the server.
* **−** The audit journal grows by one entry per level read.
* **−** The first question on an unknown database waits up to five seconds
  before going out.
* **−** `describe_schema` now contacts the server; its description tells the
  model so.
* **−** A read refused by the server (a table without privileges) is retried
  on each question that retains it — once per question, under the same bounds.
* **−** System schemas are never listed for the AI: a question on `pg_catalog`
  assumes the user has expanded it.

**Exit cost:** low in the code — the `catalog_fill` module, three calls and two
`AiEvent` variants; the announcement of what is missing and
`wanted_relations` stay useful without it. The real cost is elsewhere: going
back gives the AI back its dependency on clicks, which users will have stopped
compensating for.

**Reconsider if** a connection must forbid any server read made for the AI (an
audited server where every catalog query counts) — a connection marking read by
the `PolicyGate` would then be needed; or if the five-second timeout is
commonly reached on real databases, with a measurement to back it; or if a
background catalog load appears, which would make this completion redundant.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Introspect the whole catalog at connection time | Costs everyone, AI or not — an absent AI must cost nothing ([ADR-0006](0006-ai-privacy-tiers.md)); a database of 10,000 tables exceeds the cache budget (50,000 objects) |
| Let the model call `refresh_catalog` | Depends on the model, consumes its turns, and an external agent does not know it has to; it is the observed failure |
| `oxyn-ai` queries the driver | Bypasses the bus and the gateway ([I-01](../../CLAUDE.md#i-01), [ADR-0004](0004-command-bus.md)) |
| `Actor::Agent` for a question's reads | An external agent's identity is born at its launch, **after** its prompt is composed; and the journal would attribute to an agent reads that no model requested |
| Parallel reads | The executor serializes them on the session reserved for the catalog: no gain, and waits piled up on a lock |
| Remember failed reads from one question to the next | One more state, with no natural bound or forgetting rule; the failure costs one query per question, bounded by the timeout |
