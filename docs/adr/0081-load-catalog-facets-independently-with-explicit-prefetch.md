# ADR-0081 — Load catalog facets independently and make bulk prefetch explicit

**Status:** proposed · **Date:** 2026-11-13

**Refines:** [ADR-0018](0018-apercu-ddl.md),
[ADR-0022](0022-rafraichissement-automatique.md),
[ADR-0036](0036-l-assistant-complete-le-catalogue.md) and
[ADR-0057](0057-editor-completion-reads-the-catalog-cache.md), on requested
metadata granularity. This proposal changes no current budget or delivered behavior.

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

The host assembles demand and prefetch plans from one pure catalog policy; all
reads still cross `RefreshCatalogScope` and the policy gate. User-facing demand
takes precedence over queued prefetch. A superseded prefetch generation is
cancelled; its result cannot publish after the connection/object generation it
belongs to has ended. Scheduling preserves actor/audit attribution.

Existing catalog, definition, completion and assistant budgets remain the upper
bounds. A job reports successful, unavailable, failed and unread facets separately;
it does not loop to defeat eviction or promise a complete in-memory catalog.
No automatic object-count thresholds, unrestricted background indexing or
SQLite exemption are introduced. Preloading a sample-derived description
follows the explicit data-read disclosure in the schema-map proposal.

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
