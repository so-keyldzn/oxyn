# ADR-0003 — A capability model rather than a common denominator

**Status:** accepted · **Date:** 2026-09-05

## Context
Redis has no schema, Neo4j no tables, Elasticsearch no SQL, DynamoDB no joins. A
levelling abstraction would reduce each database to its poorest expression.

## Decision
Every `Driver` and every `Session` declares a `Capabilities: u64` (bitflags). The UI and
the agents query these flags to decide which surfaces exist. Queries carry an explicit
`QueryLanguage`; SQL is only one case among others.

## Consequences
* **+** Each database is exposed with its own strengths, nothing is simulated.
* **+** Agents do not propose impossible actions (no index for DynamoDB).
* **−** The UI must be conditional everywhere: a discipline to hold from phase 0.
* Capabilities are evaluated **per session**, not per driver: the server version
  changes what is available.

## Corollary
One driver per **protocol**, not per product: Redshift ≡ PostgreSQL, MariaDB ≡ MySQL,
OpenSearch ≡ Elasticsearch, Memgraph ≡ Bolt, pgvector and TimescaleDB ≡ PostgreSQL
extensions. The ~30 systems of the vision come down to ~14 implementations.
