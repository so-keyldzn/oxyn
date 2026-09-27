# ADR-0046 — A retained connection workspace keeps its sessions open, up to eight per window

**Status:** accepted · **Date:** 2026-09-25

**Clarifies:** [ADR-0015](0015-consoles-independantes.md), on the following
point: a workspace's sessions no longer close when another connection is
opened in the window, but only on an explicit gesture — `Disconnect`, closing a
console, quitting.

## Context

UX-SPEC ("The window also keeps the connection workspaces already open") and
IMPLEMENTATION-PLAN ("returning to a connection restores all its consoles")
promise that a round trip A → B → A gives back A's consoles as they were. The
code, surveyed on 2026-09-25, holds only one workspace: `workspace-host.tsx`
remounts it through `key={shown.session}`, and `release-workspace.ts` calls
`backend.disconnect` on the previous connection as soon as another replaces it
(issue #17). The user settled it on 2026-09-25: consoles and results are kept.

Keeping the interface state is not enough. A console is also a server session,
and that session carries two things nothing else rebuilds:

- **the transaction state** ([ADR-0039](0039-etat-de-transaction-d-une-session.md)):
  closing a session whose transaction is open rolls it back, uncommitted writes
  included. UX-SPEC requires a dialog naming the connection for that; a
  connection switch shows none;
- **the session context** ([ADR-0019](0019-contexte-de-session.md)): current
  schema, database, temporary tables, variables. Restoring it on return means
  running SQL the user did not ask for — which the issue's criterion excludes
  ("without running SQL"), and which the temporary table does not allow anyway.

On the memory side, results are already bounded **globally**, not per
workspace: the backend holds at most 16 displayed results and 256 MiB between
them (`SHOWN_RESULTS`, `SHOWN_BYTES`, `backend/results.rs`), then the retention
of [ADR-0017](0017-retention-resultats.md) evicts beyond 16 results without a
reader, 256 MiB resident and 1 GiB of spill. An evicted result reads
"expired"; it is never recreated by replay. The number of workspaces therefore
does not multiply this budget. What it multiplies: server sessions (at least
two per workspace, catalog and first console), the per-connection catalog
cache (1,024 scopes, 50,000 objects, PERFORMANCE), and the mounted React tree.

## Decision

**A retained workspace stays connected.** Hidden, it keeps its sessions — the
catalog's and each console's —, their transactions and their context.
Returning to it emits no command: neither `Connect`, nor `SetContext`, nor an
execution. Choosing on the home screen a connection that already has a
workspace makes it visible instead of opening a second one.

**Only an explicit gesture closes sessions**: `Disconnect` on the workspace,
closing a console (with its dialog), quitting Oxyn. `Disconnect` releases
everything the connection held: drafts are written first
([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)), then
`Command::Disconnect` closes all its sessions, then its conversations and its
external agents stop.

**At most eight workspaces per window** (`MAX_RETAINED_WORKSPACES`). Opening a
ninth connection is refused **before** any `Connect`, with a message that says
so: "8 connections are open in this window. Disconnect one before opening
another." No workspace is evicted automatically: eviction would close a
session whose transaction may be open, without the dialog that announces it.
Eight is a guard bound, not a measurement: sixteen server sessions at least,
and eight catalog caches at most at their ceiling.

**A transaction left in a hidden workspace is stated in plain words**, the
user's addition at acceptance, on 2026-09-25. The home screen's connection
list — which is also that of retained workspaces, marked `Open` — names the
connection one of whose consoles last reported `Open`, or `Unknown` on a
session that declares `TRANSACTIONS`
([ADR-0039](0039-etat-de-transaction-d-une-session.md)): "Transaction open in
a console of *billing*". A text, not a color. Each workspace publishes its
consoles' sessions (`features/workspace/pending-transactions.ts`); the home
screen crosses them with the last received states, without querying the
server.

**A hidden workspace is inert**: no shortcut, no dialog, no entry in the
action registry; text sent to "the active console" — by the assistant, the
inspector — goes to the visible workspace, never to a hidden one. The
`PolicyGate` does not change: it reads the configuration saved per
connection, whether a workspace is visible or not ([I-02](../../CLAUDE.md#i-02),
[I-04](../../CLAUDE.md#i-04)).

**The results of a hidden workspace are not pinned** beyond the existing
budgets: its grids stay mounted and keep their page window, but the buffers
follow `SHOWN_RESULTS` and ADR-0017. On return, a page of an evicted result
reads "expired"; nothing is re-executed.

With [ADR-0043](0043-multi-fenetre.md), the bound and the decision apply per
window, and a window's disconnection becomes the closing of its own sessions,
`Backend` emitting `Disconnect` when none holds the connection anymore.

## Consequences

* **+** A → B → A gives back consoles, texts, results, transactions and
  context without a round trip to the server.
* **+** No transaction is rolled back by simple navigation: the only session
  closing without a dialog was that one.
* **+** On the backend side, result memory stays bounded by the budgets already
  measured, whatever the number of workspaces.
* **−** On the webview side, a hidden grid stays mounted and keeps its observed
  pages (at most `MAX_PAGE_BYTES` each, in bounded number): the JS heap grows
  with the number of consoles, multiplied by eight workspaces at most. No
  PERFORMANCE budget covers it yet; the webview measurement campaign
  (IMPLEMENTATION-PLAN) must quantify it.
* **−** A hidden connection occupies its sessions on the server side: it counts
  in `max_connections`, and an open transaction holds its locks there while
  the user works elsewhere. The home screen names the connection concerned,
  but says neither which console nor since when.
* **−** A hidden session can be cut by the server (idle timeout, restart); the
  user only discovers it at the next execution, as with a visible console left
  idle for a long time.
* **−** The ninth connection requires closing one of the eight, a gesture the
  user did not have to make when Oxyn closed one in their place.

**Exit cost:** going back to a single connected workspace means restoring the
disconnection on connection switch in `workspace-host.tsx` and
`release-workspace.ts` — one day. The real cost is elsewhere: users will have
learned that a transaction left on A waits for them on return. Undoing the
decision makes silent a rollback they no longer expect.

**Reconsider if** instrumented measurements show that a hidden workspace costs
more than a window's budget (PERFORMANCE), or if users hit their server's
session limits: the answer would then be disconnecting hidden workspaces
**without** an open transaction, announced, and not a return to replacement.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep the interface and close the sessions; reopen on return | rolls back any open transaction without a dialog; loses the session context, which can only be restored by running SQL nobody asked for |
| Close the sessions of hidden workspaces without an open transaction, keep the others | two behaviors depending on a state the user cannot see from the home screen; the session context and temporary tables are lost anyway |
| Evict the least recent workspace beyond the bound | closes sessions without a dialog, possibly on an open transaction: it is the defect being fixed |
| No bound | each workspace holds at least two server sessions and a catalog cache; a repeated gesture accumulates them without a ceiling |
