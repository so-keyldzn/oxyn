# ADR-0021 — Knowing whether Oxyn stopped normally, and saying it without guessing

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0016](0016-autosauvegarde-bornee.md), on what the workspace
writes besides its drafts.

> **Clarified by [ADR-0038](0038-un-plantage-s-annonce-une-fois.md).** An
> abandoned session is announced only once (`reported_at`), and ⌘Q goes
> through the orderly shutdown. Without these two points, recovery showed up
> after every closing of the installed application.

## Context

[UX-SPEC](../UX-SPEC.md#restoring-after-an-abrupt-stop) describes the
recovery screen as the one shown "on restart **after an abnormal stop**".
The code cannot make that distinction.

`Recovery::load_page` (`crates/oxyn-app/src/recovery.rs`) lists the documents
whose `documents.is_open` column is true. That flag says only one thing:
the user did not close this document **explicitly**. It is set on every
autosave and cleared by `close_query` only; `QueryConsole::shutdown` does not
touch it. An ordinary `⌘Q` therefore leaves all tabs "open", and the next
launch shows the recovery screen exactly as after a crash.

It is the flaw that wears such a screen out the fastest: shown at every
startup, it stops being read, and the day a write was really interrupted, the
warning that matters goes by with the noise. The current message is actually
cautious to the point of asserting nothing — "copies available", without
claiming to have detected anything —, which is honest but does not deliver the
expected service.

Two constraints bound the solution.

**The repository refuses `unsafe`** ([SECURITY](../SECURITY.md#unsafe-policy),
`unsafe_code = "deny"` at the workspace level). Checking that a process
identified by its pid is still alive requires `kill(pid, 0)` or a crate that
wraps it; and a pid gets reused, so the test would lie sooner or later.

**Two Oxyn instances can run at the same time** on the same store. A binary
marker "a session is open" would make the second instance started conclude to
an abnormal stop, while the first one is working.

## Decision

The store carries an `app_sessions` table (migration 6):

| Column | Meaning |
|---|---|
| `id` | the identity of this launch |
| `workspace_id` | the open workspace, with `ON DELETE CASCADE` |
| `started_at` | when this launch started |
| `heartbeat_at` | last sign of life |
| `closed_at` | set **only** by a clean shutdown; `NULL` otherwise |

**A stop is abnormal when an earlier session has `closed_at IS NULL` and a
`heartbeat_at` older than the abandonment threshold.** Both conditions are
necessary: the first distinguishes the clean shutdown from the rest, the second
distinguishes a dead instance from a live one.

The heartbeat is written every **30 seconds**, and a session is deemed
abandoned after **2 minutes** without a heartbeat. The ratio of four lets
through a machine in brief sleep or a loaded system without concluding to a
crash. These two values are product choices, not measurements: they are
amended here, and the heartbeat is **never** written from the interface thread
([I-05](../../CLAUDE.md#i-05)).

**The clean shutdown sets `closed_at` through the same paths that already wait
for local writes** — `on_app_quit` and `on_window_closed`
(`crates/oxyn-app/src/main.rs`). Order is what matters: the closing is recorded
**after** the document write queue has been drained. The reverse would mark a
clean shutdown over unwritten work.

**What Oxyn shows is what it observed**, and nothing more. When an abandoned
session is found, the screen says so: Oxyn did not close normally. Otherwise,
the recovery screen **does not appear**, even if working copies remain — those
stay accessible through the library, which is made for that. A write with an
unknown outcome keeps its reconciliation warning, which does not depend on this
marker and is never retried ([I-13](../../CLAUDE.md#i-13)).

**The pid is not kept.** It would not survive its own reuse, and checking it
would require what the repository's `unsafe` policy refuses. The heartbeat says
the same thing without lying: it ages.

## Consequences

- **+** The recovery screen becomes a signal again: it only appears when it
  has something to say, so it will be read when it says it.
- **+** Two simultaneous instances do not declare each other dead.
- **+** The marker is a table row readable without Oxyn
  ([I-11](../../CLAUDE.md#i-11)): `closed_at IS NULL` reads at a glance.
- **−** One write every 30 seconds per instance, even when idle. It is one row
  updated in a local SQLite; it is also a periodic disk write on a laptop, and
  it did not exist before.
- **−** A crash followed by a relaunch **within two minutes** will not be
  recognized as such: the previous session still looks alive. The user loses
  nothing — their copies are in the library — but the screen will not show.
  It is the price of not wrongly accusing a concurrent instance, and it is the
  direction chosen for the error.
- **−** One more migration, hence one more format to carry.

**Exit cost:** one table and two call sites. Nothing else depends on it: drafts
keep being written as today, and the recovery screen already knows how to work
without this marker — that is its current state.

**Reconsider if** the heartbeat proves costly when measured, or if Oxyn
acquires an instance lock for another reason — in which case that lock would
say the same thing without a periodic write.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep `documents.is_open` as the only signal | It says "not closed explicitly", not "crash": it is the current state, and it shows the screen on every `⌘Q` |
| A boolean flag "a session is running" | Two simultaneous instances would declare each other abnormal |
| Keep the pid and check it is alive | Requires `unsafe` or a crate for `kill(pid, 0)`, and a reused pid makes the test lie |
| A file lock taken at startup | Says who is running **now**, not how the previous launch ended — the information sought does not survive the release of the lock |
| Write the closing marker before draining the write queue | Would mark a clean shutdown over work not yet written: exactly the case where recovery must trigger |
