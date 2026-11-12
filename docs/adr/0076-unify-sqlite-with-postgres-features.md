# ADR-0076 — Unify SQLite while enabling PostgreSQL type support

**Status:** accepted · **Date:** 2026-11-12

Refines [ADR-0010](0010-contraintes-natives-sqlite.md): the single native SQLite
constraint remains; its old version pin is replaced.

## Context

The bundled SQLite 3.50.2 in `libsqlite3-sys` 0.35 is affected by
RUSTSEC-2026-0459, a potential memory disclosure when reading a malicious WAL.
`rusqlite` 0.41.1 bundles SQLite 3.54.1 through `libsqlite3-sys` 0.39.1.
SQLx 0.9's umbrella `chrono`, `uuid` and `json` features also resolve its
optional SQLite package, whose native dependency cannot coexist with that
version. The dependency and feature checks are dated in
[RESEARCH-NOTES](../RESEARCH-NOTES.md).

## Decision

Enable `chrono`, `uuid` and `json` on the exact-pinned `sqlx-postgres` 0.9.0
workspace dependency used by the PostgreSQL driver, instead of enabling those
features on the SQLx umbrella crate. Keep the SQLx facade for driver APIs and
use `rusqlite` 0.41.1 with bundled SQLite. Cargo feature unification enables
the same SQLx core and PostgreSQL features; the host feature graphs were
compared before and after. There is one resolved `libsqlite3-sys` version.

## Consequences

The native SQLite security fix is available without changing PostgreSQL's
facade API or maintaining a fork. The PostgreSQL manifest now depends directly
on an internal SQLx component, pinned to the facade's version, and future SQLx
upgrades must check both feature graphs and native dependencies. The rusqlite
upgrade requires adapting its fallible progress-handler and metadata APIs.
Feature comparison alone does not prove runtime equivalence on every platform.

## Alternatives considered

- Retain SQLite 3.50.2: leaves the disclosure advisory unaddressed.
- Fork SQLx or its SQLite dependency: adds a maintenance and security burden.
- Use a system SQLite library: gives up the reproducible bundled version and
  does not remove Cargo's native link conflict.
- Replace the SQLx facade or PostgreSQL driver: changes a larger API surface
  than needed to resolve the feature conflict.

## Exit cost and reconsideration

Reverting the feature wiring requires coordinating both SQLx manifests and
checking feature and native dependency graphs; changing the driver or replacing
SQLite is substantially more expensive. Reconsider when SQLx can enable the
required PostgreSQL types without resolving its incompatible optional SQLite
dependency, or when a future upgrade changes these constraints.
