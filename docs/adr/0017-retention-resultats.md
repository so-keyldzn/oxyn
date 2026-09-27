# ADR-0017 — Bound retained results that no longer have a reader

**Status:** accepted · **Date:** 2026-09-10

## Context

The library can reopen a `ResultId` without executing SQL, but the executor's
registry kept all buffers until the process ended. The individual 256 MiB bound
does not prevent many results from accumulating.

## Decision

The registry keeps at most 16 results without a reader, with cumulative caps of
256 MiB of resident batches/decoded cache and 1 GiB of IPC spill for this
category. The oldest are evicted first. A reference held by a view, an execution
or an export protects its buffer from this eviction.

The check happens after execution and periodically in the backend. Removed
objects are destroyed outside the registry lock and outside the UI thread.
Results being viewed stay protected; the budgets are therefore not a global
memory cap for all open views. The index and temporary allocations remain to be
measured according to PERFORMANCE.

An evicted history reference remains visible but becomes unavailable. It is never
recreated by replaying SQL. Closing its view does not change the server's data.

## Consequences

- **+** A long session of successive queries does not keep all its unused
  results.
- **+** An export or an open view keeps its data and its local pages.
- **−** An old result can expire before the application is closed.
- **−** Results held by views remain the responsibility of their readers.

**Exit cost:** change the registry and its checkpoints, without changing the
Arrow buffers, the SQL or the persistence of history references.

**Reconsider if** on-disk retention across restarts is requested, or if
measurements impose a global budget shared between active views.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep all results | Unbounded growth of the registry |
| Evict a view that is still open | Its page reads and exports become unavailable |
| Re-run a query to recreate the result | Changes the observed data and may repeat a write |
