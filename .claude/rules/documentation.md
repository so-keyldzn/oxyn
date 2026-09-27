---
paths:
  - "docs/**/*.md"
  - "*.md"
  - ".claude/**/*.md"
  - "i18n/**/*.md"
---

# Documentation — conventions

## The four media are not interchangeable

| Medium | Nature | When |
|---|---|---|
| `docs/` | **authoritative** — the truth about the domain | when a code/doc contradiction is a bug |
| `CLAUDE.md` | **map + invariants**, loaded at every session, hence short | when it holds for the whole repository, all the time |
| `.claude/rules/` | **conventions**, loaded through `paths:` | when it only holds for one directory |
| `.claude/hooks/` | **executed**, not read — a refusal, not a reminder | when the violation is silent |

The sorting question: **if Claude ignores it, does it show?** Visible at run
time → one line of rule. Visible only in production, or never → a hook. A fact
about the domain → `docs/`.

## A rule lives in one place

It is the only thing that keeps this foundation from rotting. Other files
**point** to it; they do not copy it. A rule in three copies diverges within two
weeks, and nobody knows anymore which one is authoritative.

In practice: the privacy tier table lives in
[ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md) and nowhere else; the
`PolicyGate` policy lives in
[ADR-0004](../../docs/adr/0004-command-bus.md) and nowhere else.

## An authoritative document is specific and quantified

"Pagination is consistent" is authoritative on nothing. "`ResultBuffer`:
256 MB budget, spill to Arrow IPC, scrolling never re-runs the query" is
authoritative.

An authoritative document describes **what is decided**, not what remains to be
done: remaining work goes into
[IMPLEMENTATION-PLAN](../../docs/IMPLEMENTATION-PLAN.md).

## Every external fact carries its source and date

An undated value is a stale value nobody has spotted yet
([I-12](../../CLAUDE.md#i-12)). External facts live in
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md), not scattered.

## ADR

Template: [.claude/templates/adr.md](../templates/adr.md). Move:
[`/adr`](../commands/adr.md).

**An accepted ADR is not rewritten**: a new one is written that supersedes or
refines it, and says so at the top
([ADR-0009](../../docs/adr/0009-source-dependance-gpui.md) is the example). A
decision without a revision criterion becomes dogma: every ADR carries its
**exit cost** and the **condition that would trigger its reconsideration**.

## Language

The repository is in **English**: code, identifiers, `///`, comments, error
messages, documentation, ADRs, commit messages, pull requests
([ADR-0047](../../docs/adr/0047-english-as-the-repository-language.md)). A code
excerpt in a document stays in English.

**What was written in French.** Every document written in French before
ADR-0047, ADRs 0001 to 0046 included, was translated as it was: a translation
changes no decision — an accepted ADR can be translated, not rewritten. ADR
file names stayed French so that no link breaks.

**English is authoritative. French mirrors** live in `i18n/fr/`, at the path of
their original (`.claude/` becomes `claude/`, so that Claude Code does not load
a mirror as a second rule, command or agent). A mirror starts with:

```markdown
<!-- oxyn-translation source="CLAUDE.md" sha256="<first 12 hex of the source>" -->
```

`make socle` refuses a mirror whose source changed since its translation. After
editing an English file that has a mirror, update the mirror, then its
`sha256` — the error prints the expected value. A contributor who cannot write
French says so in the pull request, and the maintainer updates the mirror. A
mirror is a translation: nothing is ever decided there.

## `CLAUDE.md` has a budget

It is loaded at every session. What concerns only one directory goes down into
a `paths:` rule; what changes goes up into the `SessionStart` hook. Target:
under 200 lines.
