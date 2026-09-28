# ADR-0010 — A single version of libsqlite3-sys in the graph

**Status:** accepted · **Date:** 2026-09-05
**Discovered at:** the first real resolution of the workspace, not at design time.

## Context

Oxyn needs SQLite twice, for two unrelated reasons: `oxyn-store` uses it as the
local state format (history, audit log, catalog cache) through `rusqlite`, and
`oxyn-driver-sqlite` exposes it as a client database. In addition,
`oxyn-driver-postgres` depends on `sqlx`.

Three facts combine badly:

1. Cargo locks **optional** dependencies in `Cargo.lock`: `sqlx` therefore brings
   `sqlx-sqlite` into the resolution graph even with `default-features = false` and
   no SQLite feature enabled.
2. `sqlx-sqlite` 0.9 accepts `libsqlite3-sys >=0.30.1, <0.38`; `rusqlite` 0.40
   requires `^0.38`.
3. `libsqlite3-sys` declares `links = "sqlite3"`. Cargo allows **only one** package
   declaring a given `links` in the whole graph.

The intersection of the bounds is empty: the workspace does not resolve.

## Decision

Pin **`rusqlite` 0.37** (which asks for `libsqlite3-sys ^0.35`), the only version
whose bound intersects that of `sqlx-sqlite`. The graph then resolves on
`libsqlite3-sys` 0.35, shared.

## Consequences

* **+** The workspace resolves, and a single copy of SQLite is compiled and linked.
* **−** `rusqlite` is held three minor versions back, by a constraint that does not
  come from it. Any `rusqlite` API later than 0.37 is out of reach.
* **−** The constraint is **transitive and invisible**: it comes from no
  architecture decision, only from the coexistence of two libraries. It must be
  written down somewhere, otherwise someone will bump `rusqlite` in six months and
  spend an evening on a `links` error message that nothing explains.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Use `sqlx-sqlite` everywhere, remove `rusqlite` | `oxyn-store` needs simple synchronous access; going through an async runtime to read the local history is a permanent cost to avoid a temporary pin |
| Remove `sqlx` and write the PostgreSQL driver on `tokio-postgres` | possible, but we lose `sqlx`'s pool, TLS handling and typing over a version problem |
| Compile SQLite non-bundled | moves the problem to the user's machine, and "native first" does not mean "depends on whatever lies around on the system" |

**Reconsider when** `sqlx` widens its bound on `libsqlite3-sys`. On that day,
bumping `rusqlite` is a one-line change — provided this ADR has been read.
