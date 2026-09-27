---
paths:
  - "**/*_tests.rs"
  - "**/tests.rs"
  - "**/benches/**"
  - "**/tests/**"
---

# Tests and benches — conventions

## What is tested here, and what is not

| Test | Do not test |
|---|---|
| The contract: error classification, capabilities, type conversion | that `sqlx` can talk to PostgreSQL |
| What panics today if the input is hostile | accessors |
| The documented loss cases of the type table | the nominal path alone |

A test that cannot fail proves nothing and costs at every run.

## Hostile inputs are the subject, not a bonus

A driver test that only sends well-formed responses does not test
[I-09](../../CLAUDE.md#i-09). The minimal corpus: unknown type, `NULL` on a
`NOT NULL` column, out-of-range integer, invalid encoding, response truncated
mid-stream, object name containing a quote or a semicolon.

The last point is the most neglected: a table named
`"users"; DROP TABLE audit; --` is legal in PostgreSQL, and it is the test that
proves [I-10](../../CLAUDE.md#i-10).

## The two tests a driver does not skip

1. **Cancellation reaches the server.** Checked server-side — the process view,
   not the function's return. A test that checks the future stopped tests
   nothing.
2. **Streaming holds on a volume that does not fit in memory.** With a bound on
   the process memory, otherwise the test passes by accident on a well-equipped
   development machine.

## Interface tests

Two levels, which do not prove the same thing.

**1. Logic, without a DOM.** What a component computes before drawing — window
of visible rows, focus bounds, column width, reduction of an event stream —
lives in a free function and is tested with unit Vitest, without rendering. It
is the most cost-effective level, and the only one that survives a library
change. A component where nothing is testable at this level carries too much
logic in its rendering: it is a signal to split.

**2. Components and interactions: stories.** Every component of
`apps/desktop/src/components/oxyn` has one story per state, rendered in Chromium
with axe in `error` mode ([front.md](front.md)). What deserves a `play` is what
regresses **silently** — the blocking points of
[revue-ui](../checklists/revue-ui.md):

- the confirmation that **names** the connection before a write on `production`,
  and the default button that is not the destructive action
  ([I-02](../../CLAUDE.md#i-02));
- keyboard reachability, visible focus, tab order;
- the **empty** state, visibly distinct from the error;
- the means to cancel present during the "in progress" state.

None of these four regressions breaks a build or turns an existing test red.
That is precisely the criterion that makes them tests.

**The trap**, and it is a nasty one: `toBeVisible` ignores what a scroll area
clips. A row drawn outside the visible area remains "visible" to the test. What
gets asserted are the positions and heights that Oxyn **decides** itself,
measured relative to the container.

One thing that is not an interface test: checking that a component renders. It
can only fail on an exception, and it will stay green the day the component no
longer displays anything.

## Benches

`criterion` for pure code: conversion to `RecordBatch`, parsing, formatting,
schema diff
([PERFORMANCE](../../docs/PERFORMANCE.md#ce-qui-se-mesure-et-comment)).

Two things that are **not** benches:

- a measurement against a real database — the network and the server state
  dominate the signal; what gets measured is the time spent *inside* Oxyn;
- a `criterion` bench on the interface — it measures nothing useful; frame
  budgets are measured with the browser's and the system's instruments.

An optimization comes with its before and after figures, in the commit message
([`/benchmark`](../commands/benchmark.md)).

## What does not go into a test

A real credential, a connection string, a token
([I-03](../../CLAUDE.md#i-03)) — including in a "test" fixture: it will be
committed, and it is often real.
