# Database tooling completion ledger

Updated 2026-11-15. This appendix tracks the user's completion request for #308,
#373, #374, #375 and the standalone schema map. It separates implementation from
validation and personal acceptance.

## Git and dependencies

The single active worktree is
`/Users/nicolasboromee/.codex/worktrees/oxyn-schema-research/oxyn`.
`dev` was fast-forwarded to `ce58839` from `origin/main`. The feature worktree was
then fast-forwarded to that same base and the existing work reapplied. The
explicit recovery snapshot is stash commit `7c23ae3e3d83b51f331a2cd9e84545439194654f`;
it predates the new no-stash branch-transport instruction and is kept only as a
backup. No additional stash will carry a lot between branches.

Main's accepted temporary-preview ADR occupies 0079. The four proposed research
ADRs were renumbered 0080–0083, with their links and French mirrors. Historical
image-generation prompts retain the identifiers used at generation time.

| Lot | Branch | Dependency / current status |
|---|---|---|
| Shared research | initial documentation commit on #373 | Journeys, engine matrix, proposed ADRs and mockups are a shared prerequisite. A documentation-only commit across the issues is necessary to retain this common design without duplicating it. |
| #373 | `feat/multi-base-backend` | Implemented and automatically validated on `ce58839`; `make qualite` passed. Code commit `a3f5961` follows shared research `356c8d6`; personal acceptance remains pending. |
| #308 | `feat/f6-navigation` | Created from `a3f5961` after a clean #373 commit. Registry/visible-region cycle implemented; Completed: 93 action tests, five browser interaction stories, full quality gate and final frontend revalidation passed. Personal/native OS acceptance pending. |
| #375 | `feat/metadata-loading` | Created from #308 commit `4c80bfa` after a clean validated commit. Completed and automatically validated: shared capability-driven policies/facets/preload, explicit inference and shadcn controls; make qualite passed 4,733 Rust / 2,391 frontend tests. Personal acceptance awaits final dev integration. |
| Schema map | `feat/schema-map` | Created clean from #375 commit `40329e4`. Standalone tab, shared host projection, original-owner navigation and explicit neighbor/sampling controls implemented. Completed and automatically validated: make qualite passed 4,753 Rust / 2,415 frontend tests, documentation and four package archives. Personal acceptance awaits final dev integration. |
| #374 | `feat/native-backup-restore` | Not created; inherits the completed stack, reuses ownership/operation state and metadata invalidation. No native-transfer implementation yet. |

This intentional linear stack avoids duplicating common commits. Each branch
must have coherent commits, a validation ledger and a clean working tree before
the next switch. Branches are inspected before creation. Main is never a write
or integration target; no push is authorized.

## Initial uncommitted inventory

Before the first lot switch, the retained work comprises:

- Shared design: docs/implementation-plan research appendices, proposed ADRs,
  engine matrix, mockup PNGs and provenance text, English/French indexes.
- #373 core/store: server profiles and membership, migration 20, child creation,
  credential/destination guards and tests; no new schema from the main refresh.
- #373 driver/exec: optional discovery boundary, PostgreSQL implementation,
  session opening-destination capture, command policy/audit and tests. The main
  refresh's console/catalog purposes and session occupancy are preserved.
- #373 desktop/IPC: typed discovery/opening/profile APIs, native production
  confirmation, cancellation cleanup and readiness checks begun.
- #373 frontend: MultiBaseExplorer, DatabaseConnectionDialog,
  ServerProfileDialog, window-owned coordinator/data hooks, session/first-console
  policy and workspace/sidebar wiring. These are started until all acceptance
  cases and prescribed controls pass.
- Baseline reused rather than reimplemented: metadata read coordination/facets,
  assistant ERD, retained-workspace lifecycle, action registry and dialogs.

This was the uncommitted inventory at resumption; the common research is now
committed and the #373 code is stabilized for its implementation commit. Earlier
2026-11-13 gates
validate the older baseline, not the refreshed code or new explorer work.

## Current #373 validation

