# ADR-0030 — An external agent reaches the database through Oxyn's tools, served over MCP, and through nothing else

**Status:** proposed · **Date:** 2026-09-16

**Clarifies:** [ADR-0026](0026-agents-externes-acp.md), which declared an
external agent and talked to it, without ever giving it the means to read the
database.

## Context

[ADR-0026](0026-agents-externes-acp.md) set up the agent mode: the user
launches Claude Code or Codex from Oxyn, the agent carries its own
authentication, no key is entrusted to us. What that ADR did not address,
because it looked at the protocol's boundary and not the product's:
**the agent cannot query the database**.

The result is an agent that, in a database workbench, can say nothing about
the database. It answers about SQL in general, not about the schema open in
front of the user. The feedback is unequivocal: "the chat and the agents do not
talk with the databases".

The internal assistant, for its part, manages it: the model asks for a tool,
`oxyn-ai` translates the call into a `Command` carrying `Actor::Agent`, the
`PolicyGate` decides, the bus executes, and the result goes back to the model
**framed**. The whole mechanism exists. What is missing is not a capability: it
is a **transport** between the agent, which lives in another process, and that
mechanism.

The protocol offers one, and it is precisely the one agents already know how
to speak: **MCP**. `agent-client-protocol` 2.1.0 makes it possible to declare
MCP servers when opening an ACP session. The agent sees tools there as it sees
them everywhere else; Oxyn sees its own registry there.

The transport remains to be chosen, and that is where measurement corrected
intuition. The crate offers an `Acp` variant that carries the server **in
memory**, without a process or a port — obviously the most desirable. It is
unusable: it depends on a capability that **neither Claude Agent 0.78.0 nor
Codex 1.12.0 announce** (Codex even answers `"acp": false`), and it lives behind
an unstable feature. The survey is dated in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#exposing-oxyns-tools-to-an-external-agent--check-of-2026-09-16).
The transports actually available are `stdio` — mandatory for every agent
— and `http`, which both accept.

