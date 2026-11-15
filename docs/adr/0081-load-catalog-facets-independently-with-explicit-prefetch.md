# ADR-0081 — Load catalog facets independently and make bulk prefetch explicit

**Status:** accepted · **Date:** 2026-11-14

**Refines:** [ADR-0018](0018-apercu-ddl.md),
[ADR-0022](0022-rafraichissement-automatique.md),
[ADR-0036](0036-l-assistant-complete-le-catalogue.md) and
[ADR-0057](0057-editor-completion-reads-the-catalog-cache.md), on requested
metadata granularity and explicit prefetch. Existing catalog/completion/assistant
budget ceilings are retained; bounded input and publication guards are added.

## Context

At `6647180`, `CatalogRefreshScope::Relation` reads a relation description,
indexes and outgoing foreign keys as one atomic patch. Indexes and outgoing
keys also share the description's IPC freshness. Editor completion can therefore
request data it does not consume. The wide object layout requests a definition
beside its metadata tabs. Issue
[#375](https://github.com/so-keyldzn/oxyn/issues/375) asks for loading control,
but hierarchical and per-object reads already exist.

The [research journeys](../implementation-plan/schema-research-journeys.md)
identify every affected consumer. A visual setting applied only to the tree
would leave completion, the adjacent DDL panel and the assistant outside it.

## Decision

`oxyn-catalog` keeps independent value, freshness and support state for the
description, indexes, outgoing keys, constraints, incoming keys and definition.
A description retains the existing fields and preview guarantees carried by
`Relation`; separating index listing does not remove paging evidence supplied
by the driver.

`CatalogRefreshScope` gains explicit `Description`, `Indexes` and
`OutgoingForeignKeys` reads. They reuse the corresponding existing
`CatalogProvider` methods. The existing `Relation` request remains a
compatibility aggregate: all supported constituent reads succeed before its
patch is published. Its aggregate freshness is not satisfied by a description
alone. Consumers migrate to the narrow request they actually need.

An independent facet publishes atomically, preserves its earlier value on error,
and has its own cancellation outcome. Dependency validation precedes publication:
a late facet for a removed or replaced object cannot attach to its successor.
DDL invalidation affects the relevant object facets; a relisting does not label
unread details fresh. Eviction preserves the difference between never read and
known empty. The executor validates responses independently of driver validation.

`MetadataLoadingPolicy` controls explicit prefetch, not access or AI privacy.
It is stored per connection with optional namespace overrides, separate from
driver connection parameters so that a display/loading edit does not rotate
credentials. Old workspaces default to on-demand loading.

| Preset | An explicit `Preload this scope` request asks for |
|---|---|
| Minimal | Object names in the chosen scope |
| Structure | Names, descriptions, supported indexes and outgoing keys |
| Extended | Structure plus supported object definitions |

Changing the setting writes the preference but starts no bulk read. Expanding a
tree remains a one-level request. Opening a facet, using completion, asking the
assistant or opening a map is an explicit demand for that consumer's bounded
requirements, even beyond the preset. The UI explains this distinction: a preset
never promises that a type of metadata will not be read.

The host reuses native paths and typed facet scopes. Preset planning uses
`MetadataLevel::preload_facets`; consumers explicitly request their required
facets. Demand crosses scoped refresh commands; prefetch is admitted and
audited as `PreloadMetadata`, reusing the same executor read/publication path. User-facing demand
takes precedence over queued prefetch. A superseded prefetch generation is
cancelled; its result cannot publish after the connection/object generation it
belongs to has ended. Scheduling preserves actor/audit attribution.

Existing catalog, definition, completion and assistant budgets remain the upper
bounds. A job reports successful, unavailable, failed and unread facets separately;
it does not loop to defeat eviction or promise a complete in-memory catalog.
No automatic object-count thresholds, unrestricted background indexing or
SQLite exemption are introduced. Preloading a sample-derived description
follows the explicit data-read disclosure in the schema-map proposal.

### Persistence and execution boundary

A transient `CatalogGenerationId` is retired on relisting, invalidation,
forgetting and eviction. It is not persisted as metadata content. Publication
checks the captured token while holding the source cache's publication lock;
budget eviction and application then form one checked update. Narrow reads
retain an observed relation summary, so eviction cannot force a column read or
invent an object kind.

Before publication, a streaming JSON byte counter refuses patches larger than
16 MiB without allocating a second serialized copy. Nested logical types are
bounded before encoding. This bounds publication, not the DBMS's own execution
plan or every protocol's receive buffer. The existing workspace Serde and
Serde JSON packages are direct executor dependencies for this accounting;
no new external package is introduced.

The connection policy has a Minimal default and exact catalog/namespace
scope overrides. Migration 21 stores versioned readable JSON in a dedicated
`metadata_loading_policies` table keyed by the existing connection, separate
from authentication and display preferences. A missing row has revision zero;
updates use the exact observed revision and a JavaScript-safe monotonic token.
A policy accepts at most 32 distinct overrides and 1024 UTF-8 bytes per identifier
segment; the encoded document is capped at 128 KiB. These are input guards,
not thresholds that start loading.

`ReadMetadataPolicy`, `WriteMetadataPolicy` and `PreloadMetadata` are typed
human-only bus operations. A preload names a live owning session and one exact
catalog/namespace scope, and checks the observed policy revision before work.
It never opens another connection, attaches a file or changes a console's
context. At most eight jobs and 256 queued metadata reads are admitted. Each
explicit page has a 60-second deadline and at most 128 described relations,
reporting remaining,
unavailable and failed work distinctly; it never defeats the catalog cache's
existing 1024-scope / 50000-object eviction budget. Policy changes cancel active
preloads without closing editors. Interactive reads take priority over queued
prefetch, and publication rejects invalidated source generations. Continuation
is an explicit cursor tied to the exact publication generation; relisting,
invalidation or eviction retires it. A cursor whose identifier exceeds the
input bound is unavailable rather than truncated. Failure details are limited
to 16 entries and bounded identities, without raw provider diagnostics.
Submitted-scope counts include queued/cancelled scopes; a scope can require
multiple provider calls. Encoded JSON bytes are not wire traffic.

`INFERRED_SCHEMA` already declares that a description reads samples. Ordinary
metadata demand, completion, assistant context and all presets refuse such a
read implicitly. A separate explicit `SampledDescription` scope is human-only,
announces the driver's fixed sample bound and returns inferred field metadata,
never sample values. Indexes and definitions can still be loaded independently.
The existing aggregate `Relation` path does not bypass this disclosure. A
provider exposes pure `schema_sample_limits`, captured on session opening only
when `INFERRED_SCHEMA` is declared. The MongoDB descriptor matches its 1000
returned-document / 8 MiB retained-data / 10-second server-processing limits.
Network batching and the server plan can read more; elapsed cleanup is separate.
A first document exceeding retention bounds fails before cloning it.

The explorer exposes `Metadata loading…` for its active source. The dialog
offers the connection default, an exact observed namespace override, saved
preference reloading, and separate preload/continuation/cancel actions. Choosing
a scope never expands siblings. A stale save requires an explicit re-read;
closing a source cancels only its owned command. The UI exposes progress and
partial coverage even when unsupported facets remain. Saving it performs
no preload. An adjacent DDL panel waits for explicit demand rather than making
Structure load a definition as a side effect. Existing consumer requirements
are narrowed to description, indexes or outgoing keys; availability and
freshness stay independent even after eviction or a partial failure.


A console context change cancels only that session’s preload. Its cache generation
is retired before the provider call and again when it settles, even on error,
so a read queued during the change cannot publish into the resulting context.
Other owners of the same connection remain intact. Continuation also checks
the exact namespace and the reviewed policy revision.

Terminal coverage is measured against the cache after the page, separately
from successful read counts. Fresh/stale/unread facet counts are `null` until
measured, not a false zero. Supported facets lost to eviction make the result
partial even when all provider calls succeeded; the job never reloads them in
a loop. Reported bytes count encoded published patches, not current cache
occupancy or network traffic. A generation retired before the final snapshot
stops the job rather than labelling stale coverage complete.

### Schema-dependent preview preparation

`Session::plan_preview` receives an optional fresh description from the exact
owning session’s cache. The default preserves existing native planning for
non-inferred engines and refuses inference-prone adapters until they implement
an inference-free planner. The hint is not webview input or permission to
sample. No second driver-side metadata cache is introduced.

MongoDB reuses this explicit description for sort/projection/independent-page
validation. If absent or stale, preparation fails with an action to read
Structure explicitly; it never issues an extra document sample. Plain previews
continue using declared object-kind metadata without a schema sample. Inferring
Arrow types from rows the user actually requested is separate from issuing an
additional exploratory read. The same rule applies to approved row-sample
projection, which cannot silently read whole documents to discover fields.

## Consequences

* **+** Index and relationship reads stop being an implicit cost of every column read.
* **+** Loading controls have the same meaning in the tree, editor, map and assistant.
* **−** More facet states must be propagated through cache, IPC, UI and invalidation.
* **−** Separate reads do not promise a single database snapshot; freshness and
  failures remain visible. Extended loading can finish partially under the budgets.

**Exit cost:** collapse facets back into the aggregate, migrate stored policy
preferences and simplify each consumer. Provider methods stay reusable; no disk
catalog or new server protocol is introduced.

**Reconsider if** measurements show that extra round trips cost more than the
reads avoided, or repeated user workflows need a different prefetch trigger.
Any new trigger must state its scope and cancellation behavior; any enlarged
budget requires its own measured decision.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| L1/L2/L3 implemented only in the catalog tree | Other consumers would silently keep doing the supposedly disabled work |
| Prohibit explicit detail reads above the selected level | Makes opening Structure or DDL fail even when the user specifically requests it |
| Keep one freshness timestamp for all details | A successful column read would conceal missing or stale index/key reads |
| Always load everything for local files | Locality does not establish a bounded object count, payload or rendering cost |
| Automatic thresholds copied from the issue | No measurements establish those thresholds for Oxyn's drivers and consumers |