Shared research was committed as `356c8d6` after `make socle` passed with zero
warnings. It is deliberately documentation-only across the planned issues.

The explorer now separates selection, expansion and activation; repeated
openings coalesce, late responses preserve the latest explicit foreground and
cancelled openings discard their exact owned pair. Saved-but-unopened children
keep their identity, accounts stay separate, and their first console is empty.
Window-owned discovery rejects a seed that only another window holds.

Group creation, rename, membership move/unlink and removal retain ordinary child
identities. Group disconnect snapshots counts and reported transaction state,
then serializes the existing child draft reviews. Any failed or cancelled child
stops the batch and reports completed closes; backend cleanup failure stays
visible with an explicit retry. An active close reveals another retained child.

Text, object and retained-result requests carry their workspace generation.
Private-session name drops cannot enter a sibling editor; cross-source drops
require a source-console choice and insert no SQL automatically. The explorer
lists saved/discovered destinations; object browsing remains under the active
workspace, never deep-introspecting every listed base.

Verified on 2026-11-14:

- 18 desktop backend closing/ownership tests passed, including exact opening
  cleanup, foreign-window discovery refusal and idempotence.
- Focused frontend integration, activation, queue, saved-ID recovery, distinct
  accounts, pre-dispatch cancellation and draft preservation tests passed.
- New component stories passed with axe; Cancel receives focus in the two new
  source/group reviews. The full frontend run passed 2,349 tests at that point.
- Disposable PostgreSQL 17 discovery and independent child opening passed;
  server-side cancellation and bounded large-volume streaming also passed.
  Its loopback-only container was removed afterwards.
- `make qualite` passed: Clippy with `-D warnings`, 4,707 Rust tests (209
  skipped), documentation tests, rustdoc and packaged-contract verification.
  `make socle` passed with zero warnings. Later UI refinements and the three
  child-recovery tests also passed TypeScript, ESLint and targeted verification.
- Browser execution emitted ResizeObserver and React act diagnostics despite
  passing assertions; no diagnostic was suppressed. Targeted new-component runs
  and the independent three-screen visual check emitted no page errors.
- `cargo-deny` was absent, so Rust license/RUSTSEC audits were not checked.
  `cargo-about` was absent, so notices were not regenerated. Other server URLs
  and the embedding reference model were unset; those suites were not run.

Local invariant, boundary, security, organization and UI reviews found no
remaining blocker in this lot. Corrections from review include scoped text/
object/result generations, discovery window ownership and explicit cleanup
errors. Existing large workspace/session orchestration stays with its owner;
profile persistence, navigation coordination and dialogs live in their modules.
No new dependency, global configuration change or push was introduced.

These are automatic checks, not personal acceptance. macOS is the only local
platform exercised; native transfers and the later lots are not implemented by
this record.

The next full gate found a formatting-only residue in the late child-recovery
regression test. Commit `a75a5f2` corrects it after a dedicated Prettier check;
the inactive #373 branch was fast-forwarded from `a3f5961` to this descendant.
No history was rewritten and no issue code was transported between branches.
The full F6 gate revalidates the combined tree.

## Current #308 validation

F6/Shift+F6 are registry actions, placed in View and derived into the palette
and shortcut sheet. Visible workspace regions reuse shadcn sidebar markers and
explicit editor/result/side-panel markers. Optional maps use the same generic
registration, with no map import or delivery dependency. No second keyboard
listener, panel opening, activation or data read was added.

93 action/registry/dispatch tests and five Chromium stories passed. The stories
exercise a real CodeMirror selection and result grid in wide, compact and closed
panel layouts, plus a modal. The cycle returns the exact selection and text.
A stale hidden-workspace context is also rejected in the region tests.
TypeScript and ESLint passed. `make qualite` passed 4,707 Rust and 2,368
frontend tests, including the shared explorer formatting correction. A final
review added the selected-tab guard: a remembered tab that became inactive is
not a valid focus target and must not be reactivated by F6. Its regression and
the 93 action tests passed. Final `make front` passed 2,370 tests, formatting,
lint, types and build after that UI-only refinement. The fifth browser story
also invokes the real palette and verifies it returns focus after dismissal.
`make socle` passed with zero warnings after the ledger update. Native OS key routing still
awaits the final dev instance and personal acceptance. No Rust source or IPC
boundary was changed by the final focus refinement.

