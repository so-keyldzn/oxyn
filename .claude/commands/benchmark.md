---
description: Measure a performance before optimizing it
argument-hint: "<what to measure>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob
---

Purpose: measure **$ARGUMENTS**.

## The rule that governs the others

**Clear code is not replaced by fast code without the measurement that shows it
was worth it.** A benchmark before, a benchmark after, the number in the commit
message (`docs/PERFORMANCE.md` § the rule that prevents gratuitous
optimization).

Without it, the complexity is paid up front and the gain is assumed. The
corollary holds the other way: a `.clone()` on a path called once per window
opening is not a problem, and turning it into a borrow that contaminates five
signatures is a net loss.

## Choosing the instrument

| What is measured | With what | Not with |
|---|---|---|
| Pure code: conversion to `RecordBatch`, parsing, formatting, schema diff | `criterion` | — |
| Rendering, frame latency, responsiveness | the system's instruments (Instruments, `perf`) | `criterion` — a `criterion` bench sees neither the webview nor its rendering |
| Idle memory footprint | observation over several hours | a unit test — a leak does not show in three seconds |

**A measurement against a real database is not a benchmark.** The network and
the server's state dominate the signal. What is measured is the time spent
*inside* Oxyn, not the round-trip time.

## The first place to look

Row-to-batch conversion is an **expected** hot spot, not a hypothesis: drivers
built on row-by-row clients go through every value of every row there. It is
the first place to measure, and the last to optimize without measuring.

## The traps

**The benchmark on too small an input.** A thousand rows fit in the L2 cache:
the benchmark measures the cache, not the algorithm. The benchmark volume must
be of the same order as the real volume — and for Oxyn, the real volume is the
one that does not fit in memory.

**The budget adjusted to make the test pass.** If a budget of
`docs/PERFORMANCE.md` cannot be met, it is amended **by an ADR**, never
silently. A budget moved at every failure no longer measures anything.

**The measurement on a busy machine.** A `cargo build` in the background
invalidates the result, and the noise looks like a signal.

## Verify

```bash
make qualite
```

The before and after numbers go into the commit message. An optimization without
its number will be refused in review — not out of formalism, but because nobody
will be able to challenge it later.

## Reminders

- the current budgets are **decided**, not measured: no campaign has taken
  place yet. The first one must confirm them or amend them by an ADR
  (`docs/PERFORMANCE.md` § status of the numbers);
- `criterion` is at `0.8.2` as of 2026-09-05 — check with
  [`/versions`](versions.md) before adding it.
