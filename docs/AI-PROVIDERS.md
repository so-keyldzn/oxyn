# AI boundary

> **Authority**: what crosses the boundary to an AI provider, how
> the program goes about it, and what it does with the responses.

The **what** is settled by [ADR-0006](adr/0006-ai-privacy-tiers.md): three
tiers — `Local`, `Metadata` (default), `Sampled` — chosen **per connection**.
That table is not copied here; it lives in the ADR. This document carries what
the ADR does not say: the consequences in the code.

Invariants involved: [I-04](../CLAUDE.md#i-04), [I-07](../CLAUDE.md#i-07),
[I-03](../CLAUDE.md#i-03).

## Why this document exists

"Privacy first" and "AI when it adds value" are two founding principles
([VISION](VISION.md)) which, misapplied, produce exactly the incident
the product claims to avoid.

**Concrete failure:** the user sets the tier to `Sampled` for their sandbox
database, and forgets it. Three days later they open their employer's customer
database and ask "find me the duplicates". If the tier were global, real
rows would go to a third-party provider. Technically nothing failed;
contractually, it is a confidentiality breach, and it is
irreversible. That is why the tier is attached to the
**connection** and never to the application, the provider or the session.

## The gateway is single

There is **one single** function through which context can reach a
prompt, and it is the one that applies the tier. Any other path is a defect,
not an optimization.

This constraint is what makes [I-04](../CLAUDE.md#i-04) checkable: one reviews
a gateway, not every call of every agent. It is backed by the
architecture rule that supports it — `oxyn-ai` never talks to a driver, it receives
already collected context
([ARCHITECTURE](ARCHITECTURE.md#le-sens-des-dépendances)).

What goes out under **no** tier, `Sampled` included: connection
identifiers, connection strings, tokens, keychain content. There is no
dialog for that — it is [I-03](../CLAUDE.md#i-03), and the code must not offer
the path.

## `Metadata` by default is not "nothing goes out"

The default of [ADR-0006](adr/0006-ai-privacy-tiers.md) is `Metadata`: DDL,
names, types, indexes, cardinalities and execution plans
**go out** as soon as a remote provider is configured and an AI feature
is used.

It is a deliberate compromise — without a schema, a database assistant is
useless — but it must be faced squarely: *a column name is already
data*. A `patients` table with an `hiv_status` column reveals the essential
without a single row going out.

Two consequences in the code:

1. **The effective tier is permanently visible**, not in a settings
   panel. A user who cannot tell at a glance where their
   query goes is not giving informed consent.
2. **`Local` must remain usable**, not be a box that disables everything. A
   feature that only works in `Metadata` says so in the interface rather
   than failing without explanation ([ADR-0006](adr/0006-ai-privacy-tiers.md), §
   consequences).

## Local and remote are not distinguished by the API

A local provider (Ollama, LM Studio, llama.cpp) and a remote provider
(OpenAI, Anthropic, Gemini, Bedrock, Azure, any OpenAI-compatible endpoint)
often expose the **same** API. They are distinguished by a single fact: the
data leaves the machine, or not.

> **Trap to handle in the code:** an "OpenAI-compatible" endpoint
> pointed at `localhost` can be a proxy that re-sends to the cloud. The
> local/remote classification is done on the real host **after resolution**, never
> on the presence of `localhost` in the URL, and it is re-checked at every
> configuration change.
>
> **No HTTP redirect is followed; a `3xx` is an error.** A `307`
> or a `308` would resend the prompt, and a key the HTTP stack cannot
> strip (`x-api-key`, `api-key`), to an origin nobody has classified.
> The error asks to point the base URL at the final address, without copying
> the destination.


Oxyn's HTTP clients ignore environment and system proxies. The reach measured
for a send is passed to the transport as well as the privacy gate and
`ai_egress`: for `Local`, every new DNS answer must be nonempty and entirely
loopback before any address is tried. A changed or mixed answer is refused;
literal non-loopback addresses are refused as well. DNS runs off the UI and
async worker threads. A service intentionally listening on loopback can still
forward traffic elsewhere; this guarantee concerns Oxyn's TCP peer.

## An external agent: the reach is not unknown, it is **unknowable**

Since [ADR-0026](adr/0026-agents-externes-acp.md), a destination can also
be an **external agent** — a program already installed and authenticated on the
user's machine, launched as a subprocess and speaking the Agent Client Protocol. Oxyn
can use the agent's own authentication or inject explicitly declared environment
secrets from the OS keychain at launch ([SECURITY](SECURITY.md#external-agent-environment)).

Oxyn has a **preset** for two agents, Claude Code and Codex: an ACP
adapter at a pinned version, whose confinement settings have been measured
([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)). Any other agent —
Gemini CLI, which ADR-0026 cited as an example, included — is declared **by hand**,
through its command and its arguments, and is **not confined**. The declaration and what
the screen shows of it are described in
[UX-SPEC](UX-SPEC.md#provider-and-agent-configuration).

This mode moves the question of this document, and it must be said clearly. For a
declared provider, the reach is **measured**: the host is resolved, and the answer
may be stale — hence the re-check at every opening. For an external
agent, there is nothing to measure. It is an opaque process: it may talk to a
local model, to a remote service, or switch between two turns, and **nothing in
the protocol makes it possible to ask it**.

The consequence follows without any new mechanism, because the repository already treats
`Reach::Unresolved` as remote:

| Connection tier | External agent |
|---|---|
| `Local` | **refused** — the promise "nothing leaves the machine" cannot be kept by a process whose output cannot be seen |
| `Metadata` *(default)* | allowed — the database structure goes out with the first question of an agent session, and on request through the `describe_schema` tool; both rendered by `ContextBuilder` |
| `Sampled` | allowed, the structure goes out as under `Metadata`, and so does the sample **approved column by column** — pinned by the user or requested by the agent with the `request_sample` tool. The process that received values is released after the exchange: the next question launches another one ([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)). Its MCP tools go through the `PolicyGate`, re-read the tier at each call; `execute_query` only returns the shape of a result — never its values |

The details of the sample are in [Approved sample](#approved-sample).

The refusal falls **before launch**, not before sending: merely
starting the agent can be enough for it to contact its service.

> **What goes into the prompt, and through where.** For a provider as for an
> external agent, the gateway is held by the type. `run_turn` requires an
> `AgentPrompt` ([ADR-0027](adr/0027-porte-unique-pour-les-deux-destinations.md)),
> which is only born under the connection's tier: the question alone
> (`from_user`), or the question preceded by the database structure
> (`with_schema`). This structure is rendered by `ContextBuilder::build` — the
> same code, the same budget, the same framing as for the internal assistant — and
> it only goes out when an agent session opens: a question that follows a
> response in the same session carries the question alone, like a remembered provider
> conversation. `with_schema` also receives the sample approved for
> **this** question, which it passes to `ContextBuilder::with_samples`: under any
> tier other than `Sampled`, it is discarded. `from_user`, which continues a
> session, never receives one — a question with a sample always opens a
> new session ([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)).
>
> **The structure beyond this context** goes through the `describe_schema` tool,
> the same for every destination — a provider's tool loop or the MCP bridge
> of an external agent. It becomes a `Command::DescribeCatalog` that goes through the
> `PolicyGate`, preceded by the metadata reads missing from the cache
> ([What the AI sees of the schema](#what-the-ai-sees-of-the-schema-and-when-it-is-read));
> the executor returns the handle of the local catalog, and `oxyn-ai`
> renders it through `ContextBuilder::build`, under the tier re-read at the call. The rendering only
> knows the catalog's common model: a collection, a key pattern or
> a graph label is described there like a table, in the query language of
> the connection ([ADR-0030 § 4 bis](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md#4-bis-the-database-structure-one-tool-for-every-destination)).

> **What Oxyn cannot promise here**, and therefore does not promise: what
> the agent does with what it receives. Billing, retention and data
> processing are a matter between the user and the agent's provider. Oxyn can
> **name** it; it cannot guarantee it. That is precisely why `Local`
> stays closed to this mode.

### What the agent itself asks of the machine

An external agent asks its client for permissions — read a file, edit
one, run a command. **These are not Oxyn `Command`s**, and there
is no translation: "edit `/etc/hosts`" does not become a database
command. The `PolicyGate` keeps its domain; this one is handled
separately, and **refused by default**.

Oxyn is a database workshop: nothing in its scope justifies
granting a subprocess access to the file system or to a shell.
Above all, it has no screen to show *which* file or *which* command —
and a confirmation that does not say what it authorizes shifts the responsibility
without giving the means to exercise it. Only internal reasoning and mode
switching are granted: they do not leave the agent.

**Refusing is not enough.** An agent only asks what its mode forces it to
ask: measured on 2026-09-23, Claude Agent and Codex ran a command
without asking anything in their initial mode. An agent whose adapter Oxyn knows
is therefore **confined at launch**: its machine tools are
removed or cut off, its personal tools disabled, and it is kept in its
strictest mode. Oxyn's tools, for their part, are pre-authorized on the agent side,
since the `PolicyGate` controls them ([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)).
An agent declared by hand is not confined, and the screen says so.

What the panel shows in return of the agent's work — text, reasoning,
kind and state of its steps, plan — and what it never shows of it is described
in [UX-SPEC](UX-SPEC.md#what-the-panel-shows-of-an-external-agent); the
rule refines ADR-0026, which only reported the text.

## The system prompt depends on the recipient

What Oxyn writes to a model besides the context — the system prompt for a
provider, the Oxyn-written block of the opening text for an external
agent — is **composed**, not stored whole
([ADR-0049](adr/0049-agents-declared-as-markdown-files.md), accepted,
amended on 2026-10-06): the agent's role, then the fragment of the
connection's dialect (`ansi` when the dialect has none), then the fragment
of the **recipient**, in that fixed order, filled with four closed
variables — `{{dialect}}`, `{{driver}}`, `{{environment}}`,
`{{recipient}}`. The same question, on the same connection, therefore goes
out with a different system prompt to Anthropic's API, to a small model
behind `openai_compatible`, and to Claude Code.

What this changes at the boundary, and what it does not:

* **The recipient key is the only new value that goes out**, and it names a
  protocol or a preset — `anthropic`, `openai`, `gemini`,
  `openai_compatible`, `claude-code`, `codex`, `external` —, never an
  endpoint, a model, a host or a key ([I-03](../CLAUDE.md#i-03)).
* **No database content enters the composition.** The fragments are text
  shipped with Oxyn; the variables are values Oxyn holds. The schema, the
  samples and the server's identity still reach the model only through
  `ContextBuilder`, under the connection's tier, in a message of their own
  ([I-04](../CLAUDE.md#i-04)). The tier table of
  [ADR-0006](adr/0006-ai-privacy-tiers.md) is unchanged: a rendered prompt
  carries nothing a `Local` connection would have to hold back.
* **A recipient fragment describes; it does not grant.** The `claude-code`
  and `codex` fragments tell a confined agent that it has Oxyn's MCP tools
  and no shell; the `external` fragment asks a hand-declared agent not to
  use the shell or file tools Oxyn cannot see. Neither is a guarantee: the
  confinement of [ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)
  and the `PolicyGate` are, and a hand-declared agent stays **not
  confined** whatever its fragment says.
* **A user agent file only supplies a role.** It cannot replace a dialect
  or recipient fragment, so what Oxyn tells every model about its tools and
  about `EXPLAIN ANALYZE` executing the statement
  ([I-07](../CLAUDE.md#i-07)) stays Oxyn's text.

## What the AI sees of the schema, and when it is read

The context is rendered from the catalog **cache**, never from a server
round trip made by `oxyn-ai`. Yet the tree loads this cache on demand: the list
of a schema's tables when the user expands it, a table's fields
when they open it. Without a complement, the AI would only know what was clicked —
"Describing 0 of 0 known relations" on an eleven-table database. Oxyn therefore completes
the cache **itself**, without a model, before rendering it
([ADR-0036](adr/0036-l-assistant-complete-le-catalogue.md)):

| When | What is read | Actor |
|---|---|---|
| a question opens a session — provider, or external agent at its first question | the server if it has never been read; the list of relations of every never-listed schema; then fields, indexes and foreign keys of the relations **the gateway will keep** — `@` mentions first, then the search guided by the question | `Actor::Human` |
| a question that follows an already informed session | the **mentioned** relations only | `Actor::Human` |
| the panel's first `@` on a connection (`ai_list_mentionable`) | the server if it has never been read, then the list of relations of every never-listed schema — names to choose from, **no description** | `Actor::Human` |
| the `describe_schema` tool | like an opening, guided by its search words | `Actor::Agent` |
| the `refresh_catalog` tool | the server, then the list of relations of every schema, **even fresh** | `Actor::Agent` |

What frames these reads:

1. **Metadata, through the bus.** Each read is a
   `Command::RefreshCatalogScope` — the one for expanding the tree —, submitted
   by `ExecutorSink`: the `PolicyGate` decides, the journal records it with its
   actor ([I-01](../CLAUDE.md#i-01)). No row is read. The policy
   allows it everywhere, `production` included, since it writes nothing.
2. **The gateway's selection, and it alone.** What is described is what
   `ContextBuilder::build` will keep, computed by the same function
   (`oxyn_ai::context::wanted_relations`). A name the cache does not list
   is never sent to the server: a mention is checked against the list
   before being read.
3. **Bounds, without failure.** At most 32 schemas listed, at most 24 relations
   described — the gateway's ceiling —, in 5 seconds in total
   (`crates/oxyn-desktop/src/backend/ai/catalog_fill.rs`). Exceeding them
   does not fail the question: the context goes out with what is loaded, and
   the framing says what is missing — "N relations not loaded yet", "N schemas not
   listed yet", "the catalog of this connection has not been read yet". These
   lines are counts, without names: they are the same under every tier.
4. **Nothing is re-read.** A level read and not invalidated since is not re-read; a
   DDL emitted by Oxyn invalidates the catalog, and the next question re-reads
   once. A failed read is not retried within the same completion.
5. **Cancellable.** Stopping the question cancels the token of the ongoing read,
   which the driver honors as for an expansion; the timeout cuts it the same
   way, and the read is awaited two seconds to finish cleanly rather
   than abandoned in the middle of an exchange.

The rest of the product benefits without more code: the tree updates through
the `CatalogUpdated` event, and the list of `@` mentions as well as the `erd`
diagram read the same cache.

### Semantic ranking stays on the machine

"The search guided by the question" is lexical, and only lexical while
semantic ranking is off — its default. Turned on by the human, it adds local
CPU embeddings that order the relations the lexical score ties or misses, and
complete the selection with them
([ADR-0056](adr/0056-local-cpu-embeddings-for-context-selection.md)):

- **the embedding step sends nothing.** The question, and each relation's
  qualified name and comment, are embedded in Oxyn's process by
  `oxyn-embed`; the vectors are kept in memory only, keyed by a digest, never
  logged, never written, never sent. The step itself has no `ai_egress`
  record because it sends nothing, under every tier — `Local` included;
- **a score is not context, but it changes the context.** `ContextBuilder`
  receives one number per relation; a score carries no word into the prompt,
  and each relation is rendered as without it, under the same tier. But the
  scores decide **which** relations are described, and **can raise their
  number**: where the lexical search keeps only its matches, the relations it
  missed complete the selection by cosine, up to `max_relations` (24). More of
  the schema then reaches the provider — always under the connection's tier,
  within the token budget, and recorded in `ai_egress` like any context. The
  settings say so before the option is turned on. The catalog fill reads the
  same scores through `wanted_relations`, so what is described is still what
  is kept;
- **it never outranks a name.** A relation matched by name keeps its rank;
  the semantic score breaks ties and orders what matched nothing, without a
  threshold. It is bounded at 2 seconds per question, and while the model is
  off, downloading or failing, selection is exactly the lexical one.

**Only the provider path is ranked this way, for now.** The context sent to an
external agent goes through `AgentPrompt`, which takes no scores: its
selection stays lexical whatever the option says. Ranking it too is planned
work, outside the change that introduced semantic ranking.

The embedding model is not a provider: it generates no text, receives no
prompt, and needs no declaration in `ProviderRegistry`.

## What is done with the responses

**No model output is executed directly** ([I-07](../CLAUDE.md#i-07)).
An agent's proposal is a `Command` like any other, carrying
`Actor::Agent`, and it goes through the `PolicyGate`
([ADR-0004](adr/0004-command-bus.md)). It is the same gate as for a human:
there is no second execution path for the AI, and that is precisely what
makes an instruction injection hidden in a database's content produce a
visible approval request rather than an execution.

Without exception, including for what "only reads": a `SELECT` on a
view can trigger a function, and an `EXPLAIN ANALYZE` **actually executes**
the query it analyzes — including a `DELETE`.

**Concrete failure:** the assistant proposes "here is the query to clean up the
duplicates" and an "auto-execute" mode runs it. The deduplication
condition is off by one join. There is no `ROLLBACK`: the `DELETE`
was in autocommit.

A write proposal always displays, before execution: the exact SQL and
the targeted connection with its environment marking. **Those two are
guaranteed.**

An estimate of the number of affected rows is added *when it is
available* — and to date it never is: `Preview::estimated_rows` is
always `None`, and the review displays "Number of affected rows unknown."
Getting it requires an `EXPLAIN` before the review, whose cost and form differ
per driver; it is a separate batch, recorded in
[IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md).

This paragraph used to promise the estimate without reservation. It is the number that
distinguishes a one-row `UPDATE` from an `UPDATE` without `WHERE`: promising it without
providing it suggests a protection that does not exist, at the precise moment when
[I-02](../CLAUDE.md#i-02) matters. The code, for its part, was honest — it says
"unknown".

A tool call whose arguments are not readable JSON is **not executed, and does
not fail the turn**: the provider reports it as such
(`ChatEvent::ToolCallInvalid`), the model receives a rejection as that call's
result — the same as for arguments that do not fit the tool —, and the other
calls of the turn run. The rejection names the parse error, never the
arguments: they are a model output, and can copy what the model was given.

## A silent provider

A question waits on two things that can go quiet without failing: a provider's
stream, and an external agent's answer. Neither has a total bound — a long
answer that keeps arriving is never cut —; both have a bound on **silence**.

| What | Silence accepted | Reset by | On expiry |
|---|---|---|---|
| Provider request and stream (`oxyn-llm`, `IDLE_TIMEOUT`) | **300 s**, until the response headers, then between two chunks | any byte received, `ping` frames and `:` comments included | before the headers, `LlmError::ResponseTimeout`; during the stream, the end is `StopReason::Interrupted`. Both are ambiguous and never replayed ([I-13](../CLAUDE.md#i-13)) |
| External agent's answer (`external::session`, `AGENT_IDLE_LIMIT`) | **300 s**, outside a tool call | any session update, any permission request | `session/cancel` is sent, and the question ends with "the agent sent nothing for 300 seconds" |
| External agent's confirmation of a stop (`CANCEL_GRACE`) | **10 s** | — | the agent's process group is stopped; the next question starts a fresh agent |

No provider documents how often it speaks while the model thinks
([RESEARCH-NOTES](RESEARCH-NOTES.md#idle-streams--checked-on-2026-10-04)): the
values are Oxyn's, chosen long so that a reasoning model thinking for minutes
is not cut. "Stop" remains the fast way out, and it frees the session whatever
the agent does.

A tool call in flight suspends the external agent's count: a call to Oxyn's
tools can wait on an approval the user is reading, or on a long query, and the
agent says nothing meanwhile. Cutting there would cut an approval in the
middle of its reading.

**Concrete failure:** the laptop sleeps during an answer; on waking, the Wi-Fi
has changed and the socket is half-open — no reset ever arrives. Without these
bounds, the question stayed "thinking" forever; with an external agent, a stop
the agent never confirmed also held every later question behind it.

## Listing a provider's models

The provider form offers the models an endpoint serves, before anything is
saved (`ai_list_draft_models`), and a declared provider lists its own
(`ai_provider_models`). Both take one path, `oxyn_llm::list_models`.

**What goes out:** one `GET` of the provider's own list endpoint
([RESEARCH-NOTES](RESEARCH-NOTES.md#model-listing--checked-on-2026-10-04)),
carrying the key — and nothing else. No prompt, no schema, no database content,
no tier applies because there is nothing to filter. No third-party model
registry is asked: the list is the provider's word, and its metadata
(display name, context window, price) is what the provider published.

| Rule | Why |
|---|---|
| The listing is built by the transport a conversation would use: same base URL resolution, same refusal of redirects, same confinement of a loopback endpoint | a list that works on an address the conversation would not use would validate a broken configuration |
| A typed key goes only to the address typed beside it, and is never written to the keychain by a listing | typing a key in the form is the user's consent for that endpoint, not for another |
| A stored key is used only while the kind and the endpoint are those it was saved for (`AiProviderConfig::same_endpoint_as`) | the same rule as a save: an edited address must not receive the old key ([I-03](../CLAUDE.md#i-03)) |
| An endpoint is never rewritten: a missing or repeated `/v1` is said in the message, not fixed | a silent rewrite would make the listed address differ from the stored one |
| Successes are kept in memory **10 minutes**, keyed by kind and endpoint — never by key, never with the URL's credentials or query; at most 32 endpoints. "Refresh models" and "Test connection" bypass it | a form reopened within minutes need not ask again; a failure is never kept, so a fixed key is tried at once |
| Bounds: **5 s** for a loopback endpoint, **20 s** otherwise; pages and entries bounded (`MAX_MODELS`, ten pages at most) | a local server that does not answer within seconds is not running; a paginating server cannot hold the call forever ([I-06](../CLAUDE.md#i-06)) |

A failure is data the form shows, with a reason it can act on: `unauthorized`
(`401` with a key), `missingKey` (no key, or a `401` without one),
`forbidden` (`403`), `unsupported` (`404`, `405`, another refusal),
`rateLimited` (`429`), `timeout`, `unreachable` (nothing answered, or a `5xx`),
`malformed` (an answer that is not a model list), `invalidEndpoint` (an address
that cannot be one, or a redirect). The message is a short English sentence;
it never quotes the key, nor the address, which may carry a password.

## Database content is not an instruction

A table name, a column comment, a row value can contain
text imitating an instruction. They are **data**, at every tier,
including when they arrive in a prompt. See
[SECURITY](SECURITY.md#input-surface).

## Mentions

An object the user names with an `@` in their question
([UX-SPEC](UX-SPEC.md#naming-an-object-with-)) is **forced** into the context. It
enters through the single gateway, and through no other path:
`ContextBuilder::with_mentions` (`crates/oxyn-ai/src/context/mentions.rs`).

What a mention sends, and what it does not send:

1. **The object's structure, under the connection's tier.** A table, a
   view, a collection is described like any relation of the context — fields,
   types, keys, indexes, framed comments —, **first**, before what the
   lexical search finds, and under the **same** budget. A column
   (`table.column`) has its relation described and designates the column by a
   "mentioned by the user" line. **No row value**: under
   `Metadata` as under `Sampled`, a mention adds nothing of what
   the approved sample carries. Naming is not sending.
2. **An address that is checked, never believed.** The webview sends a catalog
   address (and a column name), not a label. An object absent from the local
   cache, a column its description does not list, are **discarded**: nothing
   of them goes out, and their count is shown (`ignoredMentions`). Beyond sixteen
   mentions, the question is refused before it goes out.
3. **What does not fit is said.** A mention that exceeds the budget is not
   lost silently: its name appears in the framing, "mentioned by the user
   but not described, over the context budget", and the panel header
   counts it (`omittedMentions`).
4. **A saved query** — of this connection, or of none — goes out as
   the text the user wrote, framed and bounded to 4,000 characters. The
   backend reads it back through the bus (`Command::OpenDocument`), its
   **saved** version only; a query of another connection is discarded,
   because its literals are not those of this database.
5. **Every destination, through the same gate.** For a session that
   opens — provider or external agent —, mentions lead the initial
   context. For an **already open** session, which knows the schema since its
   opening and whose system message is not rewritten, the mentioned objects
   are rendered by the same `ContextBuilder`, in `mentioned_only` mode — without
   search, since the rest has been said —, and **precede the question** in the
   user's message, `untrusted` preamble included. The same text
   (`AgentContext::follow_up`) serves `AgentSession::ask_about` and
   `AgentPrompt::following`: an already launched external agent has no second
   path.
6. **What is kept.** The saved question keeps its mentions — kind,
   label, catalog address or document identifier — in a
   JSON column readable without Oxyn ([I-11](../CLAUDE.md#i-11)), bounded to sixteen mentions
   and 32 KiB. Names, **never a value** nor the text of a saved
   query. What is kept is used to redraw the chips, nothing else:
   a conversation read back sends nothing of it to the model.

## Approved sample

What each tier lets out is settled by
[ADR-0006](adr/0006-ai-privacy-tiers.md); who can trigger a sample and
which destinations receive it, by
[ADR-0034](adr/0034-echantillon-pour-toute-destination.md). This section describes
the only path through which **row values** reach a prompt, as
it is coded in `oxyn-desktop` (`backend/ai/samples.rs`,
`backend/ai/conversation/sampling.rs`). It holds for a built-in provider
as for an external agent.

1. **The user approves, always; the agent can ask, never
   approve.** Two ways of opening the same approval screen:
   - **the user pins** an object from the catalog menu ("Pin to
     question"). The action only exists under `Sampled`, when the panel has a
     usable destination — provider or external agent —, on an object that
     carries rows; elsewhere it is absent, not greyed out. A question carries
     a single pin; it drops as soon as the question goes out, or as soon as the
     tier or the recipient no longer allow it. The offer
     (`ai_request_sample`) carries the source name, the catalog columns,
     the row bound (5) and the recipient — **no value**;
   - **the agent asks**, through the `request_sample { relation, namespace?,
     columns?, rows? }` tool — 5 rows by default, **20 at most**. Outside `Sampled`,
     the call is refused before any screen. Otherwise the screen opens, names the agent
     that is asking, only offers the columns it named (all of them if it
     names none), and the call waits for the answer **five minutes at most**.
     Refusal, expiry or stopping the question: the agent receives "the user
     declined", without any value. The only possible answer is the Tauri command
     `ai_answer_sample`; the request identifier never goes to
     the model.

   In the screen, nothing is checked by default, nothing checks everything, and
   Cancel has the focus. Selecting columns requests a native host confirmation
   before any row is read ([ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)).
   Its recipient comes from the stored declaration serving the exchange, never
   from the screen's label: endpoint host for a provider, command for an agent.
   Refusal consumes the request and records no egress; a pinned question does
   not go out. A competing native dialog leaves the decision unconsumed.
2. **A backend token, single-use.** The offer issues a random token, bound
   to the connection, the conversation, the exchange it follows, the source, the
   proposed columns and the recipient the screen names: provider,
   model and network reach. It expires after ten minutes; it disappears
   when the user cancels the screen, when the conversation is deleted or
   the connection forgotten. A connection keeps at most four pending tokens:
   beyond that, the oldest is forgotten. The front end receives it without displaying it and
   sends it back. At the question, the backend checks, in this order:
   - the native dialog slot, reserved without consuming the token if another
     critical confirmation is open;
   - the token, removed as soon as the question can reserve that slot, **even
     on refusal**, including when the question is refused before starting;
   - the tier, **re-read from the store** and still `Sampled`;
   - the identical source;
   - the checked columns included in the proposed columns;
   - the recipient the screen named: the same built-in provider and the
     same model, whose address is **reclassified** and reaches no further
     than at the offer — or the same external agent, still `Unresolved`. A
     provider and an agent never exchange a token, even under the same
     identifier.

   An agent request follows the same order, without a token: tier re-read from the
   store, relation and columns looked up in the catalog, screen, answer,
   tier re-read. A malformed answer — column not proposed, no column —
   refuses the request. A connection keeps at most four pending requests.
   No screen opens while an approval from this agent is pending, and an
   exchange opens **only one**: after an approval, a refusal or an
   expiry, any new request in the same response is refused without a
   screen, by a message that does not say what the answer was.

   Each failure is a typed refusal, **before any read**. The read then goes
   through `Command::PreviewRelation` on the bus — audited as a read
   by the user for a pin, as an agent command
   (`Actor::Agent`, `PolicyGate`) for a request —, with the row bound
   set by the backend whatever the front end or the agent sends. The driver composes
   the read and quotes the relation; the columns enter no
   statement ([I-10](../CLAUDE.md#i-10)). The tier is **re-read after the read**: if it is no longer
   `Sampled`, nothing is sent, and it is this re-read tier that governs
   the prompt and the address check. Unchecked columns are
   discarded before the prompt is built, and the sample enters the
   context through `ContextBuilder`, like the rest. Just before sending, and
   only if this rendering kept the sample, the egress is recorded in
   `ai_egress`, linked to the preview that read the rows; if the record
   fails, nothing goes out. A sample discarded for the budget is neither recorded
   nor announced as sent
   ([SECURITY](SECURITY.md#what-goes-out-to-an-ai-recipient-leaves-a-trace)).
   No value appears in a trace or in an error
   message.
3. **A question, not a memory.** The exchange that carried the sample is
   not kept: it goes out with a fresh context, and the next question emits
   `memoryReset` for the reason `sampleNotKept`, with a line that says so. The
   conversation only keeps "N rows and K columns". Regenerating or editing
   the question does not reuse the approval: it must be given again. For an
   **external agent**, which is a process and remembers: a question with a
   pinned sample opens a new session, and every exchange that carried
   values **releases the process** at its end; the next question launches
   another one. What the agent may have written to its own disk escapes Oxyn
   ([ADR-0034 § 5](adr/0034-echantillon-pour-toute-destination.md#5-an-exchange-that-carried-a-sample-leaves-no-memory-whatever-the-destination)).
4. **Secret columns: no filter yet.** No driver declares a column
   classification yet. Rather than a filter on names that would suggest
   a protection, the offer carries a dated TODO, unblocked by that classification.
   Until then, what is proposed is what the catalog reports, and the only
   barrier is the box the user checks.

## Server error messages go through the tier

A database error message **quotes what it refuses**: PostgreSQL
answers `Key (email)=(dupont@example.com) already exists`, a SQLite trigger
concatenates the value that triggered it. It is row content, and it therefore
crosses the same filter as the rest: the criterion is the tier's — only
`Sampled` allows row values.

Under `Local` and `Metadata`, what reaches the model is what cannot quote
anything: the error's **family**, its retryability — derived from the family, not
declared next to it —, and the error identifier when it is marked as
such (`SQLSTATE 23505`, `Error code 1811`). The message explicitly tells the model
not to guess what it does not see, and to ask the user to read
the full error in Oxyn — which, for its part, sees it whole.

The filter applies **at construction**, not at rendering: under a tier that
masks, the message never enters the structure. A derived `Debug` or a
log field added later therefore cannot let it out — it
is no longer there.

What this choice does not protect, and which is accepted: a hostile server can
write `SQLSTATE ABCDE` in its message and thus leak five characters
of its choice per failure. The channel is bounded and known; refusing it would cost the
only identifier a professional actually uses to look up an
error.

## Tier persistence

The tier is **written with the connection** (`connections.privacy_tier`, migration
8): it belongs to it, so it survives with it. Without this column, a setting chosen
on a customer database was lost on closing, without a message, and the reopened
connection went back to the default.

Two fallbacks, and their asymmetry is the decision:

* **missing column** — the row comes from a binary that ignored this setting,
  so the user never chose one: the default of [ADR-0006](adr/0006-ai-privacy-tiers.md),
  `Metadata`, applies;
* **unreadable value** — a setting existed and its meaning was lost. It is
  not an absence: the tier falls back to the most restrictive, `Local`. A
  database of which we no longer know what it allowed does not get the benefit of the
  doubt, exactly like a connection with no environment set counts as
  `production`.

### Setting the tier

The tier is set **in the connection form**, at creation as on
edit, through the `AI privacy` field: three choices, `Local`, `Metadata`,
`Sampled`, and under them the list of what would go out and what would stay
on the machine, which follows the selection **before** any save. A
new connection starts from `Metadata`, the default
of [ADR-0006](adr/0006-ai-privacy-tiers.md).

The choice is only applied by saving the connection: the
edit goes through `UpdateConnection`, the creation through
`CreateConnection`, two command bus commands classified as DDL on the
targeted connection ([I-01](../CLAUDE.md#i-01)). No other path writes
`connections.privacy_tier`. The safeguards:

* **on a `production` connection**, saving requires the confirmation
  that names the connection ([I-02](../CLAUDE.md#i-02),
  [SECURITY](SECURITY.md#connection-marking)), like any write on
  it; for an `Actor::Agent`, it is a refusal;
* **lowering the tier of a `production` connection** — to `Sampled`, or from
  `Local` to `Metadata` — is a critical decision: it opens the egress of
  the structure or of rows of a customer database. It is meant for the **native
  host dialog**, outside the webview, decided on 2026-09-24 for
  critical confirmations. The ADR that writes it does not exist yet: until then,
  it is the webview confirmation described by
  [SECURITY](SECURITY.md#connection-marking) that applies, and it
  alone;
* **the change applies from the next question.** The tier is re-read by the
  backend at every question, never taken from the session; an external agent
  kept alive, launched under the old tier, is released on save and
  relaunched under the new one ([I-04](../CLAUDE.md#i-04)).

## No provider

Without a declared provider or external agent, the AI workspace is **absent from
the interface** and Oxyn remains a complete client
([ADR-0006](adr/0006-ai-privacy-tiers.md); the screen behavior is in
[UX-SPEC](UX-SPEC.md#the-ai-workspace-only-exists-if-it-has-been-configured)). A code path
that calls a model to produce a result the user expects to be
deterministic — a sort, formatting, a table name completion — is a
design defect, not a feature. The local embeddings of
[ADR-0056](adr/0056-local-cpu-embeddings-for-context-selection.md) do not
break this rule: they only choose and order what the AI context keeps, a
result no user sees as a list, and the catalog search, the tree and the `@`
list stay lexical.

## What is not settled yet

- presenting, before sending, the exact list of what will go out;
- displaying, for an external agent, what it **actually** asked and
  Oxyn refused: the refusal is counted and shown, the detail of the request is
  not, for lack of a screen that can render it without copying paths;
- context compaction on databases with several thousand tables, which
  [ADR-0006](adr/0006-ai-privacy-tiers.md) designates as a component in its
  own right.
