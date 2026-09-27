# ADR-0009 — GPUI consumed from crates.io, not from its upstream repository

**Status:** superseded · **Date:** 2026-09-05
**Superseded by:** [ADR-0029](0029-interface-tauri-shadcn.md), effective when the
GPUI crates were removed on 2026-09-18: `gpui` is no longer a dependency of the
repository. What follows is kept as it was decided.
**Clarified:** [ADR-0001](0001-ui-toolkit.md), which chose GPUI "by pinning a
precise commit". That point was superseded; the rest of ADR-0001 — the choice of
GPUI and the isolation rule — then remained in force. Only the isolation rule
survives today, carried over by ADR-0029.

## Context

ADR-0001 did not settle between the two ways of consuming GPUI, and implicitly
conflated them by speaking of a "precise commit". The facts, checked against the
registry on 2026-09-05 ([RESEARCH-NOTES](../RESEARCH-NOTES.md#gpui)):

* the latest published version is `0.2.2`, from **2025-10-22**, that is almost
  eleven months without a release, while development continues in the upstream
  repository;
* `gpui` declares **no MSRV**;
* `gpui` pins several dependencies with `=`, including `cocoa =0.26.0`,
  `cocoa-foundation =0.2.0` and `core-foundation =0.10.0`.

A git dependency on a `rev` gives access to fixes and recent APIs, at the cost of
a full rebuild of the graph on every bump, of a `Cargo.lock` that references a
third-party repository, and of exposure to unversioned API breakages. A
crates.io dependency freezes a known API and a resolved graph, at the cost of
receiving neither fixes nor new features.

## Decision

Oxyn depends on **`gpui` published on crates.io**, at an exact version.

## Consequences

* **+** A reproducible dependency graph, resolved by the registry; publishing
  Oxyn on crates.io is possible later, which a git dependency forbids.
* **+** The API does not move under the project's feet during the phase where the
  architecture stabilizes.
* **−** No upstream fix, no API later than October 2025. A GPUI defect
  encountered must be worked around in `oxyn-ui`, not fixed upstream.
* **−** The gap with the upstream repository grows as long as no version is
  published; a future migration will be all the more expensive.
* **−** `gpui`'s `=` pins on macOS system crates can make it impossible to add a
  dependency that touches the same APIs. To be checked before any system crate,
  not after.

**Exit cost:** low as long as `oxyn-ui` remains the only crate depending on GPUI
([I-08](../../CLAUDE.md#i-08)) — it is a one-line change in a `Cargo.toml`, plus
fixing the API breakages.

**Reconsider if** a blocking GPUI defect is fixed upstream without being
published, or if releases on crates.io resume a regular pace.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Git dependency on a `rev` of the upstream repository | called into question by the maintainer's decision of 2026-09-05; forbids any publication of Oxyn on crates.io |
| Git dependency following a branch | not reproducible: two builds on two dates give two binaries |
| Vendoring GPUI into the repository | 5.3 MB of source and 65 dependencies to maintain by hand |
