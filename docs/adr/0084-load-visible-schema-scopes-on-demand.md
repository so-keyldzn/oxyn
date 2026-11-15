# ADR-0084 — Load visible schema scopes on demand

**Status:** proposed · **Date:** 2026-11-15

**Refines:** [ADR-0082](0082-project-schema-maps-from-provenanced-catalog-metadata.md)
on the gesture that authorizes a map read, and
[ADR-0081](0081-load-catalog-facets-independently-with-explicit-prefetch.md)
on how optional bulk prefetch is presented.

## Context

The user reports that choosing a database still leaves its map behind another
Load action. The metadata dialog also presents preloading beside ordinary
preferences, suggesting that exploration needs preparation. Object facets and
completion already use bounded demand reads. These separate gestures make the
interface inconsistent without adding useful authority for ordinary metadata.

## Decision

Opening a visible schema-map tab or choosing its native scope expresses the
human's demand to view it. The controller immediately requests the existing
bounded LoadSchemaMap command on its original connection/session. Duplicate
mounts coalesce; a superseded scope cancels its exact request and rejects late
answers. Hidden maps do not start new reads. A single listed native child may
advance automatically; ambiguous children require a choice, never a guessed
namespace. No traversal of all databases is introduced.

Refresh, cancellation, explicit neighbour expansion and partial coverage remain
available. Filters, pan, zoom and selection remain local. Closed ephemeral or
deleted-file sources are not reopened. Documentary sampling remains an explicit,
confirmed, bounded operation; choosing a scope never samples rows or invokes AI.

Metadata preferences describe normal demand loading. Optional bulk cache warming
is collapsed under Advanced and is never required before a view can be used.
Saving preferences, expanding Advanced or opening settings starts no preload.
Existing backend prefetch bounds and cancellation remain intact.

Source actions use compact shadcn sidebar controls and semantic Hugeicons.
Map, transfer and result tabs have distinct icons. Existing semantic theme tokens,
keyboard focus, source ownership and safe confirmation defaults remain authoritative.

## Consequences

- **+** Choosing what to view directly displays its supported metadata.
- **+** Common behaviour applies across engines through native scopes/capabilities.
- **−** Visible scope changes now incur a bounded read; loading and cancellation
  must be evident and failures must not cause automatic retry loops.
- **−** Ambiguous native levels and inferred structures still require an explicit
  choice or sample consent; a map can remain partial under its existing bounds.

**Exit cost:** restore the separate load gesture in the controller/view and its
interaction tests. No persistence format or driver contract changes.

**Reconsider if** measured demand reads on visible scopes exceed the existing
budgets, or user testing shows accidental scope selection causing costly work.
Do not raise bounds or add automatic sampling as a workaround.

## Rejected alternatives

| Alternative | Reason |
|---|---|
| Eagerly preload every connected database | Unbounded work unrelated to the visible demand |
| Automatically sample documents | Changes privacy/read cost without consent |
| Keep choosing a scope and loading it as separate gestures | Retains the reported friction |
| Remove backend prefetch while only hiding its controls | Leaves contracts/callers inconsistent |
