# Database tooling completion ledger

Updated 2026-11-14. This appendix tracks the user's completion request for #308,
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
| #375 | `feat/metadata-loading` | Not created; will inherit completed #308/#373. Existing lazy reads/facets are baseline implementation, not completion of configurable policy. |
| Schema map | `feat/schema-map` | Not created; depends on metadata coverage/policy from #375 and focus registration from #308. Existing assistant ERD is reusable rendering, not a standalone map. |
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
