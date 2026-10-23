# ADR-0065 — An agent declares the query languages it writes, and the assistant opens on every language an agent writes

**Status:** accepted · **Date:** 2026-10-23

**Refines:** [ADR-0049](0049-agents-declared-as-markdown-files.md), on four
points: the target of a prompt is a query language and not only a SQL dialect
(§ 10), an agent file gains a `languages` field (§ 2), the closed list of
variables gains `{{language}}` (§ 3), and a connection whose language is not
SQL reads a language fragment where a SQL one reads its dialect's (§ 8, § 9).
[ADR-0055](0055-sql-agent-is-the-default-of-a-new-conversation.md), on one
point: which agent a new conversation runs on a connection where the SQL agent
is not offered. Everything else in both ADRs stays in force.

## Context

Three drivers are being written in parallel for engines that do not speak SQL —
Redis (`QueryLanguage::RedisCommand`), MongoDB (`MongoQuery`) and
Elasticsearch (`SearchDsl`). Their parity matrices have AI rows: the bar is
that the assistant works on each of them as it works on PostgreSQL.
[ADR-0064](0064-a-session-classifies-non-sql-requests.md) gave their sessions
a way to classify a request, and `oxyn_query::console_language` the language a
console speaks; the assistant had neither. On `main` before this change:

* `AgentSpec::applies_to` is a `Vec<SqlDialect>` and `PromptTarget::dialect` a
  plain `SqlDialect` (`crates/oxyn-ai/src/spec.rs`,
  `crates/oxyn-ai/src/agent_file/target.rs`): an agent cannot say it writes
  anything but SQL, and a prompt cannot be rendered for anything else;
* `ai_ask` and `ai_start_agent` refuse a session without
  `Capabilities::SQL` (`crates/oxyn-desktop/src/backend/ai/conversation.rs`,
  `conversation/startup.rs`), and the panel closes the entry the same way
  (`apps/desktop/src/features/assistant/availability.ts`);
* six places in `conversation.rs` build the tool scope and the context with
  `QueryLanguage::Sql(dialect_for(driver))`, whatever the session speaks;
* the seven recipient fragments tell every model to tag its statements
  `sql`.

Left as it was, each driver worker would patch `oxyn-ai` and `oxyn-desktop`
on its own: three rewrites of the same files, three ways of telling a model
which language to write. This ADR is the one shared change; each driver then
ships an agent file and nothing else.

## Decision

### 1. The target of a prompt is a language

`PromptTarget` carries `language: QueryLanguage` in place of
`dialect: SqlDialect`; `PromptTarget::dialect()` is `language.sql_dialect()`.
The language is the one the connection's console speaks,
`oxyn_query::console_language(driver, capabilities)`:

* for a question or an external agent's start, from the capabilities the
  **session** declares (`Backend::session_language`); a session that declares
  no single language refuses the assistant, as it refuses a console;
* for the agent picker, which names no session, from the capabilities the
  **driver** declares at best (`Driver::capabilities`). A session narrows
  them, never to another language, and a question checks again against the
  session.

That one value is the language of the prompt, of the context
(`ContextBuilder::with_language`), of the `ToolScope` every tool call is
translated in, and therefore of every request an agent sends.

### 2. `languages` in an agent file

`AgentSpec` gains `languages: Vec<QueryLanguage>`, read and written as
`QueryLanguage::as_str` names: `sql`, `cypher`, `gremlin`, `mongo`, `redis`,
`search-dsl`, `cql`, `partiql`, `influxql`, `flux` (`agent_file::LANGUAGES`).

* **Absent means `[sql]`.** Every agent file written before this ADR, and both
  shipped agents, keep their meaning; a declaration that does not set it does
  not serialize it either.
* An explicit **empty** list is refused (an agent offered on no connection),
  and so is a non-empty `applies_to` without `sql` among `languages`:
  dialects only exist in SQL. An unknown name is refused with its line, like
  an unknown dialect.
