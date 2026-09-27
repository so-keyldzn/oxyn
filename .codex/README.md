# The project's Codex configuration

[config.toml](config.toml) sets the model, the reasoning effort and the
extended context for Oxyn. The checked limits and the choice of the compaction
threshold are documented in
[RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--extended-context).

Restart Codex from this repository and open a new conversation to load these
settings. The project must be trusted in Codex for its local configuration to
load. Launch options or settings imposed by the client can take precedence;
check the effective window in the new session. The file does not change a
conversation that is already open.

The domain instructions stay in [AGENTS.md](../AGENTS.md). Permissions,
credentials and connections stay managed by the session environment.
This configuration does not replace the repository's end-of-task checks.

## Settings chosen

- High reasoning and extended context for complex Rust tasks.
- Compaction threshold kept at its current local value; see the figures
  in [RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--extended-context).
- Live web search to check upstream versions and documentation.
- Terminal bar: model and reasoning, remaining context, Git branch.
  This display setting concerns the CLI, not the application's interface.

## Prompt cache

The cache is managed by the service. There is no `prompt_cache` key to add
to the documented Codex file. The API options `prompt_cache_key` and
`prompt_cache_retention` are not top-level options of this file.
The sources and the date of the check are in
[RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--extended-context).

To favor reuse, keep instructions stable and continue a task in its own
conversation. The cache does not guarantee a hit on every request and does not
enlarge the context window. The `cached` mode of web search refers to a search
index, not the prompt cache.

## Specialty agents

The eleven files of [agents/](agents/) define the local roles: architecte,
rustacien, driveriste, interfacier, ia-workspace, documentaliste, performance,
relecteur-invariants, relecteur-frontiere, relecteur-securite and
detecteur-divergence. Each profile points to the matching guide in
[.claude/agents/](../.claude/agents/) and applies the adaptations of AGENTS.md.
The domain rules stay at their source, not copied into the TOML profiles.

Codex discovers these files automatically in a trusted project; no
per-role registration table is needed in `config.toml`. The model and the
effort are not overridden in the profiles. The project limits simultaneous
subagents to three, in addition to the main agent. Their presence does not
authorize automatic delegation: follow AGENTS.md and the request.

The four review roles declare `sandbox_mode = "read-only"` and forbid fixes in
their instructions. Settings imposed by the session can nevertheless take
precedence over this default; check the effective permissions before treating
this read-only mode as a technical barrier.

## Figma

Two MCP servers are declared: `figma` for the remote service and
`figma-desktop` for the local application. They are optional at startup
(`required = false`). No credential is stored in these files.
Declaring them proves neither their availability nor an authentication.

If the remote service asks for a sign-in, run `codex mcp login figma`
from the repository. For the local server, open Figma Desktop with its MCP
server enabled. Use the server that matches the working context.

The formats and their sources are dated in
[RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--local-agents-and-mcp).

## Verification

From the root, `python3 .codex/verifier.py` checks the TOML files, the
referenced guides and the links of this directory. `make socle` checks the
shared foundation; `make qualite` adds the Rust checks. None of these checks
calls Figma or starts a subagent.

The local verifier checks the directory's structure; it does not replace the
client's parser. To also check the configuration keys with the installed CLI:

```bash
codex --strict-config app-server --stdio </dev/null
```
