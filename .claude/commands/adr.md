---
description: Write an ADR for a decision that is expensive to undo
argument-hint: "<the decision to settle>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch
---

Purpose: write the ADR that settles **$ARGUMENTS**.

## Does this deserve an ADR?

Yes if **undoing the decision would cost more than a day of work**, or if
someone will ask "why this choice" a year from now.

No if the choice is local, reversible, or already settled by an existing ADR. An
ADR for a choice without consequence dilutes the others: the directory is worth
something because it can be read in full.

## Before writing

1. `ls docs/adr/` — an existing ADR may already address the question, or
   constrain it.
2. `docs/README.md` — the index must be updated in the same commit.
3. The template: `.claude/templates/adr.md`.

The number is the next one in sequence, never reusing a freed number.

**A new ADR is written in English**
([ADR-0047](../../docs/adr/0047-english-as-the-repository-language.md)). ADRs
0001 to 0046 stay in French as they are: citing them is enough, they are not
translated as part of another change.

## The rule that governs the others

**An accepted ADR is not rewritten.** A new one is written that supersedes or
refines it, and says so at the top — `docs/adr/0009-source-dependance-gpui.md`
is the example: it refines ADR-0001 on one point and leaves the rest in force.

Rewriting an ADR erases the reason the old decision looked good. That is
precisely the information someone will look for later, when the same question
comes back under another name.

## What makes a good ADR / a bad one

| Good | Bad |
|---|---|
| *Context* quantified and sourced | an essay on the state of the art |
| *Decision* in the present tense, with the concrete names of types and crates | "we are considering using" |
| **Negative** consequences written honestly | a list of advantages |
| **Exit cost** assessed | nothing on reversibility |
| Explicit **reconsideration condition** | a decision without a review criterion, hence a dogma |
| *Rejected alternatives*, each with the reason for rejection | "we chose X" without the others |

The exit cost and the reconsideration condition are what distinguish an ADR
from an after-the-fact justification. Both are written when the decision is
made — afterwards, nobody will know any more what made it revisable.

## External facts

Every version and every limit cited is **checked against the registry and
dated** ([I-12](../../CLAUDE.md#i-12)), and recorded in `docs/RESEARCH-NOTES.md`
in the same commit. An ADR that relies on a number from memory builds a
decision on sand.

## Verify

```bash
make socle
```

This check catches dead links and ADRs missing from the index — the two ways an
ADR ends up written but impossible to find.

## Reminders

- status `proposed` until the first code commit that implements it
  (convention of `docs/README.md`);
- an ADR describes what **is decided**; the remaining work goes into
  `docs/IMPLEMENTATION-PLAN.md`;
- if the ADR contradicts an authoritative document, that document must be
  fixed in the same commit — not left to diverge.
