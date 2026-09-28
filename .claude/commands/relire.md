---
description: Review a change against the invariants and the authoritative documents
argument-hint: "[path or commit range, default: the work in progress]"
allowed-tools: Bash, Read, Grep, Glob, Agent
---

Purpose: review **$ARGUMENTS** (by default, the uncommitted work).

## The change

```!
git status --short
git diff --stat HEAD
```

## How to review

Delegate to the review agents rather than reading everything here — they are
read-only, and that is what makes their verdict credible:

| Agent | What it looks for |
|---|---|
| `relecteur-invariants` | the thirteen invariants of `CLAUDE.md` |
| `relecteur-securite` | secrets, `unsafe`, input surface, AI boundary |
| `detecteur-divergence` | what the code does and `docs/` says otherwise |

Run them in parallel when the change touches several boundaries.

## The order of severity

1. **Invariant broken** — blocking, no discussion.
2. **Code/documentation divergence** — blocking: it is a bug, by definition
   (`CLAUDE.md` § Documentation is authoritative). Fix the code, or fix the
   document, but do not leave them diverging.
3. **Contract not honored** — a guarantee of `docs/DRIVER-CONTRACT.md` missing.
4. **Budget exceeded** without a measurement justifying the gap.
5. **The rest** — readability, naming, duplication.

## What must be reported even if "it works"

- a second path to a driver, even in a test;
- a `#[derive(Debug)]` on a type carrying a secret;
- an `unwrap()` on a path reachable from a server response;
- an identifier concatenated into SQL composed by Oxyn;
- an ambiguous error treated as transient;
- a `TODO` without a date, an abstraction with a single caller, a catch-all
  module.

These points produce no error today. That is exactly why they are reviewed:
nobody else will see them.

## The shape of the report

By decreasing severity. For each point: **the file and line**, **the invariant
or document broken**, **the concrete failure scenario** — not the rule
recited — and **the fix**.

**If there is nothing to report, say so in one sentence.** Do not invent
remarks to justify the run: a report that always finds something ends up no
longer being read.

## Verify

```bash
make qualite
```

A review does not replace the quality gate, and the reverse is true too:
`clippy` sees none of the invariants above.
