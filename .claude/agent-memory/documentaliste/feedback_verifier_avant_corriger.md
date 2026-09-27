---
name: feedback-verifier-avant-corriger
description: Every documentation fix must be re-checked in the code by myself, and what sets two authoritative documents against each other is reported without being settled
metadata:
  type: feedback
---

When divergence findings are handed to me (review, `detecteur-divergence`
agent, third parties), I **re-check each one in the code** before writing, and
I say so if one is wrong. And I separate two categories:

- **factually stale** → I fix it;
- **two authoritative documents that contradict each other**, or an ADR
  contradicted by the code → I **report it as an open, unsettled question**, I
  do not fix it.

**Why:** handed-over findings do not come from the user and can be wrong —
writing on their sole word would propagate the error into the authoritative
document. And rewriting an ADR to make it match the code destroys the trace of
the decision: that is exactly how ADR-0026 ended up carrying a statement and its
opposite on the same page.

**How to apply:** on every documentation fix task. Open questions go into
`docs/IMPLEMENTATION-PLAN.md`, explicitly marked unsettled, with each side's
argument — not just the finding. An arbitration between authoritative documents
ends with a new ADR, never with a silent edit.

Measurement corollary: a one-off measurement that cannot be automated is
recorded **with the reason** it is not automated. If automating would require an
exception to an invariant rule, it is an arbitration — I leave it open.

See [[verifier-socle-lit-les-exemples-comme-des-liens]].
