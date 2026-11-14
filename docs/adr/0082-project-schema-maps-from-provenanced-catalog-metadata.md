# ADR-0082 — Project schema maps from catalog metadata with explicit provenance

**Status:** proposed · **Date:** 2026-11-13

**Refines:** [ADR-0003](0003-driver-capabilities.md) on presentation capabilities
and [ADR-0041](0041-registre-d-actions-menus-et-raccourcis.md) on the map's
workspace actions. This proposal does not replace the delivered assistant ERD
contract or enable unsupported database introspection.

## Context

The standalone map requested during schema research must cover relational,
documentary and key-space sources. At `6647180`, the assistant already renders
an ERD using React Flow and Dagre. Its model contains tables and foreign keys,
but omits object kind and inferred-field provenance; its starting reads ask for
incoming keys even when the driver lacks that capability.

[The engine matrix](../implementation-plan/schema-research-engines.md) shows why
one SQL-style graph cannot represent every source honestly. Declared keys,
mapping membership and sample-derived fields have different meanings.

## Decision

The map is an ordinary workspace tab, available without an AI provider. It
opens from a selected connection, namespace or object; the assistant may open
the same map with a preselected scope. UI selection and painting perform no
server access by themselves. Explicit metadata demands use the bus and
[ADR-0081](0081-load-catalog-facets-independently-with-explicit-prefetch.md).

`oxyn-catalog` owns a pure `SchemaMapProjection`: object identities, field
descriptions, typed links, provenance, freshness and coverage. It contains no
layout coordinates and performs no I/O. `oxyn-desktop` orchestrates authorized
reads and creates a bounded IPC projection. A shared front-end map feature
owns React Flow rendering, Dagre placement and navigation; the assistant retains
only its request adapter, not a second graph assembler.

A node's identity includes `ConnectionId` and the complete validated object
address. Display labels are not keys. Object kind is retained; field metadata
distinguishes declared, observed and unavailable information. Sample-derived
fields carry coverage/provenance, not example row values.

Links have explicit kinds: declared foreign key, reported object dependency,
reported membership, or field containment. Each includes its source and
coverage. Unknown cardinality stays unknown. Reversing the loaded outgoing-key
set establishes incoming edges only within that set. A link to an unloaded
object shows an unresolved endpoint instead of implying complete discovery.
Names such as `user_id` never create a link.

| Available metadata | Representation |
|---|---|
| Relations and foreign keys | Object cards and declared relationship edges |
| Relations without key introspection | Object cards; no invented relationships |
| Collections and observed fields | Collection structure with sample provenance |
| Mappings and reported membership | Field hierarchy and typed technical links |
| Sampled key space | Key/type overview; optional prefix grouping marked as a convention |
| Future graph metadata | Labels/types/properties with the driver's explicit certainty and coverage |

The engine's `GRAPH` capability does not gate this screen. Availability follows
the introspection actually exposed by the session. A missing optional facet or
one failed object produces a partial map, not a fabricated empty graph or total
failure of already read objects. Permission-denied objects reveal no metadata
beyond what the authorized response actually provided.

The map starts from a chosen bounded scope and expands neighbors explicitly.
Existing ERD display bounds and catalog budgets remain in force until measured
changes are accepted. Large scopes show drawn/omitted/unread counts and offer
filtering or a smaller scope. A completed bounded pass is not called a complete
database schema.

Sample-derived reading is disclosed before starting and follows existing
connection policy. Local maps initiate no AI request; sending their metadata
to an assistant still uses the single privacy gateway. Refresh cancels/replaces
the active generation, marks old data stale and preserves stable node positions
where identities survive. Keyboard access and an object/link list accompany
the canvas; F6 remains an ordinary workspace action.

Optional `SchemaMapView` persistence stores a versioned scope, selected IDs,
positions, filters and viewport in `oxyn-store`, through a human-only bus
command. It stores no schema snapshot, row samples or credentials. Reopening
shows saved view choices and requires an explicit read for current metadata.
Image export is an explicit human action and labels partial/stale content;
its renderer cannot expand the scope or perform additional data reads.

## Consequences

* **+** The assistant and standalone view share one interpretation of metadata.
* **+** The same UI can expose useful structure without pretending every engine has FKs.
* **−** Provenance, partial results and inaccessible endpoints complicate the model.
* **−** A saved layout is not an offline schema archive, and a bounded map can
  omit objects. Additional engine-specific edges require actual driver work.

**Exit cost:** replace the projection and renderer adapters, migrate saved view
choices and keep the existing catalog reads. Reusing current rendering packages
avoids introducing a second graphics stack; no new package version is selected.

**Reconsider if** measured graph sizes exceed the existing rendering approach,
or users need an offline catalog snapshot or manually asserted links. Either
extension changes persistence or provenance and needs a separate ADR.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Ask an LLM to reconstruct the schema | Makes a deterministic inspection depend on inference and an AI destination |
| Render every engine as tables with foreign keys | Misrepresents documentary and key-space structure |
| Parse DDL in the front end to guess dependencies | Duplicates dialect knowledge and can invent incorrect links |
| Persist the whole graph as the current schema | Freezes partial or sensitive metadata and contradicts the in-memory catalog contract |
| Fail the whole map when one optional facet is unsupported | Prevents useful inspection on drivers with fewer capabilities |