* `offered_for` adds one condition: the target's language is listed. On a SQL
  target `applies_to` still narrows by dialect; on any other target it is not
  read. The SQL agent is therefore never offered on a Redis session, and a
  Redis agent never on a SQL one.

Like `applies_to` and `recipients`, `languages` narrows where an agent is
offered and grants nothing: the tools, the `PolicyGate` and the tier are the
same whatever the language.

### 3. `{{language}}`, and `{{dialect}}` outside SQL

The closed list of variables becomes `dialect`, `driver`, `environment`,
`language`, `recipient`. `{{language}}` is `QueryLanguage::as_str` of the
target: a value Oxyn holds, from a closed set, never database content, a
server response or a secret — the rule of ADR-0049 § 3. Outside SQL,
`{{dialect}}` is filled with the language's name rather than left empty.

### 4. The composition outside SQL

```text
rendered = role body + "\n\n" + language fragment + "\n\n" + recipient fragment
```

On a SQL target, the language fragment **is** the dialect fragment, chosen as
before. On any other target it is `prompts/languages/<language>.md` when one
is written (`LANGUAGE_FRAGMENTS`), and `prompts/languages/generic.md`
otherwise. `generic.md` carries no product knowledge: it says the language is
not SQL, that Oxyn does not parse it, that a request runs at once only when
the driver proves it reads and counts as a write otherwise — held for
approval, refused on `production` —, that every request stands alone, and that
names are written as the context spells them.

The recipient fragments tag a proposed statement with `{{language}}` instead
of the literal `sql`. On a SQL target the variable renders `sql`: **every
prompt rendered for a SQL target is byte-for-byte the prompt rendered before
this ADR** — checked over the 308 combinations of the two shipped agents ×
11 dialects × 7 recipients × 2 environments when it was implemented.

### 5. What an agent sends is classified by its language, and gated

