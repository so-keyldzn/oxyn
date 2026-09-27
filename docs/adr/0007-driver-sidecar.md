# ADR-0007 — A sidecar process for drivers with native dependencies

**Status:** proposed · **Date:** 2026-09-05

## Context
Oracle (OCI), Couchbase and some cloud SDKs rely on C libraries. A segfault or a memory
leak in one of these dependencies would bring down the whole workspace and the user's
unsaved work.

## Decision
These drivers run in `oxyn-driverd`, a separate process exposing the same traits over
a local transport. Results travel as **Arrow IPC** (zero-copy). Pure-Rust drivers stay
in process.

## Consequences
* **+** A crash only degrades one connection; the sidecar restarts hot.
* **+** Memory isolation, and the possibility of limiting resources per process.
* **+** The boundary overhead is marginal thanks to Arrow IPC.
* **−** Lifecycle complexity: supervision, restart, cancellation propagation.
* **−** Heavier distribution (extra binary, signing, macOS notarization).
* Postponed to phase 4: no driver of phases 0-3 needs it.
