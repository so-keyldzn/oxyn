# Measurement report — <what was measured>

**Date:** AAAA-MM-JJ · **Commit:** `<hash>` · **Machine:** <model, cores, RAM>

## What was measured, and with what

| | |
|---|---|
| Subject | … |
| Instrument | `criterion` / system instruments / long observation |
| Input volume | … *(of the same order as the real volume; a thousand rows fit in the L2 cache and only measure it)* |
| Machine idle | yes / no *(a `cargo build` in the background invalidates the result)* |

## Results

| Measurement | Before | After | Delta |
|---|---|---|---|
| … | … | … | … |

## Against the budgets

Budgets in `docs/PERFORMANCE.md`.

| Budget | Value | Measured | Held |
|---|---|---|---|
| … | … | … | yes / no |

## Interpretation

What the figure says, and what it does not say. Explicitly name what was **not**
measured: it is what a hurried reader will assume is settled.

## Decision

- [ ] The optimization is worth its complexity — the figure goes into the commit
      message
- [ ] The optimization is not worth its complexity — the clear code stays
- [ ] A budget cannot be held → **write an ADR**, never adjust the budget
      silently

## Reminder

A measurement against a real database is not a bench: the network and the
server state dominate the signal. What gets measured is the time spent *inside*
Oxyn.
