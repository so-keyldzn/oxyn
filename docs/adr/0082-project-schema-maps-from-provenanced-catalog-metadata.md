# ADR-0082 — Project schema maps from catalog metadata with explicit provenance

**Status:** accepted · **Date:** 2026-11-14

**Refines:** [ADR-0003](0003-driver-capabilities.md),
[ADR-0041](0041-registre-d-actions-menus-et-raccourcis.md) and
[ADR-0081](0081-load-catalog-facets-independently-with-explicit-prefetch.md).

## Context

The standalone map must cover relational, documentary and key-space sources.
The assistant already uses React Flow/Dagre, but its separate assembler loses
object kinds, field provenance and complete owner identities. Native hierarchy,
foreign keys, mappings and inferred fields have different meanings; the
[engine matrix](../implementation-plan/schema-research-engines.md) records the
available metadata rather than promising every server feature.

## Decision

The map is an ordinary workspace tab available without an AI provider, opened
from the selected source or a native catalog/namespace/object. Opening, choosing
a scope, drawing, filtering, selecting, panning and zooming start no native
metadata or row read. Loading a scope, refreshing, expanding one selected
object's one-hop neighbors, and opening Data/Structure/definition are separate
human gestures. Native file/session ownership is preserved; no connection or
attachment is opened to fill a map. A removed SQLite file or closed ephemeral
owner is never replaced by a newly created empty source.

Core carries the typed human-only `LoadSchemaMap` command. `oxyn-exec` owns
its bounded, cancellable reads through the existing session metadata queue and
atomic generation checks. Demand can overtake queued prefetch, but cannot
publish into a changed source. A 60-second pass awaits cancellation cleanup;
it retains available parts and reports failed/unavailable parts independently.
The desktop bridge checks window/session ownership and returns validated IPC.

`oxyn-catalog` owns a pure `SchemaMapProjection`, with no I/O or coordinates.
A shared frontend adapter/renderer draws this projection in the standalone map
and assistant. The assistant retains only name resolution against already
listed objects and a request adapter; it does not guess ambiguous names or
automatically expand neighbors. Its coverage text names unread/stale/omitted
parts and unloaded endpoints. Map metadata reaches no AI destination by itself;
any context sent to an assistant still uses the single privacy gateway.

Object keys contain the connection, original session and all native optional
address levels. Field keys contain exact segments, preserving literal dots.
Ordered composite members distinguish unnamed keys. Display labels are never
identities. Reported object kinds, field provenance and independent freshness
are retained. Only fresh declared primary keys can prove uniqueness; absence
of proof remains unknown cardinality. Stale links remain identified as stale.

Links have distinct kinds: reported foreign key, technical dependency, native
membership and field containment. The canvas draws reported FK edges; the
accessible object/link list, breadcrumbs and inspector also expose membership
and nesting. Unknown or malformed composite members are never repaired by
pairing guesses. Reversing loaded outgoing keys gives incoming edges only
within that loaded set. References to unloaded targets stay visible in the
list/inspector. No column-name or DDL parsing invents links. Structured
technical dependencies remain explicitly unavailable until the provider
contract exposes them.

| Available metadata | Representation |
|---|---|
| Relations with FK introspection | Object cards and reported key edges |
| Relations without key introspection | Object cards with unavailable keys |
| Collections with explicit sampled fields | Collection cards, nested fields and sample provenance |
| Server mappings | Reported field hierarchy; no invented collection relationships |
| Key-space objects | Key/type cards and preview-layout provenance, not a declared document schema |

Availability follows the original session's capabilities, not an engine-name
allowlist or `GRAPH` flag. Missing optional facets do not erase useful objects.
No blanket SQLite exemption, all-database introspection, automatic AI call or
implicit document inference is introduced. A separate human confirmation names
the object/source and provider-declared document/byte/time limits before a
sample. Only type/field metadata enters the map, never sampled values.

The existing limits of 40 objects and 12 displayed fields per object remain;
nested fields share the same total. Publication/rendering additionally bounds
links at 200. Scope/field/link omissions, unread/stale descriptions and unresolved
endpoints are visible, and a completed bounded pass is never called a complete
database schema. Displayed type strings are capped at 256 characters with an
explicit shortened flag; oversized native identities/members are omitted rather
than truncated into another identity. The shared 16 MiB encoded metadata guard
also covers derived maps, whose qualified keys repeat paths. If necessary,
relationships and then field sets are reduced with omission counts while
retaining object identities. Failure to fit identities is an explicit error.

A refresh preserves node positions whose complete keys survive. Filters hide
retained nodes rather than rebuilding graph identity. Layout changes only with
the projected graph, so a local filter does not rerun Dagre. The
[completion ledger](../implementation-plan/completion.md) records measurements
and actual validation. F6 uses the existing visible-region registration.

Durable layouts, image export, an offline schema snapshot and graphical DDL
editing are separate lots. This decision introduces no persisted map format.

## Consequences

- The assistant and standalone view share one interpretation across engines.
- Original sessions and independent cache facets remain the authority boundary.
- Provenance, partial states and unresolved endpoints add model/UI complexity.
- Native dependencies and richer graph/vector/time-series views need provider
  metadata rather than an extension badge or name heuristic.
- A bounded graph can omit objects or fields; initial layout can exceed a frame
  budget. No larger rendering limit is justified by this implementation.

**Exit cost:** replace the pure projection, bus DTO and two renderer adapters;
retain native catalog reads and ownership. No persisted layout migration or new
rendering package is involved.

**Reconsider if** measured graph sizes require another rendering approach,
or users need offline snapshots or manually asserted links. Either extension
changes persistence/provenance and needs a separate decision.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Ask an LLM to reconstruct the schema | Makes deterministic inspection depend on inference and an AI destination |
| Render every engine as SQL tables with foreign keys | Misrepresents documentary and key-space structure |
| Separate frontend graph assemblers | Diverges on identity, freshness, provenance and composite keys |
| Parse DDL or names to guess dependencies | Duplicates dialect knowledge and can invent links |
| Persist the graph as current source truth | Freezes partial metadata and contradicts the in-memory catalog contract |
| Fail the entire map for an unsupported optional facet | Prevents useful inspection on sources with fewer capabilities |
