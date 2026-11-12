# ADR-0077 — Retain preview streams for native pagination

**Status:** accepted · **Date:** 2026-11-12

**Refines:** ADR-0028, ADR-0060, ADR-0061, ADR-0062 and ADR-0068: the first-page restriction remains for independent queries without a total order; a retained execution can be traversed without inventing a unique key.

## Context

Data offers pages of 50, 100 or 200 rows. ClickHouse sorting keys are not unique; Elasticsearch/OpenSearch and Redis collections expose protocol continuations instead of SQL offsets. Re-executing an unordered query for each page would repeat or omit records. Keeping a whole collection in the webview would violate the memory budget.

## Decision

`PreviewRelation` remains the command, policy and audit boundary. `Session::prepare_preview` returns the existing bounded request by default; drivers can instead declare a retained request. `Session::execute_preview` defaults to ordinary execution. The executor still reclassifies the composed request, enforces read-only, uses its normal execution events and drains a bounded page through `ResultBuffer`.

A continuation names an earlier page's opaque `ResultId`. The executor binds it to the connection, session, relation, page size, sort, filter and projection. It rejects changed shapes, expired identities and offsets beyond the rows already fetched. The next page advances the same cursor; previous pages read its retained Arrow batches. Each exposed result contains only one page. A complete page permits a next request, including the empty page after an exact boundary. No total count is requested.

History uses 8 MiB of memory and at most 128 MiB of Arrow spill per stream, at most 8192 batches and 16 retained page identities. Reaching a bound produces an explicit error, never a false end of data. A stream lives at most five minutes. A held cleanup task cancels its token and drops its cursor on expiry; releasing its last page, closing its session, disconnecting or abandoning a page also releases the stream. Interrupted cursor futures are never resumed.

ClickHouse retains one HTTP result, with backpressure, without OFFSET or a universal snapshot claim. Elasticsearch/OpenSearch retain a PIT and use `search_after`, with hit counting disabled. Elasticsearch uses `_shard_doc` as the final tie-breaker; OpenSearch 3.3 and later use `_index`, then `_shard_doc`, whose shard number is index-local. Older OpenSearch traverses one concrete index/shard at a time, in `_doc` order inside a PIT restricted by `_shards` preference and an `_index` filter. `_search_shards` lists at most 128 partitions; original alias filters and routing remain attached to the original target. Only one partition PIT is held at a time; partitions opened later may see later data. On these older releases, sorting keeps the bounded first-page preview; clearing the sort enables native traversal again. OpenSearch 2.0–2.3 has no PIT and retains its existing bounded preview without pagination. This avoids assuming that `_id` is unique when custom routing is present, and avoids loading ID fielddata. PIT creation responses and cleanup requests have owned tasks, including an unreceived channel delivery; renewed IDs are retained before decoding hits. Server identities never cross IPC. Redis traverses lists, sorted sets, hashes, sets, streams, vector sets and time series through their native bounded range or scan operations. Scan duplicate identities are removed within a byte bound; reply remainders and stream-entry fields are not discarded at a UI page boundary. Concurrent Redis changes are not a snapshot.

The UI sends a continuation only for navigation. Refresh, filter, sort, size change and reconnection start at page one. It keeps the old result until the new read settles, then releases it. The footer explains that pages belong to one retained read and that refresh starts another.

## Consequences

* **+** The same footer can navigate every implemented protocol that returns collections.
* **+** No fabricated key or client-side full materialization.
* **−** Reading holds a remote resource until release or expiry; a large sort can still cost the server a full sort.
* **−** Expiry or a quota requires refreshing or using a console. Older OpenSearch opens partitions successively and does not promise a single cross-shard snapshot or sorted pagination.
* **−** Redis mutations during traversal can affect unseen rows; returned pages remain the ones already read.

**Exit cost:** replace the retained-stream adapter and driver preparation methods; the UI and bounded page result contract remain unchanged.

**Reconsider if** every supported search variant offers an identical PIT tie-breaker, or measurements show that held search/HTTP resources exceed the five-minute and sixteen-entry bounds.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| OFFSET on non-unique keys | No stable sequence across independent reads |
| Fetch the entire source before showing pages | Unbounded latency and unnecessary server work |
| Pass server cursors to the webview | Leaks protocol state and makes ownership and cleanup ambiguous |
| PIT with a presumed `_shard_doc` on every OpenSearch release | The supported older variants do not share that guarantee |
| Scroll | Both engines require exact hit counting in a scroll context |
| PIT sorted only by `_index`, `_id` | Custom routing can create the same ID on multiple shards of one index |
