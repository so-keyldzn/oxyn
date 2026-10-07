# ADR-0049 — An agent is a Markdown file with a YAML front matter, composed with a dialect and a recipient fragment, filled only from a closed list of variables

**Status:** accepted (2026-10-06) · **Date:** 2026-09-29 · **Amended on:** 2026-10-06, 2026-10-07 ·
**Deciders:** Nicolas Boromée

**Refines:** [ARCHITECTURE §7.3](../ARCHITECTURE.md#73-agent-runtime), on one
point: the form an `AgentSpec` takes on disk. That agents are configurations,
not implementations, stays as it is.

> **Amended on 2026-10-06, before acceptance.** The first version had one
> axis: an agent, targeted by dialect, whose whole prompt was its file. A
> system prompt now depends on **three** axes — the agent's **role**, the
> connection's **dialect**, and the **recipient** that reads it (a provider
> protocol or an external agent). The amendment adds the `recipients`
> field (§ 2), the `{{recipient}}` variable (§ 3), the dialect and recipient
> fragment files (§ 8), the fixed composition order (§ 9), the Rust and IPC
> surface every implementation follows (§ 10), and the matching consequences
> and rejected alternatives. Since the ADR was still `proposed`, it is
> amended in place; nothing of the first version is withdrawn except the
> sentence "the body is the system prompt, as is", now "the body is the
> **role** part of the system prompt".

> **Amended on 2026-10-07, after acceptance — the default agent.** The
> default of a new conversation is the **SQL agent, whatever the display
> order**. § 4 sorts the picker by `name`, so Schema is listed before SQL;
> SQL stays the one selected in a blank thread, and a conversation that
> names no agent runs it on the backend. The clause of § 2 "otherwise the
> first offered one in the order of § 4" is withdrawn: the default never
> follows the display order. The rest of § 2 and § 4 stands as written.

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

**Added on 2026-10-06.** One prompt per agent is still one prompt for
every reader. The same text goes to Anthropic's API, to a 7-billion-parameter
model behind Ollama (`openai_compatible`), and to Claude Code or Codex
through ACP, which see Oxyn's tools under their own MCP prefix and have no
shell of their own once confined
([ADR-0032](0032-agent-externe-confine-au-lancement.md)). What a model must
know about the dialect — identifier quoting, `LIMIT` against `TOP` or
`FETCH`, that `EXPLAIN ANALYZE` runs the statement, whether DDL is
transactional — is the same for the SQL agent and the Schema agent, and
would otherwise be copied into each role file, per dialect.

## Decision

### 1. The format: Markdown with a YAML front matter

An agent is one UTF-8 file, `<name>.md`:

