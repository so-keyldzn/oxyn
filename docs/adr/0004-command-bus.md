# ADR-0004 — A single command bus and a Policy gate

**Status:** accepted · **Date:** 2026-09-05

## Context
Giving AI agents access to production databases is the product's number one risk. The
usual approach — a "tools" API separate from the UI — creates two execution paths, one
of them poorly audited.

## Decision
Every possible action in Oxyn is a typed `Command` value. The UI does nothing but
produce `Command`s. **The tools exposed to agents are exactly these same commands.**
Each command carries an `Actor` (`Human` or `Agent`) and goes through a single
`PolicyGate` returning `Allow` / `RequireApproval` / `Deny`.

## Consequences
* **+** An agent cannot do anything the user cannot.
* **+** One history, one audit log, one undo mechanism.
* **+** The product becomes scriptable and testable at no extra cost.
* **+** Prompt injection through a database's content produces a visible approval
  request, not an execution.
* **−** Every new feature must be expressed as a command: a real constraint on the
  pace of UI development.

## Default policy
Agents: reads and `EXPLAIN` allowed; writes and DDL on approval with a preview;
`GRANT`/`REVOKE` refused; connections marked *production* strictly read-only.
