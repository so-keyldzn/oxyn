# ADR-0026 — An external agent speaks ACP, entrusts no key, and stays out of reach of a `Local` connection

**Status:** accepted · **Date:** 2026-09-14

**Clarifies:** [ADR-0023](0023-fournisseurs-declares-et-provenance.md), which
knew only one mode — an API provider declared with its key.

## Context

[ADR-0023](0023-fournisseurs-declares-et-provenance.md) assumes a provider is
declared with a **key**, stored in the keychain, and that Oxyn talks directly
to the model. It is the only mode `oxyn-llm` knows: seven OpenAI-compatible
providers, plus Anthropic and Gemini.

This mode has a cost nobody discusses because it seems unavoidable:
**Oxyn holds a user secret**. All the work of [I-03](../../CLAUDE.md#i-03)
— six channels to watch, no `#[derive(Debug)]` on a secret carrier,
`ApiKey` that is never displayed — exists to protect that secret. Yet the only
sure way not to disclose a key is **not to have it**.

There is a second mode, and it is already proven elsewhere. The facts, checked
and dated in [RESEARCH-NOTES](../RESEARCH-NOTES.md#agent-client-protocol--check-of-2026-09-14):

* the **Agent Client Protocol** is JSON-RPC over `stdio`; a local agent is a
  **child process** of the host application;
* `agent-client-protocol` **2.1.0** (crates.io, 2026-09-04) is under
  **Apache-2.0**, declares `rust-version` 1.88.0 and edition 2024;
* a client editor declares an external agent with a command, arguments and an
  environment, and its documentation is explicit: **no API key is
  required** — the agent carries its own authentication, and billing as well
  as data retention concern the user and the agent's provider, not the editor.

In other words: the user already has Claude Code or Gemini CLI installed and
authenticated. Asking them in addition for an API key for Oxyn means asking
them to pay twice and to entrust us with one more secret.

**What makes the decision expensive to undo**, and hence an ADR: the protocol
places the tool authorization request **on the client side**. If Oxyn becomes
an ACP client, that entry point becomes a permanent security boundary. Wiring
it in the wrong place once is paid for a long time.

## Decision

**Oxyn accepts two provider modes, and an external agent never brings a
key.**

### The external agent mode

Oxyn is the ACP **Client**. An agent is declared as a command to launch —
program, arguments, environment — in the same place as the providers of
ADR-0023, of which it is a variant and not a parallel system. No secret
reference is associated with it: the field does not exist for it.

### Two authorization domains, not one

**This section corrects the first draft of this ADR**, which claimed that
`session/request_permission` "is" the entry point of the `PolicyGate`. Reading
the protocol, done while implementing, showed this was wrong — and the
correction is worth writing, because the error was tempting.

The authorization request carries a `tool_call` describing a tool **of the
agent** (`ToolKind::Read`, `Edit`, `Delete`, `Move`, `Execute`, `Fetch`,
`Search`, `Think`, `SwitchMode`, `Other`): read a file, edit one, run a
command. These are not Oxyn `Command`s, and no translation exists.
"The agent wants to edit `/etc/hosts`" does not become a database command.

There are therefore **two domains**, and confusing them would have produced a
lopsided bridge:

| What the agent asks | Who decides |
|---|---|
| …**of Oxyn** — execute a query, refresh the catalog | the `PolicyGate`, through the existing tool registry, with `Actor::Agent` — unchanged, [I-01](../../CLAUDE.md#i-01) and [I-07](../../CLAUDE.md#i-07) held as before |
| …**of the machine** — files, commands, network | `oxyn_ai::external::permission_for`, **refusal by default** |

**Oxyn is not a code agent host**, and that is what decides the second
domain. Nothing in the scope of a database workbench justifies granting a
subprocess the right to write files or run commands. Above all, Oxyn has **no
screen to show which** file or **which** command is at stake: a confirmation
that does not say what it authorizes shifts responsibility without giving the
means to exercise it, and [I-02](../../CLAUDE.md#i-02) already says a
confirmation ends up being clicked.

Only `Think` and `SwitchMode` are granted: they do not leave the agent.
Everything else is refused outright, with a reason — constant, hence unable to
copy a file path — sent back to the agent so it stops insisting. A kind unknown
to the protocol is refused too: granting what one cannot judge means granting
what the protocol will invent.

A code editor grants these rights because its scope makes them sensible and
it knows how to show them. Copying that choice with neither would have opened
access to the system behind a database window.

### An external agent counts as `Reach::Unresolved`, always

**Oxyn cannot know where an external agent's model goes.** The agent is an
opaque process; it can talk to a local model, to a remote service, or change
its mind between two turns. It is not stale information like the `Reach` of a
declared provider ([ADR-0023](0023-fournisseurs-declares-et-provenance.md)):
it is **unknowable** information.

The repository already has a doctrine for this, and there is nothing to invent:
`allows_endpoint(PrivacyTier::Local, Reach::Unresolved)` returns `false`, under
the comment "when in doubt, protect" (`oxyn-ai/src/privacy.rs`). An external
agent is therefore filed under `Reach::Unresolved`, and the consequence follows
on its own:

| Connection tier | External agent |
|---|---|
| `Local` | **refused** — the promise "nothing leaves the machine" cannot be kept by a process whose output one cannot see |
| `Metadata` *(default)* | allowed, and the single gateway applies as for a provider |
| `Sampled` | allowed, sample approved column by column as elsewhere |

The refusal is stated on screen, with its reason, rather than greying out an
entry without explanation — it is what the connection bar already requires for
`Ask AI`.

### What does not change

The single gateway of [I-04](../../CLAUDE.md#i-04) remains the single
gateway: what goes to an external agent goes through the same filter as what
goes to a provider. An external agent is a **destination**, not an exemption.
`oxyn-llm` keeps its scope — talking to a model — and learns nothing about ACP;
the agent mode lives in `oxyn-ai`, which already knows the tiers.

## What is already written

The decision is implemented in slices, and the first is the one that carries
the invariant — not the transport:

* `oxyn_core::ExternalAgentConfig` declares an agent: identifier, name,
  command, arguments, environment. **It has no secret field**, and that is the
  point. It is **not** an `AiProviderConfig`: going into it would have produced
  a structure half of whose fields mean nothing depending on the variant;
* its `Debug` is **manual** and only renders the *number* of environment
  variables. The field must not carry a secret, its documentation says so — but
  it is an instruction to the user, not a guarantee, and a `tracing::debug!`
  added later must not put it to the test ([I-03](../../CLAUDE.md#i-03));
* `validate` refuses an empty name or command, a control character in the
  command or an argument, and bounds both lists — a state file written by a
  third party must not cause allocation on opening;
* `oxyn_ai::privacy::agent_reach` and `allows_external_agent` carry the privacy
  consequence, **in `oxyn-ai`**: `oxyn-core` does not know `Reach`, and
  depending on it would reverse the direction of dependencies. Held by
  `an_external_agent_never_serves_a_local_connection`.

* `oxyn_ai::external::permission_for` decides the second authorization domain,
  kind by kind, refusal by default. Three tests, including
  `no_system_access_is_granted`, which enumerates the kinds rather than
  treating them as a block — adding a variant to the protocol must not relax the
  guarantee silently.

* `oxyn_ai::external::launch_config` translates a declaration into a launch
  configuration, **without ever gluing** the command and its arguments back into
  a string.

The protocol's example starts from a single string — `"python my_agent.py"` —
that it splits. Splitting a command line is a grammar, hence a surface: a path
containing a space, a quote or a semicolon takes on a meaning nobody intended.
`AcpAgentConfig::new(command).args(…)` passes the parts separately, and that is
the path taken. Held by `the_command_and_its_arguments_are_never_glued_back`,
which passes `/opt/mes agents/claude code` and the argument `; rm -rf /` and
checks that they arrive intact and distinct.

Validation is **redone** at launch, not only on input: a declaration may come
from a state file written by a third party or by a future version.

The `agent-client-protocol` dependency is declared since these modules use it.

* `oxyn_ai::external::option_for` chooses the response option that expresses
  the verdict, with a **deliberate asymmetry**: never "always allow", but
  "always reject" as soon as the agent offers it.

Remembering a broad grant is a decision the user did not make and that nothing
on screen would show them. Remembering a refusal is not one: what Oxyn refuses,
it will refuse every time — it is a property of the product, not a mood —, and
letting the agent ask again on every turn would make it lose its own in front of
a user who would not understand why the conversation goes in circles. Failing a
suitable option, the answer is `Cancelled`: pretending to allow by selecting a
reject option, or the reverse, would be worse than interrupting. Held by three
tests, including `no_suitable_option_is_replaced_by_its_opposite`.

* `oxyn_ai::external::turn::run_turn` wires the complete turn: tier check,
  launch, `InitializeRequest` → `NewSessionRequest` → `PromptRequest`, chunks
  passed up through the **existing** `AgentObserver`.

Three choices of this module deserve to be written, because they do not follow
from the protocol:

* **the tier is checked before launch**, not before sending. Starting the
  agent and then refusing to talk to it would already be too late: merely
  launching it may be enough to make it contact its service;
* **the announced working directory is a temporary one**, not the user's. The
  agent has no authorization to touch the disk anyway, but *announcing* a
  project directory would make it name real paths in its requests, hence in
  what is displayed. Not giving them is simpler than filtering them afterwards;
* **the protocol error does not go up as is.** Its message may copy the prompt
  ([I-03](../../CLAUDE.md#i-03)); what goes up is the fact, not the text.

Only text chunks are passed up to the interface. A `plan` or a `tool_call` from
an external agent describes work Oxyn did not authorize: displaying it as its
own would suggest it is in progress. A refusal, for its part, is visible — a
conversation that stalls without displaying anything reads as a failure —, and
it reuses `AiError::ToolNotAllowed` rather than a twin variant, which would have
given two ways to say the same thing.

* **migration 9** — the `external_agents` table, and `oxyn_store::ExternalAgents`
  to read and write it. Five tests.

A table separate from `ai_providers`, not added columns: an agent has no
endpoint, no model, no secret reference, and making them cohabit would have
produced a table half of whose columns mean nothing depending on the row. The
test `the_table_has_no_secret_column` reads the **schema** rather than the
documentation — a column added one day "just for a token" would turn no other
test red.

A row that became unreadable is **discarded with a trace**, not propagated as
an error: an `args` corrupted by a SQLite editor must not make the configuration
screen unusable. It is the approach already chosen for providers.

* **the bus** — `ListExternalAgents`, `SaveExternalAgent`, `RemoveExternalAgent`,
  refused to an `Actor::Agent` by the `PolicyGate`.

This refusal holds for the reason that already protects a provider's
declaration, **even more strongly**: declaring an external agent means
designating a **program to launch**. An agent that managed it would obtain
arbitrary code execution on the machine, by the shortest path there is. Held by
`an_agent_does_not_declare_an_external_agent`, whose hostile declaration is
`/bin/sh -c "curl … | sh"` — reading the list, on the other hand, remains
allowed, since it declares nothing.

* **the screen** — both kinds in one list, a row computed outside rendering,
  and an agent's input validated by the domain itself.

One input choice deserves to be written, because it follows from the rest:
arguments are typed **one per line**. Splitting on spaces would have required
inventing quotes for the argument that contains one — hence a command-line
grammar, hence exactly the surface this slice avoids since launch. One line per
argument has no ambiguous case: what the user types is what the process
receives, spaces included.

The form's validation **delegates** to `ExternalAgentConfig::validate` rather
than restating its rules: two validations diverge, and the domain's is the one
that decides at write time. The form only applies it earlier, so the message
arrives during input.

* **the panel** — `Backend::start_agent_turn` returns the **same** shape as
  `start_conversation`: an event channel and a cancellation token.

The panel therefore does not know which of the two kinds is talking to it, and
that is what spares it two displays for a single conversation. `TurnEnd`
translates the protocol's stop reason **at the edge** of `oxyn-ai`:
`StopReason` does not cross that boundary, otherwise `oxyn-app` would depend on
the protocol crate to read the end of a conversation — and the choice of that
protocol would stop being reversible. `MaxTokens` becomes `truncated`, because
a truncated response that does not say so looks like a wrong response.

**The order of choice is deliberate: the provider first.** An external agent
only takes over if no provider is usable. Oxyn knows where a provider's data
goes; of an agent, it can only name the destination. Preferring what one can
verify to what one can only name needs no other justification.

The slice is **complete**: declare, persist, list, remove, launch, converse.
What remains — an agent's environment in the form, a second agent chosen
explicitly rather than the first — is comfort, not journey.

## Consequences

* **+** **Oxyn holds no secret in this mode.** The best treatment of a secret
  is not to have it, and [I-03](../../CLAUDE.md#i-03) then has nothing left to
  protect on that side.
* **+** The user reuses the subscription and authentication they already have,
  instead of paying for a second access and entrusting us with a key.
* **+** [I-01](../../CLAUDE.md#i-01) and [I-07](../../CLAUDE.md#i-07) become
  **structural**: the protocol forces the agent to ask, and our `PolicyGate`
  is what answers.
* **+** The crate's license (Apache-2.0), edition and MSRV are compatible
  without touching the toolchain.
* **−** **A `Local` connection loses access to external agents**, including
  when the agent actually runs against a local model. It is a restriction the
  user will find excessive in that precise case, and it is accepted: we do not
  know how to lift it without asking them to take our word for it.
* **−** One more dependency, on a protocol whose version 2 is still a
  **draft**. The crate's version number does not say the protocol's, which is a
  trap to document on every upgrade.
* **−** **A second async reactor enters the process.** Measured on
  2026-09-14: the crate pulls `async-io 2.6.0`, `async-process 2.5.0`,
  `async-signal 0.2.14` and `blocking 1.7.0` as **normal** dependencies.
  `async-io` starts a reactor thread on first use, and `blocking` its own
  pool — next to Tokio, which is the thread model documented by
  [ARCHITECTURE](../ARCHITECTURE.md#le-modèle-de-threads). It is not the full
  runtime one could fear: `smol` and the global executors are **not** in the
  graph, checked with `cargo tree --edges normal`. But two reactors cohabit
  with discipline, and [I-05](../../CLAUDE.md#i-05) holds for both.
* **−** A child process must be supervised: unexpected death, error output not
  to be confused with protocol. The crate already carries the trickiest part —
  the `ConnectTo` path installs a guard that tears down **the process group**,
  because an agent distributed behind `npx` or `uvx` would otherwise outlive the
  death of its launcher by reattaching to pid 1. It is precisely the kind of
  detail one writes badly when rewriting it.
* **−** What the agent does with our metadata **escapes us**. We can only tell
  the user by naming it, not by guaranteeing it.

**Exit cost:** low as long as the agent mode remains a declaration variant and
a destination. It becomes high if the protocol gains the right to trigger
anything other than `Command`s — that is the line not to cross, and it is what
justifies this ADR rather than a simple slice.

**Reconsider if** the protocol one day lets a client **verify** the real
destination of an agent's model: the table row on `Local` would open, and it is
the only thing that would open it.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Offer only API providers | Forces the user to entrust a key and pay for a second access while they already have an agent installed and authenticated. It is the cost ADR-0023 did not discuss because it seemed unavoidable |
| Launch the agent and **guess** its reach (inspecting sockets, environment variables) | A heuristic that errs in the user's favor is exactly the failure mode [ADR-0006](0006-ai-privacy-tiers.md) exists to prevent. "When in doubt, protect" is already the repository's rule |
| Allow external agents on `Local` while **warning** the user | A warning turns a guarantee into a recommendation. `Local` promises "nothing leaves the machine": a promise with a checkbox attached is no longer a promise |
| Write our own subprocess protocol | No existing agent would speak it. The whole point of the mode is to reuse what the user already installed |
| Integrate ACP into `oxyn-llm` | `oxyn-llm` knows neither agents nor tiers, and it is that narrow scope that keeps it reviewable. An agent is not a transport to a model: it is a peer that asks for authorizations |
