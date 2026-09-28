# ADR-0033 — Oxyn switches Codex off in every configuration layer it can read, and leaves to the organization those it cannot read

**Status:** accepted · **Date:** 2026-09-23

**Clarifies:** [ADR-0032](0032-agent-externe-confine-au-lancement.md) on one
point. ADR-0032 switches off by name the MCP servers and plugins declared in
`~/.codex/config.toml`. But Codex also loads them from other layers. The rest
of ADR-0032 remains in force.

## Context

Codex 0.154.0, launched by `codex-acp` 1.12.0, merges its configuration from
nine layers. The merge happens table by table, so that `mcp_servers` and
`plugins` gather the entries of all layers
([RESEARCH-NOTES](../RESEARCH-NOTES.md#the-configuration-layers-codex-loads--re-read-on-2026-09-23),
re-read at tag `rust-v0.154.0`). What Oxyn writes in `CODEX_CONFIG` forms the
`SessionFlags` layer, of rank 7:

| Layer | Rank | Declares servers | Oxyn reads it | Above Oxyn |
|---|---|---|---|---|
| `/etc/codex/config.toml` | 2 | yes | **yes**, since this ADR | no |
| Workspace cloud fragments | 3 | yes | **no**: the server delivers them to Codex | no |
| `$CODEX_HOME/config.toml` | 4 | yes | yes (ADR-0032) | no |
| Project `.codex/` | 6 | yes | not applicable: the agent's directory is new, empty and private | no |
| `/etc/codex/managed_config.toml` | 8 | yes | **yes**, since this ADR | **yes** |
| macOS MDM `com.openai.codex` | 9 | yes | **no** | **yes** |

Each layer Oxyn does not read can declare an MCP server that the agent uses
without asking anything, bypassing the `PolicyGate`. That is the gap ADR-0032
aimed to close.

## Decision

1. **Oxyn reads the three local layers**: system, user and managed
   (`oxyn_ai::external::confine::codex_config_layers`). It keeps only the
   names of `mcp_servers` and `plugins`, and switches them all off with
   `enabled = false`. The rules of ADR-0032 hold for each layer: an
   unreadable, special or larger than 1 MiB file stops the launch. So do more
   than 256 names per table, all layers combined. Finally, a server named
   `oxyn` is refused.
2. **A managed layer that writes `enabled = true` wins over Oxyn.** Codex
   starts anyway, since it is the organization's policy. However, the agent is
   no longer presented as confined: it loses the "Restricted by Oxyn" badge
   and the screen warns about it like an agent Oxyn does not confine
   (`managed_turns_on`).
3. **Oxyn reads neither the MDM layer nor the cloud fragments, and accepts
   it.** Reading the MDM would require launching a process (`defaults`), which
   the repository refuses outside the single launch point, or linking
   `CoreFoundation`. The cloud fragments, for their part, are readable nowhere
   locally. These layers belong to the organization's administrator, not to
   the user, and their servers are those the organization chose. For Codex,
   the screen adds: "except what your organization's managed configuration
   turns on".

## Consequences

* **+** An MCP server declared in `/etc/codex/config.toml` or in the managed
  file is switched off like those of `~/.codex`. The gap of ADR-0032 shrinks to
  the layers that belong to the organization.
* **+** Oxyn no longer presents as confined a Codex that a managed policy
  opens. The badge says what is true.
* **−** A server declared by MDM or by the organization's cloud stays
  available to the agent, **without any indication**, since Oxyn does not see
  it. The screen says so in general, not for that particular server.
* **−** Listing agents now reads two files of `/etc` for each declared Codex,
  on the blocking pool (I-05).
* **−** An `/etc/codex/config.toml` unreadable by the user, with `0600 root`
  permissions for example, prevents Codex from starting from Oxyn, whereas it
  would start in a terminal. This refusal is intended: an unreadable file does
  not allow switching off what it declares.

**Exit cost:** low. `codex_layers.rs` is a module of about two hundred lines,
and the `confined` field already exists on the IPC side.

**Reconsider if** a pinned version of Codex or `codex-acp` offers a switch that
turns off all MCP servers except those of the session. It would replace all of
this reading. Reconsider also if organizations are found to push by MDM
servers their users refuse.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Refuse to launch Codex as soon as a system or managed layer declares a server | It punishes the user for a policy they do not control, without protecting anything more than switching off by name. |
| Only document the gap | Two layers are readable in a few lines. Leaving their servers active under a "Restricted by Oxyn" badge would make the badge lie. |
| Read the MDM with `defaults read com.openai.codex` | It launches a process, which the repository forbids outside the single launch point, because a child inherits the environment (I-03). Linking `CoreFoundation` for a single call would be one more platform dependency for a layer that belongs to the organization anyway. |
| Remove the badge from every Codex | Oxyn does confine everything it can read. Removing it from every Codex would make the warning so frequent that it would no longer signal anything. |