## #375 implementation plan

The pure policy/scope vocabulary belongs to core; store owns migration 21,
bounded JSON and revision comparison. Executor owns policy/audit, independent
facet plans, source generations, cancellation, demand priority and bounded
preload accounting. Desktop exposes window/session-checked typed IPC and
progress; frontend features connect it to shadcn settings and preload controls.
The existing catalog cache, metadata read coordinator and facet UI are reused.

Order: policy persistence/commands; independent descriptions/indexes/outgoing
keys and explicit sampling admission; scoped preload and progress; UI and all
consumers; validation and documentation. Do not change engine protocols or add an external package. Reuse the workspace
Serde/Serde JSON pins for bounded publication accounting. Existing `INFERRED_SCHEMA` already identifies sampled descriptions.

Key failures to prove: stale revisions and late publications; unread versus
empty/stale/unavailable/error; exact namespace inheritance; policy save causing
zero driver calls; cancelled/superseded jobs; budget eviction; explicit sample
admission with no values; DDL/completion/assistant requesting only justified
facets. New reads keep native session ownership and fail on a closed source.
Measure provider calls, metadata bytes and elapsed time on disposable fixtures;
no automatic threshold or blanket SQLite exemption is adopted.

Completion requires frontend/driver/core/store/executor tests, accessible
initial/empty/loaded/loading/error stories, local reviews, `cargo fmt --all`,
`make qualite`, coherent English/French contracts and a clean committed branch.
Manual acceptance stays reserved for the final integrated dev instance.

## Current #375 implementation and validation

Migration 21 stores revisioned per-connection policies independently of auth.
Scoped preload runs on the exact live owner with demand priority, explicit
128-relation detail pages, a 60-second cancellation deadline and bounded reports.
Continuation checks native scope, policy revision and cache generation. Context
changes cancel only their console’s preload and retire its cache epoch before
and after the native operation. No source/session/file is opened implicitly.

The shadcn dialog separates save from preload/next/cancel. Unsaved edits must be
saved first; stale saves require re-reading. Source hiding/removal releases only
its owned job. Object tabs/copies/completion/assistant/ERD demand narrow facets;
wide explicit DDL reads the definition, while Structure does not request it.
Inferred descriptions require a separate human gesture with declared bounds.
Preview preparation reuses fresh metadata from its original owner; missing or
stale documentary structure never triggers a hidden sample. Already inferred
cached fields remain available without being sampled again.

Terminal coverage distinguishes fresh/stale/unread/evicted from unavailable and
failed reads. Unknown counts are nullable until measured. Eviction can make a
fully attempted page partial without starting a refill loop. Publication checks
the captured generation atomically, including budget eviction. Sampling bounds
and typed generation belong to the catalog contract; core DTOs carry an opaque
UUID, so packaging does not depend on unpublished core symbols. Existing UUID,
Serde and Serde JSON packages are reused without a version/graph expansion.

Verified on 2026-11-14:

- Final `make qualite` passed: format, Clippy with `-D warnings`, 4,733 Rust
  tests (211 skipped), doctests/rustdoc, four independently verified package
  archives, and 2,391 frontend tests plus build. Foundation reported zero warnings.
- The host/provider scenarios cover capability combinations, exact inheritance,
  CAS/migrations, zero-I/O saves, independent facets, context/policy cancellation,
  late generations, fixed page continuation, eviction and cached explicit inference.
- Fifteen settings/sampling stories passed in Chromium with axe. Wide DDL and
  source replacement/late replies have feature regressions. Visual inspection
  of final partial/evicted states emitted no page errors.
- A disposable SQLite private-memory fixture exercised all three levels and
  cache reuse; measured scope/byte/time results are below. Disposable MongoDB
  8.0.34 passed 24 tests, including server-observed cancellation, a 1 GiB bounded
  batch stream, oversized-sample rejection/remote cursor cleanup and profiler
  proof that preview planning issued no hidden `find`. Its container was removed.