**What makes the decision expensive to undo**, and hence an ADR: serving tools
to a third-party process creates a **second entrance** to the command bus. An
entrance, once open, is never re-audited like the first — it is exactly what
[I-01](../../CLAUDE.md#i-01) describes. The shape we give it today is the one we
will keep.

## Decision

### 1. The MCP server only exposes the `ToolRegistry`, as is

The tools served to the external agent are **exactly** those the internal
assistant receives from `oxyn-ai`'s `ToolRegistry` — today `execute_query`,
`describe_schema` (§ 4 bis) and `request_sample` (§ 4 ter) —
with their JSON argument schemas, produced by the same code.

**No tool is written for the external agent.** A tool that existed only there
would be, by construction, a path the internal assistant never takes and
nobody reviews. Adding a tool to external agents means adding it to the
registry, hence to both destinations, hence under the same review.

The flip side: a tool added for the internal assistant also reaches, by
default, every external agent. The announced list is therefore **frozen by a
test**, which fails first. Updating it calls for a security review of the
bridge, not a touch-up of the expected list.

### 2. Each call becomes a `Command` carrying `Actor::Agent`

An MCP tool call follows the internal assistant's path, without variant:
`ToolRegistry::translate` produces the `Command`, the sink submits it with
`Actor::Agent`, the `PolicyGate` decides ([I-01](../../CLAUDE.md#i-01),
[I-07](../../CLAUDE.md#i-07)).

Consequences, which are not additions but inherited properties:

* a write on a `production` connection is a **refusal**, not a stronger
  confirmation ([I-02](../../CLAUDE.md#i-02));
* a write elsewhere requires **human approval**, displayed with the exact SQL,
  the connection name, the environment and the actor;
* stopping the current question cancels the current call.

### 2 bis. A call answers the current question, and no other

An agent's session outlives a question: it serves the next one. What ties a
call — the panel node where it is displayed, the "Stop" button that reaches
it, the number of calls allowed — does **not** outlive it. Each question
**opens a turn**, and closing it removes the tools:

* **outside a current question, nothing executes.** The user is not watching;
  a confirmation asked of someone who is not watching is clicked by reflex;
* **a cap on calls per question**, the one of the internal loop
  (`max_turns`): "the same path" also means the same bounds;
* **a single write submitted for approval per question.** The next one is
  refused until the following question: ten requests in a row is how the tenth
  gets approved without being read.

The first version tied these three things to the question that had
**launched** the agent. A stopped first question left all following calls going
out already cancelled; "Stop" on a following question reached nothing; and
between two questions, the agent could call the tools without limit, out of the
user's sight. Nothing failed.

### 3. The scope comes from the host, never from the agent

The `ToolScope` — connection, session, language — is built by Oxyn when the
conversation opens, from what the user themselves opened. The agent does not
see it and cannot propose it.

It is not a check added to the bridge: it is the shape of the schema.
`ExecuteQueryArgs` carries **only** `statement`. There is no field in which to
write another connection, a "read_only: false", or a declared intent. An agent
that tried to add one is refused by `deny_unknown_fields`.

### 4. The privacy tier governs through the rendered result, not through a second rule

This is the point on which we first concluded wrongly, and the correction
deserves to be written.

The initial intuition was: "`execute_query` returns rows, so it must be
refused to a remote agent under `Metadata`". It rests on a false premise. What
a tool returns to the model, **on both sides**, is `ToolOutcome::render()`, and
for a successful execution that text is the **shape** of the result —
`N rows, M batches` — never the values. Row values only reach a prompt through
the context, which `ContextBuilder` already governs with
`PrivacyTier::allows_row_values` ([ADR-0006](0006-ai-privacy-tiers.md),
[I-04](../../CLAUDE.md#i-04)).

**Decision:** the MCP bridge returns the **same** `ToolOutcome::render()` as the
internal loop — same summary, same failure report redacted according to the
tier, same `untrusted` framing. There is therefore **no** tier rule specific to
external agents:

| Tier | External agent | `execute_query` | `describe_schema` | `refresh_catalog` | `request_sample` |
|---|---|---|---|---|---|
| `Local` | forbidden | — | — | — | — |
| `Metadata` | allowed | allowed, returns the shape | allowed, returns the structure | allowed | refused before any screen |
| `Sampled` | allowed | allowed, returns the shape | allowed, returns the structure, no value | allowed | the columns the user ticks, after their consent (§ 4 ter) |

`request_sample` is not a tier rule specific to external agents: the refusal
outside `Sampled` is `allows_row_values`, applied in the common path of both
destinations, and values only go out through `ContextBuilder::build`.

`Local` forbids the external agent **before the process is launched**: that is
ADR-0026, unchanged.

**The tier is reread on every call, not copied at launch.** An agent's process
outlives the question that launched it; the tier, for its part, is attached to
the connection ([I-04](../../CLAUDE.md#i-04)). Each `tools/call` therefore
rereads the saved tier — the same source as `ai_ask` before a question — and a
call under `Local`, or on a connection that disappeared, is refused without
executing anything. In addition, a connection's agents are **released** when it
is modified, deleted or closed, and when a question on it is refused: the next
question relaunches the agent under what then holds.

Writing a second rule would have been the real mistake: two tier rules diverge
the day one of them changes, and diverge silently.

### 4 bis. The database structure: one tool for every destination

*Added on 2026-09-23, based on a finding.* The first version of this ADR
claimed that the external agent "reads the schema". That was false: it only
received the question, and no tool returned the structure to it. On a SQLite
database, an agent asked for "the last 10 rows" ran
`SELECT name FROM sqlite_master`, received `11 rows` — the shape, never the
values, as § 4 wants —, and proposed `SELECT * FROM your_table`. The
description of `execute_query` invited it to: it said "Reads return rows".

The instruction that settled it: access to the structure is **the same for
every destination** — built-in provider, Claude Code, Codex, any future agent —,
and adding a destination requires no schema-specific code.

**Decision:**

* **A registry tool, `describe_schema`**, which both the internal loop and the
  MCP bridge expose. It translates into a new `Command`,
  **`DescribeCatalog { connection, focus }`**, which carries `Actor::Agent` and
  goes through the `PolicyGate` like any other (§ 2). It is a read of the local
  cache: it contacts no server, is not mutating, and the `PolicyGate` allows it
  everywhere, `production` included. `focus` carries the agent's search words,
  bounded to 256 bytes — words to rank names, never query text
  ([I-10](../../CLAUDE.md#i-10)).
* **The executor returns the cache, not a rendering.** It hands the catalog
  handle (`CatalogHandle`) in its report; `oxyn-ai` renders it, in
  `ToolOutcome::from_dispatch`, through **`ContextBuilder::build`** — the same
  function as a prompt's context, under the tier **reread at the call** (§ 4),
  with the same budget and the same `untrusted` frame. There is no second
  rendering of the schema ([I-04](../../CLAUDE.md#i-04)).
* **The rendering is the same for every database.** It only knows the common
  model of `oxyn-catalog`: path, object kind (`table`, `collection`, `index`,
  `key_pattern`, `node_label`…), fields and subfields with their types **as the
  driver names them**, inferred fields, indexes, foreign keys. It states the
  connection's query language at the top. Names are quoted as the SQL dialect
  quotes them, or, outside SQL, as JSON literals. A driver that fills the
  catalog is covered without one more line in `oxyn-ai`.
* **Both destinations also keep an opening context**, rendered by the same
  function: the internal assistant in its system message, the external agent in
  the prompt that opens its session (`AgentPrompt::with_schema`). Three reasons
  justify it: the approved sample only enters through that context, a small
  local model struggles to call a tool, and the first answer does not wait for a
  round trip. The tool serves what that context left out of budget.
  *Corrected on 2026-09-24:* this version said an external agent's prompt never
  carries a sample, for lack of a way to mark an exchange as memoryless. It now
  carries one, pinned and approved, through the same `ContextBuilder`; memory is
  handled by releasing the process after the exchange (§ 4 ter).
* **What depends on the destination is derived, not copied.** The MCP server's
  name is a constant (`mcp::SERVER_NAME`) from which derive the ACP declaration,
  Claude's permission rule (`mcp__oxyn`, which covers all the server's tools)
  and Codex's approval, set on the whole server. A test checks that each
  registry tool passes both confinements without being named there.
* the description of `execute_query` now says what the tool returns: the shape
  of the result, never its values, and points to `describe_schema`.

The list frozen by the test of § 1 becomes `execute_query` and `describe_schema`.
This change **calls for the security review of the bridge** that § 1 requires.

**Accepted limit.** A `describe_schema` response follows the context budget:
twenty-four relations, about six thousand tokens. The agent browses a large
schema by search words, not by pages. And the common catalog knows neither view
definitions nor the values of enumerated fields: what it does not know stays
absent.

### 4 ter. Approved values, for every destination

*Added on 2026-09-24.* Under `Sampled`, an agent — internal or external — receives
the values the user approves column by column: pinned with the question, or
requested through the `request_sample` tool. The decision, its bounds and its
limits are in [ADR-0034](0034-echantillon-pour-toute-destination.md); what
concerns the bridge:

* the tool comes from the registry and goes through `run_tool_call`, like the
  others; no line of the bridge is specific to it;
* the request passes the same guard as a command (`WriteGate`): one at a time,
  never while an approval of this agent is pending, only for the open
  question — and an answer that arrives after the question is closed is
  thrown away;
* the agent cannot approve itself: the answer comes from the Tauri command
  `ai_answer_sample`, and the request's identifier never crosses the bridge;
* an exchange that carried values releases the agent's process; the next
  question launches another.

The list frozen by the test of § 1 becomes `execute_query`, `describe_schema` and
`request_sample`. This change **calls for the security review of the bridge** that
§ 1 requires.

### 5. The transport is `http` on the loopback, with one token per conversation

Since `Acp` does not exist in real agents, `stdio` and `http` remain.

`stdio` places the MCP server in a subprocess launched **by the agent**, not by
us. That subprocess would have to reach the running Oxyn — the only one that
holds the executor, the `PolicyGate` and the approvals — through an IPC of our
own invention. We would therefore write a second binary mode and a relay
protocol to obtain the surface `http` already gives us.

**Decision:** Oxyn serves MCP over **HTTP on `127.0.0.1`**, and:

* the port is **drawn at random**, never fixed, never written;
* listening is bound to the loopback, **never** to another interface;
* each conversation receives a **bearer token** drawn when it opens, passed to
  the agent in the `headers` of `McpServerHttp`; a request without that exact
  token is refused without revealing anything;
* the token is **neither persisted, nor logged, nor displayed**
  ([I-03](../../CLAUDE.md#i-03));
* listening **is born with the conversation and dies with it**, and the
  connections already accepted **with it**: outside a conversation with an
  external agent, Oxyn listens to nothing and serves nothing. Stopping only the
  listener is not enough — a connection kept open (*keep-alive*) would keep
  reaching the bus after the end, for a process that left the agent's group.

And because it is a network surface, four rules that are not options:

* **the token is required on *all* requests, `initialize` included.** A
  handshake left open is an enumeration point: it tells whoever knocks that
  Oxyn is listening, and what it serves;
* **the comparison is constant-time.** A comparison that stops at the first
  differing byte can be measured, and a token can be guessed byte by byte;
* **`Origin` and `Host` are checked**, and anything that is not the loopback is
  refused. Without that, a page opened in the user's browser can make a name
  resolve to `127.0.0.1` and talk to this port — it is *DNS rebinding*, and the
  firewall can do nothing about it;
* **a refusal says nothing, and it is intended.** Missing token, wrong token,
  foreign origin, `Host` outside the loopback, wrong path, wrong method: **the
  same response**, same status, empty body. A message that distinguishes causes
  teaches the caller what it almost achieved — "invalid token" confirms that
  the origin is accepted.

  This choice makes debugging less comfortable, and that is precisely why it
  is written here: **do not "improve" it by making the messages more precise.**
  Whoever needs to understand a refusal reads the server's tests, not the HTTP
  response.

Two more rules, for what a local program can do **without** the token:

* **everything is bounded before authentication**: a few connections at a time
  (beyond that, closed on accept), a timeout on the headers — which also bounds
  an idle connection —, a timeout on the body. Without these bounds, any
  program exhausts Oxyn's descriptors, and no database connects any more, no
  draft is saved any more.

  **Accepted limit.** Sixteen connections at a time, closed on accept beyond
  that: a local program can deprive the agent of its tools, but no longer
  Oxyn of its descriptors. We accept it: an agent out of tools is visible in the
  panel, an Oxyn without descriptors loses the user's work. The values are fixed by a test: without a timer, `hyper`
  ignores its timeouts silently;
* **the ACP crate is capped at `error` in the logs, whatever `OXYN_LOG` says.**
  It logs every whole message at `debug` — the token of `session/new`, the
  questions of `session/prompt` —, that is exactly what a user is asked to
  enable for a bug report; and its `warn` lines copy what the agent sent, since
  a serde "invalid type" error quotes the offending string. The cap applies to
  the **target**, not to a level: a version upgrade that moves the same trace
  stays covered. The diagnosis is not lost: where Oxyn receives a protocol
  error, it writes **its own** `warn` line — the method and the code, never the
  agent's text. The token is also redacted from what the agent writes back —
  error message, `stderr`, before the latter is truncated.

We write it as a surface rather than as an implementation detail: it calls for
a security review on every change of the bridge.

### 6. The agent is launched by Oxyn, not by the crate

`AcpAgent` can launch the agent, and we do not use it. The reason fits in one
line of its sources: `agent_client_protocol/src/acp_agent.rs:263` does
`std_cmd.envs(&self.config.env)` — an **addition**. There is neither
`env_clear` nor `current_dir`, and `AcpAgentConfig` keeps its three fields
private. Through that path, Claude Code starts with **all** of Oxyn's
environment: the user's cloud credentials, their database URLs, what their
shell exported. That is [I-03](../../CLAUDE.md#i-03), and an invariant is not
traded against the hours a hand-written launch costs.

**Decision:** Oxyn builds the command itself, with

* an **allowlist** environment — what the user confirmed, plus
  `PATH`, `HOME`, `USER`, `LANG`, `TMPDIR`, each justified where it is
  written. A denylist forgets the variable added six months later;
* an **explicit working directory**, empty, created for this agent and
  destroyed with it — never Oxyn's workspace, where the files that describe the
  connections live. This directory is also the one **announced to the ACP
  session**, and not the shared temporary directory: an agent loads there the
  instructions and MCP servers of its "project", and any user program can write
  to `$TMPDIR`. It is created with `0o700`, under a random name, **never adopted
  if it already exists, and there is no fallback**: a fallback to `$TMPDIR`
  itself once caused the user's whole temporary directory to be erased at the
  end of a conversation on a full disk;
* a guard that kills the process **group**: an agent distributed behind `npx`
  otherwise reattaches to pid 1 and survives closing the window, our tools still
  open;
* an exit code and a tail of `stderr` as **typed data**, redacted of
  everything Oxyn passed to the child: a process's `stderr` is an I-03 channel
  like any other, and a launcher that fails prints its environment.

What the crate did for free is therefore rewritten here. It is the price of the
invariant, and it is written so that nobody "simplifies" by going back to
`AcpAgentConfig`.

**And the allowlist has its own price, which must be written rather than
discovered.** A denylist forgets what will be added; an allowlist forgets what
one did not know. The difference is in the failure mode: **a missing variable
does not break the agent, it degrades it silently**.

The case happened even before the first delivery. Without `USER`, Claude Code
declares itself "Not logged in" on a machine where it is logged in — and
`initialize` **still succeeds**, with an empty `authMethods`. The handshake
looks healthy; the refusal comes at the first prompt. The user asks a question
and gets told "log in". Nothing failed, something simply stopped working.

We keep the allowlist, because returning the whole environment to avoid this
failure mode would also return the secrets. But the rule that goes with it is a
consequence of that price: **each name carries the measurement that put it
there**, a name is never added "just in case", and a name is not removed
without redoing the measurement. The measurement itself is dated in
[RESEARCH-NOTES](../RESEARCH-NOTES.md).

### 7. An absence of answer counts as a refusal

A permission request Oxyn cannot satisfy is not left pending: it is
**refused**. A client that does not answer leaves the agent waiting, and an
agent that waits looks broken; worse, a rework that "fixed" the waiting by
granting by default would reverse the rule without anything failing.

The default is therefore written, and tested, on both sides: in the code that
answers, and in this ADR so that the day someone finds it annoying, they know it
was intended.

### 8. What this choice does not close, and that must be named

The row count is a channel. `SELECT 1 FROM clients WHERE email = '…'`
returns `1 rows` or `0 rows`: one bit on a precise value, per turn.

We write it rather than keep quiet about it, and we do not address it here, for
three reasons. This channel is **identical for the internal assistant** — it is
not an opening of the agent mode. It is **bounded** by the cap on calls per
question, the same on both sides (§ 2 bis), and by the fact that the user sees
each statement go by in the panel. And closing it would require hiding from the
model whether its query returned anything, that is removing the only feedback
that lets it correct a wrong query.

If we decide one day to close it, it will be for both destinations at once,
and it will be another ADR.

## Consequences

* **+** The external agent becomes useful in a database workbench: it reads the
  database structure through the same tool as the internal assistant (§ 4 bis)
  and queries the database the user opened.
* **+** No new surface to the bus: the bridge translates to the same
  `ToolRegistry` and the same sink. What is reviewed once holds for both
  destinations.
* **+** The user sees what the agent does in the assistant's panel —
  calls, approvals, refusals — with the states already in place.
* **−** A second entrance now exists, even if it leads to the same corridor. It
  requires its own security review on every change of the bridge.
* **−** The agent learns the **shape** of results, and hence a little of the
  database, even under `Metadata`. It is already true of the internal
  assistant, and it is the price of an agent that can correct its query.
* **−** The bridge depends on how `agent-client-protocol` declares MCP servers.
  An API change of the crate will be paid here.

**Exit cost:** low. Removing the bridge makes external agents mute about the
database, without touching the internal assistant or the registry.

**Reconsider if** an external agent one day obtains the right to trigger
anything other than a registry tool — it is the line of ADR-0026, and it holds
here word for word. **Also reconsider** the day the adapters announce
`mcpCapabilities.acp`: the server would become internal to the session again,
and the network surface would disappear.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Leave external agents without tools | It is today's state, and it makes the agent mode useless: an agent that cannot read the database is of no use in a database workbench |
| Refuse `execute_query` under `Metadata` for a remote agent | Rests on a false premise: the tool returns the shape of the result, not the values. The rule would have forbidden the main use while believing it protected something the summary does not expose |
| Return to the agent a result reduced "to the row count and types" | It is already what `ToolOutcome` returns. Stating it as a distinct rule would have created the second tier rule this ADR refuses |
| Give the agent direct access to the driver, or a connection of its own | Bypasses the bus, the `PolicyGate` and human approval. It is precisely what [I-01](../../CLAUDE.md#i-01) forbids, and the second path would never be audited like the first |
| Write MCP tools specific to external agents, richer than the registry | Two tool sets diverge. The one nobody reviews is the one the agent will take |
| Carry the MCP server in memory, through the session's `Acp` variant | The most desirable, and measured unusable: neither Claude Agent 0.78.0 nor Codex 1.12.0 announce the capability, Codex answers `"acp": false`. To revisit the day they announce it |
| Serve MCP over `stdio` | The subprocess is launched by the agent, not by us: it would have to reach the live Oxyn through an IPC of our own invention. A second binary mode and a relay protocol, for the same surface as `http` |