```markdown
---
id: 0199a3c0-0000-7000-8000-000000000001
name: SQL
description: Writes, fixes and explains queries on the open connection.
applies_to: []
recipients: []
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
* The **body**, after the closing `---`, is the **role** part of the system
  prompt: what the agent is for, whatever the database and whoever reads
  it. The rest of the prompt comes from the fragments of § 8, in the order
  of § 9. Oxyn never renders it as HTML; Markdown is only the author's
  convenience and the model's.
* The parser is configured strict, and the configuration is part of the
  decision: `Budget { max_anchors: 0, max_aliases: 0, .. }`,
  `merge_keys: MergeKeyPolicy::Error`,
  `duplicate_keys: DuplicateKeyPolicy::Error`, `strict_booleans: true`,
  `reject_unsupported_tags: true`, and the crate's `include` feature off.
  An agent file has no use for references between nodes, and each of these
  is either a resource-exhaustion lever or a way for a key to mean
  something other than what it reads.
* A file is refused, with the file name and the line in the message —
  never its content beyond that —, when: it is over **64 KiB** (checked
  before parsing: `serde-saphyr`'s own size cap applies to readers, not to
  strings); it is not valid UTF-8; the front matter is missing or is not the
  first thing in the file; the YAML breaks one of the settings above;
  `validate()` fails; the prompt names an unknown variable (§ 3);
  `applies_to` names an unknown dialect, or `recipients` an unknown
  recipient (§ 2).

### 2. Targeting: `applies_to` and `recipients`

`applies_to` is a list of `SqlDialect` names as `SqlDialect::as_str` writes
them: `ansi`, `postgres`, `mysql`, `sqlite`, `sqlserver`, `oracle`,
`clickhouse`, `duckdb`, `snowflake`, `bigquery`, `redshift`.

`recipients` is a list of recipient keys, as `Recipient::as_str` writes
them (§ 8): `anthropic`, `openai`, `gemini`, `openai_compatible`,
`claude-code`, `codex`, `external`.

For both, an empty list means "all". An agent is **offered** for a target
only if both lists are empty or contain the target's dialect and recipient
(`AgentSpec::offered_for`). The agent picker shows the offered agents; the
default is the SQL agent when it is offered, otherwise the first offered
one in the order of § 4.

Targeting narrows what is offered. It grants nothing: the tools, the
`PolicyGate` and the connection's tier are the same whichever agent runs.

### 3. The closed list of variables

The prompt — role body and fragments alike — may contain `{{name}}`
placeholders, and only these:

| Variable | Value | Source |
|---|---|---|
| `{{dialect}}` | `SqlDialect::as_str` of the connection | the connection's driver |
| `{{driver}}` | the driver's name | the driver |
| `{{environment}}` | `local`, `development`, `staging` or `production` | the connection's marking |
| `{{recipient}}` | `Recipient::as_str` of the destination (§ 8) | the destination chosen in the panel |

All four are values Oxyn holds, from a closed set; none is written by the
server, the database or the model. A placeholder outside this list makes
the file **invalid**; it is never replaced by an empty string. The list is
extended only by a new ADR, and only with a value that is neither database
content, nor a server response, nor a secret. `{{recipient}}` was added by
the 2026-10-06 amendment under that rule: it names a protocol or a preset,
never an endpoint, a model or a key.

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
   become thin readers of their file; their `id` stays the same
   (`0199a3c0-0000-7000-8000-000000000001` and `…0002`), since the audit
   log records it.
2. **User agents** — second step, same format — live in `agents/` of the
   data directory (`ProjectDirs::data_dir()`, next to the local store). They
   are read at launch and when the user reloads them; an invalid file is
   listed with its error and not offered, it never blocks the others. Their
   `id` must not collide with a shipped one: a collision is refused, so a
   file cannot take the audit identity of a shipped agent. A user file
   supplies a **role**; it never replaces a dialect or recipient fragment
   (§ 8).
3. **Plugin agents** (phase 4, [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md))
   use the same format and the same validation; the manifest carries the
   file.

Order in the picker: shipped, then user, then plugin, each by `name`.

### 5. What a file can never carry

Unchanged from `spec.rs`, and now enforced by `deny_unknown_fields`: no
connection or session, no privacy tier, no endpoint, no key, no model
choice. `tools` restricts the registry; it never adds to it. `recipients`
restricts where an agent is offered; it never chooses the destination.

### 6. One conversation, one agent

* The picker sits in the header of the assistant panel. The agent is chosen
  **per conversation**: choosing another agent opens a **new conversation**;
  the current one stays in the history, unchanged. A conversation's **role**
  therefore never changes under it.
* The conversation remembers its agent: `ai_conversations` gains a nullable
  `agent_id TEXT` column (one additive `ALTER TABLE … ADD COLUMN` migration
  of `oxyn-store`). A conversation written before it reads as the SQL agent,
  which is what it ran with.
* Resumed, a conversation runs with its recorded agent, rendered again with
  the connection's current target. If that agent no longer exists — a user
  file deleted or now invalid —, the conversation reopens with the SQL
  agent, and the panel **says so**, naming the missing agent. It never
  silently runs under another prompt than the one it shows.
* Changing **destination** between two questions keeps the conversation and
  its agent, and changes the recipient fragment (§ 9). It does not put a
  history under a prompt it was not written with: a change of destination
  already sends no previous exchange to the new recipient
  ([UX-SPEC](../UX-SPEC.md#who-answers-is-chosen-in-the-panel)), so the new
  recipient starts from a fresh context with its own rendered prompt. Each
  exchange records its destination, hence the prompt that produced it can
  be named: agent × dialect × recipient. A destination the conversation's
  agent is not offered for is shown disabled, with its reason; it does not
  switch the agent.

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

### 8. Three axes, three kinds of file

Every file below is UTF-8, capped at **64 KiB**, embedded with
`include_str!`, and checked by a build-time test: size, encoding, and that
every placeholder belongs to § 3.

| Axis | File | Front matter | Carries |
|---|---|---|---|
| Role | `crates/oxyn-ai/agents/<role>.md` | yes (§ 1) | what the agent is for; `applies_to`, `recipients`, tools, context, turns |
| Dialect | `crates/oxyn-ai/prompts/dialects/<dialect>.md` | **none** | what a model must know to write correct SQL for that dialect |
| Recipient | `crates/oxyn-ai/prompts/recipients/<recipient>.md` | **none** | how this recipient behaves inside Oxyn |

**Dialect fragments.** One file per `SqlDialect::as_str` key. It says:
how identifiers are quoted; how a result is bounded (`LIMIT`, `TOP`,
`FETCH FIRST`); the `EXPLAIN` form, and that the analyzing form
(`EXPLAIN ANALYZE` and its equivalents) **executes** the statement
([I-07](../../CLAUDE.md#i-07)); whether DDL is transactional; the catalog
views; the common traps. Real fragments are written at least for `ansi`,
`postgres`, `redshift`, `mysql`, `sqlite` and `duckdb` — the dialects Oxyn
has or plans drivers for. **A dialect without its file uses `ansi.md`**:
the common denominator is a conservative default, an absent fragment would
leave the model guessing.

**Recipient fragments.** One file per recipient, and every recipient has
its file — a missing one fails the build-time test, there is no fallback.
It says how the tools look **as this recipient sees them**, the output
format (fenced `sql` blocks, the `erd` block of
[UX-SPEC](../UX-SPEC.md#an-erd-block-is-drawn-from-the-catalog-not-from-the-answer)),
and what the recipient must not attempt:

| Key | Recipient | What its fragment is for |
|---|---|---|
| `anthropic` | `AiProviderKind::Anthropic` | Oxyn's tools as native tool calls |
| `openai` | `AiProviderKind::OpenAi` | same, for OpenAI's function calling |
| `gemini` | `AiProviderKind::Gemini` | same, for Gemini's function declarations |
| `openai_compatible` | `AiProviderKind::OpenAiCompatible` | a **shorter, more explicit** style: Ollama, LM Studio, llama.cpp often serve small local models |
| `claude-code` | the `claude-code` preset | Oxyn's tools (`describe_schema`, `execute_query`, `request_sample`) under its own MCP prefix; **no shell or file tool of its own**, confined ([ADR-0032](0032-agent-externe-confine-au-lancement.md)) |
| `codex` | the `codex` preset | same, for Codex |
| `external` | an external agent declared by hand | only Oxyn's MCP tools are meant to be used; it may have shell or file tools Oxyn cannot see, and must not use them |

The `external` fragment is a request, not a fence: a hand-declared agent is
**not confined**, and Oxyn cannot enforce what the fragment asks
(Consequences).

Fragments are **shipped only**. A user or plugin agent supplies a role; it
does not add, replace or remove a dialect or recipient fragment. What Oxyn
tells every model about `EXPLAIN ANALYZE` or about its own tools is Oxyn's
text, reviewed in the repository.

### 9. The composition order is fixed

```text
rendered = role body + "\n\n" + dialect fragment + "\n\n" + recipient fragment
```

then the § 3 substitution, applied once to the whole text. The order is
fixed and nothing else is concatenated in: no catalog, no sample, no server
banner, no conversation state. Database content reaches the model only
through `ContextBuilder` ([I-04](../../CLAUDE.md#i-04)), in a message of its
own, never in the rendered prompt. Since the substitution only knows four
values Oxyn holds, a value cannot introduce a placeholder of its own.

`render_system_prompt` is the **only** way a system prompt is produced. A
`format!` that builds one elsewhere is a defect, as a second gateway would
be.

### 10. The surface every implementation follows

In `oxyn-ai`:

```rust
pub enum Recipient { Provider(AiProviderKind), External(ExternalAgentKind) }
pub enum ExternalAgentKind { ClaudeCode, Codex, Other }

