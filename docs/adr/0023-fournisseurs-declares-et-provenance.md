# ADR-0023 — A provider is declared per machine, reclassified on every opening, and signs what it proposes

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0006](0006-ai-privacy-tiers.md), on two points it left
open: where a provider's configuration lives, and what remains of an agent's
proposal once the conversation is closed.

## Context

[ADR-0006](0006-ai-privacy-tiers.md) settles the privacy tier: it belongs to
the **connection**. It also states that "no provider is required: without
configuration, the AI workspace is absent from the UI". These two sentences
describe a state the code cannot reach.

The real state of the repository, checked today: `oxyn-ai` (4,480 lines) and
`oxyn-llm` (5,988 lines) are complete and tested — single context gateway,
tool call → `Command` translation, failure filtering by tier, eight provider
families. **No crate declares them as a dependency.**
`crates/oxyn-exec/src/sink.rs` exposes the `ExecutorSink` the `CommandSink`
trait must plug into, and says so explicitly: the translation falls to
`oxyn-app`, which depends on both. It is not written there.

Between a usable agent backend and a visible feature, three things are
therefore missing that are not wiring: **where the declaration of an endpoint
is stored**, **when its local/remote classification is computed**, and **what
remains, six months later, of a text an agent proposed**.

Three constraints bound the answers.

