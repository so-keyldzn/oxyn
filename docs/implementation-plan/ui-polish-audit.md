# Interface audit and demand-loading polish

Date: 2026-11-15. Scope: the existing desktop interface in the designated dev
worktree. User direction: remove duplicate UI, improve the sidebar, use shadcn
and Hugeicons, and display selected schemas without a preparation workflow.

## Design direction

Retain the professional compact workspace, existing typography and semantic
palette. Separate connection navigation, the selected source's objects, and
source tools. Use one recognizable action/icon per purpose, progressive disclosure
for maintenance and technical detail, and readable loading/error/partial states.
No decorative theme reset or automatic database-wide preload is introduced.

## Findings and remediation

| Surface | Finding/evidence | Remediation |
|---|---|---|
| Schema map | Scope selection followed by a redundant Load gesture | Visible scope demand starts automatically; refresh/cancel remain |
| Map coverage | Zero-count badges and long technical footers compete with the diagram | Keep meaningful coverage/omissions visible; disclose detailed limitations |
| Source sidebar | Generic full-width buttons compete with sidebar navigation | shadcn sidebar tools, accessible collapsed rail and semantic Hugeicons |
| Workspace tabs | Map, transfer and result used one dashboard icon | Distinct relationship/database-transfer/table icons |
| Server groups | Expanded management controls compete with base selection | Consolidate maintenance in a menu; preserve select/expand/activate separation |
| Metadata settings | Policy saving and bulk preload appeared equally necessary | Demand-loading explanation; optional preload under collapsed Advanced |
| Native transfers | Implementation terms dominated the main path | Outcome-first copy, readable progress, concise review/refresh actions |
| Structure/DDL/indexes | Idle fallback includes Load, but active consumers already call ensure | Preserve demand reads; do not fetch hidden facets or sample documents |
| Tables, connections, settings, assistant | Existing shadcn/theme composition; source inventory alone cannot certify layout | Wide/compact synthetic-story visual review and interaction checks |

## Behavioural boundaries

[ADR-0084](../adr/0084-load-visible-schema-scopes-on-demand.md) refines the old
separate-load gesture. Native levels with several choices still require a scope
selection: the current backend does not aggregate every namespace in a database.
A sole fully listed child may advance; a partly cached child list cannot be used
to guess a default. This is not a complete cross-namespace database-map claim.

Demand reads retain source/session/generation identity, cancellation and existing
object/field/link/time/byte bounds. All engines follow their actual capabilities.
SQLite/DuckDB owners are not replaced or reopened, and documentary samples remain
explicit and confirmed. Optional bulk prefetch remains implemented and cancellable,
but never runs because a setting is saved or an Advanced disclosure is expanded.

## Validation record

Agents divide map behaviour, metadata/sidebar composition, and broad visual review.
Checks and remaining visual limits are reported after integration of their edits.
A passing synthetic story is not acceptance of every native-window journey, nor
a frame-budget measurement. The running temporary dev application uses hot reload;
main and remote branches remain outside this task's changes. No commit is made
by the audit agents.


### Broader visual evidence

A settled synthetic-story probe produced 33 screenshots across 11 routes at
1440/1100/600 pixels with no page errors. 111 interaction/accessibility stories
covered connection management/dialogs, workspace layout, object/table frame,
appearance settings and assistant. Fixed-width workspace fixtures were not treated
as responsive-product failures below their declared size; native minimum width
is 960 pixels. Supplemental workspace checks use 960/1100/1440.

One confirmed defect was corrected in the new-account dialog: an indefinite
flex/percentage height let its inner viewport grow behind the action footer. The
account-form variant now supplies a definite bounded height and contained scroll;
other dialog variants retain intrinsic sizing. Geometry plus existing form/dialog
stories passed 36 cases. This is browser-fixture evidence, not native-window
acceptance. The screenshot provided by the user separately drives the dense,
volume-aware sidebar redesign and removal of permanent Browse session.


## Screenshot-driven sidebar and session browsing

The supplied narrow sidebar capture confirms repeated destination/status lines,
a selected-row button wall, a permanent session selector, duplicated connection
navigation and a top-window padding applied to a mid-sidebar source header.
The redesign must preserve a bounded connection list and leave useful space for
the object tree. Connection filtering is local, contextual maintenance does not
activate a source, and homonymous rows retain their distinct identities. Volume
stories cover long names, many connections, collapsed navigation and keyboard use.

The permanent session selector is replaced by a console context action. Normal
connection browsing has no empty selector placeholder. An active-session indicator
names its console and offers Return. Only explorer reads change; the active
connection, editor drafts, sessions and transactions are untouched. Returning
keeps still-live owner-qualified object/preview/map tabs. Console disappearance
or handoff explicitly reports return and retires only unavailable owner views;
a stale menu callback cannot select a session that has already moved or closed.


### Large connection lists

The connections region is height-bounded independently from the object tree.
Loaded-connection filtering never opens a connection or introspects its databases.
At 30/100 saved connections and 500 objects, interaction/geometry tests require
the connection region below 34% of the fixture and object content above 50%,
with rows no taller than 36 pixels. Same-name destinations are disambiguated
visually and for accessible row/action descriptions.

