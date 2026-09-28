---
name: oxyn-versions
description: "Re-check external versions against the registry and date the result in the Oxyn project. Use for a matching request in this repository."
---

# Oxyn — versions

Read the [project's Codex instructions](../../../AGENTS.md), then the
[shared versions procedure](../../../.claude/commands/versions.md) and follow
its steps that apply to the request. The procedure stays the single source;
interpret its Claude syntax according to the adaptations of `AGENTS.md`.

Run python3 .claude/hooks/verifier_versions.py explicitly from the root. Check the official sources and date the findings. A request to check alone does not authorize upgrading dependencies.

Resolve the procedure's links from its own directory. Shell paths are
relative to the Oxyn root. Use the session's tools; Claude metadata grants
no additional permission. Do the reviews locally if no delegation is
requested or available.
