# ADR-0047 — English is the repository language; French lives in authoritative-English mirrors

**Status:** accepted · **Date:** 2026-09-27 · **Deciders:** Nicolas Boromée

**Amends:** the language rule of [CLAUDE.md](../../CLAUDE.md), which until now
put code in English and documentation, ADRs and commit messages in French.

## Context

Oxyn is becoming a public repository open to external contributors
([CONTRIBUTING.md](../../CONTRIBUTING.md), [CLA.md](../../CLA.md)). On
2026-09-27, what a contributor needs to read before writing code was in
French:

* `CLAUDE.md` and its thirteen invariants, `AGENTS.md`, `README.md`;
* `.claude/`: 7 rules, 12 commands, 12 agents, 4 checklists, 3 templates,
  about 2,600 lines — which is also what a contributor's own Claude session
  receives;
* `docs/`: about 16,400 lines, 46 ADRs;
* the commit format, enforced by `.claude/hooks/message_commit.py`.

Only the code, `CONTRIBUTING.md` and the CLA were in English. An English-speaking
developer could sign the CLA but could not read the invariants they are asked
to respect. Violating an invariant is silent by construction: an invariant
nobody can read is not held.

The maintainer works in French and wants the documentation to remain readable
in French.

## Decision

1. **The repository language is English**: code, identifiers, comments, error
   messages, documentation, ADRs, commit messages, pull requests, issues. New
   documents and ADRs are written in English.
2. **Everything is translated now**: `CLAUDE.md`, `AGENTS.md`, `README.md`,
   every document of `docs/`, ADRs 0001 to 0046, everything in `.claude/` that
   is prose (rules, commands, agents, checklists, templates, workflows, audit
   reports), the messages of the hooks, the pull request and issue templates.
3. **Translating an accepted ADR decides nothing.** ADRs 0001 to 0046 are
   translated as they are: same decision, same status, same date. File names
   stay as they were, so that no link breaks.
4. **French mirrors, English authoritative.** Every translated document has a
   French mirror at `i18n/fr/<path>`, where `.claude/` becomes `claude/`. The
   mirror starts with `<!-- oxyn-translation source="<path>" sha256="<12 hex>" -->`,
   the fingerprint of the English source it translates. `verifier_socle.py`,
   hence `make socle` and `make qualite`, refuses a mirror whose source no
   longer has that fingerprint, and prints the expected one. A mirror decides
   nothing: in case of a discrepancy, English wins.
5. **Commit messages** keep the Conventional Commits format enforced by the hook
   (`type(scope): subject`, imperative, lowercase initial, no final period), in
   English.

The language of the conversation between the maintainer and Claude is a personal
setting, not a repository rule: it leaves `CLAUDE.md`.

## Consequences

**Positive.**

* An English-speaking contributor reads the invariants, rules and procedures
  before writing code; so does their Claude session.
* The history, the pull requests and the issues become readable by everyone.
* The mirrors cannot drift silently: a stale mirror fails the quality gate.

**Negative.**

* Two copies of each foundation document. Every edit of an English document
  that has a mirror requires a French update in the same pull request. A
  contributor who does not write French cannot make the gate pass alone: they
  say so in the pull request and the maintainer updates the mirror before
  merging.
* Every heading anchor changed with the translation: links into the English
  documents use English anchors, links inside the mirrors keep French ones and
  point to the mirrors. ADR file names stay French.
* The history is bilingual: French before 2026-09-27, English after.
* The Python identifiers of the hooks, the names of `make` targets, scripts,
  commands and agents (`make qualite`, `/implementer`, `rustacien`) stay in
  French. Renaming them changes everyone's habits and the CI; it is a separate
  decision.

**Exit cost.** Low: deleting `i18n/` and the check in `verifier_socle.py`
leaves an English-only repository; going back to French means translating
back the foundation.

**Reconsider if** keeping the mirrors up to date regularly blocks pull requests
from non-French-speaking contributors, or if the mirrors stop being read — the
check would then only be a cost.

## Rejected alternatives

* **Only an English entry point** (README, templates, a guide summarizing the
  invariants). Two versions of the invariants, one of them not checked: they
  diverge within weeks, and the contributor reads the stale one.
* **Translate `docs/` progressively, before each substantial edit.** An
  English-speaking contributor would keep reading `docs/SECURITY.md` and
  `docs/DRIVER-CONTRACT.md` — the documents most invariants point to — in
  French for months, and the repository would stay in two languages for as
  long.
* **Bilingual files side by side (`rust.fr.md`)**. Claude Code loads every
  `*.md` of `.claude/rules`, `.claude/commands` and `.claude/agents`: a French
  file next to the English one would load a second rule, command or agent.
* **French authoritative, English mirror.** The contributors the change is for
  would read a translation, and could not correct the version that counts.
