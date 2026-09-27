---
description: Prepare an implementation plan before writing code
argument-hint: "<what to build>"
allowed-tools: Bash, Read, Grep, Glob, WebFetch, Agent
---

Purpose: plan **$ARGUMENTS**. Write nothing in `crates/` during this command.

## Before planning

1. `docs/IMPLEMENTATION-PLAN.md` — which phase does this fall into, and has the
   previous phase passed its exit gate?
2. The authoritative documents concerned by the boundary touched
   (`docs/README.md` gives the mapping).
3. `ls docs/adr/` — does an existing decision already constrain this work?

## What the plan must settle

| Question | Why it comes before the code |
|---|---|
| Which **crate**? | the direction of dependencies forbids some answers; finding out while coding costs a move |
| Is a new bus **command** needed? | a feature starts with a command, not with a view |
| Which **invariants** are at stake? | name them now, not discover them in review |
| What is **not** settled? | an unsettled point is resolved by an ADR, not along the way during implementation |
| How will we know it is **done**? | without a written criterion, "done" means "I have no more ideas" |

## What must come up as a decision, not as a detail

If the plan touches one of these points, it stops and proposes an ADR
([`/adr`](adr.md)):

- the direction of dependencies between crates;
- what crosses an external boundary
  (`docs/ARCHITECTURE.md` § the external boundaries);
- a persisted format ([I-11](../../CLAUDE.md#i-11));
- a new direct dependency that commits the architecture;
- a budget of `docs/PERFORMANCE.md` that cannot be met.

## The trap

**The plan that describes files instead of decisions.** "Create `mod.rs`, add a
struct, write a test" is not a plan: it is a list of moves that assumes
everything is already settled. The plan is there to find what is not.

## Deliverable

A short note: the target crate, the bus commands concerned, the invariants at
stake, the unsettled points, the completion criterion. No code.

For a broad exploration of the repository, delegate to an agent rather than
filling the context with reads — but the plan itself is decided here.
