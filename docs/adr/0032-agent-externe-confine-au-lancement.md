# ADR-0032 — A known external agent is confined at launch, and has only Oxyn's tools

**Status:** accepted · **Date:** 2026-09-23

**Clarifies:** [ADR-0026](0026-agents-externes-acp.md), on one point:
the ADR assumed that refusing an agent's permission requests was enough to
prevent it from acting on the machine. That is false, and the measurement shows
it. The rest of ADR-0026 remains in force, as does
[ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md).

## Context

ADR-0026 puts every access of an external agent to the machine (files,
commands, network) in `oxyn_ai::external::permission_for`, with a default
refusal. But that refusal only applies to what the agent **asks for**, and an
agent only asks for what its mode forces it to ask for.

Measurement of 2026-09-23 on the pinned adapters, with a client that refuses
everything, like Oxyn, and an instruction asking to run `touch` on a witness
file ([RESEARCH-NOTES](../RESEARCH-NOTES.md#confinement-of-acp-adapters--measurement-of-2026-09-23)):

* **Claude Agent**, in its initial `auto` mode (read from the user's settings),
  runs the command **without any request**. The file is created;
* **Codex**, in its initial `agent` mode, does the same. In `read-only`, it
  still runs without asking everything its sandbox allows: reading everywhere,
  writing to `/tmp`;
* Oxyn's mode selector moreover offered "Bypass permissions" and
  "Full access".

The content of a database is a hostile input ([ia.md](../../.claude/rules/ia.md)).
A table name written as an instruction was therefore enough to get a command
run on the user's machine, behind a database window. That is precisely what
ADR-0026 wanted to prevent.

Two facts make the problem worse:

* in strict mode, an agent also asks for permission before calling **Oxyn's**
  tools. Codex presents it under the `execute` kind, without a tool name;
  `permission_for` refuses it. External agents therefore could not read the
  database, which is the defect the user reported;
* Codex loads the MCP servers, plugins and hooks of the user's `~/.codex`. On
  the measurement machine, that includes a server exposing `execute_sql` on a
  remote database, as well as browser- and computer-control plugins.

## Decision

**An agent whose adapter Oxyn knows is confined at launch, through the switches
that adapter documents. It is then kept in its strictest mode until the end of
the session.** The type `oxyn_ai::external::confine::Confinement` carries this
confinement.

**"Known" means: the pinned and measured version, with exactly the arguments
Oxyn offers** (`presets::pinned_preset_of`). Another version may ignore the
switches. One extra argument may override them. Everything else is treated as
an unknown agent, warning included. Bumping a preset's version therefore
requires redoing the measurement.

**A known agent does not start half confined.** If the confinement cannot be
applied as measured, the agent is not launched
(`ExternalError::Unconfinable`). Three cases trigger it: a Codex configuration
that is present but not fully readable (size, permissions, syntax, non-regular
file), more than 256 entries to switch off, or a user server already named
`oxyn`.

### Claude Agent: Oxyn's tools, and no others

The SDK options go through `_meta.claudeCode.options` of `session/new`:

* `tools: []` removes all built-in tools (shell, files, web);
* `strictMcpConfig: true` keeps only the MCP server declared by Oxyn;
* `settingSources: []` loads neither the permission rules, nor the hooks, nor
  the user's MCP servers;
* `allowDangerouslySkipPermissions: false` removes the `bypassPermissions` mode
  from the catalog;
* `allowedTools: ["mcp__oxyn"]` pre-authorizes Oxyn's tools **on the agent
  side**. Their control is the `PolicyGate`, with `Actor::Agent`
  ([ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md)). A permission
  request that Oxyn would have to refuse would protect nothing more: it would
  only make the database unreadable.

The mode is then set to `default` ("Manual").

### Codex: no shell, no web, `read-only`, personal tools switched off by name

Two environment variables, documented by the adapter, carry the confinement:

* **`INITIAL_AGENT_MODE=read-only`**;
* **`CODEX_CONFIG`**, which contains:
  * `features.shell_tool`, `features.unified_exec`, `features.hooks` and
    `features.apps` set to `false`, and `web_search = "disabled"`;
  * Oxyn's server, **declared in this configuration** rather than through
    ACP, with `default_tools_approval_mode = "approve"`. The token goes through
    `OXYN_TOOL_TOKEN`, and the process exit report redacts it like any
    forwarded variable. The measurement shows that this setting does not apply
    to a server declared through ACP;
  * every MCP server and every plugin of the user's `config.toml`, with
    `enabled = false`. Only **the names** are read, and the file is bounded to
    1 MiB. No value is kept, and some carry tokens
    ([I-03](../../CLAUDE.md#i-03)). The file read is the one that **the Codex
    process** will load: the `CODEX_HOME` declared for the agent, otherwise
    the `.codex` of the `HOME` the agent receives. Never Oxyn's `CODEX_HOME`,
    which the child does not receive.

The mode is then set again to `read-only`.

### The mode holds for the whole session

* The mode is set **after** `session/new` and **before** the first question.
  If the agent refuses to take it, it reads no question, and the session
  fails with `ExternalError::Unconfined`.
* `AgentSettings::modes_locked` removes the modes, as well as options of
  category `mode`, from everything displayed or accepted. The selector
  disappears, and a mode change is refused by `check` before anything is sent.
* `SwitchMode`, which `permission_for` grants to an unconfined agent, is
  refused to a confined agent.
* If the agent announces a mode other than the one set, `ModeWatch` records it.
  The current turn is cancelled immediately (`session/cancel`) and ends with
  `Unconfined`; any following question is refused. There is no going back: an
  agent that has changed mode once is no longer presumed stable.
* Two weak gaps are accepted. A mode announcement arriving between the response
  to `session/set_mode` and the arming is not seen; the agent has not yet
  received any question at that point. A mode option that the adapter does not
  classify in the `mode` category stays offered; neither of the two measured
  adapters declares one.

### An unknown agent stays usable, and the screen says so

Oxyn does not know the switches of a hand-declared agent. It therefore launches
it as before, but the declaration form and the panel say that **Oxyn cannot
prevent it from acting on its own on the machine**. The user is the one who
designated this program, and the declaration already requires a double
confirmation (ADR-0026).

## Consequences

* **+** For Claude Agent, confinement is structural: the agent has no machine
  tool, hence nothing to ask for. It no longer depends on a refusal that must
  be solicited.
* **+** External agents finally read the database through Oxyn's tools. That
  was the very purpose of ADR-0030, and the refusals of `permission_for` made
  it impossible.
* **+** Choosing a dangerous mode is no longer offered on screen.
* **−** **Codex stays confined by blocklist.** A plugin active by default and
  absent from the user's `config.toml` escapes confinement. Codex's reference
  documents no way to switch off all plugins, nor to ignore that file. Codex
  also keeps file editing: it asks for permission, and Oxyn refuses it.
* **−** Oxyn reads a configuration file of another program. It keeps only
  names from it, but it now depends on its format.
* **−** Confinement relies on options **specific to each adapter**
  (`_meta.claudeCode`, `CODEX_CONFIG`), outside the ACP protocol. An adapter
  version bump may ignore them without breaking anything visible. Every bump
  must therefore redo the RESEARCH-NOTES measurement.
* **−** The user loses in Oxyn their personal Claude Code settings: default
  model, instructions, MCP servers. That is intended, but visible.
* **−** A hand-declared agent is not confined. Only a warning goes with it.

**Exit cost:** low. Everything lives in `confine.rs`, `ModeWatch` and the
`modes_locked` field; removing confinement amounts to returning `None`. What
would be expensive is removing it **without** also removing external agents.

**Reconsider if** ACP standardizes a way for a client to restrict an agent's
tools or set its mode, which would make adapter-specific options unnecessary.
Also if Codex documents a switch that turns off the user's MCP servers and
plugins wholesale: the blocklist would become unnecessary.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep only the refusal of `permission_for` | Measured: the agent asked for nothing and the command was run. A refusal that is never solicited protects nothing |
| Grant permission requests whose title designates an Oxyn tool | With Codex, the request carries neither a title nor a tool name, only an `exec-…` identifier. And a title is text produced by the agent: granting on its content is granting on what the agent says |
| Launch Codex with a private `CODEX_HOME` | Its login token lives in that directory. Copying it would make Oxyn hold a user secret, which ADR-0026 refuses. A symbolic link would be replaced by a file at the first token rotation |
| Suspend Codex as long as it cannot be fully confined | Rejected by the user in favor of the blocklist, whose gap is written here |
| Refuse any agent Oxyn cannot confine | Rejected by the user: they are the one who designates the program, and an explicit warning says what Oxyn cannot guarantee |
