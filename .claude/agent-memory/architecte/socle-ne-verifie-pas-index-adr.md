---
name: socle-ne-verifie-pas-index-adr
description: make socle passes without a warning when an ADR is missing from docs/README.md, contrary to what .claude/commands/adr.md states
metadata:
  type: feedback
---

`make socle` does not check that every file of `docs/adr/` appears in the
`docs/README.md` index. `.claude/verifier_socle.py` only reads CLAUDE.md,
AGENTS.md and the root README.md for this subject (observed on 2026-09-25: an
ADR-0042 missing from the index gave "Socle cohérent (0 avertissement)").

**Why:** `.claude/commands/adr.md` says this check "catches ADRs missing from the
index"; relying on it leaves an ADR impossible to find.

**How to apply:** after writing an ADR, check the index by hand
(`grep NNNN docs/README.md`) or report it to the caller if they kept the wiring
for themselves. Re-check the script before citing this memory: it may have been
fixed.
