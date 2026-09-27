# Codex in Oxyn

This adaptation is local to the repository: [AGENTS.md](../AGENTS.md) is the
entry point and `skills/` holds the project skills. The model settings, the
MCP servers and the subagent profiles are described in
[.codex/README.md](../.codex/README.md). No global setting is installed.

## Usage

Open a new Codex session in Oxyn to load `AGENTS.md`.
Skills can be picked automatically from the request or invoked explicitly,
for example `$oxyn-driver add a driver` or `$oxyn-relire the work in
progress`. If they do not appear in the picker, restart the session in the
repository.

| Claude command | Codex skill |
|---|---|
| `/plan` | [$oxyn-plan](skills/oxyn-plan/SKILL.md) |
| `/implementer` | [$oxyn-implementer](skills/oxyn-implementer/SKILL.md) |
| `/driver` | [$oxyn-driver](skills/oxyn-driver/SKILL.md) |
| `/commande` | [$oxyn-commande](skills/oxyn-commande/SKILL.md) |
| `/adr` | [$oxyn-adr](skills/oxyn-adr/SKILL.md) |
| `/versions` | [$oxyn-versions](skills/oxyn-versions/SKILL.md) |
| `/benchmark` | [$oxyn-benchmark](skills/oxyn-benchmark/SKILL.md) |
| `/relire` | [$oxyn-relire](skills/oxyn-relire/SKILL.md) |
| `/securite` | [$oxyn-securite](skills/oxyn-securite/SKILL.md) |

## Mapping and limits

To audit the whole repository and prepare GitHub issues, read the shared
procedure [`/audit`](../.claude/commands/audit.md). Codex follows its
[multi-agent workflow](../.claude/workflows/audit-multi-agents.md) with the
session's collaboration tools; the JavaScript file next to it requires the
Claude workflow engine. Publishing with `gh` follows the user's request and
does not authorize fixing or committing the product.

| Existing element | Handling in Codex |
|---|---|
| `CLAUDE.md` | Read as required by `AGENTS.md`, invariants kept at their source |
| `.claude/rules/` | Explicit reading by path, before modifying or creating a file |
| `.claude/commands/` | Shared procedures called by the ten skills |
| `.claude/agents/` | Specialty guides read by the profiles of `.codex/agents/`, without transposing the Claude metadata |
| Checklists, templates, workflows | Reused with the syntax adaptations of `AGENTS.md` |
| `SessionStart` | Explicit inspection of Git, the manifests and the plan |
| `PreToolUse` and permissions | No Codex hook; the session's instructions and effective permissions |
| `PostToolUse` formatting | Explicit `cargo fmt --all` after a Rust change |
| `Stop` | Explicit `make qualite` and validation report |
| Version check | Existing script run explicitly by `$oxyn-versions` |

Claude's blocking filter before writes is **not reproduced** here.
The prohibitions stated in `AGENTS.md` are instructions, not a technical
barrier. The hook tests do not prove they run in Codex.
An equivalent blocking check would need an integration specific to the
environment's tools; copying `settings.json` is not enough.

The profiles can be consulted depending on the task:

- Writing: [architecte](../.claude/agents/architecte.md),
  [rustacien](../.claude/agents/rustacien.md),
  [driveriste](../.claude/agents/driveriste.md),
  [frontiste](../.claude/agents/frontiste.md),
  [shadcniste](../.claude/agents/shadcniste.md),
  [ia-workspace](../.claude/agents/ia-workspace.md),
  [documentaliste](../.claude/agents/documentaliste.md),
  [performance](../.claude/agents/performance.md).
- Review: [invariants](../.claude/agents/relecteur-invariants.md),
  [security](../.claude/agents/relecteur-securite.md),
  [boundaries](../.claude/agents/relecteur-frontiere.md),
  [divergence](../.claude/agents/detecteur-divergence.md).

A review request stays a review. The Codex profiles of the reviewers declare
read-only by default, subject to the session's effective permissions.
The Claude guides alone create no technical isolation. The profiles
do not trigger a subagent automatically.

## Maintenance

Change the shared rules and procedures in `.claude/`; keep the skills for
entry points and Codex adaptations. Old notes about the absence of code do not
replace inspecting the repository.

`make socle` also checks the links of `AGENTS.md` and `.agents/**/*.md`.
`make qualite` keeps the shared validation gate (foundation, format, clippy,
tests and documentation). No review command authorizes a commit.

Formats checked in the official documentation on 2026-09-06:
[AGENTS.md instructions](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
and [local skills](https://learn.chatgpt.com/docs/build-skills).
