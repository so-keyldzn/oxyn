---
description: Implement a change while honoring the invariants
argument-hint: "<what to implement>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch
---

Purpose: implement **$ARGUMENTS**.

## Before writing

If the move has its own command, use it rather than this one — it loads the
precise contract:

| Move | Command |
|---|---|
| Database driver | [`/driver`](driver.md) |
| Command bus command | [`/commande`](commande.md) |
| Interface screen or component | [`/ecran`](ecran.md) |
| Structuring decision | [`/adr`](adr.md) |

Otherwise: reread the authoritative documents of the boundary touched
(`docs/README.md`), and the `.claude/rules/` rule of the target directory —
**especially if the file does not exist yet**, because a `paths:` rule only
loads when an existing file is read.

## The rule that governs the others

**Nothing bypasses the command bus** ([I-01](../../CLAUDE.md#i-01)). No
temporary shortcut, no "until the command exists". A second execution path
never goes away, and it is the one the AI will take.

## Constructs to use / never

| Use | Never | Why |
|---|---|---|
| `?`, `let … else` | `unwrap()`, `expect()` on a network path | [I-09](../../CLAUDE.md#i-09): the panic kills the application and the unsaved work |
| `u32::try_from(n)?` | `n as u32` | truncates silently: an identifier of 5 billion becomes 705,032,704 |
| Manual `impl fmt::Debug` on a secret carrier | `#[derive(Debug)]` | [I-03](../../CLAUDE.md#i-03): a leak invisible in review |
| A module named after its subject | `utils`, `common`, `helpers` | universal coupling point |
| `TODO(2026-09-05): what unblocks it` | a bare `TODO` | an undated TODO is never reread |

## Cross-cutting traps

**The abstraction for a single caller.** A trait with one implementation that
is not a boundary is an indirection, not a decoupling: it makes the code harder
to read without making anything replaceable.

**Optimization without measurement.** Clear code is not replaced by fast code
without the before and after numbers (`docs/PERFORMANCE.md`). The complexity is
paid up front, the gain is assumed.

**Dead code "just in case".** Git remembers it. Commented-out code will be read
by someone who will believe it matters.

**The comment that repeats the code.** A comment says *why*. What the code
does, the code says — and it stays true after the next change.

## Verify

```bash
make qualite
```

**Nothing is done without this command.** Neither "it compiles" nor "the test
passes": the gate includes formatting, clippy with `-D warnings`, the tests and
the documentation.

Then `.claude/checklists/fin-de-tache.md`.

## Reminders

- an external version is never written from memory: [`/versions`](versions.md);
- code, identifiers, comments, commits and documentation are in **English**
  ([ADR-0047](../../docs/adr/0047-english-as-the-repository-language.md));
- if the code contradicts a document of `docs/`, it is a bug: report it, do not
  settle it alone.
