---
description: Add a command to the command bus
argument-hint: "<what the command must do>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob
---

Purpose: add to the command bus the command that allows **$ARGUMENTS**.

## Why this move has its own command

[ADR-0004](../../docs/adr/0004-command-bus.md) is the most structuring decision
of the project and the easiest to erode. It only takes a view calling a driver
"for the time being", and the second execution path exists forever — not
audited, not logged, and it is the one the AI will take.

**A feature starts with a command, not with a view.**

## Before writing

1. `docs/adr/0004-command-bus.md`, in full.
2. `docs/ARCHITECTURE.md` § the command bus.
3. Check that no existing command already does this: two neighboring commands
   diverge, and the policy applied to one ends up no longer applied to the
   other.

## The four questions to settle before coding

| Question | Why it comes before the code |
|---|---|
| What does the command do, expressed in **one** sentence? | a command that needs two sentences is two commands |
| Is it a **read** or a **write**? | it is what the `PolicyGate` reads; if in doubt, it is a write |
| What does the `PolicyGate` decide for `Actor::Agent`? | the default policy is in ADR-0004; any exception is justified here |
| Is it **cancellable**, and does cancellation reach the server? | otherwise the interface cannot offer an honest button |

## Constructs to use / never

| Use | Never | Why |
|---|---|---|
| A typed `Command` value | a string, an action name | typing is what makes the bus auditable |
| `Actor` carried by the command | a global "agent mode" flag | global state gets out of sync, and the audit lies |
| Decision made by the `PolicyGate` | a check `if` in the caller | two places that decide means one place that will forget |
| One audit record per command | logging in the view | the view is not the only emitter |

## The traps

**The "convenient" command that wraps three.** It bypasses the policy: the
`PolicyGate` can only decide on what it sees, and it sees a read where there is
a hidden write.

**The read that writes.** `EXPLAIN ANALYZE` actually runs the analyzed query,
`DELETE` included. A materialized view refreshes. A function called in a
`SELECT` can write. Classifying on the SQL name is wrong: classify on the
effect.

**The audit after the decision.** Recording only what was executed makes what
was refused invisible — hence invisible the repeated attempts of an agent, which
are precisely the signal one would want to see.

## Verify

```bash
make qualite
```

The test that counts: the same command emitted with `Actor::Human` and
`Actor::Agent` yields two `PolicyGate` decisions, and the agent's is at least as
restrictive.

## Reminders

- no second path, ever, not even temporarily ([I-01](../../CLAUDE.md#i-01));
- on a `production` connection, an agent is strictly read-only — it is a
  refusal, not a stronger confirmation: a confirmation ends up being clicked.