**A secret does not go into the database** ([I-03](../../CLAUDE.md#i-03)). The
`connections` table already carries the chosen shape: `params` as non-secret
JSON, `secret_ref` pointing to the system keychain.

**DNS resolution is blocking** (`oxyn_llm::reach::resolve_reach`, whose
documentation says so) and [I-05](../../CLAUDE.md#i-05) forbids it on the
interface thread. It is also perishable:
[AI-PROVIDERS](../AI-PROVIDERS.md#local-and-remote-are-not-distinguished-by-the-api)
requires the classification to "be re-checked on every configuration change",
because an OpenAI-compatible endpoint listening on `127.0.0.1` may be a proxy
that forwards to the cloud.

**The audit log does not answer the question asked.** It carries the `Actor` of
each command, hence who **executed**. The text of a query, for its part, lives
in `documents` — and a `SELECT` proposed by an agent, pasted in a console,
saved under a name, reread next year, is indistinguishable there from what the
user wrote themselves.

## Decision

### A provider is declared per machine, not per workspace

The store carries an `ai_providers` table (migration 7), **without `workspace_id`**:

| Column | Meaning |
|---|---|
| `id` | the identity of this declaration |
| `kind` | the family: `anthropic`, `openai`, `gemini`, `openai_compatible`… |
| `label` | the name the user gives it |
| `base_url` | the endpoint, **stripped of its credentials** before writing |
| `model` | the default model of this declaration |
| `secret_ref` | reference to the keychain, `NULL` for an endpoint without a key |
| `created_at`, `updated_at` | |

The absence of `workspace_id` is the decision, not an oversight: an Ollama
listening on the machine serves all workspaces, and duplicating it per
workspace would create as many places where its configuration can diverge.
**What stays per connection is the tier** — and it is exactly the separation
ADR-0006 protects: a shared provider does not make a shared tier.

No key is written to SQLite. `oxyn_llm::ApiKey` does not derive `Debug`
and is obtained only from the keychain, when a runtime opens.

### The local/remote classification is never persisted

`Reach` **has no column**. It is recomputed on the blocking pool on every
save and on every runtime opening. A value in the database would be
yesterday's DNS answer applied to today's send, and it is precisely the proxy
trap the documentation asks to avoid.

A visible and intended consequence: the configuration screen shows the
classification **with the instant it was measured**, and `Reach::Unresolved` is
displayed as such rather than rounded to "local". An unresolved endpoint counts
as remote wherever a decision is made (`Reach::leaves_machine`).

### The AI entry exists if and only if a provider is declared

The condition is the list returned by the bus, never a compilation flag nor a
preference. Zero providers: no button, no panel, no mention — Oxyn remains a
complete client. The first registered provider makes the entry appear without
a restart.

**And it does so without a new channel.** `SaveAiProvider` is a local command:
its outcome already comes back to the caller through the `oneshot` of
`Backend::dispatch`, and the view that emitted it notifies the parent view
through a GPUI event — the same mechanism the connection form uses today. The
execution bus gains **no variant**: what changed is not the state of a
database, it is a local configuration whose fate the view that writes it knows.
Publishing an event there would do exactly what [ADR-0022](0022-rafraichissement-automatique.md)
refuses — a second place to remember to publish.

On a connection in `PrivacyTier::Local` with a single remote provider, the
entry is **shown and disabled**, with the reason — ADR-0006 requires `Local`
to remain usable, and a feature that only works in `Metadata` to "say so in
the interface rather than fail without explanation".

### What an agent writes carries its provenance

`documents` gains a `provenance` column, JSON bounded to 512 bytes, `NULL` by
default. `NULL` means "written by the user" — it is the case of all existing
rows, and it is true.

Otherwise, four fields: the agent, the agent session, the provider family,
the model, and the instant. **Neither the prompt, nor the model's response, nor
the URL, nor the key**: provenance says *where this text comes from*, it does
not archive the conversation. A document whose content the user rewrites keeps
its provenance: it dates the origin, not the last keystroke.

Provenance does not replace the audit log and is not derived from it. The log
says who **launched** an execution; provenance says who **wrote** a text. An
agent can propose a `SELECT` nobody executes, and a human can execute a hundred
times what an agent wrote once.

### Nothing changes on the execution path

`oxyn-app` implements `CommandSink` with a logic-free translation of
`DispatchReport` into `DispatchOutcome`, as `sink.rs` describes it, and adds no
driver call. An agent's proposal is a `Command` carrying `Actor::Agent` that
goes through the `PolicyGate`
([I-01](../../CLAUDE.md#i-01), [I-07](../../CLAUDE.md#i-07)). No model output
is executed without this crossing, `EXPLAIN` included.

## Consequences

- **+** The absence of a provider remains a first-class, testable state: the
  list is empty, the entry does not exist.
- **+** A proxy installed on the loopback cannot pass itself off as a local
  provider durably: the classification dies with the process.
- **+** Provenance outlives the conversation, which, for its part, does not
  outlive closing the window.
- **+** No second execution API to audit: the agent sink calls only what the
  interface calls.
- **−** A DNS resolution on every runtime opening, hence latency on the first
  message of a conversation — bounded by the system's resolution timeout,
  which we do not control.
- **−** A provider shared by all workspaces means a user cannot isolate a
  "customer" workspace from a provider configured for another use. It is the
  connection's tier that must carry that separation; if usage shows it is not
  enough, it is this ADR that must be reopened.
- **−** One more migration, and a column whose existence the old binary
  ignores: an older Oxyn reopening the store will read the documents without
  their provenance, and will rewrite it to `NULL` if it saves them. Provenance
  is therefore a trace, not an integrity guarantee — claiming it tamper-proof
  would be lying.
- **−** Provenance is not propagated by copy-paste between two documents: the
  system clipboard carries no metadata, and adding one would be one more
  channel to audit under I-03.

**Exit cost:** a table with no dependents, a nullable column, and a trait
adapter in `oxyn-app`. `oxyn-ai` and `oxyn-llm` know neither: removing AI
entirely from the interface would come down to removing the dependency and the
screen, without touching the rest.

**Reconsider if** a user needs distinct providers per workspace — in which case
the table gains a scope, not a copy —, or if resolution on every opening proves
costly when measured, in which case a cache **with an explicit validity
period** would replace the recomputation, never a persisted column.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Store providers per workspace, like connections | Duplicates the same configuration; the separation that matters is the tier's, and it stays per connection |
| Persist `reach` next to `base_url` | Applies a stale DNS answer to a current send: exactly the local proxy trap the documentation asks to avoid |
| Classify on the presence of `localhost` in the URL, without resolving | A name can resolve elsewhere, and `127.0.0.1` can be a proxy; the classification is about the real host |
| A configuration file outside the store | A second format to carry, and a second place where the `secret_ref` can be forgotten |
| An environment variable as the only source (`ApiKey::from_env`) | Useful in tests, invisible to the user, and not revocable from the interface |
| Derive provenance from the audit log | The log says who executed, not who wrote; a text proposed and never executed does not appear there at all |
| Archive the conversation next to the document | Brings prompts and model responses into the workspace file, which I-03 counts among the six channels |
| Hide the AI entry on a `Local` connection without a local provider | ADR-0006 wants `Local` to remain usable and the refusal to be explained; an entry that disappears without reason reads as a bug |