Nothing new is added on this path; it is stated because it is what makes the
rest safe ([I-07](../../CLAUDE.md#i-07)). `execute_query` translates into a
`Command::Execute` in the scope's language, carrying `Actor::Agent`.
`oxyn-exec` reclassifies it before the `PolicyGate`: `oxyn-query` for SQL, the
session's `classify_intent` for any other language (ADR-0064), `Unknown`
otherwise, which counts as mutating. On `production`, an agent's write — an
unknown command included — is a **refusal** ([I-02](../../CLAUDE.md#i-02)); a
proven read leaves with `limits.read_only` set; elsewhere a write waits for the
user's approval.

### 6. Where the assistant opens, and which agent a new conversation runs

The assistant is offered on a connection when at least one valid agent —
shipped or user — is offered for its language. A SQL session always has the
shipped SQL agent; a session of another language has one when a driver ships
its agent, or when the user writes one.

A new conversation that names no agent runs `AgentCatalog::default_for`:

* the SQL agent when it is offered for the target — every SQL connection,
  exactly as ADR-0055 decides;
* otherwise the first valid agent the picker lists for the target: the
  shipped agents first, then the user's, each by name. A user file therefore
  never displaces the agent Oxyn ships for a language;
* none: the question is refused, naming the language no agent writes.

The panel selects the same agent in a blank thread (`defaultAgentId`).

### 7. The context keeps its single gateway

The schema, the approved samples, the `@` mentions and the `erd` blocks of a
connection that does not speak SQL go through `ContextBuilder`, under the
connection's tier, as for SQL ([I-04](../../CLAUDE.md#i-04),
[ADR-0006](0006-ai-privacy-tiers.md)). `ContextBuilder` already states the
query language in its header and quotes names according to it; the catalog
kinds are those the driver reports — keys and key patterns, collections,
indices. No path is added.

### 8. What a driver adds

A driver that wants the assistant on its language ships
`crates/oxyn-ai/agents/<name>.md` with `languages: [<language>]` and adds it
to `AGENTS` in `crates/oxyn-ai/src/agent_file/files.rs` — one line. A language
fragment shared by several agents is optional, and one line in
`LANGUAGE_FRAGMENTS`. No change to the desktop or the front end is needed.

## Consequences

* **+** One mechanism for every language: Redis, MongoDB and Elasticsearch get
  the assistant by writing a file each, with no shared code to merge three
  times.
* **+** SQL is untouched: same agents offered, same default, same prompts to
  the byte, same agent files.
* **+** The language an agent writes, the language its requests run in and the
  language the executor classifies them by are one value read from the
  session; they cannot disagree.
* **+** A user can bring the assistant to a language Oxyn ships no agent for,
  under the same gate.
* **−** Outside SQL, a model gets only `generic.md` from Oxyn until the
  driver's agent file says more: what the product accepts is the agent
  author's to write, and a thin agent gives thin answers.
* **−** The production guard of an agent's request on these languages rests on
  the driver's `classify_intent` (ADR-0064): an over-claimed `Read` lets an
  agent run a write there.
* **−** The picker lists by the driver's declared capabilities, a question by
  the session's. A driver declaring a language its session then drops would
  show an agent the question refuses; none of the shipped or planned drivers
  does.
* **−** The default outside SQL depends on the catalog's order: two shipped
  agents for the same language are chosen between by name.
* **−** A conversation whose recorded agent is gone still falls back to the
  SQL agent (ADR-0049 § 6); on a session of another language that fallback is
  refused rather than replaced by the language's default.
* **−** The front end's « Open in console » still recognises a block tagged
  `sql` or untagged; a block tagged `redis` offers « Copy » only until the
  panel learns the session's language.

**Exit cost:** moderate. Going back means removing `languages` from
`AgentSpec` — every agent file that set it, the drivers' included, then fails
to parse —, `{{language}}` from the recipient fragments and the variable list,
`generic.md`, and the language arm of the render; `PromptTarget` goes back to
a dialect. What bounds it: absent `languages` means `[sql]`, so SQL agent files
never carry the field, and the SQL prompts never depended on it.

**Reconsider if** a language needs per-version notes that a single fragment
cannot hold (that would argue for a language × version axis, as dialects are
for SQL), if two shipped agents ever write the same language (the default
would then need to be named, not ordered), or if a session can speak two
languages besides SQL at once, which `console_language` refuses today.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Each driver patches `oxyn-ai` and `oxyn-desktop` for its own language | Three concurrent rewrites of the same files and three ways of naming a language to a model; the second and third would conflict with the first |
| Overload `applies_to` with language names (`applies_to: [redis]`) | Mixes two axes in one list: `[postgres, redis]` would mean "PostgreSQL SQL, or any Redis", and "every dialect" (empty) would also mean "every language" |
| Empty `languages` means every language, as an empty `applies_to` means every dialect | An agent written for SQL with an empty list would be offered on Redis and write SQL to it; for languages, the useful default is SQL, not "all" |
| Keep `PromptTarget::dialect` and add an optional language beside it | Two fields that can disagree; the dialect is a property of the SQL language, and `QueryLanguage::Sql(dialect)` already says it |
| A `prompts/dialects/redis.md` fragment | Dialects are SQL's; a Redis fragment under that name would be read as a SQL dialect by the next reader and by `dialect_named` |
| No language fragment outside SQL | The model would not be told, in Oxyn's own words, that Oxyn does not parse the language and treats an unknown command as a write — the one rule that matters on production |
| Keep the literal `sql` tag in the recipient fragments | A Redis agent told by its role to write Redis commands and by the recipient fragment to tag them `sql` gets two contradicting instructions in one prompt |
| The assistant opens on a language from a list in the front end (`ASSISTED_LANGUAGES`) | A list in code goes stale the day a driver ships its agent, and cannot see a user's agent; the catalog already knows what is offered |
| The SQL agent stays the default everywhere, and a non-SQL session asks the user to pick | Every new conversation on Redis would start with a refusal; ADR-0055's reason — a default the user did not choose must not change with the list — holds by preferring shipped agents |
| Ship the Redis, MongoDB and Elasticsearch agents here | Product knowledge belongs to the worker who knows each driver, and is reviewed with it; this change ships the mechanism and a test fixture only |
