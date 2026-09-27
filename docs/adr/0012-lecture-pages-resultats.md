# ADR-0012 — Read result pages outside rendering and bound their cache in bytes

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0002](0002-arrow-result-model.md), on re-reading and its
budget; [ADR-0004](0004-command-bus.md), on the local read command.

## Context

`DataGrid` correctly refuses I/O during rendering, but triggers no re-read of
spilled batches. They remain drawn as ellipses. `SpillCache` keeps four batches
with no bound in bytes, on top of the resident batches. This limit does not
bound memory when batch sizes vary. The budget decided by PERFORMANCE remains
256 MiB per result.

The spill code already uses self-contained Arrow IPC streams, read with
`StreamReader`. It does not use memory mapping. This decision makes that
divergence explicit without introducing `unsafe`.

## Decision

- `ReadResultPage` carries connection, result and batch number. It is a local
  read subject to the same bus and the same policy for human and agent. The
  executor checks that the result belongs to the connection. It does not contact
  the server, composes no SQL and re-runs nothing.
- The view emits a request when a visible batch is missing. Reading and decoding
  happen off the UI thread. The response is correlated with the result and the
  grid generation; an old response does not replace current data. Cancellation
  and error are visible, with no automatic retry after an error.
- The retention budget of `ResultBuffer` covers the resident batches and the
  re-read cache. When spilling is allowed, a quarter is reserved for the cache
  and three quarters for the initial batches. Without spilling, the whole budget
  remains available for the initial batches. The cache evicts by use and counts
  Arrow bytes and entries, not only batches.
- A positioned read keeps the existing Arrow IPC format. Its decoding copy and the
  references temporarily held by readers are not cache retention. They must be
  measured separately; a bound on the cache does not prove a bound on the
  process RSS.
- A batch that alone exceeds the re-read budget is not kept in this cache. The
  view receives an explicit error rather than a loading loop or unbounded growth.
  Export keeps reading batches as a stream.

## Consequences

- **+** Spilled rows become viewable without disk access during rendering.
- **+** The retention budget really covers both categories of batches.
- **+** The command carries no cell content to the log or to the model; the
  result stays in the shared buffer.
- **−** Initial batches spill earlier, in favor of re-reading.
- **−** Positioned re-reading adds a temporary copy; it does not benefit from the
  memory mapping initially described by ADR-0002.
- **−** A custom budget smaller than the size of a single batch can prevent it
  from being displayed, even though its export remains possible.

**Exit cost:** replace the page storage in `oxyn-data` and its loading in
`oxyn-app`, then rework the memory and cancellation tests. The driver protocol,
the Arrow data and the user's SQL do not change.

**Reconsider if** measurements show a dominant copy, excessive memory pressure
during concurrent reads, or an eviction that prevents a normal viewport from
settling. A possible memory mapping requires its `unsafe` review and is not
introduced as a mere optimization.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Read `batch()` directly during rendering | Blocks the UI thread on disk and decoding |
| Re-run the query on scroll | Changes the result and may repeat server effects |
| Add a UI cache without sharing the budget | Doubles retention and scatters its control |
| Consider four batches a memory bound | The size of a batch is not constant |
