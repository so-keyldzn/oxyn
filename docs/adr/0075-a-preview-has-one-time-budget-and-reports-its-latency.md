# ADR-0075 — A preview has one time budget for preparing and executing, and reports the latency the user waited

**Status:** accepted · **Date:** 2026-11-05

## Context

An internal QA report (2026-10-31) observed three related defects on previews,
which share one session per connection:

1. the 30 s deadline of an execution started when the driver was called
   (`ExecLimits::timeout`, armed in `run_statement`), but a preview first
   **prepares** its request — waits for the session, describes the relation —
   and that happened before the deadline existed: a blocked preparation waited
   without bound, and a preparation of 20 s was followed by a full 30 s
   execution (50 s in all, reproduced on a paused clock);
2. the footer showed `ExecStats::total_time`, which a driver measures from the
   moment it sends the statement: after a long wait for the session, a read
   displayed `0 ms`;
3. an automatic read that became invisible kept the session busy (front end,
   issue #357; no decision here).

## Decision

1. **One budget, one clock.** The clock of a command starts when the executor
   begins it (`started`). A preview draws its preparation and its execution on
   the same deadline: `ExecLimits::DEFAULT_TIMEOUT` counted from `started`.
   A preparation of 20 s leaves 10 s. The 30 s is not raised, and the
   execution's own timeout is capped to it (`min`), never extended.
2. **Preparation is cancelled at the budget**, through its token, and fails
   with `OxynError::Timeout`, like an execution does.
3. **Known limit.** The budget bounds the *work*, not the *cleanup*: after the
   cancellation the executor awaits the driver until it lets go, as it does for
   `execute_within`, because dropping the future would leave the statement or
   the session busy (DRIVER-CONTRACT §2). A driver that no longer answers its
   cancellation keeps the call waiting past the budget.
4. **Two durations, never mixed.** `ExecStats::total_time` stays the SQL
   duration. A new `ExecTimings { prepare, start, stream, cleanup, total }`
   (`oxyn-core`), carried by `Outcome::Executed`, says where the time of the
   command went; `total` is what the user waited. The time to the first row is
   not a phase of its own: a driver that computes the first batch inside
   `execute` counts it in `start`, the others in `stream`.
5. **Where the user feels it.** `CommandOutcome::Executed` carries `latencyMs`
   (= `timings.total`) next to `elapsedMs` (= SQL duration). The preview footer
   shows `latencyMs`; the consoles keep `elapsedMs` until their own report asks
   otherwise. The phases are logged at `debug` level, without any statement
   text (I-03).

## Consequences

- A preview ends within 30 s of its start, or the cleanup is the only thing
  left waiting.
- Adding a field to `Outcome::Executed` and to the IPC mirror
  (`src/lib/ipc/types.ts`) changes both sides in the same commit.

## Reconsider if

- a driver's cancellation of a preparation is seen to outlive the budget in
  practice: the cleanup would then need a bound of its own, as
  `WRITE_SETTLE_GRACE` bounds a stopped write.
