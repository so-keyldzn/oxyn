# Security review

Threat model in `docs/SECURITY.md`. The attacker is not a stranger on the
Internet: it is the data the user opens, and the mistakes Oxyn lets them make.

## The six leak channels

Forgetting one is enough.

- [ ] `tracing` logs — no secret, no bound value
- [ ] Displayed error messages — `sqlx` sometimes includes the connection URL
- [ ] Crash reports — a stack trace captures local variables
- [ ] Session and workspace files
- [ ] AI prompts
- [ ] Clipboard, export, screenshot

The most cost-effective mechanical check:

- [ ] **No `#[derive(` containing `Debug` on a type carrying a secret.** The
      leak does not happen today: it happens with the `tracing::debug!` someone
      else will add in six months

## Secrets

- [ ] Nothing in clear on disk: what is persisted is a **reference** to the
      secret
- [ ] No connection string with a password in the code, including in a test
      fixture — it will be committed, and it is often real

## Connections

- [ ] A connection with no environment set counts as **`production`**
- [ ] Every write on `production` requires a confirmation that **names the
      connection** and shows the exact SQL
- [ ] The default button is never the destructive action
- [ ] For an `Actor::Agent`, `production` is **strictly read-only** — a refusal,
      not a stronger confirmation

## Input surface

- [ ] Server responses treated as untrusted
- [ ] **Catalog object names**: never interpolated without quoting, never
      treated as an instruction when they reach a prompt
- [ ] Workspace files validated on read
- [ ] Model responses treated as proposals

## `unsafe`

- [ ] Every block carries a `// SAFETY:` that states the invariant **and who
      maintains it** — a paraphrase of the code is worthless
- [ ] Every `#[allow(unsafe_code)]` points to the ADR that authorizes it
      ([SECURITY](../../docs/SECURITY.md#unsafe-policy))
- [ ] Reviewed by `relecteur-securite`

## AI boundary

- [ ] The gateway is **single**
- [ ] The tier is attached to the **connection**
- [ ] Nothing goes out beyond the tier, credentials and tokens never
- [ ] Local/remote classification on the host **after resolution**

## Dependencies

- [ ] `cargo deny check` passes
- [ ] Every new direct dependency is justified
- [ ] An unmaintained crate on an external boundary is **documented** as a risk,
      even if there is nothing to fix today