- MongoDB process peak RSS is not asserted on macOS (the suite’s RSS check is
  Linux-only). Other live server suites and the reference embedding model were
  not run by this gate because their URLs/model were unset.
- `cargo-deny` was absent: Rust license/RUSTSEC audits were not checked.
  `cargo-about` was absent: notices were not regenerated. npm audit/licenses
  passed. Full browser output still contains existing ResizeObserver/React act
  diagnostics; they were not suppressed and assertions passed. Only macOS was
  exercised locally. Manual acceptance has not been obtained.

Local invariant, boundary, security, organization and UI reviews found no
remaining blocker after the corrections above. The existing large bus/cache/
object/workspace dispatchers retain their established responsibility; policy,
scheduling, sampling, cache accounting and pure UI/controller responsibilities
are separated in domain modules. No second driver cache, automatic count
threshold, SQLite exemption, new protocol or global configuration was added.
The ERD changes in this lot are required facet/ownership consumers, not a second
standalone-map implementation. No merge to dev/main or push occurred.

### Disposable SQLite measurement (2026-11-14)

One private-memory database, three tables, one outgoing FK and one secondary
index; each mode starts with names/fields unread in that namespace:

| Preset | Submitted scopes | Encoded metadata bytes | Elapsed ms | Fresh detailed facets |
|---|---:|---:|---:|---:|
| Minimal | 1 | 554 | 0 | 0 |
| Structure | 10 | 5,164 | 1 | 9 |
| Extended | 13 | 7,082 | 0 | 12 |

Repeating each page submitted zero reads and zero encoded bytes. The pure
provider-probe tests separately assert exact description/index/key/definition
method calls. Scope counts can differ from provider method/network calls.
Milliseconds are floored, so zero is below report resolution, not zero work.
This is a local smoke measurement, not a network benchmark or evidence for
an automatic threshold/SQLite exemption. The current checked implementation
must still pass the complete gate before the branch changes.

## Current standalone map implementation and validation

The human-only `LoadSchemaMap` bus demand reuses exact live session metadata,
interactive queue priority and atomic cache generations. The catalog projection
is pure and shared with assistant diagrams; name resolution remains the
assistant's only graph adapter responsibility. A memory-only generation getter
also rejects projections superseded while their IPC response was delivered.
No new database session/file, automatic AI request or implicit sample is opened.

The explorer root, native container/object menus and object-view toolbar open
owner-qualified retained tabs. A selected object and a whole native scope are
separate load choices. Scope selection, filters, inspection, pan/zoom and the
accessible object/link list remain local. Neighbor expansion is explicit and
one hop. Data/Structure/definition navigation retains the original source and
changes an already-open object's section. Closing/hiding releases only the map's
command; a console cannot move while that owned demand uses it. New/taller cards
are placed without covering retained neighbours. Compact layout scrolls diagram
and inspector instead of overlapping them; a browser story checks geometry.

Native hierarchy absence, homonyms, composite/cyclic/unnamed or malformed keys,
unloaded endpoints and reported/sample/preview-layout provenance are preserved.
Unknown cardinality stays unknown; stale descriptions cannot prove uniqueness.
Coverage names omissions, stale/unread descriptions, failed/unavailable facets
and stopped reads. Technical dependencies stay unavailable without provider
metadata. SQLite/DuckDB attachments are never opened automatically, and a map
cannot replace a deleted file or a closed ephemeral owner with an empty source.
Durable layouts, image export and graphical DDL editing remain separate scope.

Targeted evidence on 2026-11-14:

- Eight pure projection tests and nine executor/publication tests passed,
  covering identities, provenance, bounds, capability combinations, exact
  facet calls/cache reuse, unknown-object refusal, human/owner checks,
  cancellation cleanup and late generation rejection. Later review additionally
  refuses cardinality proof from malformed keys; the final gate rechecks it.