A keyboard-visible Skip to database objects action provides a constant-step path
to the current source's object filter instead of traversing two controls per
connection. It changes focus only. The 1,000-connection case is measured separately;
DOM size and dev-browser timings must not be reported as a native p99 budget or
unqualified smoothness guarantee. Virtualization requires evidence and must keep
selection, menus and keyboard routes intact.


### 1,000-connection baseline

A reproducible browser-only diagnostic on the shared synthetic sidebar fixture
found 13,338 sidebar elements for 1,000 mounted connection rows. The list had
183 pixels of viewport and 35,996 pixels of content; the catalog retained
456 pixels with the real tools footer. No overflow or page errors were observed.
Three fresh browser contexts against the warm development server recorded
1.309–1.756 second mount long tasks. Six local filter-to-one samples committed
in 122.6–896 ms (median 388.6); six clear-to-1,000 samples in 655–2,108 ms
(median 1,593.6). Initial navigation also included Storybook/import startup.
These are development diagnostics under a shared machine workload, not native
p99 or idle-machine certification. They justify investigating bounded virtual
rendering; the candidate must be compared with the same interaction/geometry
checks before claiming an improvement.


### Logos and measured virtual candidate

Brand identity uses the existing DriverLogo for explicit driver IDs; Hugeicons
remain action/navigation pictograms. The nine registered protocol IDs have marks.
The frontend does not receive a connected server-product field, so protocol
aliases cannot establish MariaDB/Valkey/Redshift/OpenSearch product identity.
Names and host strings must never be used to guess that identity. The selected
connection form title and explorer rows reuse known identity instead of a generic
database glyph. Existing curated logo assets and brand restrictions are preserved.

An identical browser interaction probe of the virtual saved-list candidate reduced
sidebar elements from 13,338 to 482 and mounted connection rows from 1,000 to 11.
Six-sample median filter commit fell from 237.8 to 7.7 ms; clear-to-list from
784.6 to 15.7 ms. Two-frame medians were 273.9 to 27.9 ms and 844.5 to 41.0 ms.
No overflow/page errors were observed. Navigation samples remained highly noisy
and include application bootstrap; these numbers certify neither native p99 nor
all distributions of expanded groups. The virtual presentation preserves stable
IDs and pins focused/menu rows; grouped distributions and keyboard/menu behaviour
are checked separately before final validation.


### Distributed groups

A 100-group × 10-connection case exposed the limitation of independent inner
virtual windows: offscreen small groups still mounted their members. Viewport
suspension now observes the outer connections scroller, retains fixed placeholders
and keeps focused/menu-owned lists enabled. It does not collapse groups or forget
selection. The identical browser diagnostic reduced mounted rows 1,000→10 and
sidebar elements 15,237→2,367. Median filtering changed 176→19.4 ms, and clearing
2,279.2→153.35 ms. Group headers remain mounted; this is not constant DOM for
an unlimited number of groups. No overflow/page errors were measured. The same
development/concurrency and non-native-budget caveats apply.


The timing comparisons preceded the final curated-logo substitution. Final
branded screenshots independently confirmed bounded mounted rows, no horizontal
overflow/page errors, shared source tools and the collapsed rail. The console
source indicator was checked in its own real-component fixture rather than
fabricated into the volume layout. Full native-window acceptance remains separate.


### Final invariant review

An independent read-only review found no confirmed blocking violation of I-01
through I-13. It checked live-entry menu resolution, connection/session owner
partitioning, stale quick-open and map replies, retirement of vanished owners,
keyboard/menu pinning and explicit-driver branding. No new direct driver/IPC,
SQL execution, transaction closure or secret logging path was introduced.
This review does not establish real-database integration or native-window
acceptance; the complete quality-gate result is recorded separately.


### Completed quality gate

On 2026-11-15, `CARGO_BUILD_JOBS=2 VITEST_MAX_WORKERS=1 make qualite
SHARD='1/1 --no-file-parallelism --maxWorkers=1'` passed. The shard is the entire
suite; no test, assertion or timeout was removed or relaxed. Results: 276 frontend
files / 2,470 tests passed; 4,779 Rust tests passed, 213 skipped; doctests, strict
Clippy, rustdoc, formatting, TypeScript, ESLint, production builds and package
contracts passed. Independent focused checks also covered session ownership and
retirement, stale responses, preserved drafts, keyboard/context menus and volume.
Earlier concurrent runs timed out in existing assistant/object-view tests; the
complete sequential run passed unchanged.

Validation limits remain explicit: server suites and embedding reference tests
did not run without their configured resources. Missing cargo-deny leaves Rust
advisory/license auditing unexecuted; missing cargo-about leaves third-party
notices ungenerated. The npm production audit reported no known vulnerabilities.
The build retained large-chunk warnings. Browser logs retained ResizeObserver
and React act warnings also observed in earlier gates; no persistent resize cycle
or new blocking violation was established by scoped tests or review. This is not
native frame-budget certification, real-server integration or full native-window
acceptance. Work remains uncommitted on dev; main and remote branches are untouched.