pub struct PromptTarget {
    pub dialect: SqlDialect,
    pub driver: DriverId,
    pub environment: Environment,
    pub recipient: Recipient,
}

impl AgentSpec {
    pub fn offered_for(&self, target: &PromptTarget) -> bool;
}

pub fn parse_agent_file(name: &str, text: &str) -> Result<AgentSpec, AgentFileError>;
pub fn render_system_prompt(spec: &AgentSpec, target: &PromptTarget) -> Result<String, AgentFileError>;
pub fn shipped_agents() -> Vec<AgentSpec>;
```

* `Recipient::as_str` gives the file keys of § 8. `ExternalAgentKind` is
  parsed from the preset id (`claude-code`, `codex`); an external agent
  without a preset is `Other`, key `external`.
* `AgentSpec` gains `applies_to: Vec<SqlDialect>` and
  `recipients: Vec<Recipient>`, both `#[serde(default)]`, empty meaning
  "all".
* `AgentFileError` carries the file name and the line, never the file's
  content beyond that.

Between `oxyn-desktop` and `apps/desktop`:

* `ai_list_agents { connectionId, destination? } -> AgentOption[]`, with
  `AgentOption = { id, name, description, origin: "shipped" | "user", error: string | null, disabledDestinations }`.
  Only the agents offered for that connection come back; an invalid user
  file comes back with `error` set, and is not selectable.
