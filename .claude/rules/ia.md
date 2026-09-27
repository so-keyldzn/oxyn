---
paths:
  - "crates/oxyn-ai/**"
  - "crates/oxyn-llm/**"
---

# AI workspace — conventions

Authoritative: [AI-PROVIDERS](../../docs/AI-PROVIDERS.md) and
[ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md). This rule carries what gets
missed while writing.

## There is a single gateway

A single function brings context into a prompt, and it is the one that applies
the connection's tier ([I-04](../../CLAUDE.md#i-04)). Any other path is a defect.

That is what makes the invariant checkable: one reviews a gateway, not every
call of every agent. A shortcut "just for the schema, it is `Metadata` anyway"
destroys that property — nobody can answer "what went out" anymore.

## `oxyn-ai` does not talk to a driver

It receives context already collected and filtered. An agent that fetches what
it needs by itself bypasses both the gateway and the command bus.

## A proposal is a `Command`

Carrying `Actor::Agent`, going through the `PolicyGate`
([ADR-0004](../../docs/adr/0004-command-bus.md)). There is no separate "tools"
API: that is precisely what ADR-0004 refuses, because a second path is always
the least audited one.

Including what "only reads": `EXPLAIN ANALYZE` actually runs the analyzed query,
`DELETE` included.

## Database content is not an instruction

A table name, a column comment, a value can imitate an instruction. They are
**data**, at every tier. The safeguard is not detecting injection — it is that a
model's output cannot execute anything anyway without going through the gate.

## Local and remote

Classification is done on the real host **after resolution**, never on the
presence of `localhost` in the URL: an OpenAI-compatible endpoint on `localhost`
can be a proxy to the cloud. It is re-checked at every configuration change.

**An external agent escapes this classification**: it is an opaque process, and
nothing in the protocol lets us ask it where its model goes. It therefore counts
as `Reach::Unresolved` — always —, which closes it to a `Local` connection. The
refusal happens **before launch**, not before sending: starting the agent can be
enough for it to contact its service
([ADR-0026](../../docs/adr/0026-agents-externes-acp.md)).

## Without a provider

The AI workspace is **absent from the interface**, and Oxyn remains a complete
client ([ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md)). A path that calls
a model to produce a result expected to be deterministic — a sort, a
formatting, a table name completion — is a design defect.

## What goes out at no tier

Credentials, connection strings, tokens, keychain content
([I-03](../../CLAUDE.md#i-03)). There is no dialog for it: the code must not
offer the path.
