# Schema research: user journeys

Status: **research proposal, not an accepted product contract**. Prepared on
2026-11-13 against source snapshot `6647180`. This is an appendix of the
[implementation plan](../IMPLEMENTATION-PLAN.md), not a second roadmap.
The [engine matrix](schema-research-engines.md) owns the capability inventory.

**Revalidation — 2026-11-13:** compared `6647180..f456ff1` after synchronizing
with local `main`. The SQLite URI/private-reader fix, preview-popover test
adjustment and preparation of version 0.0.18 introduce no blocking contradiction
with these journeys or proposed ADRs 0079–0082. The initial snapshot above is
retained as historical evidence. SQLite acceptance cases are expanded below
and in the matrix. This is static revalidation: the new SQLite tests were read,
not executed; it does not establish a passing application quality gate.

Scope: issues [#308](https://github.com/so-keyldzn/oxyn/issues/308),
[#373](https://github.com/so-keyldzn/oxyn/issues/373),
[#374](https://github.com/so-keyldzn/oxyn/issues/374),
[#375](https://github.com/so-keyldzn/oxyn/issues/375), and the requested standalone
schema map. The initial research pass specified journeys and observable acceptance cases only:
no application code, server access, new dependency or accepted ADR.

**Implementation revalidation — 2026-11-14, base `ce58839`:** the multi-base
work now preserves main's separate catalog/console purposes and retained native
preview occupancy. New PostgreSQL discovery stays separate from catalog
traversal; grouping leaves other engines' native namespaces unchanged. Proposed
research ADRs are now numbered 0080–0083 because main accepted ADR-0079 for
original-session previews. SQLite cases below remain applicable; none receives
an automatic prefetch exemption. Current code and test status are tracked in the
[completion ledger](completion.md), separately from this historical inventory.

## Reading this proposal

**Existing** means found by static inspection, not exercised against a server.
**Proposed** describes the intended journey, subject to the decisions below.
Screen labels are working English labels, not approved copy.

The applicable contracts remain [UX-SPEC](../UX-SPEC.md),
[SECURITY](../SECURITY.md), [DRIVER-CONTRACT](../DRIVER-CONTRACT.md),
[AI-PROVIDERS](../AI-PROVIDERS.md) and [PERFORMANCE](../PERFORMANCE.md).
This proposal does not declare their phase gates passed. The plan still records
end-to-end performance measurements not produced; the standalone map also touches
the ER-diagram work listed in phase 4. Changing phase order needs a separate decision.

## J01 — Browse several databases on one server

**Entry:** saved connection or proposed server profile; human chooses
`Browse databases`.

**Existing:** PostgreSQL introspects its connected database; several other
drivers already list multiple databases or namespaces. The workspace identity
is a connection, not a server group.

**Proposed flow:** show the profile's destination and access scope; discover
database names without opening them all; expand a selected database; open its
schemas/objects; open a console whose destination stays visible.
Use a separate technical connection per PostgreSQL database. Preserve each
driver's existing hierarchy rather than adding an artificial level everywhere.

Setup chooses an authorized seed connection or creates one first. Each child
opening reviews its own credentials and protections. The explorer stays common,
but each base keeps its workspace and tab strip. One click selects; the
disclosure control expands; Enter, double-click or Open workspace activates.
Header and assistant follow the active workspace, not the selected row.
Connected inactive bases keep their sessions, drafts and reported transactions.

**Failure and recovery:** a denied database stays identifiable with a local
error; other databases remain usable. Cancellation stops the pending opening.
Deleting or editing a profile must account for its open children and credentials.
Track opening and cancellation per child, reserve capacity before opening,
and show the existing window limit without evicting another base's work.

**Acceptance:** with identically named tables in bases A and B, selecting B
changes neither A's draft nor A's transaction. Discovery opens no session per
listed database. No old secret is reused outside its authorized destination.
Selecting B while A is active cannot redirect A's menus or execution.
Repeated opening of B does not allocate duplicate sessions.
**Decision:** D01.

## J02 — Inspect a table, view or other object

**Entry:** catalog, search result, related object or map node.

**Existing:** Data, Structure, Indexes, Constraints, Relations and DDL share an
object view. The former shared `detail` request is split into independent
description, indexes and outgoing-key reads by #375. The adjacent definition
now waits for an explicit request; validation is recorded in the completion ledger.

**Proposed flow:** identify connection, database, namespace and object; open the
appropriate view; request only its required facets; show freshness and
availability independently; navigate to a related object without changing the
original console. The Data view can request the key metadata needed for paging.

**Failure and recovery:** preserve previous information with a stale marker;
separate unread, empty, unsupported, inaccessible and failed. A late response
for the previous object cannot replace the selected object's display.

**Acceptance:** opening Indexes does not reload unrelated facets. A view without
a proven paging key does not gain a fabricated key. Constraint summaries do not
claim there are no unique indexes when indexes have not been read.

**SQLite acceptance:** exercise [SQ01–SQ05](schema-research-engines.md#sqlite-target-cases)
when opening Data from the tree or map. Preserve the source URI's identity and
non-access options when creating an isolated read-only preview. Named shared
memory must reach the same live database. For keyless objects in private-memory
or unnamed temporary sources, keep the bounded read on the original session:
neither offer an unsupported retained continuation nor open an empty substitute.
Failure to reopen a removed file is an error, not an empty preview or file creation.
Metadata availability and retained-preview availability remain distinct.
**Decision:** D02.

## J03 — Control metadata loading

**Entry:** `Metadata loading…` in the active source explorer.

**Existing:** hierarchical reads, per-object facets, bounded cache, editor and
assistant completion passes. #375 adds manual Minimal/Structure/Extended
preferences and exact native namespace overrides, with validation tracked in
the [completion ledger](completion.md).

**Proposed flow:** choose a preset, see whether it is inherited, inspect its
scope, and explicitly request further details when needed.

| Working preset | Anticipated reads in the selected scope | Explicit reading |
|---|---|---|
| Minimal | Object names | Any supported facet when opened |
| Structure | Names, columns, indexes and outgoing keys | Definitions and other facets |
| Extended | Structure plus supported definitions | Unsupported object families remain unavailable |

These manual presets do not enlarge existing budgets or promise that the
whole database remains in memory. Selecting a preset is distinct from starting a bulk read;
the trigger is an explicit selected-scope preload action. Saving the preference
or expanding a level starts no bulk read. Each explicit page stops at 128
relations / 60 seconds and offers a generation-bound next page; unsupported,
failed, fresh, stale and unread facets remain distinct. Cancellation reaches
the original session. Preferences are local readable JSON, separate from auth.

**Failure and recovery:** show pending, cancelled and incomplete work; lowering
the preset stops obsolete prefetch work without discarding the current editor.
Keep missing information distinguishable from an empty result.

**Acceptance:** tree, completion, diagram and assistant apply one policy;
switching screen width does not bypass it through the adjacent DDL panel.
No automatic object-count threshold or SQLite exemption is assumed.
**Decision:** D02; measurements do not authorize automatic thresholds.

## J04 — Open a relational schema map without AI

**Entry:** `View schema map` on the explorer source or a native
catalog/namespace/object. The assistant uses the same host projection; its
block remains in the conversation.

**Existing:** the assistant ERD uses catalog metadata, React Flow and Dagre.
Its source names are resolved only among already listed objects. The standalone tab now reuses its renderer with the shared host projection.
Movable saved layouts and image export remain separate scope.

**Proposed flow:** select scope and object kinds; list available objects; draw
the selected objects; load supported columns and links progressively; expand
neighbors on request; inspect a node/link or open Structure/Data/DDL.
Zooming, panning and selecting a node request no rows. Opening Data retains the
ordinary preview behavior and its policy.

**Failure and recovery:** keep useful nodes if one read fails; mark unknown
links and unloaded endpoints. A source without incoming-key introspection can
still draw objects and available outgoing keys. Inverting loaded outgoing keys
only establishes incoming links within that loaded scope.

**Acceptance:** cycles, composite keys and homonymous objects remain distinct;
no link is invented from a column name; unknown cardinality stays unknown.
Counts distinguish drawn, omitted and unread objects, with an accessible list.
Persisting a layout must not persist secret-bearing metadata or samples.
**Decision:** D03; use existing ERD bounds until measurements justify changes.

## J05 — Inspect a documentary or key-space structure

**Entry:** the same map action, with a representation selected from actual
introspection support; see the engine matrix for the candidate renderers.

**Proposed flow:** show collections/mappings/key groups; expose nested fields
where available; distinguish declared structure from observed samples.
For sample-derived structure, explain that data will be read, show the bounded
scope and allow cancellation. No observed value belongs in an exported diagram.
Local viewing does not initiate an AI request.

**Failure and recovery:** an empty sample means no observed fields, not proof
of an empty schema. Mixed types remain mixed. Inaccessible structure is not
replaced with guessed relationships. Grouping Redis keys by a naming convention
is labelled as grouping, not a database constraint.

**Acceptance:** MongoDB observations, Elasticsearch mappings and Redis key
groups cannot be mistaken for SQL foreign keys. A structure view can succeed
without any edges. No exhaustive collection/key-space scan starts implicitly.
**Decision:** D03; a new driver is a prerequisite for future engine families.

## J06 — Query, copy and ask about the selected object

**Entry:** object menu, SQL console, completion or assistant.

**Proposed flow:** copying an INSERT template requests columns; copying a
definition requests that definition; opening text in a console executes nothing.
Completion resolves names within that console's context. The assistant names
its connection and the coverage of the metadata it received.

Grouped opening starts a new base with an empty console. Copy SQL to database
names source and destination explicitly. A cross-base object drop inserts
nothing and offers a console on the object's base or Cancel. Delayed text
requests retain their destination connection and console generation; a changed
active base is never their replacement target. This refinement is proposed in
ADR-0080; the existing ungrouped draft-copy behavior is not claimed changed.

**Failure and recovery:** switching the active database cannot retarget a
pending command, approved sample, pinned object or conversation silently.
Unavailable metadata remains visible as missing context.

**Acceptance:** two same-named objects from different bases never exchange
suggestions, result buffers or approved samples. Existing Rename/Drop/Truncate
actions remain capability-checked and go through their current review and policy.
Adding a full visual table/view editor is outside this research scope.
**Decisions:** D01/D02; existing AI gateway and production protections remain.

## J07 — Export a PostgreSQL database

**Entry:** `Database transfers` on the explorer source.

**Implemented flow:** identify source; choose an installed executable, supported
format and destination; review options; start; show progress without inventing
a percentage; cancel or receive a confirmed completion and file location.
Do not conflate this with exporting a grid's retained rows.

**Failure and recovery:** missing/incompatible executable, authentication
failure, denied destination, disk full or child-process failure are explicit.
An interrupted file is never presented as a successful backup; an existing
destination is not overwritten silently.

**Acceptance:** no shell command text or secrets appear in IPC/logs; successful
completion requires process success and finalized output. Closing the progress
surface has an explicit job-lifecycle rule, not an orphaned process.
**Decision:** D04. PostgreSQL is the initial restricted adapter; validation is recorded in the completion ledger.

## J08 — Restore into a deliberately selected database

**Entry:** Restore archive in Database transfers; file and target are explicit.

**Implemented flow:** inspect supported format/options; show target identity and
destructive effects; authorize through the bus and required native confirmation;
run as a tracked job; report its outcome; invalidate affected metadata/previews.
The reviewed target must be the actual target even if the archive names another.

**Failure and recovery:** distinguish refused, failed-before-start,
partially-applied, cancelled-with-unknown-outcome and successful.
Cancellation promises stopping work, not undoing it. Offer inspection after an
ambiguous outcome; never replay automatically.

**Acceptance:** read-only targets are refused; production confirmation names
the actual connection; an agent cannot write to production. After interruption,
an old diagram is marked stale rather than shown as the restored state.
Raw tool diagnostics must not expose secrets or sensitive row contents.
**Decision:** D04, including authentication and process-tree cleanup.

## J09 — Return to work after switching, disconnecting or restarting

**Entry:** retained workspace, reopening a tab or startup recovery.

**Existing:** consoles, retained results and object locations carry their
connection; restored object tabs wait for an explicit reading gesture.

**Proposed flow:** recover the saved connection/object identity; show unavailable
targets clearly; reconnect only through the authorized flow. Recover optional
map positions/filters separately from live metadata.

**Failure and recovery:** a deleted database or removed profile cannot fall back
silently to another destination. Closing one base releases only its work;
closing a server profile accounts for every dependent session/job.
Disconnect group first lists affected bases, unsaved work, running operations
and open/unknown transactions. Resolve transactions through their existing
review before completing the group close; Cancel closes nothing. Report partial
close failures per base. Unlink, disconnect, delete profile and database DDL
remain distinct actions; another window's resources stay under its ownership.

**Acceptance:** old workspaces remain readable; recovery issues no surprise
query/restore. The existing retained-workspace limit still bounds opened bases.

**SQLite acceptance:** [SQ05/SQ06](schema-research-engines.md#sqlite-target-cases)
cover a removed file and an expired in-memory/temporary source. Reopening a
saved tab must not create a database as a side effect. A named shared-memory
URI preserves identity only while that database still exists; recording its
name does not preserve its contents across closure or process restart.
An explicit creation starts a new source and cannot silently inherit the old
preview, diagram freshness or continuation. Retained result buffers remain
distinguishable from a live database. These recovery cases are proposed
acceptance criteria, not behavior proven by the new URI tests.
**Decisions:** D01/D03/D04 for persistence and lifecycles.

## J10 — Navigate the whole workspace by keyboard

**Entry:** F6 / Shift+F6 from the active workspace.

**Existing:** UX-SPEC names these keys; the action manifest has no binding.

**Proposed flow:** cycle visible focus areas in reading order and reverse it
with Shift+F6. Skip closed regions; preserve caret/selection; stay inside modal
dialogs. The map's own navigation remains scoped to its focused canvas.

**Acceptance:** catalog, editor, result, map and side panel are reachable when
present, including compact layout; hidden workspaces never receive focus.
Menus, palette and shortcut sheet derive the labels from the same registry.
This repairs #308 without a new database command.

## Proposed boundaries and ADR records

The ADRs below record concrete proposals; they are not implemented or accepted.
They own the design choices, while these journeys retain the acceptance work.

| ID | Proposed ADR | Responsibility and affected journeys |
|---|---|---|
| D01 | [0079 — Server grouping without shared authority](../adr/0080-group-database-connections-under-server-profiles.md) | Core/store model; driver discovery; executor sessions; host credentials; J01/J06/J09. Independent child credentials; no server-wide grant in this revision. |
| D02 | [0080 — Independent facets and explicit prefetch](../adr/0081-load-catalog-facets-independently-with-explicit-prefetch.md) | Catalog model; executor policy; driver reads; desktop consumers; J02/J03/J06. Preference changes start no bulk read. |
| D03 | [0081 — Maps with explicit metadata provenance](../adr/0082-project-schema-maps-from-provenanced-catalog-metadata.md) | Catalog projection; host orchestration; shared renderer; J04/J05/J09. View persistence is separate from metadata. |
| D04 | [0083 — Authorized native transfer jobs](../adr/0083-run-native-database-transfers-as-authorized-jobs.md) | Executor policy/audit; oxyn-transfer boundary; desktop dialogs; J07/J08/J09. Unresolved authentication modes remain unavailable. |

Existing metadata reads use `RefreshCatalogScope`; painting a cached graph
needs no execution command. Discovery and native transfers need explicit bus
contracts before implementation; their ADRs name the proposed commands, not
final method signatures.
The affected invariants are I-01/I-02/I-03/I-04/I-05/I-06/I-07/I-08/I-09/I-10/
I-11/I-13; external versions or limits also require I-12 evidence.

### Research and validation entry gates

- D01: review credential-entry friction, migration and group deletion against
  existing workspace/window ownership. No implicit shared-secret fallback.
- D02: measure real facet traffic and peak bytes; demonstrate freshness,
  cancellation and atomic legacy reads before changing consumers or budgets.
- D03: validate partial maps and each engine's provenance; establish bounded
  view serialization and rendering before adding saved layouts or exports.
- D04: prove an authentication mode and descendant cleanup on each supported
  platform with synthetic credentials. Until then the mode stays unavailable.
  Record any selected external library/API evidence in RESEARCH-NOTES before
  implementation. No plaintext-file exception is assumed.

## Evidence and validation boundary

The matrix links each driver. Cross-cutting sources inspected:

- [Object view and load triggers](../../apps/desktop/src/features/workspace/object-view.tsx),
  [facet hook](../../apps/desktop/src/features/metadata/use-relation-facets.ts),
  [IPC schema](../../apps/desktop/src/lib/ipc/metadata.ts).
- [Catalog reads](../../crates/oxyn-exec/src/catalog.rs),
  [completion pass](../../crates/oxyn-desktop/src/backend/completion_load.rs),
  [assistant fill](../../crates/oxyn-desktop/src/backend/ai/catalog_fill.rs).
- [ERD assembly](../../apps/desktop/src/features/assistant/erd.ts),
  [renderer](../../apps/desktop/src/components/oxyn/erd-diagram.tsx),
  [model](../../apps/desktop/src/components/oxyn/erd-model.ts).
- [Workspace ownership](../../apps/desktop/src/features/session.ts),
  [object location](../../apps/desktop/src/features/workspace/object-location.ts),
  [window persistence](../../crates/oxyn-core/src/window_layout.rs),
  [action manifest](../../apps/desktop/src/lib/actions/actions.json).

The acceptance cases above are a future validation inventory, not passing
tests. No live database, sample, benchmark or native transfer was exercised.
The research deliverable is reviewable when every journey has a trigger,
failure/recovery path, acceptance case and a capability/decision reference.

## J04/J05 implementation revalidation — 2026-11-14

The map branch follows metadata commit `40329e4`, with no main/dev integration
yet. Opening a map starts no native read. A selected native container is loaded
one level at a time; a chosen object seeds only that object. Local filtering
covers loaded names, paths and kinds. Inspect first; Data/Structure/definition
and one-hop expansion are separate gestures on the original owner. Opening a
view already present in the workspace changes its displayed section.

The host rejects unknown object requests and source-generation changes. A
memory-only generation check also rejects a map superseded during IPC delivery.
Failed/partial/stopped reads keep their distinctions, and a failed refresh
retains the old map as stale. Closing/hiding cancels the exact owned demand.
Document inference requires a separate source/object/bounds disclosure; ordinary
loads never sample. Sampling settings and availability follow the live provider.

Composite keys, cycles, homonyms, nesting, reported/sample/preview provenance,
unknown cardinality and unloaded endpoints are shared projection contracts.
Only loaded outgoing keys establish displayed incoming coverage. Technical
dependencies are explicitly unavailable until structured provider metadata
exists. No saved map layout or automatic source reopening is introduced.
SQLite/DuckDB native smoke results and other-engine validation limits are
recorded in the engine matrix and completion ledger. The existing SQ01–SQ06
source-identity cases remain applicable; private-memory and deleted owners must
never turn a map load into creation of an empty replacement database.

## J07/J08/J09 implementation revalidation

The retained explorer transfer tab now implements the common preparation,
progress, history and reconciliation journeys. Initial execution is restricted
to the PostgreSQL mode documented in ADR-0083 and the engine matrix. Host pickers
issue opaque capabilities; preparing runs only local tool/archive checks. Every
restore uses exact backend destination text and host confirmation. Closing asks
before cancelling and waits for owned cleanup; switching does not cancel. A
restarted job is historical/interrupted, never resumed. Existing files are always
refused rather than overwritten, and SQL-script restore is not offered. SQLite
and every other unsupported adapter retain an explanatory unavailable state,
without root introspection or an invented backup fallback. Validation evidence
and remaining platform/manual limits live in the completion ledger.


## Visible-demand refinement — 2026-11-15

[ADR-0084](../adr/0084-load-visible-schema-scopes-on-demand.md) changes J03/J04/J05
interaction: opening the visible map or choosing a scope starts its bounded
demand without a separate Load gesture. Native levels remain explicit when
ambiguous; only a sole listed child may advance automatically. This applies to
all existing engines without eager traversal, inferred links, implicit samples
or reopening SQLite/DuckDB sources. Refresh, cancel and partial coverage remain.
Bulk prefetch is optional under Advanced in metadata preferences, never a
prerequisite for structure, definitions, completion or maps.