* Starting a conversation takes an optional `agentId`; absent means the
  SQL agent.
* A conversation summary and a transcript carry `agentId: string | null`
  and `missingAgent: { name: string } | null` — set when the recorded agent
  no longer exists and the SQL agent replaced it (§ 6).

> **Amended on 2026-10-06, by the implementation of this section.** Three
> details the first wording left open:
>
> * `destination` is optional, a `DestinationChoice` as a question takes
>   it. Given, the list is the agents offered for that destination. Absent
>   — the panel asks before a destination is chosen —, it is the agents
>   offered for at least one declared destination.
> * `disabledDestinations: { kind: "provider" | "agent", id: string }[]`
>   names the declared destinations an agent is not offered for: the panel
>   shows them disabled while a conversation runs that agent (§ 6). It is
>   empty for an entry in error. A question or an external agent start
>   toward such a destination is refused by the backend before anything
>   starts, whatever the panel shows.
> * `missingAgent.name` carries the recorded agent's **id**: the store
>   keeps the id and not the name, and a file that is gone can no longer
>   say what it was called. `agentId` is the agent the conversation runs
>   now — the SQL agent's id after that fallback.

## Consequences

* **+** Writing or tuning an agent is editing text: no Rust, no recompile
  for user agents, a one-file diff for shipped ones.
* **+** One agent per database becomes one file per database, offered only
  where it applies.
* **+** What every model must know about a dialect is written once, in one
  fragment, and every role benefits from it. What a recipient must know
  about Oxyn's tools is written once per recipient.
* **+** A small local model behind `openai_compatible` gets a prompt
  written for it, and a confined external agent is told what it has and
  has not, instead of looking for a shell it no longer has.
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
  ([SECURITY](../SECURITY.md#input-surface), "Workspace files"): a file written by someone
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
* **−** The `external` fragment asks a hand-declared agent not to use its
  own shell or file tools; Oxyn **cannot enforce it**, since such an agent
  is not confined ([ADR-0032](0032-agent-externe-confine-au-lancement.md)).
  The panel's permanent mention for a non-confined agent stays the truth,
  not the fragment.
* **−** The prompt a model read is no longer one file: it is three, and an
  exchange is explained by naming the agent, the dialect and the recipient.
  A reviewer of a prompt change reads the composition, not one file.
* **−** The `ansi` fallback is right for no dialect in particular: until a
  dialect has its own fragment, its models get the common denominator.
* **−** Four variables will feel short. Every request for a fifth one has
  to go through an ADR, on purpose.

**Exit cost:** low for shipped agents — about a day to put the prompts back
into Rust literals; the `AgentSpec` type does not change. The fragments
add a day: their text goes back into the role literals, one copy per
role. Higher once user agents exist: their files are the user's, and
dropping the format would need a converter; the `agent_id` column stays,
readable, whatever the format. What bounds it: the front matter is
`AgentSpec`'s own serde form, so any other serde format (TOML, JSON) reads
the same data.

**Reconsider if** a provider needs per-agent settings that do not fit a
flat front matter (structured outputs, tool schemas written by the agent
author), if user agents turn out to be used mostly to work around the
`PolicyGate`'s refusals — which would argue for signing them instead of
reading them freely —, or if a recipient fragment turns out to need
different text for two models of the same protocol, which would argue for
a model-family axis rather than a longer fragment.

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
| One complete file per role × dialect × recipient | Combinatorial: 2 shipped roles × 11 dialects × 7 recipients is 154 files, nearly all copies of each other; a correction to the `EXPLAIN ANALYZE` warning would be made in dozens of places, and missed in one. |
| Recipient-specific or dialect-specific text inside the role file, behind conditionals | Needs conditionals in the prompt, that is the template engine rejected above; and every role would carry its own copy of the dialect and recipient knowledge. |
| Recipient guidance written in Rust, in each provider adapter of `oxyn-llm` | Back to prompt text in string literals, outside the review of prompts, and absent from the external-agent path, which has no adapter. |
| Recipient keyed by model name rather than by protocol or preset | The model list is open and changes every month; a closed key is what the § 3 rule allows. `openai_compatible` approximates "often a small local model", and the reconsideration clause covers the day it is not enough. |
| No dialect fragment when a dialect has no file | The model would get no SQL guidance at all; `ansi` is conservative and always present. |
| User or plugin files that override a fragment | A user file could remove the warning that `EXPLAIN ANALYZE` executes, or tell an external agent it has a shell. Fragments are Oxyn's own text; a role body can still add guidance of its own. |
| Concatenate the rendered prompt with the schema context | Puts database content into the system prompt, outside `ContextBuilder`'s fencing (I-04); the context stays a message of its own. |
