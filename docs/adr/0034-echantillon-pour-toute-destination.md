# ADR-0034 — An approved sample reaches any destination through the same gateway, and an agent can request one without ever approving it

**Status:** proposed · **Date:** 2026-09-24

**Clarifies:** [ADR-0006](0006-ai-privacy-tiers.md), on two points it did not
settle: **who** can trigger the `Sampled` sample, and **which destinations**
receive it. The tier table is unchanged.

**Complements:** [ADR-0030](0030-outils-oxyn-exposes-a-un-agent-externe.md), whose
tool list (§ 1), per-tier table (§ 4) and the sentence "an external agent's
prompt never carries a sample" (§ 4 bis) change with this ADR.

## Context

ADR-0006 promises, under `Sampled`, a "row sample explicitly approved, column by
column". What was delivered on 2026-09-23 held only one form of it:

* **a single destination** — the built-in provider. An external agent (Claude
  Code, Codex) was refused, on the grounds that its session outlives the
  question and that Oxyn "cannot mark an exchange there as memoryless";
* **a single trigger** — the user, who pins a relation from the catalog. No tool
  allowed an agent to request values, and tools return the shape of a result,
  never its values
  ([ADR-0030 § 4](0030-outils-oxyn-exposes-a-un-agent-externe.md)).

Consequence: on a connection the user had themselves set to `Sampled`, an
external agent could never see a value, and no agent could say "I need to see
how `status` is written" at the moment it needs it. The tier said "values may go
out, approved"; the product said "never to an agent, and only if you think of it
beforehand".

Three constraints frame the answer, and none is negotiable:

