# ADR-0027 — The I-04 gateway holds for **both** destinations, or it holds for neither

**Status:** accepted · **Date:** 2026-09-15

**Clarifies:** [ADR-0026](0026-agents-externes-acp.md), which opened a second
destination without giving it the typed gateway of the first.

## Context

[I-04](../../CLAUDE.md#i-04) says nothing reaches an AI prompt outside the
single gateway that applies the connection's tier. What the repository made of
it is stronger than an instruction: `ContextBuilder::build` is the **only**
factory of `AgentContext`, `AgentSession::new` requires that type, and the
invariant is therefore held by the compiler. `crates/oxyn-ai/src/lib.rs`
announced it this way: "checked by the compiler, not by review".

ADR-0026 added a second destination — an external agent, a child process
speaking ACP. Its entry point is:

```rust
pub async fn run_turn(
    agent: &ExternalAgentConfig,
    tier: PrivacyTier,
    prompt: &str,          // ← n'importe quelle String
    cancel: &CancelToken,
    observer: Arc<dyn AgentObserver>,
) -> Result<TurnEnd>
```

The tier there governs the **launch**: `allows_external_agent` refuses before
the process starts, and it is well tested. But it does not govern **the
assembly of what is sent**, because there is nothing to assemble: the
parameter is a free string.

### What is not a problem today

There is **no leak**. The only caller, `Workspace::ask_external_agent`, only
passes the question typed by the user. No catalog content, no row value, no
table name enters that prompt. Checked by rereading the only path that leads to
`start_agent_turn`.

### What is

The **guarantee** changed nature without anyone deciding it. For a provider,
the question "what went out?" is answered by rereading one function. For an
agent, it requires rereading all present and future callers — that is, the
property I-04 exists precisely to remove.

`.claude/rules/ia.md` names the shortcut that destroys it: "a shortcut 'just
for the schema, it is `Metadata` anyway' destroys this property — nobody can
answer 'what went out?' anymore". Here, that shortcut would be written as a
`format!`, without any type or test turning red. It is an invitation, not a
flaw: the next person who wants to give the schema to an external agent will
have nothing to bypass.

## Decision

**Option B is chosen**, and implemented on 2026-09-15:
`crates/oxyn-ai/src/external/prompt.rs` carries an `AgentPrompt` whose only
constructor, `from_user`, requires the connection's tier.
[`run_turn`](../../crates/oxyn-ai/src/external/turn.rs) no longer takes a `&str`.

What carried the choice, against **A**: `AgentContext` carries a rendered
schema, retained relations and a token budget of which an external agent uses
none. Against **C**: the change was still mechanical — a single caller —, and
that is when it costs the least.

`run_turn` **keeps its own tier check**. It is not a redundancy to clean up:
the gateway protects assembly, the check protects the launch, and neither must
rely on the other. The test
`le_niveau_local_refuse_avant_meme_de_lancer_le_processus` now composes its
prompt under a permissive tier to exercise that second line alone.

What follows remains the record of the three options as they were weighed.

### A — `run_turn` takes an `&AgentContext`

The agent path takes the provider path's gateway. I-04 becomes checked by the
compiler again for both destinations, and the sentence of `lib.rs` becomes
true again without reservation.

*Cost*: `AgentContext` was designed for a tool conversation — it carries the
schema, the capabilities, the dialect. An external agent uses none of them: it
only receives a text. Passing a rich context where a sentence suffices is the
kind of abstraction [CLAUDE.md](../../CLAUDE.md#code-organization)
discourages — "no abstraction for a single caller".

### B — A dedicated, thin prompt type, produced by the same gateway

An `AgentPrompt` (name to decide) that **only** a constructor applying the
tier can build, and that `run_turn` requires. The invariant becomes typed
again without imposing a context the agent has no use for.

*Cost*: one more type, and the discipline of never giving it a naive public
constructor. It is little, but it is a boundary to maintain.

### C — Status quo, accepted and written down

We accept that the agent path holds I-04 by review, on the express condition
that **nothing other than the user's input** enters it — and we write it where
someone will read it before adding a `format!`.

*Cost*: the guarantee remains unequal between the two destinations, and the
documentation must stop claiming the opposite — which was done on
2026-09-15 in `oxyn-ai/src/lib.rs` and
[AI-PROVIDERS](../AI-PROVIDERS.md).

## What this ADR does not decide

The exact placement of option B's constructor, and whether the same gateway
must filter what the agent **returns**. The return is already handled
separately: only text chunks go up, `plan` and `tool_call` are discarded, and
no output becomes a `Command`
([I-07](../../CLAUDE.md#i-07)).

## Exit cost

**Low as long as there is a single caller, and it grows fast.** Today, moving
from **C** to **A** or **B** touches one signature, one call site and two
tests — half a day. The cost then follows the number of callers of
`run_turn`: each will have to build its prompt through the gateway, and each
will in the meantime have had good reasons to compose its string itself. That
is the reason to write this ADR **now**, while the change is still mechanical.

Going back, for its part, is trivial in every direction: no persisted data,
no format, no public API outside the workspace.

## Reconsideration condition

Three facts would reopen the question, even if **C** were chosen:

* **a second caller of `run_turn`** appears, whatever it is;
* anything other than the user's input reaches an agent's prompt — a table
  name, a schema excerpt, a server error message;
* the ACP protocol gains a way to pass structured context, in which case the
  question will no longer be about a string but about what one puts in it.

The first is mechanically checkable: `grep` on `run_turn` must return a single
call site outside tests. If that count changes and this ADR stayed at **C**, it
no longer has a foundation.

**The second fact happened on 2026-09-23**, and option **B** absorbed it
without changing shape: the database structure now reaches the prompt that
opens an agent session, through a second constructor, `with_schema`, which has
it rendered by `ContextBuilder::build` under the same tier. `AgentPrompt`
carries the produced `AgentContext`, so the caller can say what went out. The
rest of the structure goes through the `describe_schema` tool, shared by both
destinations and rendered by the same function. The decision and its limits
are in
[ADR-0030 § 4 bis](0030-outils-oxyn-exposes-a-un-agent-externe.md#4-bis-the-database-structure-one-tool-for-every-destination).

## Consequences

Whichever option is chosen, one thing must not stay as it is: an authoritative
document claiming a compiler guarantee that only holds for one destination out
of two. It is fixed; this ADR exists so that the fix is a recorded choice and
not an oversight.

If **A** or **B** is chosen, the change is mechanical and local: one
signature, one caller, and the tests of `crates/oxyn-ai/src/external/tests.rs`.
If **C** is chosen, there is nothing to write in the code — only not to forget
why.
