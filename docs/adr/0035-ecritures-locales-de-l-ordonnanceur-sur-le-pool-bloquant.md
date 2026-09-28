# ADR-0035 — The scheduler's local writes go through the blocking pool, as owned operations

**Status:** accepted · **Date:** 2026-09-24

**Clarifies:** [ADR-0012](0012-lecture-pages-resultats.md), which moves page
reads off rendering with the same gesture (`spawn_blocking`, no runtime of its
own) — this decision extends it to writes; [ADR-0004](0004-command-bus.md), on
the "journal before and after" sequence, whose unfolding changes here without
the sequence itself being called into question.

## Context

`Executor::local_worker` (`crates/oxyn-exec/src/executor.rs:1541`) already
submits reads — result pages, history, connections — to the application's Tokio
blocking pool: `tokio::runtime::Handle::try_current()`, never `Runtime::new`,
then `spawn_blocking(move || …).await`, the join failure mapped to
`OxynError::Internal`. It is the gesture [ADR-0012](0012-lecture-pages-resultats.md)
established for reads.

A TODO dated 2026-09-10 (`executor.rs:43`) marked what remained to be done:
the *local* Store accesses that are not page reads — saving and deleting a
connection, reading its configuration before connecting, resolving credentials
in the keychain (`self.credentials.resolve`), and the export write
(`export_result`, which does `File::create` then `export(...)` synchronously) —
as well as the audit writes — policy decision (`journal_decision`), execution
outcome (`journal_result`), history start and finish (`history_start`,
`history_finish`) — all ran inline on the worker that executes
`Executor::dispatch`. It is the shared worker of the multi-thread Tokio runtime
described in
[ARCHITECTURE §9](../ARCHITECTURE.md#9-execution-and-threading-model): the
same one that carries the drivers, the network and the LLM calls. A synchronous
write that blocks there delays every other command in flight on that thread,
which [I-05](../../CLAUDE.md#i-05) forbids.

The move is not mechanical for audit writes. `Executor::run`
(`executor.rs:552-574`) arms an `OutcomeGuard` (`crates/oxyn-exec/src/abandon.rs`)
just before executing the command and calls `guard.settle()`
(`executor.rs:569`) **before** `journal_result` and `history_finish`
(`executor.rs:571-572`). The guard exists precisely so that a future abandoned
between execution and the write of its outcome — window closing, losing
`select!`, runtime shutdown — still leaves a trace in the journal, classified
`Ambiguous` ([I-13](../../CLAUDE.md#i-13)) rather than a silent hole. Turning
`journal_result`/`history_finish` into a plain `spawn_blocking(...).await`
without thinking about it would have moved the abandonment window *after*
`settle()`, where nothing covers it anymore. The same question arises, less
seriously, for `journal_decision` on an `Allow` decision: between the
authorization returned by `self.policy.authorize` and the write of the
decision, an abandonment must leave neither an executed command without a
trace, nor a trace for a command never executed.

`Executor` does not build a runtime: it only uses the blocking pool that
`main.rs` hands to Tauri ([ARCHITECTURE §9](../ARCHITECTURE.md#9-execution-and-threading-model)).
A call to `spawn_blocking` therefore assumes a current Tokio runtime — absent,
for example, during application shutdown. `journal_abandoned_off_runtime`
(`executor.rs:1999-2020`) already handles this case: a successful
`Handle::try_current()` goes to `spawn_blocking`, otherwise the write happens
inline, for lack of a pool thread to hand it to. It is the precedent the audit
writes follow.

## Decision

Each local write that the dispatch worker used to execute inline becomes an
**owned operation**: its data is built in memory on the worker — cloned or
moved, never borrowed from `&self` or from `dispatch`'s stack — then executed by
`spawn_blocking(move || …).await`, with the same join-failure mapping as
`local_worker`, already established by
[ADR-0012](0012-lecture-pages-resultats.md). Concerned: `save_connection`,
`delete_connection`, `connection_config` (hence `connect`), `export_result`, and
the four audit writes — `journal_decision`, `journal_result`,
`history_start`, `history_finish`.

**The connection cache follows the disk in the same operation.** The
scheduler's `connections` registry provides the environment submitted to the
`PolicyGate` ([I-02](../../CLAUDE.md#i-02)). `save_connection`,
`delete_connection` and `connection_config` therefore update it **in the pool
task**, right after the Store write or read, under a dedicated lock
(`connection_writes`) that `register_connection`, `forget_connection` and
`load_connections` also take. Updated on return from the `.await`, the cache
could keep `development` when the disk says `production`: two concurrent
updates write to it in a different order than on disk, and a future abandoned
after the write never updates it. Cache readers do not take this lock.

**Without a current Tokio runtime**, a command operation (connection, export)
fails with a configuration error, as `local_worker` already does; an audit
write runs inline, as `journal_abandoned_off_runtime` already does — the audit
must never be lost for lack of a pool, even at shutdown.

For an `Allow` decision, the `OutcomeGuard` is armed as soon as
`self.policy.authorize` returns `Allow`, **before** the decision is written,
with no `.await` in between: no abandonment can therefore occur between the
authorization and the arming of the guard. The `Allow` decision and the
"in progress" history entry form a single owned operation, submitted to the pool
before any execution.

The refusal (`Deny`) and its history entry form one operation. The two writes
it contains are still attempted independently — the failure of one does not
prevent the other, as today.

The `RequireApproval` decision is a separate operation: `approvals.submit` and
the `Event::ApprovalRequested` event only go out after it succeeds.

The outcome of an execution — `journal_result` and `history_finish` — forms a
single operation, submitted to the pool **immediately after** `guard.settle()`,
with no interleaved `.await`: the window `OutcomeGuard` covers therefore stays
exactly the one it covers today, between the start of execution and the
queuing of this operation.

Each write is submitted to `spawn_blocking` before any suspension point that
follows it in the calling code. No task is detached: the handle (`JoinHandle`)
is always awaited by an `.await` on the normal path.

Credential resolution in `connect()` (`self.credentials.resolve`, system
keychain) follows the same gesture: `Arc::clone(&self.credentials)` and an owned
clone of the connection configuration go into the closure. The `Credentials` it
returns are never journaled, nor formatted with `Debug`
([I-03](../../CLAUDE.md#i-03)) — the filtering already in place before the Store
write (`executor.rs:1634-1637`) is unchanged by this move.

## Consequences

* **+** The worker that executes `Executor::dispatch` no longer blocks on any
  local I/O: the TODO of 2026-09-10 is resolved in full, including the audit
  writes it did not literally name.
* **+** The gesture is the one, already reviewed and in production, of
  [ADR-0012](0012-lecture-pages-resultats.md): no new pattern to learn, no new
  error class.
* **+** No detached task: each operation keeps its handle until its `.await`,
  in accordance with [rust.md](../../.claude/rules/rust.md#async).
* **−** (a) An abandonment during the write of the `Allow` decision leaves an
  "abandoned, outcome unknown" outcome (`Ambiguous`, never retried), even if
  nothing was executed. It is the truth as seen by the scheduler — it cannot
  know, at that moment, whether it was abandoned before or after reaching the
  server — never a hole in the journal.
* **−** (b) An abandonment of the `dispatch_as` future during the write of a
  `RequireApproval` decision leaves in the journal a `RequireApproval` line with
  no pending request: `approvals.submit` is never reached,
  `Event::ApprovalRequested` is never published, and nobody can approve it.
  Nothing is executed and nothing is ambiguous — I-13 is not at stake, since no
  command could reach the server without a pending request to carry it. The
  visible state is that of a rejected (`Executor::reject`) or expired
  (`PendingApprovals::sweep`) request, which no journal line already follows
  today. It is an accepted limit, not an outcome hole.
* **−** (c) The insertion order of journal lines may differ from their `ts`:
  several threads of the blocking pool write, serialized only by the Store's
  internal lock, not by emission order.
* **−** (d) A pool operation not yet started when the runtime stops may never
  run — already true of any use of `spawn_blocking`, now more frequent since
  more writes go through it.
* **−** (e) Each command makes two more trips to the blocking pool (decision,
  outcome), with no latency figure measured to date. Any future optimization
  that would group these trips must rest on a measurement, not an assumption
  ([PERFORMANCE](../PERFORMANCE.md#the-rule-that-prevents-gratuitous-optimization)).
* **−** Out of the scope of this decision, still inline on the worker: the
  eviction of retained results after an execution (`prune_results`, which
  deletes spill files) and `load_connections`, called when the backend is
  assembled, before the window exists.

**Exit cost:** moderate, confined to `crates/oxyn-exec/src/executor.rs` and
`crates/oxyn-exec/src/abandon.rs` — no schema, no new `Command`, no IPC
touched. Going back to inline writes reopens I-05 on all the handlers
concerned.

**Reconsider if**
* a measurement shows that pool trips dominate the latency of small commands;
* the journal's insertion order becomes a requirement (would need a dedicated
  audit writer, with an ordered queue);
* an approval request lost on abandonment (consequence b) becomes an observed
  problem, calling for a reconciliation of `RequireApproval` decisions with no
  follow-up;
* the Store stops being synchronous;
* a write of the connection cache appears outside the `connection_writes`
  lock: it is the I-02 window reopening, not a simplification.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep audit writes inline on the dispatch worker (option A) | Resolves neither the TODO nor I-05 for the most frequent part of writes: each command journals at least twice. |
| A dedicated writer thread, fed by a channel | Would resolve consequence (c) on ordering, but introduces a queue, a clean shutdown protocol and a delivery latency that nothing here justifies without a prior measurement. |
| Detached `tokio::spawn` for writes | Nobody holds the handle: violates [rust.md §Async](../../.claude/rules/rust.md#async) and silently drops a write if the runtime stops before it runs. |
| One `spawn_blocking` per elementary write, including separate decision + history | Multiplies pool trips without reducing the useful abandonment window; grouping into one operation per logical step (decision + "in progress" history, outcome + "finished" history) costs nothing more and reduces the number of trips. |
| Write the `RequireApproval` decision inline, to keep a zero abandonment window at that spot | Reopens I-05 for this one case, for a window already covered by a visible state and with no ambiguous consequence (b). |
| Put the approval request on hold *before* writing its decision, withdrawing it if the write fails | A concurrent approval in between would execute a command whose decision was never written to the journal. |
