# ADR-0052 — PostgreSQL and MySQL require verified TLS outside Local

**Status:** accepted · **Date:** 2026-10-03

**Refines:** [ADR-0050](0050-mysql-driver-on-mysql-async-prepared-first.md),
decision 6: its TLS policy becomes the common rule for PostgreSQL and MySQL.
The rest of ADR-0050 remains in force.

## Context

Issue [#149](https://github.com/so-keyldzn/oxyn/issues/149) found two different
policies for the same connection environment: MySQL defaults to `verify-full`
and rejects weaker modes outside Local, while PostgreSQL defaults to `prefer`
and accepts every mode in production. Encryption without server identity
verification is insufficient against an active intermediary; the distinction
between the PostgreSQL modes is sourced in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#postgresql-tls-modes--checked-on-2026-10-03).

## Decision

**One TLS rule applies to `oxyn-driver-postgres` and `oxyn-driver-mysql`:**

- The default is `verify-full`. Both the certificate chain and the server
  hostname must be verified.
- Only `Environment::Local` permits an explicit weaker mode. PostgreSQL's
  `disable`, `allow`, `prefer`, `require` and `verify-ca` are refused elsewhere;
  MySQL refuses every supported mode weaker than `verify-full` there too.
- An unset environment is production, as
  [SECURITY](../SECURITY.md#connection-marking) already specifies. Neither a
  loopback hostname nor a development or staging label grants the exception.
- `ConnectSpec::from_config` refuses a weaker mode before opening a network
  connection, with `OxynError::Config` and the existing MySQL message naming
  `Local` and `verify-full`. Saved configurations and pasted DSNs pass through
  the same validation; they are not silently migrated or retried with weaker
  security.
- Driver form metadata defaults to `verify-full`. A disposable local test
  server without TLS must be marked Local and explicitly select a weaker mode.

This ADR is the common policy's source; SECURITY links here. The historical
MySQL-only decision is preserved rather than rewritten.

## Consequences

* **+** Selecting another database protocol no longer changes the protection
  of credentials and queries on the same environment.
* **+** An omitted field fails closed; the user must explicitly choose the
  Local exception.
* **−** Existing non-Local PostgreSQL configurations with weaker TLS modes stop
  connecting until they use `verify-full` with a verifiable server certificate
  and matching hostname. Only a genuinely local disposable database should be
  marked Local to retain a weaker mode.
* **−** Local test URLs that relied on the old default must name their TLS
  mode. A Local environment alone does not disable verification.
* **−** Neither driver currently exposes a per-connection private CA field.
  PostgreSQL's `sslrootcert` parameter is forwarded as a server session option,
  not applied as a TLS trust root: adding it to a saved configuration or DSN
  is not a supported migration path. A private-CA server whose root is not
  available to the TLS verifier therefore remains unavailable under this
  rule. This decision introduces no new CA or client-certificate configuration
  surface, and the connection must not be relabeled Local merely to bypass
  verification.

**Exit cost:** small in code (two option builders and their form defaults),
but substantial in the security contract: relaxing the rule would change the
protection users rely on for existing saved connections. It requires a new ADR
and regression coverage in both drivers.

**Reconsider if** Oxyn introduces another explicit, verified server identity
mechanism with equivalent protection and a tested migration path. Connection
failures on an unverified server alone are not grounds to weaken the default.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep PostgreSQL's `prefer` default | makes the environment's security depend on the selected protocol |
| Accept `require` or `verify-ca` outside Local | does not meet the common certificate-and-hostname verification requirement |
| Infer Local from `localhost` or allow development/staging exceptions | host spelling and non-production labels do not establish the explicit disposable Local boundary |
| Automatically rewrite saved modes or fall back after a TLS error | hides a security decision and changes the user's configuration without consent |
