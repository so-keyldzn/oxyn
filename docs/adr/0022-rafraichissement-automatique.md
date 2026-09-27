# ADR-0022 — What refreshes on its own, and what never will

**Status:** proposed · **Date:** 2026-09-10

**Clarifies:** [ADR-0004](0004-command-bus.md), on what the bus publishes besides
what it executes.

## Context

After a `CREATE TABLE` run in the console, the explorer does not move.
After an `INSERT`, the preview of the displayed table still shows the previous
state. After an execution, the history stays empty until one clicks. One has
to click `Refresh` everywhere, and the user who forgets looks at wrong data
with nothing telling them.

The code audit gives three facts that drive the solution.

**The mechanism already exists and a second one must not be written.**
`Backend::subscribe()` returns a `broadcast::Receiver<ExecEvent>`; `QueryConsole`
and `Workspace` already subscribe to it, each in a `cx.spawn` loop that calls
its `on_exec_event`. Neither does any I/O: they read a buffer already in
memory and redraw.

**`Event::CatalogUpdated` exists, and nobody subscribes to it.** It is only
published by `Command::RefreshCatalog`, that is when someone has already asked
for the refresh. Symmetrically, `CatalogCache::invalidate` exists, its
documentation says "to call immediately after any DDL issued from Oxyn", and
**nothing calls it**. [ARCHITECTURE](../ARCHITECTURE.md) promises this behavior.
It is a gap between the document and the code, not a feature to invent.

**No event says what changed.** `Event::Completed` carries a `ResultId` and
statistics. The statement is classified — `StatementIntent` — for the
`PolicyGate`, then that classification is thrown away. A subscriber can
therefore only know "a command of this connection just finished", never "the
`orders` table changed".

## Decision

**The event carries the intent, and nothing more for now.**
`Event::Completed` gains `intent: StatementIntent`, which the executor already
holds when publishing. It is enough to distinguish the three cases that
matter — read, write, DDL — and it invents no structure.

What it does **not** give: the name of the objects touched. A refresh is
therefore, at this stage, **scoped to a connection**, not to a table. We
accept that rather than pretend: extracting the objects referenced by
arbitrary SQL is an analysis job in its own right, and doing it halfway would
produce wrong invalidations in both directions — that is either stale views, or
useless rereads, without knowing which.

**What refreshes on its own, after a successful execution:**

| What was executed | What refreshes | Why it is acceptable |
|---|---|---|
| DDL | the catalog cache of **this** connection is invalidated, the tree reloads | it is what `ARCHITECTURE` already promises, and introspection is lazy: only the open levels are reread |
| DDL or write | the **visible** preview of this connection is reread | a read bounded to 200 rows, on a table the user is looking at right now |
| anything | the local history, if the library is open | a local SQLite read, no network |

**What never refreshes on its own:**

- **nothing after an error.** [UX-SPEC](../UX-SPEC.md#data-of-a-selected-table)
  already says so: "no automatic refresh follows an error". A reread that
  follows a failure masks the failure;
- **never the user's query.** Rereading a preview means reissuing
  `PreviewRelation` — a command Oxyn composes, bounded and read-only.
  Replaying the console's statement would be something else entirely: its cost
  is arbitrary and a `SELECT` may not be idempotent
  ([PERFORMANCE](../PERFORMANCE.md#memory-budgets));
- **nothing that is not on screen.** A preview whose tab is not displayed
  is not reread: it will be when one comes back to it. Invisible work is work
  nobody asked for;
- **nothing on another connection.** `ExecEvent` carries its connection; a
  subscriber that is not on that one ignores the event.

**An automatic reread never destroys an ongoing state.** If a preview is
already loading, the refresh does not interrupt it. If the user set a filter
or a page, the reread **keeps the requested shape**: the refresh shows the same
rows up to date, not a return to the first page.

**The channel is fallible, and the safety net is explicit.** The `broadcast`
has a depth of 256: under load, a slow subscriber receives `RecvError::Lagged`
and loses events. Today it has no consequence, because terminal states go
through a reliable channel and the broadcast only carries display comfort. As
soon as a refresh depends on it, a lost event would leave a stale view
**forever**. Therefore: **on `Lagged`, the subscriber refreshes as if it had
missed everything.** One does not know what was lost; the only honest answer is
to reread everything.

## Consequences

- **+** The user stops looking at wrong data without knowing it. It is the
  flaw this decision fixes, and it is silent.
- **+** `Refresh` stays, and keeps its meaning: forcing a reread even when
  nothing changed on Oxyn's side — for example when someone else wrote to the
  database.
- **−** A write that does not touch the displayed table still rereads its
  preview: 200 rows for nothing. It is the price of not knowing the target, and
  it is bounded.
- **−** A DDL invalidates the catalog of the whole connection, so the tree
  rereads its open levels. On a database with tens of thousands of objects, it
  is what lazy loading already limits, not what this decision makes worse.
- **−** A looping execution — a script that writes a hundred times — would
  cause a hundred rereads. The subscriber **coalesces**: a reread in progress
  absorbs the requests that arrive while it runs.
- **−** A `Lagged` triggers a full reread. It is more costly than a targeted
  refresh, and it is intended: the only alternative is a wrong view.

**Exit cost:** one event field, one invalidation in the executor, and
subscriptions in three views. Nothing is persisted, no format changes.

**Reconsider if** SQL analysis learns to name the objects a statement touches —
then refresh would become targeted, and the two negative consequences above
would disappear. It is the natural next step, and it invalidates nothing
decided here.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| A second notification channel, dedicated to invalidation | The bus exists and already carries the connection and the command; a second channel would be a second place to forget to publish |
| Poll the server periodically to detect changes | Polling costs on every connection all the time, including when nothing happens, and it would still not see what was just written elsewhere |
| Replay the console's statement to refresh | Arbitrary cost, and a `SELECT` may not be idempotent |
| Also refresh what is not visible | Work nobody asked for, on views nobody looks at |
| Ignore `Lagged`, as today | Tenable as long as the broadcast only carries comfort; wrong as soon as a view depends on it to be right |
| Wait to know which table changed before shipping anything | Leaves the flaw in place for a future improvement; the "connection" scope is already correct, only broader than necessary |