- Real disposable SQLite and DuckDB fixtures passed view/composite-key/native-
  scope maps: three objects, four links, seven scope reads, then zero on repeat.
  Two separate private-memory sources retained distinct object keys. Deleted-
  file/closed-owner coverage was added for the final gate.
- Controller and IPC tests reject late/replaced owners and post-delivery epochs,
  retain previous maps after failed refresh, coalesce clicks and avoid mount
  reads/sampling. Pure placement regressions cover overlapping added/taller
  nodes and owner replacement. Fifty-two focused map/tree stories passed with
  axe, including compact geometry and menu dispatch; shared ERD/assistant
  stories also passed earlier in this lot.
- Workspace-wide Clippy with `-D warnings` passed after moving a misplaced
  core re-export. Initial full gates caught frontend lint and a tree-menu
  regression; all were corrected without suppressions. Final full-gate counts
  are recorded below after it completes.
- Final `make qualite` passed on 2026-11-15: 4,753 Rust tests (211 skipped),
  2,415 frontend tests, format/lint/types/build, doctests/rustdoc and four
  independently verified package archives. The deleted-file/closed-owner test
  and malformed-key regression passed in this final tree. Foundation had zero
  warnings. npm audit and 415 production package licenses passed.
- `cargo-deny` remains absent (Rust license/RUSTSEC audits not checked), and
  `cargo-about` remains absent (notices not regenerated). Live server/model
  suites were unset; only the disposable SQLite/DuckDB native map fixtures ran
  in this lot. Existing browser ResizeObserver/React act diagnostics remain
  unsuppressed; assertions passed. Only macOS was exercised locally.
- Local invariant, boundary, security, organization and UI reviews found no
  remaining blocker after the corrections above. The map holds a console-use
  lease during its demand, and its cache-only getter performs no native read.
  Existing large workspace/object/cache/bus entry points retain their established
  composition responsibility; projection, placement, loading, ownership state,
  IPC validation and pure presentation are separate modules.
- Personal acceptance is not obtained. The feature branch has not been merged
  into dev/main or pushed.

### Exploratory schema-map rendering (2026-11-14)

Chromium via the installed Playwright, Storybook on loopback, 1,440 × 1,050
viewport; synthetic 40-object/12-field/200-FK graph. Both 40 DOM nodes and
200 edges rendered, with no page errors. Five direct Dagre calls took
181.2 / 141.4 / 133.2 / 121.3 / 117.2 ms. A filter previously reran this placement
for each input; placement is now keyed only by the graph, independently from
filter/selection handlers. Stable positions and collision handling are pure
and separately tested.

Input-to-two-animation-frame observations for four local filters were
309.6 / 247.3 / 200.6 / 447.1 ms before and
511.6 / 113.7 / 73.1 / 226.7 ms after separating placement. The cold first value
regressed; these runs overlapped compiler/browser activity and establish no
reliable speedup, p99 or budget compliance. They are exploratory diagnostics,
not an accepted idle-machine/system-instrument benchmark. The initial layout
and continuous interaction budgets remain unverified on the Tauri webview;
no budget or object/link limit was raised. Reference timing needs a separate
controlled run. A local build must not be reported as user acceptance.

## Validation and acceptance gates

Each lot records actual format/lint/type/test/story/build and `make qualite`
results, server fixtures used, failures, skipped suites, absent tools and
unverified platforms. No assertion or timeout is weakened to produce green.
Local reviews use the repository invariant, boundary, security and UI checklists.

After all lots are complete, inspect dev and the branch ancestry, integrate
without losing new dev work or replaying shared commits, and run `make qualite`
on the integrated tree. Only then start a new `make desktop-dev` instance on a
temporary workspace from a clean dev checkout, prepare disposable data and
verify the process/checkout correspond to the integrated commits. Do not reuse
an old instance or offer a feature/main instance for the user's personal tests.

Personal acceptance is **not obtained**. The final manual checklist will cover
F6/Shift+F6, discovery/opening, repeated activations, A→B→A, drafts, errors/cancel,
metadata levels, standalone map and disposable PostgreSQL export/restore.
Wait for the user's report before claiming manual approval.