* **[I-04](../../CLAUDE.md#i-04)** — a single function brings content into a
  prompt, under the connection's tier: `ContextBuilder::build`. A second path
  "for agents" would be the path nobody reviews;
* **[I-01](../../CLAUDE.md#i-01), [I-07](../../CLAUDE.md#i-07)** — what an agent
  triggers is a `Command` carrying `Actor::Agent`, which goes through the
  `PolicyGate`, reads included;
* **memory** — a provider only has the session Oxyn hands it; an external agent
  is a **process**, and what it has read, it still knows at the next question.

## Decision

### 1. A single gateway returns values, for any destination

Row values reach a prompt **only** through `ContextBuilder::with_samples`, then
`build`, which drops them under any tier other than `Sampled`. Three callers,
one rendering:

| Destination | Through |
|---|---|
| Provider, pinned sample | the system message context (unchanged) |
| External agent, pinned sample | `AgentPrompt::with_schema(tier, question, cache, language, samples)` — the prompt that opens the session |
| Any destination, sample requested by the tool | `ToolOutcome::from_dispatch` on `DispatchOutcome::Sampled`, rendered by `ContextBuilder` with no described relation (`max_relations: 0`) and at the request's cap |

A sample that the gateway drops — lowered tier, budget exceeded — becomes a
**refusal** returned to the model, never an empty "done".

### 2. The `request_sample` tool, in the shared registry

`ToolRegistry::builtin` gains `request_sample { relation, namespace?, columns?,
rows? }`, granted to `sql_agent()` — hence to the internal loop **and** to the
MCP bridge, without a single line specific to either ([ADR-0030 § 1](0030-outils-oxyn-exposes-a-un-agent-externe.md)).

* **No new command.** The translation produces a `SampleAsk` that carries
  `Command::PreviewRelation` — the very read of the pinned sample —, bounded to
  `rows` (5 by default, **20 at most**), without order or predicate, and the
  requested columns (**64 at most**, names of **256 bytes at most**). The
  connection and the session come from the `ToolScope`: the agent does not name
  them, and `deny_unknown_fields` refuses any invented field.
* **Names stay data** ([I-10](../../CLAUDE.md#i-10)). The relation travels as a
  command field; the driver quotes it when composing the read; the columns join
  no statement — they designate what is copied from the result. Relation and
  columns are checked against the local catalog before any screen: an unknown
  name is refused, never guessed.
* **`CommandSink::request_sample`**, a method of the existing sink, whose
  default **refuses**. A method and not one more `dispatch`, because
  column-by-column approval is not a `PolicyGate` decision — it chooses what
  goes out, not what runs — and because the wanted columns are not in the
  command. It is not a second path to the executor: the approved read goes back
  through **the same** sink, with `Actor::Agent`, and the `PolicyGate` still
  decides.

### 3. The order, and nothing read before the user's answer

1. `oxyn-ai` refuses outside `Sampled` **before** soliciting the sink, under the
   loop's tier — re-read at call time for the MCP bridge (ADR-0030 § 4). The
   refusal states the tier and "do not ask again". No screen opens;
2. the sink (`oxyn-desktop`) **re-reads the tier from the store** — that of an
   internal question may have been lowered since it started;
3. relation and columns are looked up in the catalog;
4. the approval screen opens — **the same** as for pinning, which names the
   requesting agent — and the call waits, **five minutes at most**, released
   when the question stops;
5. refusal, expiry or stop: the agent receives "the user declined", and nothing
   is read;
6. approval: the tier is re-read, the exchange is marked memoryless, the read
   goes on the bus, only the checked columns are copied, the tier is re-read a
   third time, and the rows go back to `oxyn-ai`;
7. `ToolOutcome::from_dispatch` renders them. **Only if that rendering keeps
   them** is the egress recorded in `ai_egress` and the panel learns "sent" —
   if recording fails, nothing goes out. The sink does not record it itself: it
   hands the loop a receipt (`SampleReceipt`) that the loop releases once the
   rendering is known. A sample dropped for the budget is neither recorded nor
   announced, and the tool line states the refusal the model read.

Two bounds hold **in the sink**, hence for the internal loop as for the MCP
bridge, before any screen:

* **never while an approval from this agent is pending** — read in the
  executor, which holds the requests;
* **a single screen per exchange.** Approved, refused or expired, the first
  request exhausts the exchange: any following request is refused without a
  screen, by a constant message that does not say what the answer was. Without
  this bound, nothing limited repetition — the internal loop has no cap on calls
  per turn, the bridge has eight per question —, and a "no" asked again ends as
  a "yes" out of weariness.

On the MCP bridge, the request moreover goes through the same guard as a
command: one at a time, and only for the open question. A connection keeps at
most **four** pending requests.

### 4. An agent cannot approve itself

The only answer to a request is the Tauri command `ai_answer_sample` — a user
gesture in the screen. The request identifier only goes to the webview, never to
the model; nothing an agent sends carries an identifier or a decision, and the
tool arguments refuse any field they do not declare. A malformed answer —
column not offered, no column — **refuses** the request rather than leaving it
open to a second try.

### 5. An exchange that carried a sample leaves no memory, whatever the destination

* **Provider**: unchanged for pinning; a sample requested by the tool marks the
  exchange the same way, and its session is not kept;
* **external agent**: a question that carries a pinned sample **never**
  continues a session — it opens a new one, which receives structure and rows
  through `with_schema`. And any exchange that carried a sample, pinned or
  requested, **releases the process** at its end, whatever the outcome. The
  next question launches another process and emits `memoryReset` with the
  reason `sampleNotKept`.

Releasing also withdraws the approval requests this agent left pending
(`WithdrawOnRelease`): it is the rule of every release. A write proposed in the
same response as a sample is not one: its tool call waits for the user's
decision, and the response only ends — hence the process is only released —
once the request is decided, expired or withdrawn.

*Clarified on 2026-09-24.* The first version said the write was "to be asked
again": the call returned "awaiting approval" immediately, the response ended,
and the release withdrew the request **without saying so** — the card still
offered "Review…", and approval answered "no command is awaiting approval under
this identifier". The wait in the call and the card that states the withdrawal
are described in
`crates/oxyn-desktop/src/backend/ai/conversation/decisions.rs`.

**The option examined and rejected**: opening a new ACP session (`session/new`)
in the same process. It erases the **protocol's** history, not the
**process's** state: what the adapter keeps in memory or cache is not
observable, and the guarantee would depend on each agent's implementation.
Killing the process is the only option whose guarantee depends only on Oxyn.

**Accepted limit**, written so as not to be promised: what an agent writes to
**its own disk** — a session history, a log — escapes Oxyn, released or not.
It is the limit that [AI-PROVIDERS](../AI-PROVIDERS.md) already names for this
mode: Oxyn can name what goes to the agent, it cannot guarantee what the agent
does with it. `Local` stays closed to external agents for this reason.

## Consequences

* **+** Every destination receives approved values through **one** function:
  the question "what went out?" is answered by re-reading
  `ContextBuilder::build` and `ai_egress`, not each caller.
* **+** An agent that needs to see a value can say so when it needs it, and the
  user decides in the screen they know.
* **+** No new command, screen or tier rule: the read is `PreviewRelation`, the
  screen is the pinning one, the refusal outside `Sampled` is
  `allows_row_values`.
* **−** A consent screen can now open **without the user having caused it** —
  the case most exposed to the reflex click. What bounds it: nothing is
  checked, the screen names who is asking and says that Cancel refuses, Cancel
  has the focus, Enter sends nothing, a single screen per exchange, never while
  a write is pending.
* **−** An exchange with a sample costs the agent's session: the next question
  relaunches the process — several tens of seconds at the first launch of an
  adapter served by `npx` — and the agent has lost the thread.
* **−** Under `Metadata`, the tool is announced and refuses. A model may try it
  once; the refusal tells it not to ask again.
* **−** The frozen list of tools served to external agents changes: the
  bridge's security review that ADR-0030 § 1 requires is **due**.

**Exit cost:** low. Removing `request_sample` from the registry and from
`sql_agent()` removes the "requested" part without touching the rest; removing
the `samples` parameter of `with_schema` brings the external agent back to
structure only. No persisted format changes: `ai_egress` already recorded one
recipient per declaration identifier.

**Reconsider if** the ACP protocol gains a **verifiable** way to forget a
session (the release would become unnecessary); if a driver declares a
classification of secret columns (they would leave the offer, as the TODO of
`ai_request_sample` plans); or if usage shows approvals given without reading —
the answer would then be to remove requests by the agent, not to make the
screen heavier.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| A `Command::RequestSample` that the `PolicyGate` classifies "to approve" | Generic approval approves a whole command, without checking any column, and hands control back to the agent immediately ("awaiting approval"): there would be neither column-by-column selection nor a value to return to it after the approval |
| Return the values of `execute_query` under `Sampled` | An arbitrary query is not a sample approved column by column; it would be the second tier rule that [ADR-0030 § 4](0030-outils-oxyn-exposes-a-un-agent-externe.md) refuses |
| Pre-check the columns the agent requests | A box checked by the agent is a consent the user did not give; the requested columns **restrict the offer**, they check nothing |
| Keep the external agent out of `Sampled` | That is the state that motivated this ADR: a tier the user chose and that a whole destination ignores |
| New ACP session in the same process | Erases the protocol's history, not the process's state; the guarantee would depend on each adapter (§ 5) |
| A tool API specific to the MCP bridge for samples | Two tool sets diverge, and it is the one nobody reviews that the agent takes ([ADR-0030 § 1](0030-outils-oxyn-exposes-a-un-agent-externe.md)) |
