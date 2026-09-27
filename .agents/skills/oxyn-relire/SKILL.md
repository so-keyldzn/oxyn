---
name: oxyn-relire
description: "Review a change against the invariants and the authoritative documents in the Oxyn project. Use for a matching request in this repository."
---

# Oxyn — relire

Read the [project's Codex instructions](../../../AGENTS.md), then the
[shared relire procedure](../../../.claude/commands/relire.md) and follow
its steps that apply to the request. The procedure stays the single source;
interpret its Claude syntax according to the adaptations of `AGENTS.md`.

Review without modifying the sources: relecteur-invariants and detecteur-divergence profiles, then relecteur-frontiere or relecteur-securite depending on the change. Without an explicit scope, include staged and unstaged changes and the relevant new files. Fixes require a matching request.

Resolve the procedure's links from its own directory. Shell paths are
relative to the Oxyn root. Use the session's tools; Claude metadata grants
no additional permission. Do the reviews locally if no delegation is
requested or available.
