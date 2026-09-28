---
name: memoire-a-la-racine-du-depot
description: The agent memory is written in the repository's /.claude/agent-memory/frontiste/, never under apps/desktop — otherwise prettier sees it and make qualite fails
metadata:
  type: feedback
---

Write the agent memory in `<repository root>/.claude/agent-memory/frontiste/`,
**as an absolute path**, never relative to the working directory.

**Why:** written under `apps/desktop/.claude/…`, it falls within the scope of
`prettier --check .` that `make front` runs **from `apps/desktop`**. Prettier
reformats the Markdown and judges these files badly formatted: `make qualite`
failed on that and nowhere else, so on a symptom unrelated to the code. The team
lead had to move the files and merge the index by hand.

**The root cause, observed on 2026-09-16**: the session instruction of the
`frontiste` agent itself designates
`apps/desktop/.claude/agent-memory/frontiste/` as the memory directory ("This
directory already exists — write to it directly"), because it is resolved from
the working directory. The tooling **recreates this directory, empty**, even
after deletion. Following the instruction to the letter reproduces the mistake.
The fix belongs to the agent's definition, not to this memory.

**How to apply:** at every `Write` in `agent-memory`, **ignore the path
announced by the session instruction** and write under the root. This session's
working directory is `apps/desktop`, not the root — that is precisely what makes
the trap invisible. Two corollaries: the root's `MEMORY.md` index is **shared**
(add a line to it, never overwrite it), and everything that lands under
`apps/desktop/` goes through the front gate, including what is not code.
