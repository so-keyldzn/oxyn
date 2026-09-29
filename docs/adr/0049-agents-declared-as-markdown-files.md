# ADR-0049 — An agent is a Markdown file with a YAML front matter, targeted by dialect, filled only from a closed list of variables

**Status:** proposed · **Date:** 2026-09-29 · **Deciders:** Nicolas Boromée

**Refines:** [ARCHITECTURE §7.3](../ARCHITECTURE.md#73-agent-runtime), on one
point: the form an `AgentSpec` takes on disk. That agents are configurations,
not implementations, stays as it is.

## Context

ARCHITECTURE §7.3 says an agent is a configuration: an `AgentSpec`, with a
system prompt, a subset of tools, a context policy and a turn limit. It says
that adding one needs no Rust code. The code does not follow it yet:

* the two shipped agents, `sql_agent` and `schema_agent`
  (`crates/oxyn-ai/src/builtin.rs`), are **Rust functions** whose prompts are
  `concat!` literals of about 30 lines each. Editing one word of a prompt
  means a Rust change, a recompilation and a review of a `.rs` file;
* the conversation always uses `sql_agent()`, hard-coded
  (`crates/oxyn-desktop/src/backend/ai/conversation.rs`). `schema_agent` is
  declared, tested, and never offered;
* `AgentSpec` has no field saying which databases an agent is written for.
  An agent that knows PostgreSQL (`EXPLAIN (ANALYZE, BUFFERS)`,
  `pg_stat_statements`, `LATERAL`) and one that knows SQLite
  (`EXPLAIN QUERY PLAN`, no `ALTER COLUMN`) can only be the same agent;
* seven agents of the vision are still to be written
  (`REMAINING_AGENTS`), and more per-database agents are asked for. Written
  as Rust functions, each is a code change for what is text.

`AgentSpec` is already `Serialize + Deserialize` and already validated as
untrusted input (`AgentSpec::validate`: unknown tool refused, turns capped
at `MAX_TURNS_CEILING` = 64). What is missing is a file format, a targeting
field, and a place to read files from.

## Decision

### 1. The format: Markdown with a YAML front matter

An agent is one UTF-8 file, `<name>.md`:

```markdown
---
id: 0199a3c0-0000-7000-8000-000000000001
name: SQL
description: Writes, fixes and explains queries on the open connection.
applies_to: []
tools: [execute_query, describe_schema, request_sample]
max_turns: 8
context:
  max_relations: 30
---
You help a data professional write and fix queries against the
{{dialect}} database they have open. …
```

* The **front matter** carries every field of `AgentSpec` except the
  prompt, under the same names; `tools` is `allowed_tools`. It is parsed
  with `serde-saphyr` 1.3.0 (MIT OR Apache-2.0, MSRV 1.89, checked on
  2026-09-29 — [RESEARCH-NOTES](../RESEARCH-NOTES.md#yaml-parsing-for-agent-files--checked-on-2026-09-29)),
  with `deny_unknown_fields` on the front-matter struct: a misspelled key is
  an error, not a silently ignored setting.
* The **body**, after the closing `---`, is the system prompt, as is. Oxyn
  never renders it as HTML; Markdown is only the author's convenience and
  the model's.
* The parser is configured strict, and the configuration is part of the
  decision: `Budget { max_anchors: 0, max_aliases: 0, .. }`,
  `merge_keys: MergeKeyPolicy::Error`,
  `duplicate_keys: DuplicateKeyPolicy::Error`, `strict_booleans: true`,
  `reject_unsupported_tags: true`, and the crate's `include` feature off.
  An agent file has no use for references between nodes, and each of these
  is either a resource-exhaustion lever or a way for a key to mean
  something other than what it reads.
* A file is refused, with the file name and the line in the message, when:
  it is over **64 KiB** (checked before parsing: `serde-saphyr`'s own size
  cap applies to readers, not to strings); the front matter is missing or is
  not the first thing in the file; the YAML breaks one of the settings
  above; `validate()` fails; the prompt names an unknown variable (§ 3);
  `applies_to` names an unknown dialect (§ 2).

### 2. Targeting: `applies_to`

`applies_to` is a list of `SqlDialect` names as `SqlDialect::as_str` writes
them (`postgres`, `sqlite`, `mysql`, `duckdb`…). An empty list means every
connection. An agent is **offered** for a connection only if the list is
empty or contains the connection's dialect. The agent picker shows the
offered agents; the default is the first offered one, in the order of § 4.

Targeting narrows what is offered. It grants nothing: the tools, the
`PolicyGate` and the connection's tier are the same whichever agent runs.

### 3. The closed list of variables

The prompt may contain `{{name}}` placeholders, and only these:

| Variable | Value | Source |
|---|---|---|
| `{{dialect}}` | `SqlDialect::as_str` of the connection | the connection's driver |
| `{{driver}}` | the driver's name | the driver |
| `{{environment}}` | `local`, `development`, `staging` or `production` | the connection's marking |

All three are values Oxyn holds, from a closed set; none is written by the
server or the database. A placeholder outside this list makes the file
**invalid**; it is never replaced by an empty string. The list is extended
only by a new ADR, and only with a value that is neither database content,
nor a server response, nor a secret.

**Database content is never a variable**: no `{{tables}}`, `{{schema}}`,
`{{comments}}`, `{{sample}}`. The structure and the samples reach the model
only through `ContextBuilder`, which applies the connection's tier and
fences what the database wrote ([I-04](../../CLAUDE.md#i-04),
[AI-PROVIDERS](../AI-PROVIDERS.md)). A template variable would be a second,
unfenced path into the prompt, and a column comment would arrive as an
instruction of the system prompt. The server's product and version are not
a variable either, for the same reason: they are a server response, and
`ContextBuilder` already sends them, bounded, when the agent's
`ContextPolicy` has `include_server_info`.

### 4. Where agents come from

1. **Shipped agents** live in `crates/oxyn-ai/agents/*.md` and are embedded
   with `include_str!`. They are parsed and validated by a test at build
   time (`shipped_agents_are_valid`, which already exists), so a broken
   shipped file never reaches a user. `sql_agent()` and `schema_agent()`
   become thin readers of their file; their `id` stays the same, since the
   audit log records it.
2. **User agents** — second step, same format — live in `agents/` of the
   data directory (`ProjectDirs::data_dir()`, next to the local store). They
   are read at launch and when the user reloads them; an invalid file is
   listed with its error and not offered, it never blocks the others. Their
   `id` must not collide with a shipped one: a collision is refused, so a
   file cannot take the audit identity of a shipped agent.
3. **Plugin agents** (phase 4, [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md))
   use the same format and the same validation; the manifest carries the
   file.

Order in the picker: shipped, then user, then plugin, each by `name`.

### 5. What a file can never carry

Unchanged from `spec.rs`, and now enforced by `deny_unknown_fields`: no
connection or session, no privacy tier, no endpoint, no key, no model
choice. `tools` restricts the registry; it never adds to it.

### 6. One conversation, one agent

* The picker sits in the header of the assistant panel. The agent is chosen
  **per conversation**: choosing another agent opens a **new conversation**;
  the current one stays in the history, unchanged. A conversation's system
  prompt therefore never changes under it.
* The conversation remembers its agent: `ai_conversations` gains a nullable
  `agent_id` column (one `ALTER TABLE … ADD COLUMN` migration of
  `oxyn-store`). A conversation written before it reads as the SQL agent,
  which is what it ran with.
* Resumed, a conversation runs with its recorded agent, rendered again from
  the file with the connection's current variables. If that agent no longer
  exists — a user file deleted or now invalid —, the conversation reopens
  with the SQL agent, and the panel **says so**, naming the missing agent.
  It never silently runs under another prompt than the one it shows.

### 7. External agents receive the agent's prompt too

The chosen agent applies to both destinations, provider and external agent
([ADR-0026](0026-agents-externes-acp.md)):

* **Provider:** the rendered prompt is the system message, as today.
* **External agent:** ACP has no system message; the agent has its own. The
  rendered prompt is placed in the **opening** text of the session, by
  `AgentPrompt::with_schema` — the only constructor that opens one —, as an
  Oxyn-written block before the schema, never mixed with it. Follow-up
  questions do not repeat it: the process remembers
  ([ADR-0027](0027-porte-unique-pour-les-deux-destinations.md)). No new
  public constructor of `AgentPrompt` is added.
* `tools` restricts the **MCP tools** Oxyn exposes to that session
  ([ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md)) exactly as it
  restricts the provider's tool list. `context` sets the `ContextPolicy` of
  the opening schema. `max_turns` does not apply: an external agent runs its
  own loop, and Oxyn bounds it by its existing budgets, not by this field.
* The prompt reaches the external agent as **instructions in a user
  message**, below the agent's own system prompt. It is weaker than on a
  provider, and the picker cannot promise more: the external agent's
  confinement ([ADR-0032](0032-agent-externe-confine-au-lancement.md)) and
  the `PolicyGate` stay the guarantees, not the prompt.

## Consequences

* **+** Writing or tuning an agent is editing text: no Rust, no recompile
  for user agents, a one-file diff for shipped ones.
* **+** One agent per database becomes one file per database, offered only
  where it applies.
* **+** A user agent is readable and portable without Oxyn
  ([I-11](../../CLAUDE.md#i-11)).
* **+** The seven remaining agents and plugin agents land in the format
  that already exists, with the same validation.
* **−** One more dependency, `serde-saphyr`, and YAML's surface: implicit
  typing (by default `serde-saphyr` reads `no`, `off` as `false` into a
  boolean field — hence `strict_booleans`), indentation errors.
  `deny_unknown_fields` and typed fields catch most of it; the rest shows as
  a validation error.
* **−** User agents are a **new input surface**
  ([SECURITY](../SECURITY.md), "Workspace files"): a file written by someone
  else can carry a prompt that asks the model to misbehave. It cannot
  obtain a tool, a tier or a write the `PolicyGate` would refuse, but it can
  make the model insistent. The picker therefore marks user agents as such,
  and the step that reads them needs a security review before it merges.
* **−** A prompt error in a shipped file is found by a test, not by the
  compiler: the `concat!` literals were at least type-checked as strings.
* **−** One more persisted column, `ai_conversations.agent_id`: a
  conversation now depends on a file that can disappear, hence the fallback
  of § 6.
* **−** On an external agent, the agent's prompt is advice the external
  agent may weigh against its own system prompt. The same `.md` can behave
  more loosely there than on a provider.
* **−** Three variables will feel short. Every request for a fourth one has
  to go through an ADR, on purpose.

**Exit cost:** low for shipped agents — about a day to put the prompts back
into Rust literals; the `AgentSpec` type does not change. Higher once user
agents exist: their files are the user's, and dropping the format would
need a converter; the `agent_id` column stays, readable, whatever the
format. What bounds it: the front matter is `AgentSpec`'s own
serde form, so any other serde format (TOML, JSON) reads the same data.

**Reconsider if** a provider needs per-agent settings that do not fit a
flat front matter (structured outputs, tool schemas written by the agent
author), or if user agents turn out to be used mostly to work around the
`PolicyGate`'s refusals — which would argue for signing them instead of
reading them freely.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep agents as Rust functions | Contradicts ARCHITECTURE §7.3; every prompt change is a code change, and per-database agents multiply that. |
| TOML front matter (`+++`), with the `toml` crate already in the workspace | No new dependency, and the prompt is the body either way; but YAML front matter is the convention authors of agents and skills already know, and a file in another convention is one more thing to learn for no gain in what it can say. Kept as the fallback if `serde-saphyr` is dropped: the data is the same. |
| A full template engine (`minijinja`, `tera`) | Conditions and loops in a prompt make it untestable file by file, and an engine that can read any value invites passing it database content — the path § 3 forbids. |
| Open variables filled from the catalog (`{{tables}}`) | Bypasses `ContextBuilder`, hence the tier and the fencing (I-04); a comment in the database becomes an instruction of the system prompt. |
| A `{{server_version}}` variable | A server response in the system prompt, outside the fencing; the context already carries it, bounded (`include_server_info`). |
| One JSON file per agent | Readable without Oxyn, but a 30-line prompt as one escaped JSON string is unreadable and unreviewable. |
| Change the agent inside a running conversation | The system prompt would change under an existing history, and a turn could no longer be traced to the prompt that produced it. |
| Do not persist the agent of a conversation | A resumed conversation would silently run under whatever agent is selected, with a history written under another prompt. |
| Agents for providers only | Leaves the external-agent path without the per-database knowledge the feature is for; the opening text already carries Oxyn-written instructions, so the prompt has a place there. |
| `applies_to` on driver name instead of dialect | Redshift speaks the PostgreSQL dialect through the PostgreSQL driver ([ADR-0003](0003-driver-capabilities.md)); the dialect is what the prompt is written for. `{{driver}}` stays available to the text. |
