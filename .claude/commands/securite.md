---
description: Review a change from a security standpoint
argument-hint: "[path or commit range]"
allowed-tools: Bash, Read, Grep, Glob, Agent
---

Purpose: security review of **$ARGUMENTS**.

## The threat model

It is not a server's. The attacker is not a stranger on the Internet: it is
**the data the user opens** and **the mistakes Oxyn lets them make**. Oxyn runs
with an administrator's rights on production systems (`docs/SECURITY.md`).

A review that looks for authentication flaws is looking in the wrong place.

## The six leak channels

Forgetting one is enough. Go through all six:

| Channel | The concrete trap |
|---|---|
| `tracing` logs | a derived `Debug` prints a whole connection structure |
| Displayed errors | `sqlx` sometimes includes the connection URL in its error |
| Crash reports | a stack trace captures local variables |
| Session and workspace files | persistence "to restore the state" |
| AI prompts | `docs/AI-PROVIDERS.md` |
| Clipboard, export, screenshot | sharing features copy what is displayed |

The most cost-effective mechanical check: **look for `#[derive(` containing
`Debug` on any type carrying a secret.** It is the most frequent leak mode
because it is invisible in review — the leak arrives six months later, with a
`tracing::debug!` added by someone else.

## The five input surfaces

In order of underestimation (`docs/SECURITY.md` § input surface):

1. server responses — a server returns whatever it wants;
2. **catalog object names** — a table can be called
   `"users"; DROP TABLE audit; --`, or contain text imitating an instruction;
3. workspace files;
4. plugins — the sandbox bounds the damage, it does not exempt from entrusting
   them with nothing;
5. model responses — proposals, never orders.

## The blocking points

- a secret reaching one of the six channels;
- an identifier concatenated into SQL composed by Oxyn
  ([I-10](../../CLAUDE.md#i-10));
- a write reaching a `production` connection without going through the
  `PolicyGate`;
- an `Actor::Agent` obtaining more than read access on a `production`
  connection;
- an `unsafe` block without a `// SAFETY:` stating the invariant **and what
  maintains it** — a paraphrase of the code is worth nothing;
- data leaving the machine beyond the connection's privacy tier
  ([ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md)).

## How to proceed

Delegate to the `relecteur-securite` agent: it is read-only, and that is what
makes its verdict credible.

Then the checklist: `.claude/checklists/revue-securite.md`.

## Dependencies

```bash
cargo deny check
```

Licenses and security advisories. An unmaintained crate on an external boundary
is a risk to **document**, not to ignore — note it in the report even if there
is nothing to fix today.

## Reminders

- if a flaw is found, describe the **class** of problem and the fix; do not
  write a working exploit;
- if there is nothing to report, say so in one sentence.
