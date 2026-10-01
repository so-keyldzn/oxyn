# Implementation plan

> **Authority**: the order of the phases and the exit gate of each one.
> It is the only document that talks about what **remains to be done** — the
> authoritative documents describe what is decided.

State as of 2026-09-18: the interface is the Tauri application (`apps/desktop` and
`crates/oxyn-desktop`); the GPUI interface was removed that day, see
[Migration to the Tauri interface](#migration-to-the-tauri-interface). The dated
states that follow describe the product at their date — often the GPUI interface:
what they say is done is not necessarily done in `apps/desktop`, and the gate of
that section keeps the list of what is missing there.

State as of 2026-09-10: the fifteen crates exist, with a GPUI application,
a connection form and an SQL execution flow. The UI/UX integration
follows the Figma mockup: collapsible sidebar, light/dark themes, embedded
Hugeicons and Geist, real catalog loaded in tiers through the command bus.
The fixes for mouse interaction, native input, session and cancellation are
in place.
A first measurement campaign took place on 2026-09-10 and covers **pure
code**: row-to-batch conversion of the SQLite driver, latency of the first batch,
query analysis ([PERFORMANCE](PERFORMANCE.md#measurement-campaign-of-2026-09-10)).
It contradicted no budget and amended none. Still to measure, and none of them
is measured with `criterion`: the frame, cold start, expanding a cached catalog
node, and RSS — including the `ResultBuffer` budget, of which only the spill
triggers are covered, by tests with tiny budgets. The existence of the code does
not by itself validate the exit gates below.

The first workspace offers the SQL editor, metadata exploration and
the automatic preview of the first 200 rows of an SQL table. Visible history, saved queries and recovery of SQL drafts are
wired as detailed below. The preview now accepts a predicate
written by the user, a sort by columns and a next page when the order
is total ([ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md)).
Asynchronous reading of spilled result pages now goes through
`ReadResultPage`, with cancellation, explicit error and rejection of stale
responses. The page cache shares the result's budget according to ADR-0012.
The byte bound of the metadata cache, the global budget of results held by views
on replacement/close, and the RSS measurements remain to be completed.

The table/console frame now follows the dense bars and tabs,
the environment pill and the 1200 px compact threshold. The wide
sidebar preference is kept, and the compact catalog remains accessible in
an overlay panel. The confirmation focuses `Cancel`, keeps focus in
the review and refuses Enter alone. The help for disabled editing and the real
export of the preview are wired; the latter distinguishes a complete preview from a
truncated reception and keeps its cancellation during a result change.

Full alignment with [ADR-0011](adr/0011-structure-commune-workspace.md)
is still in progress: advanced PostgreSQL DDL cases and the full compact menu remain to
be integrated. Restoring the object location has held since
2026-09-25 (see "Recovery and library"). The Indexes and Relations tabs now read the indexes and outgoing foreign
keys actually present in the cache, with loading through the bus, a distinct empty
state and an explanation of missing capabilities. They move into `More` at
compact width. A metadata request made during another load
is queued; cancelling also removes that request.
The Relations tab now opens incoming PostgreSQL/SQLite references,
with a switch to outgoing keys. The From / To / Cardinality columns
follow Figma `229:33051` (340/300/200, 32 px rows). The uniqueness status
stays unknown for complex comparisons that are not established. Enter or Open
source table views the source; if it was not yet in the catalog,
the bus describes it then loads its preview, without changing or running the draft.
The reverse path also opens a target from the outgoing keys.
The Constraints tab reads PostgreSQL and SQLite constraints on demand through a specific
bus scope. It distinguishes unread, empty, cancellable loading and error;
keyboard and mouse selection lets the user view and copy the
engine's definition without opening or running SQL. The control joins `More`
at compact width. SQLite keeps the text of the clauses stored in the
schema, without inventing names or losing `ON CONFLICT`, references or
CHECKs. The Status column follows Figma `229:7998`: actual PostgreSQL validation
state, or `Not reported` when it is not provided. The columns use
the proportions 280/220/200/140 and Geist 13 px; the Constraints tab has a
minimum width of 113 px. The native visual acceptance test remains to be done.
The row inspector is wired to the console and preview grids, with
keyboard field selection and full paginated reading through `InspectResultValue`.
The Columns menu, at the foot of the console and preview grids, hides and shows
columns locally by Arrow index, announces the number of visible
columns and offers `Show all`; the grid only virtualizes visible
columns, copy ignores hidden ones, and export keeps all columns, which
the menu states. In compact mode, the inspector opens as an overlay
and these actions move into Actions. The initial width of the inspector is
Figma's (`190:1872`, 280 px). The 8 px handle resizes the panel
with mouse and keyboard, saving at the end of the gesture. Collapsing the sidebar
now keeps the active table instead of returning to the console.
Privacy markers reflect the actual provider: the label of
the top bar reads `Metadata · Cloud` or `Metadata · Local` according to the
measured location of the destination that would respond, and has no suffix
without a known provider. No fictitious Cloud tier is displayed.

The mockup corrections listed in [FIGMA-HANDOFF](FIGMA-HANDOFF.md)
also specify the checkbox recovery selection, now wired to persisted SQL drafts.
The two reading presets are wired to Settings and to the results
bar, with versioned persistence of the theme, panels, NULL indicator
and digit grouping according to ADR-0013. Save errors are
visible and old writes do not replace new ones.
Closing the last window waits for committed writes; the other
native shutdown paths and the GPUI hook delay remained to be tested — that hook
disappeared with the GPUI interface on 2026-09-18.
Absent cells now use `∅ NULL`; grid cells
use Geist 13 px from node `190:1652`, or 14 px in comfortable reading.
Headers use the single line of node `190:1639`. The Figma prototype validates neither
the persistence nor the interactions of the product.

The backend of the Figma library `47:7638` now has the paginated
document and history commands, separate reading of the full text,
versioned drafts and named copies, close with discard and
deletion markers (ADR-0014, migration 5). Reopening a retained
result checks its original connection without a driver or a new execution.
Local searches can be cancelled while they scan SQLite.
GPUI navigation now opens History / Saved queries / Recent results
through the sidebar or `⌘⇧H`. Paginated lists, menu filters, search with
a 250 ms delay and read-only inspection go through the bus. Named
and working copies are viewed separately; this view does not replace the
console and submits no execution. Recent results filters entries that have
a result reference; that does not guarantee that their buffer is still
retained. The connection menus expose the configurations known at opening
and the active connection; completing their choices from the global history is still
needed for deleted connections or other workspaces.
Consoles now have independent controllers: `⌘T` opens a
dedicated session, `Ctrl+Tab` / `Ctrl+Shift+Tab` switch console, `⌘W` closes the
active console after an explicit choice if it holds SQL or an operation.
Each tab keeps editor, result, confirmation, pages and export, including
off screen. The first console is also separated from the session reserved for the
catalog and the preview. `CloseSession` closes only the designated session and
cancels its preparation or draining before release (ADR-0015).
Connection workspaces are kept in the window, connected, eight at
most ([ADR-0046](adr/0046-workspaces-retenus-restent-connectes.md)); returning to
a connection restores all its consoles, without reconnection or SQL
(`features/workspace/workspace-host.tsx`, held by `workspace-host.test.tsx`).
The home screen names a hidden connection one of whose consoles holds a
transaction (story `SavedConnections/OpenWithPendingTransaction`). A copy of the draft to a new
connection is flagged, without execution, and keeps the original. Common
preferences are reapplied when a retained workspace becomes visible again.
Explicit save from the editor is wired: editable name,
`⌘S`, cancellation, distinction between the acknowledged version and later changes,
and the choice Save and close / Discard and close / Cancel. Closing a saved
document also updates its open state in the store, without deleting its named
copy. A conflict proposes a new identity to preserve both texts.
The last window waits for already committed local document writes,
like those of preferences; this does not validate every native shutdown path.
Text and name edits now submit a local autosave
through a bounded queue: one active operation, the latest pending draft,
one named save and one close. The backend finishes the already submitted queue
even if the view disappears. An empty working title is kept, while
a named save always requires a name. Expected revisions are checked
in the transaction; a conflict stops the writes of that controller.
Closing also protects a document whose first write has not yet
started. Cancelling a close puts its latest draft back in the queue, without
requiring the view to come back (ADR-0016).
The library now opens an SQL copy on the connection explicitly
named by the action, without execution. On the original connection, Resume working
query resumes the working text, its named copy and its revision; if the
document is already open, its console and its changes are kept.
To edit the original on another connection, that connection must first be
selected; the copy on the current connection remains available.
Entries that need reconciliation do not offer editable opening.
Selective recovery of SQL drafts now appears at startup when
open working copies exist. The selection is paginated; bodies
are loaded on demand into editors without a session. They remain editable
and saveable offline. Explicitly choosing a connection transfers the
same editor and its write queue; another connection creates a copy. No
query leaves during restoration or during that attachment.
Object tab positions, persistent qualification of an abnormal
shutdown and persistent provenance of agent texts are done:
`ObjectLocation` carries the restored location, the `app_sessions` table
(migration 6) distinguishes a clean shutdown from a crash by the heartbeat, and the
`provenance` column of documents records who wrote a text.
The library now opens retained buffers without a session or SQL, with
IPC page reading and export of the same result. A truncated, incomplete
or uncertain result does not become exportable. An expired reference shows
unavailability without offering an implicit replay. Returning to the library
lets a committed export continue; closing the result requires its end or
its cancellation.
The registry evicts results without a reader according to ADR-0017: at most 16,
256 MiB resident/cache and 1 GiB of spill for this category. Views
and exports remain protected by their references. The check runs after
execution and every second in the backend; it does not cap the memory
of all active views. The recovery message therefore announces
available copies, without claiming to have detected a crash. The native visual
acceptance test of this library remains to be done.

## Migration to the Tauri interface

[ADR-0029](adr/0029-interface-tauri-shadcn.md) replaces GPUI. State as of 2026-09-15:
`apps/desktop` and `crates/oxyn-desktop` exist and pass `make qualite`; the connection
screen, the SQL console (execution, cancellation, approval, export), the paginated
grid, the catalog tree and the object view (Data, Structure) work in
the Tauri window.

The list that still had to be done at that time before deleting `oxyn-ui` and `oxyn-app`:

- compact switch below 1,200 px and `More` menu — **partly**: the switch
  exists (`COMPACT_BELOW_PX`, `features/workspace/use-compact.ts`), the `More` menu
  and the `Actions` menu do not, see the gate below;
- ~~cell display settings (`FormatOptions`) and switchable light theme~~ — done (`features/settings/`);
- ~~restoring drafts after an abrupt shutdown~~ — done (`features/recovery/`);
- measurement campaign of the [PERFORMANCE](PERFORMANCE.md) budgets **in the webview**,
  which is the reconsideration condition of ADR-0029 — **not done**. It
  opens windows and puts the machine under instrumentation (frame, cold
  start, RSS, scrolling a million rows), which the instruction "no
  window opened on my screen" rules out on the workstation: it requires
  **a dedicated session or machine, designated by the user** (decision of
  2026-09-24). Until one is designated, the campaign cannot start;
- verification of rendering under WebView2 and WebKitGTK — **not done**.

**Exit gate — reopened on 2026-09-25.** It required every flow
of the GPUI interface to have its equivalent in `apps/desktop`, with its stories.
It was declared passed on 2026-09-18 (commit `6ecb8ce`), which removed
`oxyn-ui`, `oxyn-app` and the `gpui` dependency in a single commit, along with the
`make app` and `make lancer` targets — `make desktop-dev` now launches
the application, and `.claude/verifier_socle.py` refuses `gpui` everywhere. The removal
holds; **parity does not**. The audit of 2026-09-24, re-checked in the code on
2026-09-25, finds flows that this plan declares done, or held by GPUI
tests that `6ecb8ce` deleted, and that do not exist in `apps/desktop`:

| Flow | What the plan said about it | State in `apps/desktop` as of 2026-09-25 |
|---|---|---|
| `Actions` menu at compact width | Columns, Export and the inspector move into `Actions` in compact (state as of 2026-09-10) | **done on 2026-09-25** in the object preview: below 1,200 px, `Actions` (foot of `ResultPanel`) groups Columns, Export preview… and Inspect row, with no duplicate in the bar; the inspector is closed by default (story `Oxyn/ObjectView`, `Compact`). The console keeps its bar |
| `More` menu | Indexes, Relations and Constraints move into it in compact; gate ticked above | **done on 2026-09-25.** In compact, Data and Structure stay tabs; Indexes, Constraints, Relations and DDL move into `More` (story `Oxyn/ObjectView`, `Compact`) |
| Persisted panel preferences | wide sidebar kept, inspector handle "saving at the end of the gesture", panel persistence according to ADR-0013 | **done on 2026-09-25.** `sidebar_collapsed` and `inspector_open` are read at startup and written on each toggle at normal width, never in compact (`use-panel-preferences.test.ts`); `inspector_width` follows the handle, saved at the end of the gesture (`b03e0b0`) |
| Read-only library | History / Saved queries / Recent results, read-only inspection without execution | **partly.** History, Saved and Recent results exist, paginated, filtered, search at 250 ms; selecting an entry shows its full text read-only (`components/oxyn/library-entry-view.tsx`, on 2026-09-25); the library only exists in a connected workspace |
| Recent results | a view dedicated to retained results; held by `history_and_recent_results_read_the_same_execution_without_replaying_it` | **done** on 2026-09-25. The `results` view of `library-panel.tsx` reads the history filtered by `resultsOnly`; the cited test exists again in `oxyn-desktop/src/backend/library.rs`. The reference does not prove retention: opening checks it |
| Offline restoration of editors | recovered drafts remain editable and saveable without a session, then attach to a chosen connection | **done**, within a workspace. The chosen copies are added without replacing those already recovered and reopen as consoles **without a session** (`features/consoles/use-consoles.ts`): text, name, autosave and named save work, execution waits. `Connect to …` gives the same console a session on the original connection, `Open a copy on …` opens a copy elsewhere; cancelling leaves the console offline. Held by `an_offline_copy_attached_to_its_connection_keeps_its_identity`, `a_copy_on_another_connection_leaves_the_original_untouched` and the stories `OfflineConsoleBar`, `ConsoleView/Offline`. Remaining: cold, with no open connection, the copies wait for the first workspace — there is no workspace without a connection |
| Export of a retained result | the library exports the retained buffer without replaying it | **done** on 2026-09-25. `retained-result-tab.tsx` mounts `ExportMenu` under the grid, exportable only for a complete, successful, untruncated result; the tab refuses to close during an export, which survives a change of side view. Held by the stories `RetainedResultView/ExportOffered`, `ExportRefusedWhenIncomplete` and `WorkspaceTabs/ResultExporting` |
| Side DDL panel | read-only DDL panel (`193:2433`), resizable | **absent** as a panel: the DDL is a tab of the object view (`RelationDefinition`) |
| Disabled `Edit rows…` and its tooltip | required by [UX-SPEC](UX-SPEC.md) in the read-only preview | **absent**: no occurrence in `apps/desktop` |
| The library follows executions | the open library rereads itself after an execution (automatic refresh batch, ADR-0022) | **absent.** `LIBRARY_QUERY_KEY` is only invalidated by Refresh and the library's actions; the comment "a run invalidates it" has no code behind it |
| Restoring the object location | "done (`ObjectLocation`)", held by `a_restored_location_and_its_sub_tab_come_back_without_reading_anything` | **done on 2026-09-25.** `read_object_location` / `write_object_location` (outside `ipc/settings.rs`, which still excludes it); the tab comes back held, without reading, and the recovery screen offers it — see "Recovery and library" |

The Columns menu, for its part, is delivered (`ef2ada8`, `components/oxyn/columns-menu.tsx`)
in the console, the preview and the retained result. Each batch that delivers one of these
flows sets its row back to "done" here, with the test or story that proves it.

What remains open after the removal, without any of these points being settled
here:

- **the last two points of the list above.** They were announced
  "before deleting"; the exit gate did not require them, and the
  removal took place without them. As long as the campaign is not done, the
  reconsideration condition of ADR-0029 has not been evaluated, and the frame,
  cold start and idle memory verdicts of
  [PERFORMANCE](PERFORMANCE.md#comparison-with-the-budgets) concern
  the removed interface;
- ~~**the status of ADR-0009.**~~ — `superseded` since 2026-09-24; the
  status of ADR-0029 itself falls under
  [open question no. 3](#3-the-status-of-adrs-review-of-2026-09-24);
- **what the GPUI interface did and `apps/desktop` does not**, found while
  realigning [ARCHITECTURE](ARCHITECTURE.md): ~~the window title that
  signaled a `--temporary-workspace`~~ — carried over on 2026-09-25 (`window_title`,
  `oxyn-desktop/src/main.rs`); the file picker via `⌘O`, or `⌘N` for
  an SQLite database to create, is not requested (decision of 2026-09-24) — the front end
  only offers a `Browse…` to an existing file;
- ~~**`assets/fonts/` and `assets/ui/`**, which only `oxyn-ui` read~~ — removed
  on 2026-09-25, no reader remaining; readable at commit `8a1b7ff`;
- ~~**code comments still name `oxyn-app`, `oxyn-ui` or GPUI**~~ —
  done on 2026-09-25: the last one, in the tests of `oxyn-core/src/value.rs`,
  now points to the `∅ NULL` rendering of
  `apps/desktop/src/components/oxyn/cell-value.tsx`.
  `grep -rn -i "gpui\|oxyn-ui\|oxyn_ui\|oxyn-app\|oxyn_app" crates drivers apps/desktop/src Cargo.toml`
  returns nothing anymore.

The dated validations of this document that cite a "GPUI test" or an
`oxyn-ui` test describe the state at their date: those tests were deleted with the
two crates. On the front-end side, [I-01](../CLAUDE.md#i-01) is now held by
`.claude/hooks/code_interdit.py`, which refuses any caller of `invoke` outside
`apps/desktop/src/lib/ipc/client.ts`.

## Desktop application interactions — decided on 2026-09-25

Goal: that Oxyn handles like a native application — menu bar,
right-click on every surface, shortcuts, palette, drag and drop, several
windows. The behavior is in [UX-SPEC](UX-SPEC.md#menus-shortcuts-and-gestures),
the decisions in [ADR-0041](adr/0041-registre-d-actions-menus-et-raccourcis.md),
[ADR-0042](adr/0042-revue-sur-place-des-operations-destructrices.md) and
[ADR-0043](adr/0043-multi-fenetre.md). Batches 1 to 6 are done (4 under
the list), 8 too, 7 for its first part; the others are not
implemented. The interfaces are written in shadcn/ui (`menubar`, `context-menu`, `command`,
`alert-dialog`, `kbd`), except the native macOS bar, built in Rust.

User decisions, on 2026-09-25: a menu bar on Windows
and Linux too; `Drop…`, `Truncate…`, `Rename…` in V1, through an in-place review;
multi-window in V1; cancellation with `Esc`; no `⌘1…9`; `⌘/` and `⌘F`
change action depending on the area that has focus; closing the last window
quits the application, macOS included; `Toggle side panel` is `Ctrl+Shift+B`
on Windows and Linux; the consoles of an ordinary close come back in
their window, offline; an open transaction holds the exit
(`Commit`, `Rollback`, `Cancel`), except the Dock's Quit, which cancels it and
logs it; `File ▸ Exit` on Windows and Linux.

Batches, in order:

1. **"Not a browser" hygiene** — webview context menu limited to
   fields, `⌘R`, page zoom and pinch neutralized, autocorrect and
   typographic quotes turned off in fields, external links to the
   system browser (ADR-0041).
   **Done on 2026-09-25**, except `open_external`, postponed (below):
   - `apps/desktop/src/lib/browser-defaults.ts`, installed once by
     `routes/__root.tsx`, cancels with `preventDefault` alone the page menu outside
     a text field, `⌘R`, `⌘⇧R`, `Ctrl+R`, `F5`, `Mod` + `=`, `-`, `0`
     (read through `event.code`), the back keys and buttons 4 and 5, `Alt+←`
     and `Alt+→` outside macOS, and pinch (`wheel` with `ctrlKey`,
     `gesturestart`, `gesturechange`) outside a `data-own-zoom` area — the
     `erd` diagram. Batch 2 takes these combinations into the manifest as
     neutralized (ADR-0041 § 8): until then, this module holds them alone;
   - fields: `components/oxyn/text-field.tsx` wraps `Input`, `Textarea`,
     `InputGroupInput` and `InputGroupTextarea` from `components/ui` with
     `spellCheck`, `autoCorrect`, `autoCapitalize` turned off and `autoComplete`
     set to `off` by default; ESLint (`no-restricted-imports`, hence `make front`)
     refuses unwrapped fields. The SQL editor
     (`contentAttributes`), the assistant's composer and the search of the
     driver choice receive the same attributes;
   - `crates/oxyn-desktop/src/webview_guard.rs`: the `main` window is
     declared `"create": false` and built in `setup` by
     `WebviewWindowBuilder::from_config`, with `on_navigation` (refusal outside
     the application's origin: `devUrl` in development, `tauri://localhost`
     or `http(s)://tauri.localhost` on Windows) and `on_new_window` (always
     refused). `allowLinkPreview` becomes `false`. No permission added;
   - selection: unchanged, `styles.css` already held it as ADR-0041 § 8
     describes;
   - in passing: `main.rs` called `tauri::Builder::setup` twice, which
     **replaces** the previous one instead of adding to it (`tauri` 2.11.5,
     `src/app.rs`); the first, which gave the native dialog of ADR-0037 its
     `AppHandle`, did not run, and every critical confirmation was
     refused. The two are merged.

   Deviations and postponements of batch 1:
   - **`open_external` postponed** (decision of 2026-09-25): no link
     that Oxyn declares exists yet, and the command would have no caller.
     It will require a dependency — `clippy.toml` forbids
     `std::process::Command::new` —, hence `/versions`. **What
     unblocks it**: the first external link Oxyn renders (documentation);
   - the React Flow attribution (`erd-diagram.tsx`) is an external `<a href>`
     rendered in the webview, against ADR-0041 § 8. Its click is now
     refused by `on_navigation` and does nothing; `open_external` will give it
     a destination;
   - WebView2's `Ctrl+P` and `Ctrl+F` are not absorbed: they go back to the
     dispatcher (batch 2) and to `⌘P` (batch 5);
   - points to check no. 4 (macOS typographic quotes despite
     `autocorrect="off"`), 5 (pinch under WKWebView and WebKitGTK) and 6
     (`preventDefault` against WebView2 shortcuts): still not
     checked by hand; the tests cover the listeners, not the engine. Also
     to check in `make desktop-dev`: opening the window through
     `from_config` and refusing a click on the React Flow attribution.
2. **Action registry** — `actions.json` and `registry.ts`, single keyboard
   listener, wiring of existing buttons; native macOS bar
   (`menu.rs`, `subscribe_menu`, `set_menu_state`), which replaces
   `application_menu` from `commands/recovery.rs` while keeping the Quit
   of [ADR-0038](adr/0038-un-plantage-s-annonce-une-fois.md) handled in Rust;
   `menubar` bar on Windows and Linux, with `File ▸ Exit` and the
   `request_exit` command (ADR-0041 § 5).
   **Done on 2026-09-25.** `apps/desktop/src/lib/actions/` (manifest,
   behaviors, dispatcher, menu model), native bar read by
   `crates/oxyn-desktop/src/menu.rs`, `subscribe_menu`, `set_menu_state`,
   `request_exit`; `@tanstack/react-hotkeys` removed. Held by
   `manifest.test.ts` (identifiers, uniqueness per area, forbidden
   combinations, `⌘W` on `Close tab` only), `keyboard.test.ts` (areas, AZERTY,
   Cyrillic, `⌥`, composition, dialog, native bar), the stories
   `Oxyn/AppMenubar` and `Oxyn/SqlEditor`, and the tests of `menu.rs`. Deviations
   and leftovers:
   - the bar does not yet carry `New window`, `Open Recent ▸`, `Save as…`,
     `Export…`, `Text size ▸`, `Theme ▸`, `Format`, nor the `Help` menu
     (`Documentation` waits for `open_external`, batch 1; `Keyboard shortcuts`,
     declared with neither place nor behavior, the sheet of batch 5);
   - `Settings…` has no place set by UX-SPEC outside macOS: it is put
     in `File`, above `Exit`;
   - the native Quit item is now called `app.quit`, the action's
     identifier, and no longer `oxyn-quit` as cited by ADR-0038 and ADR-0043;
   - `Escape` is still handled by the listeners of the connection screen, its
     form, saved connections and the recovery screen, in the
     bubbling phase: the dispatcher, in capture, would take it from an open
     list or menu. Only `⌘[` and `Alt+←` go through the registry there;
   - `kbd`s written by hand outside the console, the tabs and
     the workspace header remain to be read from the manifest (batch 5);
   - `⌘R` / `F5` reloads are not declared in the manifest: they
     belong to batch 1;
   - on macOS, whether `⌘W` closes the tab and not the window depends on
     the delivery order of a keystroke between WKWebView and the bar (ADR-0041,
     point to check no. 1), which no automated test reaches: manual test
     procedure in the batch's PR.
3. **Context menus** — grid and column headers, tabs, SQL editor,
   saved connections, library, assistant, ERD; catalog
   additions.
   **Done on 2026-09-25.** Each surface lists registry actions
   (`lib/actions/context-menus.ts`); the target of a right-click joins the
   context through menu-specific sources (`menuContext`,
   `lib/actions/targets.ts`), the conditions are in
   `lib/actions/menu-behaviours.ts`, and `ActionMenuContent` renders the entries,
   greyed out with their reason in the entry's text. Copied SQL is composed
   in Rust: `copy_result_rows` (six formats, 2,000 rows at most, 32 MiB at
   most, literals through `oxyn_catalog::push_string_literal`, which refuses NUL and
   control characters other than tab and line endings) and
   `compose_object_sql` (`Quoted name`, `SELECT *` without `LIMIT`, `INSERT
   template`). `edit.cut`, `edit.copy` and `edit.paste` now exist on
   macOS too, without a place in the native bar, for the editor's menu;
   `tab.close` takes the label `Close` on a tab; `⌘⇧T` is
   `tab.reopen`. Held by `context-menus.test.ts` (including: never a `Run` on an
   assistant code block), the stories of each surface, and the tests of
   `literal.rs` and `backend/results/copy`. Deviations and leftovers:
   - **greyed out, with what is missing as the reason**: `Filter by this value`,
     `Exclude this value`, `Is NULL` (the preview only carries a text predicate;
     **follow-up** once the apercus-execution batch is merged, issues #14
     and #16: a bound value in the preview's form), `Freeze`, `Open
     referenced row`, `Send to assistant` under `Sampled` (the approval
     screen only opens at a model's request), `Format` (no
     SQL formatter: a dependency to go through `/versions`), `Ask assistant about
     selection` (the composer does not receive text from outside), `Re-layout`
     and `Export image…` of the ERD, `Reveal in Finder`/`Explorer` (waits for a
     system command, like `open_external`); `Open in new window` is
     active since batch 7;
   - **greyed out** in the library, with what they wait for (batch 3 ter):
     `Rename…`, `Duplicate` and `Copy path` — they would need
     `rename_query_document` and `duplicate_query_document` in Rust, the
     open-then-save composition not being atomic, and
     `DocumentEntry` carries no path;
   - `Duplicate` of a connection opens the creation form pre-filled with
     only the non-secret parameters; the copy starts in `production`, at the
     default AI tier, and nothing is saved before `Connect`. It is
     not offered in the settings, which have no creation form;
   - `Copy answer` now copies the readable text, button included; `Copy as
     Markdown` copies the source;
   - the catalog's `Rename…` is displayed in red, like any
     `destructive` action;
   - `Open object under cursor` resolves the name in TypeScript against the tree already
     read (case ignored outside quotes): a resolution by the backend
     would follow the dialect;
   - `⌘⇧T` reopens in a new console and session, without the bound
     parameters; `Reveal in library` opens the library without selecting
     the entry in it;
   - `Filter…` of a preview header gives focus to the `WHERE` field;
   - the rows of the Structure tab do not take focus: `⇧F10` does not
     target a column there;
   - to wire the preview and the Structure tab, `features/workspace/object-view.tsx`
     receives a few lines (grid menu, column rename review);
   - TSV, CSV, JSON and Markdown copies copy values as
     they are, control characters included, like the grid's `⌘C` copy;
     only copied SQL refuses them. To be settled in SECURITY whether pasting into
     a terminal should apply to data too;
   - found in review, predating this batch: `quote_identifier` only doubles
     the closing character, whereas BigQuery (and probably ClickHouse) reads
     backslash escapes in a quoted identifier. No driver
     produces these dialects today; to be fixed before the first one.

   **Batch 3 bis — defects found in use, fixed on 2026-09-25**:
   - **copies refused by WKWebView** ("The request is not allowed by the
     user agent…"): the clipboard was written after an IPC round trip,
     outside the user's gesture. `writeClipboard` (`lib/clipboard.ts`)
     is now the only write: called in the click, it hands a
     text still to be read to a `ClipboardItem` resolved later
     ([RESEARCH-NOTES](RESEARCH-NOTES.md#webview-clipboard--check-of-2026-09-25)),
     and states plainly a late refusal where the engine has no `ClipboardItem`.
     Going through it: the catalog (`Copy qualified name`, `Copy as ▸`),
     `Copy value` and the grid's row copies, the grid's `⌘C`,
     the editor, the assistant, the diagram's `Copy name`. No dependency,
     no Tauri permission;
   - **`Copy as ▸ INSERT template` without columns read**: the catalog first reads
     the structure through the bus (`refresh_relation_facet`, `detail`
     facet), then composes; `compose_object_sql` keeps its refusal, which is no
     longer reached from the menu. A copy failure is the copy's toast,
     and no longer the panel's "The catalog could not be read" alert
     (`features/workspace/catalog-copies.ts`);
   - **a node's tooltip over the context menu**: the system
     tooltip (`title`) is replaced by Base UI's, closed and
     disabled while the menu is open; an object's comment remains
     the accessible description of its row;
   - **two menus for two sibling tables**: not reproduced. The catalog
     menu has only one source (`menuSources` of `catalog-tree.tsx`) and
     `Collapse all` is always present in it; the list found on `customers`
     is exactly the hand-written menu from before the registry (`f038428`),
     hence the hypothesis of a build predating batch 3. The story
     `SiblingTablesShareTheirMenu` holds the equality; the user re-checks
     on the installed build.

   **Batch 3 ter — an entry no longer disappears for lack of a handler, done on
   2026-09-25.** The capture of batch 3 bis showed an `invoices` menu without
   `Copy as ▸` or operations, for lack of handlers given to the tree: this is
   very probably what gave the user two menus. From now on
   (`onTarget` of `lib/actions/menu-behaviours.ts`), the target's state alone decides
   absence — the UX-SPEC cases, "One action, one label, one
   shortcut": object type, undeclared capability, no AI destination,
   agent —, and a handler the surface does not give **greys out** the entry
   with a reason specific to the surface. The catalog always provides its source
   of operations ("Catalog operations are not available here." without a session).
   Behavior changes, on every surface:
   - `Pin to question` under a tier other than `Sampled` is **greyed out** with the
     tier, like the grid's `Send to assistant`; it used to be absent,
     against UX-SPEC ("an AI tier that refuses" greys out);
   - `Delete…` of an open connection is greyed out ("Disconnect it to delete
     it"), `Reveal in library` of an object tab greyed out ("Only a console is
     saved in the library"), `Open` of a write awaiting inspection
     greyed out, `Open in console` of a code block greyed out with the button's reason,
     `Hide` of the last column greyed out;
   - in the settings, the connection entries the screen does not perform
     (`Refresh catalog`, `New console`, `Duplicate`…) are greyed out.
   Held by `context-menus.test.ts` ("never lose an entry because its surface
   left the handler out", on every surface) and the stories of the
   surfaces, including `Oxyn/CatalogTree` `ContextMenu`.

   **Batch 3 quater — the reason for a greyed-out entry says where to act, done on 2026-09-25.**
   Generic reasons per surface ("Not available for this connection
   here"…) left no way out: seen from the settings, `Disconnect` was
   greyed out without saying where to disconnect. `onTarget` now requires, for each
   entry, a reason that says what is missing **and where to do it** ("Connect
   from the start screen", "Show its workspace from the start screen, then
   press ⌘T", "Disconnect it first, from this menu"). The settings
   wire `Disconnect` (`closeConnection`, like the home screen;
   the home screen is shown if it was the displayed workspace) and `Refresh
   catalog` (`features/connections/refresh-catalog.ts`, shared with the home
   screen). `New console` and `Duplicate` stay greyed out there: the first
   shows a workspace, the second needs the home screen's creation form,
   which the settings do not have. The "Not available
   yet: …" reasons keep their form: they name what Oxyn is missing, and no
   other place does. Held by `context-menus.test.ts` (no generic
   reason, neither on a surface without handlers nor in the sources) and
   the story `Oxyn/ConnectionManager` `ContextMenu`.
4. **Destructive review** — `review_object_operation`, `run_object_operation`,
   which approves a command held by `confirm_held`, hence by the native
   dialog of [ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)
   on `production`; capability flags `TRUNCATE`, `TRANSACTIONAL_DDL`,
   `RESTRICT_DEPENDENTS`, each proven by a driver integration test
   before being declared (ADR-0042).
5. **Palette and quick open** — `⌘K`, `⌘P`, shortcut sheet.
   **Done on 2026-09-25.** The palette (`components/oxyn/command-palette.tsx`),
   quick open (`quick-open.tsx`) and the sheet (`shortcut-sheet.tsx`)
   are installed once by `features/actions/action-overlays.tsx`, opened
   by the actions `palette.open`, `object.quickOpen` and `help.shortcuts`, and
   read only the registry: `lib/actions/listings.ts` draws the entries from it,
   greyed out with the reason from `enabled`. `⌘P` goes through `search_catalog`, under
   the query key of the catalog search: loaded objects
   only, bounded by the backend, no read from the server. Added to the
   registry and to both bars: `New window`, `Export…`, the `Help` menu
   (`Documentation`, `Keyboard shortcuts`), `Text size ▸`, `Theme ▸` and
   `Format`, through the manifest's `parent` and `check` fields that ADR-0041
   (§ 1 and § 4) now describes. Held by
   `listings.test.ts` (the palette equals the context's registry, reasons
   included; `⌘/` depending on the area; `⌘P` greyed out and not absent without a catalog),
   `action-overlays.test.tsx` (`⌘P` only calls `search_catalog`),
   `manifest.test.ts`, the stories `Oxyn/CommandPalette`, `Oxyn/QuickOpen`,
   `Oxyn/ShortcutSheet` and `Oxyn/AppMenubar` (`ThemeIsAChoice`), and the tests
   of `menu.rs`. Decisions and deviations:
   - `Settings…` stays in `File` on Windows and Linux, above `Exit`,
     and UX-SPEC now says so (decided on 2026-09-25);
   - `Format` is present everywhere, **greyed out** with its reason: no SQL
     formatter is shipped (batch 9 below);
   - `Documentation` is present, **greyed out** with its reason, until
     `open_external` (postponed from batch 1);
   - `New window` is present; active since batch 7;
   - `Export…` opens the export menu of the displayed result, located through
     `data-action-export`: its greyed-out state and reason are the button's. But
     the end of an object preview changes no registry source: on
     macOS, the native entry can stay greyed out until the next change
     of focus or tab (`invoke` re-evaluates on click anyway);
   - an already active checked choice, clicked in the native bar, has its
     check toggled by AppKit: the front end sends back the state after each
     native activation to restore it;
   - `Open Recent ▸` and `Save as…` are still not in the bar;
   - the hand-written `Kbd`s of `⌘,`, `⌘↵`, `⌘J` and `⌘⌥B` (preferences,
     result, side panel) now read the manifest; those that
     remain describe keys internal to a component (`Esc`, `↵`), which
     the manifest does not declare;
   - the sheet does not yet list the keys internal to components
     (arrows, `Home`, `End`…) that ADR-0041 § 2 has it show: they are not
     declared in the manifest;
   - `Open object…` is declared in the `app` area, greyed out under a dialog:
     the dispatcher only resolves that area there, and `Ctrl+P` would have reached
     WebView2's printing there.
6. **Drag and drop** — internal on pointer events, dropping files
   from the system (ADR-0041).
   **Done on 2026-09-25.** Local implementation, without a library:
   `components/oxyn/pointer-drag.ts` (4 px threshold, window listeners,
   `Escape` that cancels, click swallowed after a drop). Tabs
   (`workspace-tabs.tsx`, order held by `workspace-screen.tsx`), grid
   columns (`column-order.ts`, `result-grid.tsx`), catalog relation
   to the editor (`catalog-name-drag.ts`, `features/workspace/name-transfer.ts`,
   `insertAt` of the area handle of `sql-editor.tsx`, at the
   `posAtCoords` position). The inserted name is the `qualifiedName` of `relation_facets`,
   quoted by the backend. File drop: `crates/oxyn-desktop/src/file_drop.rs`,
   `subscribe_file_drops`, `lib/ipc/file-drops.ts`, `features/file-drops/`;
   no permission added. Held by `pointer-drag.test.ts`,
   `grid-selection.test.ts`, `result-grid.test.ts`, `name-transfer.test.ts`
   (hostile name), `use-file-drops.test.ts`, the tests of `file_drop.rs`
   (extension, existence, size, symbolic link, UTF-8, number of
   files) and the stories `Oxyn/WorkspaceTabs`, `Oxyn/ResultGrid`,
   `Oxyn/CatalogTree`, `Oxyn/SqlEditor`, `Oxyn/ConnectionForm`. Deviations and
   leftovers:
   - **keyboard equivalents chosen by this batch**, which UX-SPEC did not set:
     `⌥⇧←` and `⌥⇧→` move the focused tab or the column of the active
     cell (`item.moveLeft`, `item.moveRight`, a single pair at the
     `global` level with a label per area: two specific areas cannot
     share a combination); `⌥↵` on a catalog relation puts its
     name in the active console (`catalog.insertName`, `tree` area, which
     the tree now declares). All in `binding: component`;
   - **no catalog column**: the tree does not show columns
     (see batch 4). Only relations can be dragged;
   - tab order lives as long as the workspace is open, **without
     persistence**; column order is forgotten at the next result, like
     their visibility. A copy follows the displayed order, since it copies what is
     shown; export keeps the result's order (UX-SPEC,
     "Columns and value inspection");
   - a `.sql` dropped **without an open connection** is refused with a message:
     a console belongs to a connection, and a queue
     would have made it pop up in the workspace opened afterwards;
   - the recognized extensions (`.sqlite`, `.sqlite3` for `sqlite`;
     `.duckdb` for `duckdb`) are a table in `file_drop.rs`: drivers
     do not declare the files they read. No DuckDB driver is
     registered, so a `.duckdb` is refused with its reason. Bounds: 4 MiB
     per `.sql`, 16 files per drop; a symbolic link is refused;
   - each window receives the drops made on it, and only those (batch 7);
   - no automatic scrolling when a tab or a column is dragged
     to the edge of the view, nor a drop marker in the editor: the ghost
     follows the pointer;
   - **to check by hand** in `make desktop-dev`, on Windows especially:
     that a dropped file arrives with `dragDropEnabled` left true, and that
     internal dragging does not suffer from it. The stories simulate pointer
     events, not the engine.
7. **Multi-window** — per-window subscriptions, `WindowRegistry`, capability
   by `workspace-*` pattern, layout migration, `Open in new window`,
   per-window restoration, consoles included after an ordinary close
   (ADR-0043). Delivered in two pull requests; **the first is done**, see
   below.
8. **Open transaction at exit** — step before the ordered shutdown,
   `ShutdownSignal::ResolveTransactions`, `shutdown_acknowledged`,
   `cancel_exit`, log of the forced exit (ADR-0043). **Done for one
   window**, see below.
9. **SQL formatting** — `Query ▸ Format`, declared and greyed out since batch 5:
   choose a formatter through [`/versions`](../.claude/commands/versions.md),
   date the choice in RESEARCH-NOTES, then give the action back to the editor.
   Added on 2026-09-25 by the decision of batch 5.

**Batch 4 done on 2026-09-25.** `backend/object_operations.rs` composes, reviews and
submits; `review_object_operation` and `run_object_operation` are the two
Tauri commands; `ObjectOperationReviewDialog` is the dialog, with its stories;
`Drop…`, `Truncate…` and `Rename…` are in the catalog's context menu.
The flags `TRUNCATE`, `TRANSACTIONAL_DDL` and `RESTRICT_DEPENDENTS` (bits 44
to 46) are proven by `ddl_tests` in each driver that declares them. The test
`object_operations_are_not_tools` guards `oxyn-ai`. Deviations and leftovers:

- **SQLite enforces foreign keys by default.** ADR-0042 (table of
  sources) and the header of `drivers/oxyn-driver-sqlite/src/session.rs` said
  the opposite. The engine embedded by `libsqlite3-sys` (`bundled` feature)
  is compiled with `SQLITE_DEFAULT_FOREIGN_KEYS=1`. A `DROP TABLE` therefore fails
  on a child row, but not on a view nor on a key without rows.
  The decision holds: no `RESTRICT_DEPENDENTS`. The header of `session.rs`
  is corrected, and the test `foreign_keys_are_enforced_by_the_bundled_engine`
  pins the behavior. The line of ADR-0042 remains to be corrected by a clarifying
  ADR;
- `review_object_operation` also takes the **catalog session**: its
  capabilities decide what is offered, and the session registry does not
  designate the catalog session of a connection;
- **renaming a column**: the backend and the dialog know how. No entry
  offers it yet, because the catalog tree does not show columns.
  The entry comes back in the Structure tab's menu (batch 3) — done;
- an error returned by `decide` after approval does not carry its class
  (`IpcError` only has `retryable`). The dialog shows the message and
  `Refresh catalog`, without ever offering to retry. It cannot write
  the ambiguous-error sentence with certainty;
- Redshift declares `TRUNCATE` through the PostgreSQL driver, proven against
  PostgreSQL. No Redshift server is available for testing: the removal of
  `TRANSACTIONAL_DDL` and `RESTRICT_DEPENDENTS` is checked on the variant
  (`variant.rs`), not against the engine;
- several windows (ADR-0042, "Several windows"): closing a
  window rejects the pending approval it owns (batch 7), and the
  review session closes with it.

**Batch 7, first part, done on 2026-09-25: live windows.**
`backend/windows.rs` holds the registry. It gives each window its
`WindowKey` and its label `workspace-<uuid>`, bounds windows to 16 and follows
focus. It also says what each window owns: sessions, commands,
results, documents, a connection's assistant, held connections. The
commands of `commands/` receive the calling `Webview` and refuse what another
window owns. `tauri.conf.json` now declares only one template,
`workspace`, and `capabilities/main.json` covers `workspace-*` with its three
permissions. `clippy.toml` forbids the six emit methods. The execution,
catalog, shutdown, menu and drop streams and the new
`subscribe_window` are per window, filtered in Rust. The native menu shows
the state of the focused window and sends it its actions, to it alone.
`Disconnect` is only emitted by the backend, when the last window releases
the connection. Closing a window that is not the last resolves its own transactions only,
then asks in a grouped dialog what to do with its consoles
(`CloseWindowDialog`). Exit asks each window about its own
transactions and flushes the drafts of all of them. `New window` is active. The tests
are in `backend/windows/tests.rs` (registry, ownership, bound, routing) and
`backend/windows/closing/tests.rs` (exit with several windows, closing a non-last
window, rejected approval, flushing all windows). The clarifications
and deviations are written in ADR-0043, "Implementation clarifications":
assistant per connection, two-step close, agent events.

**Batch 7, layout and restoration, done on 2026-09-26.** Migration 18
adds `workspace_windows` and `workspace_window_consoles`, written by
`Command::WriteWindowLayout`, refused to an agent, counted as a local
write, and read back as hostile input by `oxyn-store/src/windows.rs`.
`backend/windows/layout.rs` holds each window's layout and
serializes its writes. At launch, `build_launch_windows` reopens each
window under its key, in its place if it overlaps a connected screen. The
rectangle is written one second after the last move and before shutdown.
Each window's consoles come back offline after an ordinary
close, and each window offers its own after an abnormal shutdown. The
copies no window claims go to the first one. `object_location` is
per window. The tests are in `oxyn-store/src/windows/tests.rs` (round
trip, moving a console, live instance, bound of 16, hostile
row) and `backend/windows/layout/tests.rs` (consoles of another window
set aside, closed window that does not come back, orphan copies once, refusal
to an agent). Clarifications in ADR-0043, "Implementation clarifications,
2026-09-26".

**Batch 7, `Open in new window`, done on 2026-09-26.** A tab's menu
moves a console to a window built for it, with its session,
its document, its result and its bound values; nothing is re-executed.
`backend/windows/handoff.rs` prepares the `ConsoleHandoff` before building the
window, and the new webview adopts it once (`take_console_handoff`).
The registry transfers session and document under a single lock; the target reads the
result before the source releases its view. A window that fails to
build gives the console back. An object tab leaves as a location.
The entry is greyed out, with its reason, during an execution, a confirmation,
an export or a save. The tests are in
`backend/windows/handoff/tests.rs` (same session, single adoption, result
always read, console of another window refused, console given back) and
`context-menus.test.ts`. Remaining:

- reopening from the library a document that another window writes: it
  is refused for writing, but the owning window does not yet come
  to the foreground on that tab;
- the manual test on Windows and Linux: closing a window, the
  web bar of each one, creating a window from an
  `async` command, and restoration on two screens with different scales.

**Batch 8 done on 2026-09-25, for one window.** ADR-0039 is accepted and
implemented. `backend/exit.rs` holds the console sessions and the last observed
state of each, and the `exit_step` step precedes `begin_shutdown` on all
ordered paths (`⌘Q`, closing the window, `request_exit`,
`ExitRequested`). `shutdown_acknowledged` and `cancel_exit` are the two
commands without arguments; `ExitTransactionsDialog` is the dialog, with its
stories; `features/recovery/exit-transactions.ts` sends `COMMIT` or
`ROLLBACK` through `run_console`. The tests of `backend/exit/tests.rs` cover
the absence of a transaction, the held exit, `Commit` and `Rollback` then
exit, the refused `COMMIT` (SQLite deferred foreign key), the unknown outcome
never replayed, the silent webview and the Dock's Quit. Deviations and leftovers:

- **the state read is the one the executor published**, and not a read through
  `Session::transaction_state` from the bridge: ADR-0043 is clarified in that
  sense (call to the driver outside the bus, I-01);
- the signal carries the session, not the document: the window names the console
  after its tabs (`features/consoles/console-labels.ts`), and the log
  names the connection and the session;
- only SQLite declares `TRANSACTIONS`: PostgreSQL will only do so with
  pinning one connection per session (`variant.rs`, phase 1). On that day,
  the trap noted in DRIVER-CONTRACT applies here too: a production `COMMIT`
  is a read bounded to read-only, and the bound must
  not touch the transaction it commits;
- **extended by batch 7**: the list is grouped by window and each
  window receives its own on its own shutdown channel; the acknowledgment is
  awaited from each; `ExitCancelled` goes to all those that were
  asked; closing a non-last window concerns only its own
  sessions, without going through `begin_shutdown`; only windows that have
  a transaction come to the foreground.

Deviations found while writing, not settled, re-checked on `origin/main` on
2026-09-25:

- on macOS, `⌘W` is bound twice: by `workspace-screen.tsx` (close
  the active tab) and by the predefined `Close Window` item that the menu
  of ADR-0038 keeps in `File` and `Window`. If the keystroke reaches the menu,
  `performClose:` starts the ordered shutdown and `⌘W` quits Oxyn; the delivery
  order is not checked (ADR-0041, point to check no. 1). ADR-0041 removes
  this predefined item; until then, check it by hand in `make desktop-dev`;
- `⌘1` (catalog) and `⌘⇧H` (library) were bound by the code without being
  written in UX-SPEC; they now are, section "Keyboard";
- ADR-0025 cites `workspace/definition.rs`, `open_definition_console` and
  `library::OpenQuery::Copy`, gone with GPUI; the current composer is
  `crates/oxyn-desktop/src/backend/proposal.rs`;
- DRIVER-CONTRACT §6 talks about a driver quoting function; the code
  calls `oxyn_catalog::quote_identifier` everywhere;
- ADR-0029 says that a Tauri command only emits a `Command`, which
  `subscribe_events` and `shutdown_flushed` already contradict;
- the description of `capabilities/main.json` says the window has no title
  bar: this is only true on macOS;
- `.claude/commands/adr.md` claims that `make socle` catches an ADR missing from
  the index: `verifier_socle.py` does not check it.

Still to check by hand: the double binding of `⌘W` on macOS (first deviation
above), in `make desktop-dev`, before batch 2.

## Where these phases come from

### Full integration of the mockup — work started on 2026-09-10

The request covers the front-end flows **and** their back-end behavior.
The live Figma survey confirms the structure of `190:1163`, with the regions
`190:1543`, `190:1558`, `190:1588` and `190:1860`. Earlier local changes
to this document, to UX-SPEC and to FIGMA-HANDOFF are kept.

Integration order and validation criteria:

1. **Table/console workbench** (`oxyn-app`, `oxyn-ui`): dense frame, permanent
   context, keyboard navigation, compact width, read-only help,
   safe confirmation focus and preview export through `Command::Export`.
   Console and table exports keep distinct identities.
2. **Exploration and results** (`oxyn-catalog`, `oxyn-data`, `oxyn-exec`, UI):
   real sub-tabs according to capabilities, inspector, columns, reading of
   spilled pages off the UI thread and cache bound. No query on resize.
3. **Local work** (`oxyn-store`, `oxyn-app`, UI): history, documents and
   consoles, persisted preferences, selective offline restoration.
   Any clarification of a persistent format requires an ADR before implementation.
4. **Connections and AI workspace**: integrate the screens with the existing
   connection and privacy contracts, with empty states, cancellation,
   refusal and review of the context actually sent.
5. **Full acceptance test**: `cargo fmt --all`, `make qualite`, GPUI interactions
   and isolated SQLite, real captures at the reference widths and in both
   themes, then measurements according to PERFORMANCE.

The flows labeled phase 4 in the mockup respect the entry condition
of that phase; a board does not make a capability available.
The end requires evidence for the integrated flows; a compilation or a
Figma capture alone does not allow declaring this integration complete.

Validation of the first batch: `make qualite` passed format, Clippy, tests
and documentation after fixing three catalog constructors
(`with_default`, `with_system`, `with_inferred`). The tests cover safe
focus, keeping drafts on resize, late responses,
the preview bound and CSV export limited to that preview. PostgreSQL tests
requiring a server remain ignored; performance measurements remain
absent. The native acceptance test opened the temporary SQLite database, its catalog
and 200 rows, then switched between themes. The user was also using their computer during this acceptance test: the
window interferences are not application defects. The compact native
acceptance test remains inconclusive; only the GPUI test proves that path.
Desktop interactions were stopped at their remark; the rest of the work
continues without handling their windows.
The user also confirmed that export works in
the application on 2026-09-10; the exact format and file were not
specified. This manual validation complements the CSV tests, without proving the
other formats or all interruption cases.
The last presentation touch-ups must be taken up in the final visual
acceptance test of the full integration.

Validation of the page-reading batch: GPUI + SQLite test on a result of
100,000 rows, with a 512 KiB budget to force spilling. Reaching
a spilled page loads it through the bus without a new SQL execution. The tests
also check connection ownership for human and agent, the log
without cells, cancellation, refusal of pages too large and of stale
responses, and sharing of the retention budget. `make qualite` passes with
1,268 tests passed and 18 ignored. This measures neither RSS nor frame
budgets and does not replace a native scrolling acceptance test.

Validation of the inspector/columns batch: `make qualite` passes with 1,274 tests
passed and 18 ignored. The GPUI/SQLite tests go through a real value of
30,000 bytes with the keyboard, cancel the inspection when the result is replaced,
and check that the wide preference is kept at 1024 px. The formatting
tests rebuild a long Unicode text through bounded pages without omission
or repetition; absence, empty text and `NULL` remain distinct. The tests
also check the Arrow index of hidden columns and the ownership of the
value by its connection for human and agent. The native visual acceptance test and
resizing the inspector by its handle remain to be done.

Validation of the retained-results batch: `make qualite` passes with 1,331 tests
passed, no failure and 18 ignored. The GPUI test reopens a real result of
100,000 rows, checks sharing of the original buffer, loads its last IPC
page and exports it while the library is displayed again. No additional
execution event is observed. The tests also cover
unavailability of an expired reference and refusal to export a
truncated result. The registry is tested on eviction of the oldest results without
a reader, keeping an active reference and the memory and
spill limits. These checks do not prove the global RSS or the frame
budgets of open views; their native acceptance test remains to be done.

Validation of offline SQL restoration: `make qualite` passes with
1,326 tests passed, no failure and 18 ignored. The GPUI tests deliver the Space
key to the recovery checkboxes, restore only the chosen documents,
modify their text without a session and reread their autosave. `⌘Enter`
starts no offline execution. Saving then closing works without
a connection. Another test transfers the same restored editor to an
explicitly opened connection, without an execution event. The checked box glyph
comes from Figma node `232:9100`, is embedded and keeps its two colors.
These tests validate neither native pixels, nor recovery of object tabs,
nor crash detection or the actual inspection of an interrupted write.

Validation of autosave: `make qualite` passes with 1,323 tests passed,
no failure and 18 ignored. A controlled runtime checks that 200 submissions only
keep the latest draft and the intermediate explicit save;
after the receivers disappear, versions 200 and 50 are correctly reread in
their respective columns. The tests cover two concurrent writers,
the marker of a close before the first write, a cancelled close whose
draft is taken up without a view, and refusal of mutation under a stale expected
revision. The GPUI harness checks recovery of text without a named
save, keeping the named copy during a later edit, an empty working
name and discarding an autosaved console. The selection
and restoration flow at startup, the native acceptance test and its close paths
remain to be validated.

Validation of the open-from-library batch: `make qualite` passes with
1,316 tests passed, no failure and 18 ignored. The GPUI tests check opening
a copy without running or replacing the current draft, refusal of editable
opening of an ambiguous entry, resuming a working text under its kept identity
and language, then returning to its already modified console. A copy
to another connection keeps the original intact. Save conflicts at
equal revision and at higher revision lead to a new identity.
Legacy titles remain readable up to 4 KiB; beyond, reading is
refused before Rust materialization, without changing the 256-byte bound of
new writes. Native validations and automatic recovery are
not proven by this batch.

Validation of the explicit-save batch: `make qualite` passes with 1,312 tests
passed, no failure and 18 ignored. The new tests go through the editable name,
`⌘S`, editing after saving, returning to the saved version then cancelling
that edit, discarding while preserving the named copy and Save and close awaited
until storage. A save cancellation followed by a late response does not
close the console. A concurrent version is kept when the console
saves under a new identity. The input tests cover a Unicode composition
refused without altering the previous value and read-only mode during
closing. Twelve saves whose receivers are abandoned keep the
latest version after waiting for the backend. This proof concerns submitted
commands; it proves neither autosave on each keystroke, nor restoration at
startup, nor all native close paths.

Validation of the independent-consoles batch: `make qualite` passes with 1,306 tests
passed, no failure and 18 ignored. The GPUI events check two consoles
with distinct session and result identities, their off-screen
responses, export of the first result while the second tab is visible,
and the Cancel / Discard choice before closing. Returning to a connection
finds its two consoles and their texts, without execution. The SQLite tests
check an uncommitted transaction, its end by closing a single session,
and the availability of the neighboring console and of the catalog session.
A controlled driver checks closing during preparation and draining for
human and agent, with refusal of a wrong connection. The SQLite contract also covers
separate in-memory profiles, the copy shared between their sessions,
read-only mode that the caller's limits cannot lift, and destruction
after closing the last session. The local reviews covered
response correlation, late confirmations, cancelled openings,
the SQLite boundary and the absence of I/O in rendering. The native acceptance test and
performance measurements remain open; the GPU-less test does not replace them.

Validation of GPUI library browsing: `make qualite` passes
with 1,300 tests passed, no failure and 18 ignored. The tests deliver
keyboard and mouse events to the menus, to the read-only editor and to
the `⌘⇧H` / `⌘J` navigation. They check the named copy against the draft,
stale responses, cancelling an inspection, refusal of native input and
composition, and the absence of an execution event during browsing.
The text and result identity of the console remain unchanged. A menu
selection is only applied on validation; Escape and an outside click close
the menu without changing the filter. The recent-references test checks that their
filtering precedes pagination. The rows and headers use the 40 px of
`47:7881`, the labeled filters the 74 px of `47:7868`, and the bottom panel
the 238 px of `47:7937`. These source findings and GPU-less tests do not prove
pixel fidelity or the native frame budget.

Validation of the library backend: `make qualite` passes with 1,294 tests
passed, no failure and 18 ignored. The new cases cover migration of
old named copies, save ordering, revision conflicts,
discarding and deletion markers, workspace scope, literal
filters and pagination, refusal to open a truncated text, legacy
writes to reconcile and local reopening of a buffer without a driver.
Cancellation is exercised during an SQLite scan and a write transaction,
with verification of the rollback and of handler removal before the next operation.
The local review of the invariants and of the serialization boundary led
to also protecting the legacy save/delete paths and the reading
of an unreadable named copy. This validation covers the backend; the library's editing actions and
recovery remain open, beyond the GPUI browsing
validated separately.

Validation of the preferences/panels batch: `make qualite` passes with 1,282 tests
passed and 18 ignored. The tests cover reopening the local SQLite,
writes received out of order, conflicts and unreadable payloads, refusal
of an agent and the correct workspace target. The GPUI harness checks the
24/28 px rows, independence from the theme and viewport, the
8 px handle through delivered mouse events, its keyboard and keeping the active
table on collapse. Thirty saves whose UI receivers are abandoned
keep the latest revision after waiting. Native shutdown paths other than
closing the last window, the visual acceptance test and performance
measurements remain open.

Validation of the PostgreSQL constraints batch: `make qualite` passes with
1,335 tests passed and 20 ignored. Two additional tests were run
on a disposable PostgreSQL 17.11 instance, initialized for this batch then stopped:
composite keys, atypical names, CHECK, uniqueness, foreign keys, NOT NULL and
refusal of limit overruns. The cache/bus tests cover explicit
reading, missing capabilities, non-publication after cancellation and
compatibility of old JSON. The GPUI test checks selection and copy without
modifying or running the draft. Pixel fidelity, frame budgets
and cancelling an introspection already started on the server side are not
proven by these new tests. The absence of nextest and the two Rust
warnings on upstream dependencies remain reported by the gate. The foundation, for its part, no longer
reports anything: `make socle` returns 42/42 compliant cases and 0 warnings.

Validation of the SQLite constraints and validation status batch: `make qualite`
passes with 1,346 tests passed and 21 ignored. The new SQLite tests cover
the embedded engine: atypical names, kept clauses, adjacent constraints
without a comma, absence/empty, virtual tables, limits and unknown
status despite a declared CHECK. The GPUI metadata test goes through the bus and
the SQLite worker and keeps the draft intact. Three PostgreSQL tests are
run separately on a disposable instance then stopped, including the real
transition `NOT VALID` → `VALIDATE CONSTRAINT`. The old-JSON test keeps
an unknown status. Figma `229:7998` was reread for the columns and proportions;
this reading does not prove the pixels of the native rendering. The pre-existing
warnings of the gate remain identical to the previous batch.

Validation of the incoming-relations batch: `make qualite` passes with 1,353 tests
passed and 22 ignored. The SQLite tests cover link direction, composite order,
implicit references, namespace separation, affinity
and collation comparisons, partial/expression indexes and refusal of incomplete or
too large lists. The cache and bus tests check independence of directions,
capabilities and non-publication after cancellation. The GPUI test loads the links,
opens with the keyboard a source that was not in the catalog, receives its
real preview and keeps the draft intact. The PostgreSQL test run on a
disposable instance covers references across schemas, INCLUDE columns and
comparisons that are not established; the three constraint tests were also
rerun during this batch. The instance was stopped after the tests. Native
pixels and frame budgets remain to be checked.

The read-only DDL panel (`193:2433`, taken up in `229:7749`), Copy DDL
and Open DDL in console are wired to the bus (ADR-0018). The copy creates a
distinct console without execution. SQLite provides the stored declarations of
tables/views, indexes and triggers, virtual tables included. PostgreSQL
rebuilds ordinary tables, partitioned roots, views, materialized views
and sequences; owned sequences, constraints, indexes, rules, user
triggers and RLS policies are included. The scope excludes data, privileges,
comments and external dependencies, and its notes remain visible.
Partition children are now rebuilt with `PARTITION OF`,
qualified parent, native bound and local options. Sub-partitions keep
their `PARTITION BY`; constraints, indexes and triggers cloned from the parent are
not recreated twice. The fixtures check the parent link and the bound
after reapplying the DDL. On PostgreSQL 12, a partition carrying
user triggers remains refused for lack of native lineage metadata;
no heuristic comparison decides to omit a trigger.
Temporary, inherited, typed or
foreign PostgreSQL objects, as well as unvalidated NOT NULLs, are still refused: their
wiring remains to be done. Persisting the DDL width across
launches, full native visual alignment and frame measurements also remain
open.

Validation of the DDL batch: `make qualite` passes with 1,365 tests passed and
25 ignored. The GPUI tests check refusal of input/execution in the preview,
the exact copy, the real click opening a distinct console without execution,
cancellation when leaving the DDL and the mouse/keyboard handle without a new
read during resizing. The SQLite tests recreate table, index,
trigger, view and virtual table in temporary databases. Three
PostgreSQL tests run separately on a disposable instance check identity,
serial, generated columns, constraints, indexes, trigger states, RLS, views,
sequences and partitioned roots; the cluster was stopped. The DTO, the
bus publication and eviction of definitions, even invalidated, are tested.
The pre-existing warnings (absence of nextest, upstream dependencies) remain
identical; the foundation, for its part, no longer reports anything. This gate does not replace a
pixel acceptance test.

The scope of Run is now aligned with the console's help: explicit
selection or current statement. The helper `oxyn_query::current_statement`
preserves compound bodies and refuses ambiguous boundaries. The existing
classifier remains unchanged. The stale sidebar text claiming that
history was unavailable was removed.

Important flows still to be wired, checked in the code:

| Flow | Remaining work |
|---|---|
| Home: native acceptance test | The overlap is fixed and tested on the rendered rectangles; observing it on screen, at several widths and in both themes, remains to be done |
| Table preview | Done, according to [ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md): `PreviewSort` and `PreviewFilter` are in the command, the capabilities `PREVIEW_SORT` and `PREVIEW_FILTER` are declared, both drivers compose the quoted translation, and the next page exists when the order is deterministic. A pending decision remains on the default sort — see the open questions at the end of the document |
| Session recovery | The shutdown marker is in place ([ADR-0021](adr/0021-marqueur-d-arret.md)): a `⌘Q` no longer triggers recovery, a session without a close whose heartbeat has aged does. Restoring the object location that [UX-SPEC](UX-SPEC.md#restoring-after-an-abrupt-stop) promises is held again in `apps/desktop` since 2026-09-25 (`ObjectLocation`, read by `read_object_location`) — see "Recovery and library" |
| Cross-workspace library | Done: the filters carry the historical connections that were deleted or are outside the current workspace, through `Command::ListHistoryConnections` |
| AI workspace | The [I-04](../CLAUDE.md#i-04) leak is closed: `ToolOutcome::Failed` carries a report with private fields whose only constructor requires the connection's tier, and the filter applies **at construction** — under `Local`/`Metadata`, the server's message never enters the structure. `PrivacyTier` now lives on the `ConnectionConfig`, as its documentation already claimed. Provider configuration, the conditional `Ask AI` entry (`190:1549`), proposals through the bus and persistent provenance of documents are done, and `oxyn-ai` is a declared dependency of `oxyn-app` — of `oxyn-desktop` since the removal of GPUI |
| Product acceptance test | Full native rendering, accessibility and performance measurements, distinct from GPU-less GPUI tests — today Storybook stories, which do not replace the acceptance test either |

Validation of the automatic-refresh batch: `make qualite` passes with
1,536 tests passed, no failure and 39 ignored. The views reread themselves after
a successful execution, according to [ADR-0022](adr/0022-rafraichissement-automatique.md):
a DDL rereads the explorer, a DDL or a write rereads the **visible** preview while
keeping the applied predicate, sort and page, and the open library
follows. Nothing after an error, nothing on another connection, nothing on a hidden
tab, and never the user's statement — it is the preview read
Oxyn composes that is re-emitted. Invalidating the catalog cache after DDL
and publishing `CatalogUpdated` already existed on the executor side; what
was missing was the intent in `Event::Completed` and the subscribers.

Two findings of this batch are worth keeping. The first is a pre-existing defect
of the **manual** path: `refresh_catalog` ignored a request with the same
scope arriving during an in-flight read, so that a DDL run during a
`Refresh` left the tree stale indefinitely. The second concerns the
library: a `reload()` triggered behind the user's back would go back to
the first page and throw away the open detail, so the automatic reread
abstains as soon as an entry is selected, a retained result is displayed
or the first page has been left. Coalescing is a per-view flag, cleared
when a read starts and consumed when it arrives, and only if that arrival
brought back rows — otherwise a reread would cover the error. It is
checked by mutation.

What this batch does not prove: the `RecvError::Lagged` branch itself, whose
reliable provocation would amount to manufacturing the failure rather than observing it;
only the catch-up function is tested directly. And the window between the
start of a read and its arrival at the server remains open — a write
dispatched just before can apply just after, and nothing requests it again.
This is inherent to a refresh without a shared clock; the ADR does not promise
a snapshot.

Validation of the session-context batch: `make qualite` passes with 1,450 tests
passed, no failure and 37 ignored. The selector of `191:2003` is wired,
conditional on the `SESSION_CONTEXT` capability: on an engine that does not declare it
— SQLite — it is **absent**, not greyed out, and a test anchors it. Its five states
are distinct, including the initial one, which says that the server placed the session
rather than naming a schema Oxyn did not ask for. Nothing there is optimistic:
the display shows what the session reports.

Two facts changed the design along the way and are recorded in
[ADR-0019](adr/0019-contexte-de-session.md). First, `oxyn-core` cannot
name `CatalogPath`: the command carries two `Option<String>`, like
`PreviewRelation`. Then, a PostgreSQL session is a pool of four
connections and `search_path` is a **per-connection** state: the `SET` is therefore
set on each execution, on the connection that execution borrows, and
undone before it goes back to the pool. This second half was not planned
and comes from a review: `pg_get_indexdef`, `pg_get_constraintdef`,
`pg_get_expr` and `format_type` render their text **relative to the
`search_path`**, so that the same object could have been described differently
from one read to the next. An error path that cannot undo the `SET`
closes the connection rather than giving it back.

The PostgreSQL tests on a disposable instance cover consistency across the
pool — four concurrent cursors, four distinct pids read by
`pg_backend_pid()` in the query itself —, the return to the default, the refusal
of an absent schema and of another database, two schemas actually named
`oxyn_ctx"weird` and `oxyn_ctx.dotted`, preserved read-only mode, and the fact
that the server itself attests, through `pg_stat_activity.query`, that the submitted
text was not rewritten. The introspection test forces borrowing a
recycled connection by pinning three cursors on a pool of four. The
cluster was stopped after each campaign. Proof by failure — disabling
the reset to the default to see the test turn red — was not done; the
sensitivity of the renderings to `search_path` was checked separately in `psql` on
17.11 and the table of both forms is in the test's comment.

Validation of the home / Explain / bound values batch: `make qualite` passed with
1,431 tests passed, no failure and 27 ignored.

The overlap reported on September 10 had two distinct causes, both
fixed. The home actions were drawn by `oxyn-app` as
`absolute()` layers placed 20 px from the top, exactly on the form's 56 px
header; they now belong to its title bar, which shares a
single row between the brand and the actions. The form also drew two
brands — `logo(theme.mode)` and `icon(IconName::Logo)` — whereas the migration
recorded in `assets/ui/README.md` planned the removal of the second; the
`IconName::Logo` variant had no other caller and was deleted. The
test checks the rendered rectangles: the brand and each action do not overlap,
and the actions stay on the right. An action without a purpose is absent rather
than displayed inert. The guard of `ConnectionForm::on_key` now applies what
its comment announced, which makes the bar reachable by keyboard.

`ParameterEditor` existed without being instantiated anywhere. It now
belongs to each `QueryConsole`: values isolated per console, ephemeral,
passed through `ExecRequest::params` and never inserted into the SQL text. The
parsing moved down into `oxyn-core`, the only place where `ParameterType` now
exists; UUIDs, dates, times, timestamps and JSON are actually built,
and the accepted formats were established by tests before being documented.
A timestamp without an offset is refused rather than silently interpreted as UTC.
Decimal validation no longer goes through `f64`, which accepted `1e400`.
The tests cover a hostile value actually bound on an SQLite session,
the independence of two consoles, and a non-convertible value that stops
submission without the message repeating the input.

The bar follows group `273:37036`: Run, Stop and Explain are three
controls, the one with nothing to do being dimmed **and** inert, checked by
a test that clicks both. The `Parameters` control follows the 144 px of
`191:1993`.

The leak found by the review is fixed at the driver boundary. When a
statement carries at least one bound value from the caller, the server's primary
message is no longer propagated: PostgreSQL returns a protected message followed by its
SQLSTATE, SQLite the extended result code and its canonical label. The original
error is not kept in the type, so that no `Debug` can
re-expose it. The error class, cancellation and the refusal to replay an ambiguous
error are preserved, and a test checks it. Two PostgreSQL tests run
on the disposable instance — an invalid cast of `$1` and a plpgsql trigger that
copies the value — then the cluster stopped. The symmetrical test **without** a bound
value shows that the engine's message arrives whole: it is what proves that the
first one tests something. The message of the `sqlx` encoder is no longer
interpolated in `bind_params`. PostgreSQL transport errors keep
their text: `sqlx` composes them without pouring the arguments into them, and holding them back
would cost a diagnosis without protecting anything.

These proofs are not a pixel acceptance test: the native rendering of the fixed
home screen and of the bar was not observed on screen, and frame budgets
remain unmeasured. The pre-existing warnings of the gate (absence of
nextest, two upstream dependencies) are unchanged; the foundation no longer reports anything.

Validation of the partitions/current-statement batch: `make qualite` passes with
1,377 tests passed and 25 ignored. The resolution tests cover UTF-8,
dollar-quotes, comments, separators, incomplete SQL and trigger bodies.
The review detected and had fixed a false closing on an SQLite column
named END: the closing must be at a statement boundary, and
the CREATE TRIGGER prefix is strict. The GPUI test checks a review covering
the whole trigger, never its inner UPDATE, and that the following INSERT
is not executed. Another test shows that neighboring writes only leave
after explicit selection of the script. Three real PostgreSQL fixtures were
rerun for partitions, RLS and other definitions; the disposable
cluster was stopped. The PG12 metadata fallback is checked structurally,
but was not run on a PG12 server. The native and
performance reservations as well as the pre-existing warnings of the gate remain
open.

Documentation deviations found: the phasing of §11 of ARCHITECTURE differs from
that of this document, which is authoritative on the order of phases — §11
points to it since 2026-09-25;
the stale wording of PERFORMANCE claiming the absence of code was
corrected. The budgets remain to be measured.

Validation of the object-restoration and library-filters batch:
`make qualite` passes. Restoration reopens the object tab, its selected
path and its sub-tab without overwriting a draft. `ListHistoryConnections`
returns the page of connections **as the history recorded them**, which
allows filtering by a connection deleted since or belonging to another
workspace; each entry says whether the workspace still owns it. The menu
field carries a `ConnectionId` and not a rank, because the list grows
when the page arrives — a rank would have designated another connection. A test
goes through the view with the keyboard and checks **zero execution events**: filtering
opens no session. `deny_unknown_fields` was removed from
`WorkspacePreferences` and from `ObjectLocation`: an earlier version of Oxyn
refused a file written by a more recent version and failed at
startup, which is recorded in ADR-0013.

Validation of the AI integration batch: `make qualite` passes with **1,635 tests
passed**, output checked. The batch is described by
[ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md).

State found before the batch, which justifies its scale: `oxyn-ai` (4,480 lines)
and `oxyn-llm` (5,988 lines) were complete and tested, and **no crate
declared them as a dependency**. `oxyn-exec/src/sink.rs` exposed the `ExecutorSink`,
saying explicitly that the translation was up to `oxyn-app`; it was not
written there.

What is held, and by what:

- **the AI workspace only exists when configured** — the `Ask AI` entry and the privacy
  badge are absent as long as no provider is declared, and a
  test goes through the four layers (the screen emits, the workspace translates into a
  `Command`, the store writes, the reclassified read comes back, the entry appears
  without a restart). It turns red as soon as a link breaks, checked by sabotage;
- **the local/remote classification is never persisted**: `Reach` has no
  column and is recomputed at each opening, on the blocking pool. A DNS
  answer from yesterday applied to a send today is the proxy trap that
  AI-PROVIDERS asks to avoid;
- **the user / model asymmetry is carried by the type**: the conversation
  observer receives the whole facts — server message included — while
  the prompt only receives what `ToolOutcome::from_dispatch` filtered under
  the connection's tier. The gap is **observed by comparing the two
  values**, never by replaying the filter's rule: a copied rule diverges
  silently the day the original changes;
- **provenance is written end to end**, with a single rule in the
  repository: an absent provenance means "nothing new to write", never
  "nobody". An ordinary autosave therefore does not erase it. Both directions
  were checked by sabotage.

Three defects found during the batch, outside the plan:

1. **`controls.rs` claimed something false on a blocking accessibility point** —
   that GPUI would activate a focused control on Enter and Space. A probe
   refutes it, with a witness that counts the keys *received*: without it, zero
   activations would read just as well as "GPUI does not activate" as
   "the keys never arrived". Three buttons of the conversation panel
   were unusable by keyboard because of it. The comment is corrected
   and the fact pinned by a test, which would also turn red if GPUI started to activate
   on Enter — that would then be a security regression, a production
   write approval button never being activatable with the
   Enter key ([I-02](../CLAUDE.md#i-02));
2. **a classification reduced to a boolean** crushed the distinction between
   "remote" and "unresolved", which the configuration screen needs — it
   says "unresolved" rather than asserting a measurement that did not take place. Leaving
   it would have required a second DNS resolution campaign and a second
   place where the two answers can diverge;
3. **a language divergence**: the whole interface is in English, `format_settings`
   was the only exception. Settled for English, both screens follow,
   and `Reach::as_str` stops returning French in source code.

Two independent reviews followed — invariants and security —, each
instructed to check the claims against the code rather than
believe the comments. They converged on a missing link and found
five defects the batch had not seen. All are fixed, and `make qualite`
passes again with **1,640 tests passed**.

1. **The privacy tier was not persisted.** `ConnectionConfig`
   carried it from the start; the `connections` table did not have the column. Every
   reread connection therefore started again at `Metadata`, which made `Local`
   unreachable from one session to the next: a setting chosen on a customer database
   was lost on closing, **without a message**, and the next `Ask AI`
   sent the DDL and column names to a remote provider. Migration
   8, with two distinct fallbacks that are the decision: column **absent** →
   ADR-0006 default, because the row was written by a binary that ignored
   this setting and the user therefore never chose one; value
   **unreadable** → `Local`, the most restrictive, because a setting whose
   meaning was lost does not get the benefit of the doubt. Both directions are
   tested and checked by sabotage.
2. **Provenance was emitted then thrown away.** Everything existed — the column, the
   `coalesce`, the type, the event — and no application path ever set
   anything other than `None`. The repository therefore formally asserted a
   falsehood on exactly the line ADR-0023 exists to mark. The
   link is laid: `AiEvent::Started` → `Assistant::provenance` →
   `OpenQuery::Copy` → the console → its two writes. A history copy,
   for its part, inherits no mark — marking in excess would pass off as written
   by an agent a text the user had written themselves.
3. **The only type carrying the unfiltered server message derived `Debug`.**
   `DispatchOutcome::Failed` carries `Key (email)=(dupont@example.com)` by
   construction: that is its purpose, and the user has the right to read it. But
   a derived `Debug` made it copyable by a `tracing::debug!` added
   later to diagnose something else — the exact leak mode that the
   checkable corollary of I-03 names. `Debug` written by hand: the class and the
   **length** of the message, never the message. The adapter's translation
   fallback likewise logged a whole report; it now only logs
   the command identifier.
4. **A provider write was not awaited on closing.** A ⌘Q
   within a second of a "Save" recorded a **clean** shutdown over
   lost work: on restart, no provider, no entry, an orphan key
   in the keychain, and nothing anywhere to explain it. Both commands
   join the awaited local writes.
5. **An unreadable provenance made the document impossible to open.** Adding a
   provider family requires no migration: a more recent Oxyn can
   write, on a schema this one accepts, a mark it cannot read
   — the trap `deny_unknown_fields` had already set for preferences
   (ADR-0013). It is now set aside with a loud warning, and **nothing is destroyed**:
   the value stays in the database, protected by the `coalesce`, and a binary that can
   read it will find it again. A test checks it on the raw column.
6. **The identity of a declaration was minted twice** — once for the
   keychain reference, once in the factory — and reconciled by an
   assignment line no test guarded. Deleting it compiled, passed
   the gate, and produced a declaration whose key could not be found,
   reported at the first message and long after a registration announced as
   successful. It is minted once and passed as an argument.

What the two reviews looked at and found sound, and which is worth
recording: an agent's execution path has no second API
(I-01, I-07); the single context gateway cannot be bypassed and
the observer cannot close the loop back to a prompt (I-04); keychain,
DNS and SQLite all go through the blocking pool (I-05); three
independent barriers prevent an agent from escalating — refusal of actor impersonation before
the scheduler, policy refusal on `production` and on declaring a
provider, a tool perimeter that lets it choose neither the connection nor the
endpoint (I-02); no `unsafe` in the eight crates.

Reservations of this batch, explicitly open: the choice of provider when
several are declared follows the first usable one under the tier, for lack of a
specification — a selector is a product decision, and ADR-0023 states in the
present tense that the user chooses, which the code does not do yet;
unavailability on a session without `Capabilities::SQL` is explained rather
than hidden, by analogy with ADR-0003, but no document names it; no
assistant icon exists in `assets/ui` and the glyph used is borrowed;
the privacy tier is now persisted and set in the
connection form
([AI-PROVIDERS](AI-PROVIDERS.md#setting-the-tier)). The native and performance
reservations remain open.

Accessibility validation — keyboard batch: a census of the repository's controls
showed that **six** buttons were reachable with Tab and inert, all for the
same reason. `oxyn_ui::control` sets `tab_stop`, hence reachability, and nothing
more; a comment in `controls.rs` claimed that GPUI activated by itself
on Enter and Space, which is false — a probe refutes it, with a witness that
counts the keys **received** so that the test cannot pass for lack of
keys rather than for lack of activation.

The most troublesome of the six is the **Cancel button of a running execution**.
UX-SPEC states that "the *running* state always carries a way to cancel"; that
way required a mouse. Two things had made it invisible: its
comment described the intent and not what was done, and it had no
`debug_selector` — so no test could reach it, neither for its existence
nor for its keyboard. The other five: the three buttons of the conversation
panel, `Add` in the parameter editor, and the segments of the number
format.

**Reachability audit, run on the whole repository**: each file carrying an
`on_click` was compared with its number of `tab_index`. Eight files have
fewer — and none is a defect, checked one by one. They all follow
the same model, which is the right one: a **single-focus** component with internal
navigation. The connection form handles `enter`, `escape` and movements
in its own `on_key`; the results grid carries seven navigation
keys and its own focus — putting a `tab_index` on each row would create
thousands of tab stops, which would make tabbing unusable
rather than the opposite.

What this audit establishes, and which was not a given: the six fixed buttons
were not the visible part of a general problem. They were the only six,
and they shared a single cause — a false comment.

Recurrence is closed by `oxyn_ui::activable`, declared **next to** `control`
and never inside it. The reason is written on the spot: the approval dialog
of a production write must not be approvable with the Enter key
([I-02](../CLAUDE.md#i-02)), and its test
`production_focus_stays_inside_review_and_enter_never_approves` holds that.
Activation is therefore declared view by view; what is not declared is not
activatable. Only one helper exists in the repository.

Performance measurements — first campaign recorded here, `cargo bench -p
oxyn-query --bench analysis`, 100 samples:

| Path | 1 statement | 10 | 50 |
|---|---|---|---|
| `split` | 1.12 µs | 11.2 µs | 55.1 µs |
| `classify` | 52.0 µs | 516 µs | **2.60 ms** |
| `words` | 1.59 µs | 14.6 µs | 66.5 µs |
| `format` | 2.23 µs | 21.8 µs | 108 µs |
| `current_statement_at_end` | 1.15 µs | 11.4 µs | 55.6 µs |

What these figures say, and which was not a given: `classify` is the only one to
stand out from the noise — 2.6 ms over fifty statements, a third of the 8 ms frame
budget. It is **not** on the typing path: `QueryConsole` calls it
on submission and says so (`console.rs:386`). What runs per frame is
`current_statement`, at 55.6 µs for the same document — two orders of magnitude
below the budget. The day someone moved `classify` into rendering, these
two lines say what it would cost.

Native acceptance test — run on 2026-09-11, real application, without touching a
real database or taking the machine's focus. `--temporary-workspace` opens an
in-memory store; `open -g` launches the bundle without activating the application.

| What was observed | Result |
|---|---|
| Opening the window | `Oxyn · Temporary workspace`, 1280 × 852 |
| Migrations applied on a fresh store | **all 8**, `initial` → `connection_privacy_tier` |
| Driver registry | 2 drivers, 0 saved connections |
| Warnings or errors at startup | **none**, over three launches |
| Widths tried | 1440, 1024, 900, 760 — the window follows and the process survives all four |
| Closing | the cross terminates the process, with no residue |

**Cold start is measurable, contrary to what this document
claimed**: it is enough to timestamp between the launch of the process and the
`window ready` of the log. Three consecutive launches, `dev` profile:
**274 ms, 249 ms, 235 ms**, for a budget of **1 s**. The margin is comfortable,
and the measurement is reproducible without Instruments.

**A capture of the home screen was indeed obtained**, in the dark theme, and it establishes
what phase 4 asked for first: the brand "Oxyn / Personal workspace"
at the top left and the action "Saved working copies" on the right **do not
overlap** — the defect reported on 2026-09-10 is visually fixed.
The capture also shows the list of drivers, the empty state "Your first connection
starts here", the keyboard help and the status bar.

The path to obtain it is worth noting, because it is not the one you
think: `screencapture -l<windowid>` has disappeared from recent macOS, and the remaining window
modes are interactive. The window must therefore be raised with
`AXRaise` — which puts it in front **without** giving it keyboard focus — then
captured by region. A first attempt without raising the window photographed
the user's screen and not Oxyn; the image was destroyed. A second one
captured a dialog box of a third-party application; it was destroyed and
recropped.

**The light theme was captured too.** It comes from the persisted preferences,
which a temporary workspace does not carry; the way is a temporary `$HOME`, which
`directories` follows — Oxyn creates a disposable store there, where `appearance: "light"`
is written before relaunching. The user's store is not touched, checked by its
modification date.

**The comparison with the boards was done**, by fetching the images from the Figma
server and comparing them with the captures. It found two real deviations:

* the privacy badge was in capitals — `METADATA · CLOUD` — where the
  survey `190:1163` writes **`Metadata · Cloud`**. Capitals are reserved for
  environment marking, where they carry the alert. Fixed;
* the mockup's grid footer announces `200 rows loaded · 284 ms · Total
  count not requested`; the code does not write this last mention. It says
  something useful — that the total was **not** requested, hence that the
  number displayed is not the table's. Not implemented, recorded.

**The console board `191:1521` was compared in the same way**, and it
says something useful: the bar there is compliant — `Run ⌘↵`, `Stop`,
`Explain`, `Parameters · 3`, the selector `commerce-prod / public` — and the mockup's
**result area** has since been caught up on all its elements
but one:

| Mockup element | State |
|---|---|
| `Messages` tab next to `Result 1 · 10 rows` | absent — blocked **upstream**, see below |
| `Explain plan` tab next to `Result 1 · 10 rows` | **done** |
| `Find in loaded results…` field | **done** |
| `Read-only console` mention near `Parameters` | **done** |
| Footer `10 rows received · Display timezone UTC · Results belong to this execution` | **done** |

The absence of `Messages` is not a naming divergence — checked, it
does not exist under any other name. It is a feature not implemented for an
upstream reason, described below. The console is therefore compliant on its toolbar, and
on its result area with this single reservation.
The mention "Results belong to this execution" is
the one that matters most: it says that what is displayed belongs to **this**
execution and not to the previous one, which is exactly the kind of ambiguity
UX-SPEC tries to close elsewhere.

**The other boards named by the plan were compared in the same way.**
The result is constant: the **structures** are compliant, the **secondary
features** are missing.

| Board | Compliant | Absent from the code |
|---|---|---|
| Preview `190:1163` | bar, inspector (`Record · …`, `Inspect full value`, `Hide inspector`), filter `WHERE … Apply … Sort` | mention `Total count not requested` |
| Console `191:1521` | `Run ⌘↵`, `Stop`, `Explain`, `Parameters · 3`, context selector, `Read-only console`, full footer including time zone, **`Find in loaded results…`** | `Messages` tab (blocked upstream, see above) |
| Constraints / DDL / Indexes `229:7637` | `Refresh structure`, `Copy DDL`, `DDL · Read only`, `Open DDL in console`, `Validated` status, **sections `NOT NULL columns` and `Unique indexes`**, and — since then — **`Propose change…`** ([ADR-0025](adr/0025-proposition-de-changement-de-schema.md)) | — |
| Relations `229:32690` | `Incoming relationships`, the structure bar, **`Selected relationship`**, and — since then — **`Bounded related-row preview`** and **`Review related-row query`**: the query is composed bounded, quoted by the driver ([I-10](../CLAUDE.md#i-10)), and opened in the console **without being run** | — |
| Recovery `232:9100` | `Ask AI` and the badge carry `hidden` — the conditional entry is indeed the one the mockup prescribes | — |
| Connection bar `190:1543` | 48 px, badge 136 px, `Ask AI` 96 px at x = 1188, in order | — |

The **seven object sub-tabs** are implemented — `Data`, `Structure`,
`Indexes`, `Constraints`, `Relations`, `Ddl` — plus `IncomingRelations`, which the
mockup does not show on this board but which `229:32690` covers.

**Three of these absences are closed right away**, because they are not
cosmetic — each closes an ambiguity nothing else closed:

* the preview footer now announces `Total count not requested`. A preview of
  200 rows on a table containing fifty million looked in
  every respect like a preview of 200 rows on a table containing 200;
* the result footer carries `Results belong to this execution`. A console
  keeps its previous result displayed while a new execution runs,
  and nothing said which execution the rows being read came from;
* the console bar displays `Read-only console` when the connection is. Without
  it, the user writes their `UPDATE` and discovers the refusal at execution.

**A fourth followed on 2026-09-12: the display time zone.** It was not
only an absence from the mockup — [UX-SPEC](UX-SPEC.md) already promised it,
listing "the time zone" among the secondary information of the status bar.
It was therefore a code/documentation divergence, not a mere gap.

The label is not hard-coded: `oxyn_data::timestamp_display` **deduces it from the
schema**, because Oxyn converts no timestamp. Checked against a real
database in `integration.rs`: the PostgreSQL driver returns `timestamptz` as
`Timestamp(µs, Some("UTC"))` and `timestamp` as `Timestamp(µs, None)`. Hence three
cases, and the third is the one that matters:

* a single declared zone → `Display timezone UTC`, the common case;
* several zones → `Display timezone varies by column`, because naming one
  would describe the other columns wrongly;
* no dated column → **nothing**. Writing "UTC" by default would assert something
  about the content, and a `timestamp without time zone` carries no time zone:
  announcing one would invent information the server did not send.
  This is what `a_timestamp_without_time_zone_does_not_announce_one` holds.

The time zone travels in the `ExecutionStatus::Completed` variant, not next to it:
stored in the bar, it would survive the next result and describe rows
that are no longer on screen.

`format_stats` was translated on this occasion — it returned "2 lignes en 30 ms
(serveur 8 ms)" in an English interface.

**It was not the last one**, contrary to what this paragraph claimed:
a divergence survey of 2026-09-14 found **sixteen others**, and among
them the two states that [UX-SPEC](UX-SPEC.md#states-of-a-view) asks to care for
the most — the **empty** state of the grid ("Aucune ligne") and its **error** state
("L'erreur est transitoire…"). Added to them were the initial state, the running state,
cancellation, the grid footer, the cancellation label, the editor's execution
mention, and — the most troublesome — the **approval** screen: `actor_label`
returned "Vous demandez" / "Un agent demande" on the very surface that holds
[I-02](../CLAUDE.md#i-02), plus "Nombre de lignes touchées inconnu.".

The environment labels of the connection selector were half translated
("développement", "préproduction" next to "local" and "production"), which
is worse than a clean translation: it is the marking that decides the
production confirmation.

All translated. The lesson is worth more than the batch: **declaring a work item closed without
an exhaustive sweep reopens it**, and it is this very paragraph that had done so.

### The four remaining surfaces, and what each one requires deciding

They are not missing labels. Each one runs into a question to
settle **before** writing, and settling it lightly would produce exactly what
this campaign spent its day fixing.

**`Propose change…`** (`229:7663`) — **the requested ADR is written:
[ADR-0025](adr/0025-proposition-de-changement-de-schema.md).** This batch was
classified "product decision pending". The comparison with the board on
2026-09-13 showed that this classification was wrong: the mockup had already
decided, and nobody had gone to read its sentence.

The definition panel `229:7749` carries, between the DDL and `Open DDL in console`,
the mention "**Changes require a SQL review naming commerce-prod before
execution**". That is word for word [I-02](../CLAUDE.md#i-02) — a review that names
the connection, before execution. There was therefore no need to arbitrate between
"apply" and "propose": the control proposes, and execution remains a
separate, reviewed gesture, exactly as `Open DDL in console` already does.

The ADR chains existing decisions rather than inventing new ones: the path is
`open_library_query` with `OpenQuery::Copy`, hence **no new command** and
[I-01](../CLAUDE.md#i-01) held by construction; identifiers are quoted by
`quote_identifier` ([I-10](../CLAUDE.md#i-10)) and expressions taken
verbatim from the catalog; the gesture is **unavailable** for an
`Actor::Agent`, not "confirmable" — I-02 names a stronger confirmation as
insufficient. Provenance stays `None`, as for the bound-query template: a
skeleton composed by Oxyn is written neither by the user nor by an agent, and
provenance marks **who wrote**. A first draft of this paragraph
said the opposite; ADR-0025 and the code are authoritative.

**Implemented on 2026-09-14** — `workspace/propose.rs`, eight tests.

The chosen form is that of its twin `related_row_query`: a **template to
complete**, not a statement to run. Oxyn provides what it knows — the
relation, the selected column, their correct quoting — and leaves to
the user what only they know, the new name or the new
expression. Reusing the existing pattern rather than inventing a form
avoided recreating a flow that already existed.

Four guarantees, each held by a test that fails when it is removed:

* **every line is a comment.** Nothing can leave on an absent-minded `Run`,
  which makes the board's sentence literally true;
* **identifiers are quoted**, hostile ones included — sabotage checked: without
  `quote_identifier`, a column named `x"; DROP TABLE audit; --` goes as is
  into the template;
* **the proposed direction is the one that changes the state**: a nullable column is
  offered `SET NOT NULL`, never both. A template that does nothing reads like
  a template that failed;
* **SQLite only gets the rename.** It has neither `ALTER COLUMN` nor
  `DROP CONSTRAINT`; the control disappears rather than producing a text that
  would fail at execution ([ADR-0003](adr/0003-driver-capabilities.md)).

The current default is taken **verbatim** from the catalog: reformatting it would change
its meaning without saying so. And the header names the connection in an **SQL comment**,
because it must survive copy-pasting into a ticket — that is where the
proposal will be reviewed, often by someone else.

**`Explain plan`** (`191:1521`) — **the half that mattered is done.** The
initial finding was right but incomplete: `Explain` existed as an execution
mode, and its result arrived in the grid **without anything saying that
it was a plan**. A user who runs `Explain`, steps away and comes back reads
their grid as data — a misreading the mockup's label
existed precisely to prevent. The result area now carries the
mention and its explanatory sentence.

What remains is the **enriched rendering**: a plan is a tree, with costs and
estimated rows, and the formats differ between PostgreSQL and SQLite.

An earlier version of this paragraph claimed that `EXPLAIN` "returns rows
readable as they are, so the absence of this display loses no
information — only comfort". **That was false, and verification
showed it.** `Metrics::max_column_width` was 480 px and `char_width` 7.2 px: the
`QUERY PLAN` column was fitted to about 66 characters, whereas a PostgreSQL
plan line commonly has more than a hundred. What fell outside the
cell was the tail `(cost=… rows=…)` — the only part one reads a plan to
see. The information was recoverable by widening the column with the mouse
(`set_column_width` only bounds by the minimum), but the default display
lied.

The defect is fixed, and not by a special case: `fit_columns` treated the
cap as an absolute maximum, whereas it says what a column can take
**at the expense of the others**. It therefore no longer applies when the columns
fit together in the window — the case of a single, wide column, of which
`EXPLAIN` is the example. Held by
`une_colonne_seule_depasse_le_plafond_plutot_que_de_couper_le_plan`.

The tree rendering, for its part, remains comfort — and this time the sentence is checked
rather than assumed. PostgreSQL's indentation, which *is* the plan's tree
structure, survives to the screen: no `trim` on the `format_cell` →
`render_cell` path, none either in the text system of `gpui 0.2.2`, whose
`WhiteSpace` only governs line wrapping — there is no CSS-style
whitespace collapsing.

One limit remains, and it is honest to write it: the grid composes in a
proportional font. Indentation is therefore present but not aligned to the
character, and the internal columns of a plan (`cost`, `rows`, `width`) do not read
as columns. This is what the enriched rendering would bring.

**`Messages`** (`191:1521`) — the server's notices (`NOTICE`, `WARNING`,
`VACUUM` warnings). This paragraph said that the driver contract did not
provide a channel for them, and concluded with a `DRIVER-CONTRACT` batch.
**That was true but too optimistic**: the block is not in our contract,
it is one level lower, in the client library. Checked on 2026-09-12
in the sources of `sqlx-postgres 0.9.0`.

`sqlx` **receives** the notices — `BackendMessageFormat::NoticeResponse` is decoded
in `connection/stream.rs` — then **throws them away**: it turns them into a
`tracing`/`log` event on the target `sqlx::postgres::notice`, and nothing else. Its
own comment at that spot says "do we need this to be more configurable?
if you are reading this comment and think so, open an issue". The `Notice` type
is not reachable from outside: `mod message` is private in
`src/lib.rs`, which only re-exports `PgSeverity`. **There is therefore no
subscription API.**

Three ways were open, none free:

1. **Listen to the `tracing` target.** A layer filtering `sqlx::postgres::notice`
   retrieves the text. The problem is **attribution**: the event carries neither
   connection nor session, and it would have to be deduced from the ambient `tracing`
   scope at the moment the stream is polled. That works on a test with a single
   session and starts attributing the notice to the wrong console as soon as there
   are two — a silent defect, which displays a real warning under a
   query that did not produce it. That is worse than displaying nothing.
2. **Push the need upstream to `sqlx`.** The comment invites it. Zero cost
   for us, a delay outside our control.
3. **Switch the PostgreSQL driver to `tokio-postgres`**, which exposes notices
   as asynchronous messages of the connection. `tokio-postgres` is not in
   the graph (checked in `Cargo.lock`): it is a new dependency, hence
   [I-12](../CLAUDE.md#i-12) and a check of GPUI's `=` pins, and
   above all it is rewriting the product's most used driver. **Requires an
   ADR.**

On the SQLite side, there is nothing to do and nothing to regret: without a server, there is
no notice. The capability will be declared absent, and the tab will not exist for
this driver — which [ADR-0003](adr/0003-driver-capabilities.md) already requires.

**Decided on 2026-09-24 (audit, D13): way 2.** The `Messages` tab waits
for `sqlx` to expose notices; the driver does not switch to `tokio-postgres`, and
way 1 stays rejected for the wrong attribution it would produce. The upstream
state — ticket [#3621](https://github.com/transact-rs/sqlx/issues/3621), without
an answer — is tracked and dated in
[RESEARCH-NOTES](RESEARCH-NOTES.md#upstream-follow-ups); the batch reopens when a
version of `sqlx` ships the subscription.

**`Find in loaded results…`** (`273:37024`) — **implemented on 2026-09-14.**
`oxyn-data/src/find.rs` and `workspace/find.rs`, nine tests.

This batch was blocked too long, and for a bad reason: **mine**.
I had read it as a filter, which made it run into "what is exported
is what is displayed". Rereading the board was enough. It writes
**`Find`**, not `Filter`; a `WHERE` filter already exists, elsewhere, on the
preview board; and its grid shows **the ten rows** of the result under the
field, with no match counter or hidden row.

A search that **reveals** removes nothing. The UX-SPEC rule therefore stays
true without touching it, and the export/display decision — still open for
hidden columns — did not govern this batch. It only governed my
reading.

What the code guarantees, each point held by a test:

* **spilled batches are never read.** `find_rows` only looks at the
  resident part ([I-05](../CLAUDE.md#i-05)), and
  `a_spilled_batch_is_counted_and_never_read_from_disk` checks that no
  row of a spilled batch appears in the matches;
* **what was not scanned is said.** The number of skipped batches is displayed in
  the warning color. Without it, "No match" would mean "no
  match in what I cared to read", and the user would conclude
  that their value is not there;
* **`NULL` matches nothing.** Searching for "null" would otherwise find every
  absent value, and whoever searches for a `nullable` column would have no
  way out;
* **the scan does not block.** It goes to the background executor; the field
  searches on validation, because a scan per keystroke would be work
  thrown away at the next keystroke;
* **no row disappears.** `la_recherche_revele_une_ligne_sans_en_retrancher_aucune`
  compares the number of rows before and after: it is the assertion that keeps the batch
  on the side where it does not touch export.

A match is **marked** with an accent marker, it is not colored: an
additional background would change the contrast of the text on top of it, which the themes
test holds. A new result clears the matches — `None` there means
"no search", never "no match".

**What is not proven, and why it is written here.** The mockup gives 300 px
to the field. At a 760 px window, a fixed 300 px pushes the summary — hence
the warning about unscanned batches — out of the frame. The field is therefore
bounded (`flex_1` + `max_w`) and the row wraps.

This fix **has no test**, and that is deliberate. A first test was
written, then removed: it passed just as well with a fixed 300 px as with the bound,
because the text metrics of the GPUI harness are deterministic *and* wrong — the
label there is narrower than on screen, and the overflow therefore never
happens. This is exactly the trap that [tests.md](../.claude/rules/tests.md) names:
"never assert a dimension that depends on the width of a text". A test
that cannot fail proves nothing.

This point therefore remains **to be seen on screen**, and it joins the pending native
acceptance test. That one cannot be run for now: the user asked
on 2026-09-10 that we stop handling their windows, and this request has not been
lifted.

**What was done in the meantime, and why it is not settling.** Leaving
the screen silent while the question stays open means leaving the leak. The
export bar therefore carries a reservation — `2 hidden columns will still be
written` — which is **true under both readings** and closes neither: under
the first it will disappear with the defect, under the second it *is* the
answer. It is the form UX-SPEC already uses elsewhere: keep the gesture,
remove the promise.

The count is taken on a visibility change (`GridEvent::ColumnsChanged`) and
not on the arrival of the result, because the user hides **then** exports.
Held by `une_colonne_masquee_part_quand_meme_a_lexport_et_la_barre_lannonce`,
which actually exports and checks that the hidden column is in the CSV: a
reservation displayed without telling the truth would be worthless, and the day
someone makes export follow visibility, this test will fail and force
removing the reservation at the same time.

### What two independent reviews found, on 2026-09-14

The day's work was reviewed by an **invariants** agent and a
**security** agent, each on only the files written or modified that day. Both
found the **same serious defect**, which none of the tests written at the same time
as the code saw. It is the best argument for independent review
one could produce: the code and its tests came from the same hand, and they
therefore shared the same false assumption.

**Serious — a line break escaped the SQL comment** (`propose.rs`). `--` only
comments until the next line break. `quote_identifier` protects against
escape by quote, not against this one: it doubles the closing quote and
leaves the `\n` intact. Now `CatalogPath` refuses control characters — so
the relation name is safe — but **`Field::new` and `Constraint::new` validate
nothing**, and a multi-line default expression (`DEFAULT 'a` + break + `b'::text`)
is commonplace, with no adversary at all. An escaped line turned a template
announced as "Nothing has been executed" into a statement that `Run` executes.

The test `aucune_instruction_nest_active…` did check the right property —
"every line starts with `--`" — but on a single-line fixture. The
tested guarantee was narrower than the announced guarantee.

Fixed **structurally**: no `--` is written in the `format!`s anymore.
The body is composed bare, then `en_commentaire` prefixes **each physical line**.
The guarantee no longer depends on what the identifiers contain. Held by
`un_saut_de_ligne_dans_un_nom_ne_sort_pas_du_commentaire`, which fails on the
earlier code.

**Five other findings, all fixed:**

* **the export reservation was silent in the object preview.** Its columns can be
  hidden by the same panel and exported by the same path, but only
  the console's export was wired: hiding `email` on the `Data` tab then
  exporting wrote the column without saying anything. It was the leak I thought
  I had closed, still open right next to it;
* **the reservation survived the result it described** — "2 hidden columns"
  displayed on a result where nothing is hidden. A reservation that can be
  false stops being a reservation, and a warning taken for noise is
  ignored the day it is true. `ResultExport::reset` resets the count to zero;
* **a search could apply to the next result.** The generation only
  captured a *new search*, never a *new result*: a scan
  started on A and returned after B highlighted in B rows found in A. The
  return now compares the **identity of the scanned buffer**;
* **the match index was not bounded.** On a narrow column and
  a poorly selective needle, the `Vec` of indices could exceed the budget that
  everything else respects. Capped at `MATCH_LIMIT`, and **admitted** in the same way
  as skipped batches. Cloning this vector on the interface thread on each
  keypress is gone;
* **the search only saw the first 512 characters** of a value, without
  saying so. An identifier further into a `jsonb` gave "no
  match". It now covers the whole value;
* **the time zone name coming from the server** was rendered without a bound, in the middle of the
  sentence that ends with "Results belong to this execution". Filtered and
  bounded to 64 characters.

**And a reachability defect**, outside the thirteen invariants: "previous
match" existed for **no gesture**. The code listened to `FieldEvent::Next`,
which `TextField` only emits on **Tab**, and only if it was built with
`with_managed_tab_order()` — which is not the case here. The arm was dead. The
two buttons that the mockup places next to the field (`273:37076`) replace it.

Last point, raised as a doubt and handled: `proposed_change_text()` was
called **at render time** to decide whether the button exists, hence composed the whole
template on each frame to throw away the result. The decision moved down into
`peut_proposer`, and `le_bouton_existe_exactement_quand_un_modele_existe` sweeps the
tab × index × dialect matrix so that the two cannot diverge.

### External agents: what is written, and the only remaining batch

[ADR-0026](adr/0026-agents-externes-acp.md), opened by the user on
2026-09-14 ("look at […] two modes, one with an API and the other external
agents"). **The back end is complete and proven; only the
interface is missing.**

> **State as of 2026-09-23.** The table and the two paragraphs that follow it
> describe the GPUI interface of 2026-09-14: `provider_settings.rs`,
> `row_display` and their **windowless** tests were removed with it on
> 2026-09-18 ([Migration to the Tauri interface](#migration-to-the-tauri-interface)).
> The interface now exists in `apps/desktop`: a common list of
> providers and agents, Claude Code and Codex presets detected when
> the screen opens, confinement ([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)),
> the "Who answers" selector in the panel. The behavior is authoritative in
> [UX-SPEC](UX-SPEC.md#provider-and-agent-configuration), not here.

| Layer | State | What holds it |
|---|---|---|
| Declaration | done | `oxyn_core::ExternalAgentConfig` — **no secret field**, manual `Debug` that only renders the number of environment variables |
| Persistence | done | migration 9, table `external_agents`. `the_table_has_no_secret_column` reads the **schema**, not the documentation |
| Bus | done | three commands, **refused to an `Actor::Agent`** — `an_agent_does_not_declare_an_external_agent` |
| Privacy | done | `Local` closed, and refused **before launch** — `the_local_tier_refuses_before_even_launching_the_process` |
| Permissions | done | default refusal of everything that touches the machine; `option_for` never authorizes "always" |
| Launch | done | command and arguments **never glued back** into a string — `the_command_and_its_arguments_are_never_glued_back` |
| Conversation turn | done | `run_turn`, fragments passed up by the existing `AgentObserver` |
| Screen row | done | `provider_settings::row::row_display` — the computation goes out, the view draws; 5 **windowless** tests |
| List and removal | done | both kinds in **one** list, one cursor, one confirmation; `le_rang_dun_retrait_designe_la_bonne_liste` |
| Application-side loading | done | `Backend::external_agents`, `reload_external_agents`, `refresh_agent_settings`, `remove_external_agent` |

**The chosen split, and why.** `provider_settings.rs` is 1,386 lines,
and its `DeclaredProvider` carries `kind`, `base_url`, `model`, `key` and `reach`:
none of these five fields makes sense for an agent. Duplicating the list, the row,
the focus and the confirmation would have copied 400 lines of flow, which
[CLAUDE.md](../CLAUDE.md#code-organization) forbids.

The path taken is the one the interface rule prescribes — "the computation goes out, the
view draws". `row_display` returns, for both kinds, **the same six text
fields**; `render_row` therefore exists only once, and is tested **windowless**.
A test checks that no kind leaves a field empty: that would be the first step
towards a `when` in the rendering, and the flow would start diverging again.

Two nuances of vocabulary are held by tests, because they decide
what the user believes:

* an agent does **not** display "key missing" — that would read as a setting
  that is missing, whereas there is no key to configure;
* its destination is announced as **unknowable**, not "unknown":
  the warning is permanent, since no measurement will come to lift it. The
  test refuses that the mention contains "measured".

Both kinds share **one** list, one cursor and one confirmation. The global
rank is translated into a list rank **only once**, in the screen, and the caller
receives a rank that indexes the list it knows. Sabotage checked: translating it wrong
removes one declaration instead of another, without a message — both gestures
succeed.

**Loading is wired**, mirroring that of providers and
deliberately **separate** from it: reading agents carries no reach
classification, so nothing would justify making one wait for the other. A
read that fails **keeps the list** instead of emptying it, for the reason that
already holds for providers — a local failure must not read as an
absence of declaration.

`remove_external_agent` is shorter than its twin, and that is the point: **there
is no key to forget**, the gesture stops at the bus.

**What remained as of 2026-09-14:** the environment field in the declaration
form, and the explicit choice of a second agent rather than the first
declared. The path itself — declare, persist, list, remove, launch,
converse, refuse on `Local`, refuse to an agent — is written and proven
([ADR-0026](adr/0026-agents-externes-acp.md)).

**What remains as of 2026-09-23:**

- ~~the explicit choice of a second agent~~ — done: the panel's "Who answers"
  selector offers each declared provider and agent
  (`features/assistant/availability.ts`);
- ~~the environment field of the manual form~~ — done on 2026-09-23
  (`components/oxyn/provider-form.tsx`, `parseEnvironment`). The same day,
  argument input went back to **one argument per line**, as
  ADR-0026 requires: the port to Tauri had reintroduced splitting on
  spaces with quotes;
- ~~**the "unknowable" nuance is no longer held by a test.**~~ — settled
  on 2026-09-24 for a word specific to the agent, which tells the truth about the cause: an
  agent's destination is unknowable, no measurement will lift it, and it
  counts as remote. The header's reach badge says `Agent-managed`
  for an agent and keeps `Unresolved` for an endpoint **not yet**
  resolved; the stories `ExternalAgent` and `UnresolvedProvider`
  (`assistant-header.stories.tsx`) hold both directions. **Remaining:** the sample
  screen still says "to an unresolved address" when an agent
  receives it — for a pinned sample, it does not know that the recipient is
  an agent.

### Phase-by-phase verification, with the evidence

Closed on **2026-09-14**, quality gate passed, **1,676 tests**. Each
row names a real test output, not a code reading: the right-hand column
can be replayed with `cargo test -p <crate> <pattern>`.

| Phase | State | What establishes it, by name |
|---|---|---|
| **0 — state and split** | held | Authoritative documents reread, Figma boards compared through the MCP server, file ownership distributed without overlap between subagents |
| **1 — sorted, filtered, paginated preview** | held | `PreviewShape { sort, predicate, offset }` in `oxyn-core/src/preview.rs`, carried by the bus and implemented in **both** drivers. 16 tests, including `a_total_order_is_required_by_sorting_as_much_as_by_paging`, `the_user_text_is_not_rewritten`, `preview_reclassifies_driver_sql_and_refuses_writes_for_both_actors`, `preview_sqlite_is_bounded_preserves_hostile_table_and_correlates_events_and_audit`, `preview_enforces_read_only_and_row_limit_even_for_an_incorrect_driver`, `two_consecutive_pages_do_not_overlap` |
| **2 — security and AI** | held | `PrivacyTier` governs `oxyn-ai/src/context.rs` under [I-04](../CLAUDE.md#i-04); `a_tool_call_becomes_a_command_carrying_actor_agent` holds [I-07](../CLAUDE.md#i-07); `declaring_a_provider_reaches_no_database_and_stays_refused_to_an_agent` and `an_empty_secret_reference_is_refused` hold provider configuration; `an_unresolved_endpoint_is_treated_as_remote` keeps the cautious side; `under_sampled_the_server_message_arrives_whole` covers raw server output |
| **3 — recovery and library** | held on 2026-09-14, **in the GPUI interface**: the tests cited here lived in `oxyn-app` and disappeared with `6ecb8ce`; the state in `apps/desktop` is at the [migration gate](#migration-to-the-tauri-interface) | `recovery_opens_only_after_an_abnormal_shutdown` and `l_ecran_de_reprise_annonce_l_arret_anormal_et_seulement_alors` hold the shutdown marker ([ADR-0021](adr/0021-marqueur-d-arret.md)); `returning_to_a_connection_restores_all_of_its_console_entities` restoration; `a_deleted_connection_stays_choosable_in_the_history_filter`, `merging_history_connections_appends_and_marks_without_moving_ranks` and `history_and_recent_results_read_the_same_execution_without_replaying_it` the cross-workspace filters |
| **4 — Figma fidelity and accessibility** | held, **except one surface** | `la_marque_et_les_actions_de_l_accueil_ne_se_superposent_pas` tests the home screen **at four widths × two themes**; `chaque_theme_garde_son_texte_lisible` holds WCAG contrast; `un_controle_focalise_nest_pas_active_par_le_clavier` and `production_focus_stays_inside_review_and_enter_never_approves` hold [I-02](../CLAUDE.md#i-02) on the keyboard. `Messages` remains — see below |
| **5 — final validation** | held | `make qualite` green at each batch, real output cited; budgets measured and dated in [PERFORMANCE](PERFORMANCE.md), "not measured" stated where they are not |

**What prevents marking the `/goal` as complete**, and it is now a single
surface of board `191:1521`, which is not waiting for code but for an upstream
constraint.

* `Messages` waits for upstream: `sqlx-postgres 0.9.0` throws notices into a
  `tracing` event without connection identity, and `Notice` is not exported.
  Waiting rather than rewriting the driver in `tokio-postgres` has been decided since
  2026-09-24 (D13); the upstream state is tracked in
  [RESEARCH-NOTES](RESEARCH-NOTES.md#upstream-follow-ups).

Everything else in the mockup was then implemented and proven — in
the GPUI interface. What the port to `apps/desktop` did not take over is listed
at the [migration gate](#migration-to-the-tauri-interface).

### Requirement-by-requirement verification

Resumed on **2026-09-15**, quality gate passed, **1,719 tests** — sum of the
`test result: ok` lines of `make test`, doctests included. Actual breakdown:
`oxyn` 194, `oxyn-core` 167, `oxyn-ui` 156, `oxyn-llm` 151, `oxyn-query` 136,
`oxyn-store` 124, `oxyn-ai` 100, `oxyn-data` 90, `oxyn-exec` 84,
`oxyn-catalog` 83.

"Held" means: checked by real output, and not by reading the code.
**Two rows are not held**, and they are written as such — a
table whose rows are all green teaches nothing.

| Requirement | State | What establishes it |
|---|---|---|
| **Code** | held | `make qualite`: format, Clippy `-D warnings`, tests, doc, foundation, dated TODOs |
| **Bus** | held | No second execution API. Provider **and agent** declarations are refused to an `Actor::Agent` — `an_agent_does_not_declare_an_external_agent`, whose hostile declaration is `/bin/sh -c "curl … \| sh"`. An `oxyn-ui` test forbids any component from building a `Command`, and a second one checks that **all** sources are covered by this guard |
| **Drivers** | held, with its reservation | 136 tests pass without a server; **34 remain ignored** for lack of PostgreSQL. They passed twice in a row during the 2026-09-11 campaign with a disposable cluster; they are not replayed at every gate |
| **Native UI** | **not held** | The native acceptance test is **forbidden**: the user asked on 2026-09-10 that we stop handling their windows, and the instruction has not been lifted. It is the only possible pixel proof; everything else in the interface is proven without a screen, which does not replace it |
| **Figma** | held, **except `Messages`** | Boards compared through the MCP server; `Propose change…`, `Find in loaded results…`, the display time zone and the hidden-columns reservation implemented since. `Messages` is blocked **upstream**: `sqlx-postgres 0.9.0` throws notices into a `tracing` event without connection identity, `mod message` being private |
| **Security** | held | Two independent reviews on 2026-09-14 found the **same** serious defect — a line break escaping an SQL comment — which no test written at the same time as the code saw; fixed structurally. Six other findings fixed. **RUSTSEC-2026-0285** on `rustls` reported by `cargo deny` and fixed the same day |
| **Accessibility** | held | **32** `tab_index` in the interface, **15** keyboard and focus tests, WCAG contrast of both themes, and `production_focus_stays_inside_review_and_enter_never_approves` which holds [I-02](../CLAUDE.md#i-02) on the keyboard |
| **Performance** | held, with its gaps written down | Startup 235–274 ms / 1 s ✓; cached node 6.70 µs / 50 ms ✓; RSS 82 MiB ✓; frame 0 hitches at idle and while typing ✓; rereading a spilled batch 4.5 µs ✓. Remaining **unmeasured** and stated as such: the RSS of the 256 MB budget, and scrolling a populated grid under instrumentation |
| **Quality** | held | Gate green at each batch, real output cited. An unexplained failure on 2026-09-14 was reported rather than silently trimmed: six later runs are green, the probable cause is CleanMyMac wiping `target/debug`, and it remains a hypothesis |

**What prevents marking this `/goal` as complete**, in one sentence: the native
acceptance test is forbidden by an instruction that has not been lifted, and `Messages` is
blocked in a dependency. Neither can be lifted by code.

Three decisions also remain with the user, without blocking the rest: the
**second reactor** that `agent-client-protocol` brings, the **export/display
decision** on hidden columns, and the choice between waiting for `sqlx` or
rewriting the PostgreSQL driver in `tokio-postgres`.

The ADRs refer to them without defining them. The milestones below marked
**[ADR]** are constraints already settled, extracted from the ADRs:

| Constraint | Source |
|---|---|
| UI conditional on capabilities is a discipline **from phase 0** | [ADR-0003](adr/0003-driver-capabilities.md) |
| ~~Switching to egui remains possible until the end of phase 1~~ — moot since GPUI was replaced | [ADR-0001](adr/0001-ui-toolkit.md), superseded by [ADR-0029](adr/0029-interface-tauri-shadcn.md) |
| No driver of **phases 0 to 3** needs the sidecar | [ADR-0007](adr/0007-driver-sidecar.md) |
| WASM plugins arrive in **phase 4**, after 6+ native drivers | [ADR-0005](adr/0005-wasm-plugins.md) |
| The sidecar arrives in **phase 4** | [ADR-0007](adr/0007-driver-sidecar.md) |

The rest — the content of each phase and its exit gate — is **proposed** and
requires validation. It has no authority until the first code
commit has proven it.

What the former §11 of ARCHITECTURE ordered without these phases placing it was
sorted on 2026-09-25: the self-sufficient SQL client at the end of
[phase 2](#phase-2--the-protocols-that-matter), the non-relational
protocols in [phase 3 bis](#phase-3-bis--beyond-the-relational),
the broadening in [phase 4](#phase-4--extension-and-isolation).

## Phase 0 — Framework

The goal is not to display something, it is to make the decisions
executable. A botched phase 0 is paid for in every following one.

- Cargo workspace, pinned `rust-toolchain.toml` ([ADR-0008](adr/0008-chaine-outils-rust.md));
- `oxyn-core`, `oxyn-core` with `Command`, `Actor`, `PolicyGate` ([ADR-0004](adr/0004-command-bus.md));
- `oxyn-driver`: `Driver`, `Session`, `Capabilities`, `QueryLanguage` ([ADR-0003](adr/0003-driver-capabilities.md));
- `oxyn-data`: Arrow `ResultBuffer` with spill to disk ([ADR-0002](adr/0002-arrow-result-model.md));
- a reference driver: **SQLite**, in process, without network;
- the quality command `make qualite` passing.

**Exit gate**: a read `Command` emitted by a test goes through the
`PolicyGate`, reaches SQLite, and comes back as a `RecordBatch`. Without an interface.

> Choosing SQLite first is not a shortcut: it is the only driver
> that isolates the contract from everything that comes from the network. A bug at this stage is a
> contract bug, not a protocol bug.

## Phase 1 — First visible path

- window, query editor, grid virtualized over `RecordBatch` — delivered
  first in GPUI in `oxyn-ui`, wired by `oxyn-app`, then ported to
  `apps/desktop` and `oxyn-desktop`; the two GPUI crates were removed on
  2026-09-18 ([ADR-0029](adr/0029-interface-tauri-shadcn.md));
- end-to-end cancellation, including on the server side;
- the five view states ([UX-SPEC](UX-SPEC.md#states-of-a-view)).

**Exit gate**: the budgets of [PERFORMANCE](PERFORMANCE.md) are
**measured**, not assumed — it is the first measurement campaign, and it
confirms or amends the budgets through an ADR.

> **Taken over from §11 of ARCHITECTURE**, removed on 2026-09-25 in favor of this plan.
> Its exit criterion tests the gate above on a specific case: a
> `SELECT` of 10 M rows, first display within the
> [PERFORMANCE budget](PERFORMANCE.md#interaction-budgets), stable memory,
> `Escape` that really cancels. **Measured**
> ([PERFORMANCE](PERFORMANCE.md#comparison-with-the-budgets)): the first batch in
> 2.6 ms regardless of the table's size, and 2 GiB going through a buffer
> of 256 MB for 195 MiB of RSS growth. **Still to produce**: this `SELECT`
> end to end, and scrolling a populated grid under instrumentation, now
> in the webview. The editor is CodeMirror 6, highlighted per dialect, with the
> basic completion of `basicSetup`; a completion fed by the catalog is not
> wired to it.

> **[ADR]** ~~This is the last phase where switching to egui remains a rewrite
> of two crates ([ADR-0001](adr/0001-ui-toolkit.md)). After that, the cost changes
> in nature. If GPUI is to be called into question, it is here.~~ — GPUI was called into
> question by [ADR-0029](adr/0029-interface-tauri-shadcn.md) and removed on
> 2026-09-18; the reconsideration conditions of ADR-0029 now
> apply.

## Phase 2 — The protocols that matter

- `oxyn-driver-postgres`, `oxyn-driver-mysql` — **both exist**; the MySQL
  driver, which also serves MariaDB, since 2026-09-30, on the choices of
  [ADR-0050](adr/0050-mysql-driver-on-mysql-async-prepared-first.md);
- `oxyn-catalog`: introspection, cache, tree;
- environment marking of connections and storage in the keychain
  ([SECURITY](SECURITY.md)).

**Exit gate**: the [driver checklist](../.claude/checklists/revue-driver.md)
passes entirely on the three drivers, server-side cancellation included.

**`oxyn-driver-mysql` exists since 2026-09-30.** It was written through
[`/driver`](../.claude/commands/driver.md) and its checklist, `KILL QUERY`
included; its integration tests pass against MySQL 8.4 and 9.7 and MariaDB
11.8. Its `oxyn-query` prerequisite is **done**: the MySQL scanner keeps the
`BEGIN … END` body of `CREATE PROCEDURE`, `FUNCTION`, `TRIGGER` and `EVENT` as
one fragment, and honors `DELIMITER` as a batch directive. The exit gate of
phase 2 names three drivers; all three exist, and it is passed once the
checklist is walked on each of them in review.

*History.* On 2026-09-24 the MySQL driver was postponed until after the exit
gate of phase 3: phase 3 was under way with a dated deadline, and two
delivered drivers, PostgreSQL and SQLite, were enough to test the traits of
[`oxyn-driver`](../crates/oxyn-driver/src/traits.rs). The postponement ended
with ADR-0050 on 2026-09-30.

**Proposed — the self-sufficient SQL client.** Placed here on 2026-09-25
from the former §11 of ARCHITECTURE, which put it before AI: at this stage,
Oxyn is a good SQL client without a single line of AI.

- `oxyn-driver-duckdb` and `oxyn-driver-clickhouse` — **to be written**, through
  [`/driver`](../.claude/commands/driver.md);
- exporting a result — **exists**: CSV, JSON and Arrow IPC, streamed
  from the `ResultBuffer` (`crates/oxyn-data/src/export.rs`), offered by
  `ExportMenu` under the console, the Data preview and the retained result;
- the execution history — **exists**: the `query_history` table
  (`crates/oxyn-store/src/history.rs`), separated from drafts and saved
  queries by [ADR-0014](adr/0014-documents-et-historique.md);
- data editing with a DML preview — **to be written**. What exists is
  the Data preview of a table, read-only
  (`apps/desktop/src/features/workspace/object-view.tsx`,
  [ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md),
  [ADR-0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)):
  it is the surface where editing will sit. Neither editing nor DML shown before
  sending exist.

## Phase 3 — The AI workspace

- `oxyn-ai`: local and remote providers, single gateway;
- privacy tiers per connection ([ADR-0006](adr/0006-ai-privacy-tiers.md));
- agents proposing `Command`s carrying `Actor::Agent`.

**Exit gate**: Oxyn remains a complete client, without degradation, with zero
providers configured — checked by a test, not by conviction.

**Done on 2026-09-24 — the sample only reads the checked columns.** Reading
a sample, pinned or requested by an agent
([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)), is a
`PreviewRelation` whose shape carries a projection: `PreviewShape::columns`,
names the driver checks against the relation and quotes
([I-10](../CLAUDE.md#i-10), [DRIVER-CONTRACT](DRIVER-CONTRACT.md#6-it-escapes-every-identifier-it-composes)).
The server only returns the approved columns, PostgreSQL and SQLite included;
nothing unchecked enters the `ResultBuffer`.

**Still to do, noted on 2026-09-24 — the command journal does not show the
projection.** `audit_journal` records no text for a
`PreviewRelation`: no sort, no predicate, no columns. The projection is found
in the `Command` and in the `ai_egress` audit, linked to the journal entry through
`command_id`, but not in the journal itself. Writing the composed SQL there would
put the user's predicate into the journal: it is a decision in its own
right, not a side effect of the projection. **What unblocks it**: that
decision, taken for all previews and not for the sample alone.
**Deadline**: before the exit gate of this phase, and at the latest on
2026-10-31.

**Still to do, noted on 2026-09-25 — sending rows to an agent is still
confirmed in the webview.** [ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)
decides that a write on `production`, sending rows to an agent and
a change of marking are confirmed in a native dialog. The
`HostConfirm` port exists (`crates/oxyn-desktop/src/backend/confirm.rs`), and
`decide`, `decide_connection`, `decide_connection_change` and
`update_connection` go through it. What remains is family 2 — `ai_answer_sample`
and the `sample` field of `ai_ask` still grant on a simple call — and
`ai_save_external_agent`, which still opens its dialog without going through the
port. **What unblocks it**: the end of the work in progress on the assistant
(`commands/ai.rs`, `backend/ai`), then one commit per path, with the tests of
§ 4 of the ADR. **Deadline**: at the latest on 2026-10-31.

## Phase 3 bis — Beyond the relational

Proposed on 2026-09-25, taken over from the former §11 of ARCHITECTURE. Numbered
"3 bis" so as to move neither the anchor nor the references of phase 4.

- `oxyn-driver-mongodb`, `oxyn-driver-redis`, `oxyn-driver-elasticsearch` —
  OpenSearch speaks the Elasticsearch protocol, it has no driver of its own
  ([ADR-0003](adr/0003-driver-capabilities.md)).

**Why here, before plugins.** This is where the capability model
([ADR-0003](adr/0003-driver-capabilities.md)) and `QueryLanguage` are put to
the test: Redis has no schema, Elasticsearch no SQL, MongoDB no fixed
columns. If they are badly designed, better to find out now
than at the twentieth driver — and before phase 4 opens these traits to
plugins, where a mistake has to be supported indefinitely. The sidecar stays in phase 4
([ADR-0007](adr/0007-driver-sidecar.md)): a driver of this phase that
needed it would reopen that ADR.

**Exit gate** proposed: the three drivers pass the
[driver checklist](../.claude/checklists/revue-driver.md), and no
interface surface nor any agent tests a driver's name to know
what to offer — only its `Capabilities` and its `QueryLanguage`. A capability
or a language that had to be added for them is recorded in an ADR that
clarifies [ADR-0003](adr/0003-driver-capabilities.md), not slipped into the trait.

## Phase 4 — Extension and isolation

**[ADR]** What was deliberately postponed here:

- `oxyn-plugin`: wasmtime host, WIT interfaces ([ADR-0005](adr/0005-wasm-plugins.md));
- `oxyn-driverd`: sidecar for Oracle, Couchbase, cloud SDKs ([ADR-0007](adr/0007-driver-sidecar.md));
- broadening to the NoSQL, vector, graph and time-series families —
  Neo4j, Qdrant, Cassandra, DynamoDB, Influx — and, taken over from the former §11
  of ARCHITECTURE on 2026-09-25, the remaining agents, ER diagrams, data
  dictionaries and version comparison.

**Entry condition**, not exit condition: at least six native drivers delivered.
Opening an extension boundary on traits that too few implementations
have tested freezes mistakes that will then have to be supported indefinitely.

## Requirement-by-requirement verification — 2026-09-15

Each row names **the test or the code that holds it**, not an impression. A
requirement without named evidence is marked not held, even if the code seems to
cover it.

### Table preview — held

| Requirement | What holds it |
|---|---|
| `PreviewSort`, `PreviewFilter`, pagination in the command | `oxyn-core/src/preview.rs` (`PreviewShape`: `sort`, `filter`, `offset`), carried by `Command::PreviewRelation` |
| Capabilities | `Capabilities::PREVIEW_SORT`, `PREVIEW_FILTER` |
| SQL composed with quoted identifiers, both drivers | `quote_identifier` in `drivers/oxyn-driver-sqlite/src/preview.rs` and `.../postgres/src/preview.rs` |
| Deterministic sort | `a_page_is_offered_only_where_the_order_is_total` — a next page is only offered if the order is total |
| Cancellation | `escape_in_the_filter_field_still_stops_the_read_at_the_server` |
| No involuntary re-execution | `a_stale_answer_never_overwrites_a_newer_filter` |
| Empty state | `a_predicate_that_matches_nothing_is_empty_not_failed`, `an_empty_answer_says_whether_a_filter_caused_it` |
| Running state | `typing_dispatches_nothing_and_the_bar_says_the_draft_is_not_in_force` |
| Error state | `an_invalid_predicate_shows_the_server_message_and_keeps_no_false_rows` |
| Filtered / sorted | `a_predicate_filters_the_real_rows_and_leaves_the_sql_draft_alone`, `a_sort_changes_the_order_the_server_returns` |
| Next page | `two_consecutive_pages_do_not_overlap` |
| Missing capability | `a_session_without_the_capabilities_has_no_bar_and_dispatches_nothing` |

### Security and AI — held, except for one type guarantee

| Requirement | What holds it |
|---|---|
| Refusal of production writes to an agent | `oxyn-core/src/policy.rs`: `actor.is_agent() && mutating && env.is_production()` returns `Decision::deny` — a refusal, not a confirmation ([I-02](../CLAUDE.md#i-02)). Test: `an_agent_is_never_less_restricted_than_a_human` |
| Single context gateway | `ContextBuilder::build` (`oxyn-ai/src/context.rs`), the only factory of `AgentContext` |
| No raw server output | `FailureReport` filters **at construction**, under the connection's tier |
| No AI output executed | `open_proposal` opens a console and runs nothing; it even refuses to write without provenance |
| Persistent provenance | `documents.provenance` column, `coalesce` on write, visible on the tab **and** in the library |
| Sentinel test | `no_sentinel_reaches_the_workspace_file` — sweeps **all** tables and columns via `sqlite_master`, without naming any |
| I-04 gate typed for **both** destinations | **held since 2026-09-15.** `run_turn` takes an `AgentPrompt`, whose only constructor requires the tier ([ADR-0027](adr/0027-porte-unique-pour-les-deux-destinations.md), option B). `run_turn` keeps its own check: the gate protects assembly, the check protects launch. Tests: `under_local_no_prompt_is_composed`, plus two `compile_fail` that forbid any naive constructor |
| I-03 — log channel | **Preventive** test: nothing formats a secret on this path today, so removing a guard elsewhere does not turn it red on its own; checked by hand by adding a `tracing::debug!` that formats a provider key in `save_ai_provider`, confirming the red, then reverting. `no_sentinel_reaches_the_journal` (`crates/oxyn-desktop/src/sentinel_tests.rs`) — the binary's whole tracing output at `OXYN_LOG=trace`, through the two real filters of `logging::layer`, during a refused connection and a provider edit |
| I-03 — displayed error channel | Tests the removal of an existing guard — `redact_key` (`oxyn-llm`). `no_sentinel_reaches_an_error_shown_to_the_front` (`crates/oxyn-desktop/src/sentinel_tests.rs`) — serialized `IpcError` and all messages of the `Channel<AiUpdate>`, with a hostile provider that copies the key it receives and a provider URL that carries a secret |
| I-03 — AI prompt channel | **Preventive** test: `ContextBuilder::build` takes neither the connection nor the provider as an argument, so nothing formats the secret there today; checked by hand by making the provider key appear in the question assembled by `converse` (`backend/ai/conversation.rs`), confirming the red, then reverting. `no_sentinel_reaches_the_prompt_sent_to_a_provider` (`crates/oxyn-desktop/src/sentinel_tests.rs`) — the exact body of the `chat/completions` request sent to the provider, prompt assembled by `ContextBuilder` included |
| I-03 — clipboard channel | Tests the removal of an existing guard — the choice of the caller of `copyToClipboard` never to pass it a connection or session identifier. `describe("copying from an open connection's object inspector")` / `it("copies the qualified name only, never the connection or session that opened it")` (`apps/desktop/src/features/metadata/clipboard.test.tsx`) — limited to copies that go through `copyToClipboard` (`src/features/metadata/clipboard.ts`); the direct copies of `assistant-panel.tsx` and `result-grid.tsx` are not covered |
| I-03 — crash report channel | **Does not exist.** No crash report crate (`sentry`, `minidump`, `crashpad` absent from `Cargo.lock`), no `std::panic::set_hook`, and `panic = "abort"` in the `release` profile (root `Cargo.toml`). The only text of a crash is the default panic message on `stderr`: nothing to sweep |

### Recovery and library — partly held

Reread on 2026-09-25: the tests marked "deleted" lived in `oxyn-app` and
disappeared with `6ecb8ce`, with no equivalent in `apps/desktop`. Their row
is no longer held; the [migration gate](#migration-to-the-tauri-interface)
says what is missing.

| Requirement | What holds it |
|---|---|
| Clean/abnormal shutdown marker | `PreviousShutdown { Never, Clean, Abnormal }` ([ADR-0021](adr/0021-marqueur-d-arret.md)) |
| Crash/restart tests | `a_first_opening_reports_no_abnormal_shutdown`, `an_ordinary_close_does_not_trigger_recovery`, `a_session_left_open_and_silent_is_an_abnormal_shutdown`, `an_instance_still_beating_is_not_a_crash`, `a_heartbeat_does_not_revive_a_closed_session` (`oxyn-store/src/sessions.rs`) |
| Object location and sub-tab restored without reading | **done on 2026-09-25.** The single field `object_location` of [ADR-0013](adr/0013-preferences-workspace.md), read and written by `read_object_location` / `write_object_location`. On the Rust side, `a_restored_location_and_its_sub_tab_come_back_without_reading_anything` (`crates/oxyn-desktop/src/backend/settings/location_tests.rs`) proves that rereading the location only emits preference commands, without a session. On the front-end side, `a restored object tab` (`apps/desktop/src/features/workspace/object-view.test.tsx`) proves that the reopened tab neither makes the preview visible nor loads metadata before "Read it now", and does not move the saved location; the recovery screen offers it (story `Oxyn/RecoveryList`, `WithAnObjectTab`) |
| Restoration without overwriting data | **done on 2026-09-25.** `a_restored_object_that_vanished_is_explained_and_never_erased` (same Rust file) proves that neither a read nor another write erases it, and that only closing the tab forgets it. The explanation (story `Oxyn/RestoredObjectNotice`, `Vanished`) appears when rereading the columns no longer finds the relation — on Structure, Indexes and outgoing Relations; elsewhere, the server's error says so |
| Deleted or out-of-workspace connections in the filters | **not held on the interface side** — `a_deleted_connection_stays_choosable_in_the_history_filter` and `merging_history_connections_appends_and_marks_without_moving_ranks` deleted |
| Bounded pagination | `document_pages_are_bounded_literal_and_do_not_open_oversized_bodies` (`oxyn-store`) |
| Read-only inspection | `history_and_recent_results_read_the_same_execution_without_replaying_it` (`oxyn-desktop/src/backend/library.rs`); the stories `LibraryEntryView` (`History`, `NeedsInspection`, `SavedWithWorkingCopy`) and `LibraryPanel` (`RecentResults`) hold the Tauri view |

### What remains not held, and why

> **One requirement was set aside by the person who asked for it, and it is a contradiction
> that must be read as such.** This session's work plan asked to
> "run the application's native acceptance test at several sizes and in both
> themes" **and**, in the same sentence, "without disturbing the user's
> computer". Asked on 2026-09-15, the user maintained their instruction
> of 2026-09-10: no window opens on their screen.
>
> Both halves of the requirement therefore cannot be satisfied together
> on this machine. It is not a decision an agent can make in their
> place, and it was made: **the second half wins**. The three rows
> below follow from it, and are not meant to be filled as long as that
> decision holds.


| Requirement | State |
|---|---|
| **Native UI** — acceptance test at several sizes and in both themes | **not held, and it is a decision.** Asked on 2026-09-15, the user **maintained** their instruction of 2026-09-10: no window is opened on their screen. It is therefore not a forgotten to-do item, but a deliberate trade-off between pixel proof and not interrupting their work. No GPUI test replaces it: the harness installs text metrics that are deterministic *and* wrong |
| **Accessibility** — full keyboard acceptance test | partial. The blocking points of [revue-ui](../.claude/checklists/revue-ui.md) have their tests; the tab `Close` was made reachable **without** a test, for a reason written next to the code |
| **Performance** — scrolling a populated grid | not measured: requires `xcrun xctrace` attached to the process, hence the application launched. The **256 MB buffer value**, for its part, is now measured — 2 GiB pushed, RSS +195 MiB, 1.84 GiB on disk ([PERFORMANCE](PERFORMANCE.md)); only its **automation** remains blocked by [I-03](../CLAUDE.md#i-03) or `unsafe_code = "deny"` |
| **Figma** — visual fidelity | the nine boards are compared *in measurements and texts*; **pixel** fidelity is a matter for the native acceptance test |

## Validation log of 2026-09-15

Three independent reviews with disjoint scopes — invariants, security,
code/docs divergences — then comparison of the Figma frames against the server. What
follows is the record of what was **found**, not of what was gone through.

### Code defects found and fixed

Each one is closed by a test, and each test was tested by sabotage: the
guard removed, the test turns red. The sabotages were undone.

| Defect | Why it turned red nowhere |
|---|---|
| **The external agent mode was unreachable** — the guard of `ask_assistant` and `provider_for` decided separately, with overlapping conditions: past the guard, `provider_for` always returned `Some`, and a user who had only declared an agent saw "no AI" | the whole ACP batch compiled, and its security — permissions, default refusal, tier check — was exercised by no real path |
| **The stop button of an agent turn cut nothing**: the token was created, given to the panel, never passed to `run_turn` | the display did switch to "stopping"; only the subprocess kept going |
| **The panel died after an agent turn** — no reset of `active` | the provider path had it; nothing compared the two |
| **`Debug` of `AiProviderConfig` rendered the whole URL**, relying on `validate()` — which is only called at the other end of the bus | a key pasted into the "endpoint" field lived in clear text in a value in transit |
| **An agent's environment values were neither bounded nor sanitized** | a NUL there is silently truncated at launch: the agent receives something other than what is displayed **and** persisted |
| **Agent provenance was missing from the library** — `DocumentSummary` had no field for it | the mark was visible on the tab, that is where it is not needed, and absent where one rereads a text a year later ([ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md)) |
| **`Metrics::toolbar_height` was 52; the mockup says 48** | documented, sourced, dated **and** anchored by a green test — but the field had no reader, the view hard-coding `48`. A textbook case of [I-12](../CLAUDE.md#i-12) |
| `Open Indexes` absent (`229:8021`), write warning title absent (`232:9100`), `Edit rows…` tooltip absent, message with twenty spaces | interface deviations no compilation sees |
| **The library's [I-13](../CLAUDE.md#i-13) banner did not exist** (`268:36866`) — the text was there, at the end of a grey line, after two other mentions | what this guarantee allows is browsing one's history **without fearing that a click replays a write**; a footnote does not carry that, and it is precisely the entry with the unknown outcome that one most needs to inspect |
| **The `Close` of a console tab was unusable with the keyboard** — visible, clickable, without `tab_index` | `⌘W` does not cover it: it closes the **active** console, and only from the SQL panel. Closing another tab therefore had no keyboard path — the blocking point of [revue-ui](../.claude/checklists/revue-ui.md). Fixed **without a test**: checking it would require reaching that button by tabbing from a known point, but their number depends on the open tabs; a test that set focus with a click would be green without proving anything |

### Memory measurement

400 MiB pushed into a `ResultBuffer` bounded at 4 MiB: RSS growth of
**4.25 MiB**, 404 MiB spilled to disk. Sensitivity check: the same
scenario under a 400 MiB budget grows the RSS by 317 MiB. The measurement
therefore discriminates by a factor of ~75, and [I-06](../CLAUDE.md#i-06) holds on the
memory of the **process**, not only on the buffer's accounting. Details and
the reason it is not automated: [PERFORMANCE](PERFORMANCE.md).

### Frames compared against the Figma server

Preview and inspector (`190:1163`), constraints (`229:7637`), relations
(`229:32690`), recovery (`232:9100`), console bar (`191:1958`), side
bar (`221:4573`), preferences (`47:8222`). Surveys and accepted deviations:
[FIGMA-HANDOFF](FIGMA-HANDOFF.md).

The library board (`47:7638`) was first believed impossible to find: the
page list returned by the server showed only three of them, whereas the
file has thirty. It was found by enumerating `figma.root.children`
through the Plugin API, read-only. **All nine boards are therefore compared.**

### What this session could not do

- **The native acceptance test**, hence any pixel proof. The user's instruction
  of 2026-09-10 — "desktop interactions were stopped at their remark"
  — has not been lifted, and it was not bypassed. "Native UI" therefore remains
  **not held**, and no headless GPUI test replaces it: the harness installs
  text metrics that are deterministic *and* wrong
  ([tests.md](../.claude/rules/tests.md#interface-tests)).
- The questions below, which require a decision.

## Open questions — not settled

Found on **2026-09-15** by comparing the authoritative documents with the code.
**None is settled here**: each one pits two authoritative documents against each other, or an
authoritative document against the code, and it is a decision that settles them — not an
editorial fix. They are listed so that they stop being
invisible, not to be resolved in this file.

### 1. The default sort of a preview: the ADR says the opposite of the code

[ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md) states twice that, without
a requested sort, the driver orders by the primary key alone, and counts among its
consequences "a sort imposed by default on the primary key".

**The code does the opposite.** Neither driver composes any `ORDER BY` in
the absence of a request — the test `a_preview_without_request_composes_neither_where_nor_order_by`
holds it explicitly in `oxyn-driver-sqlite` as in `oxyn-driver-postgres`.
[UX-SPEC](UX-SPEC.md) confirms the code: "without a requested sort, the order of rows
is not guaranteed".

**What is not settled.** Which of the two positions is right. The ADR
justified the default order by the correctness of `OFFSET`: paginating on an order
that is not guaranteed silently duplicates and omits rows. This argument was not
refuted — it was bypassed. An ADR is not rewritten on the quiet: it is a
**new ADR** that must say which of the two behaviors is intended, and why
pagination remains correct in the chosen case.

**Answered by [ADR-0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)**,
the new ADR this paragraph asked for: no imposed order, no page
offered as long as the order is not total — this is how pagination remains
correct. It carries `accepted` since the
[review of 2026-09-24](#3-the-status-of-adrs-review-of-2026-09-24), which cites its two
tests. The status of ADR-0020, which it clarifies, falls under that same review.

### 2. Two values for the first-display threshold

**Resolved.** [ARCHITECTURE](ARCHITECTURE.md) no longer puts a number on this threshold since
2026-09-15 and points to the
[interaction budgets of PERFORMANCE](PERFORMANCE.md#interaction-budgets):
**300 ms after the server's first response**. Its §11, which carried this reference,
itself points to this plan since 2026-09-25. The original finding follows.

[ARCHITECTURE](ARCHITECTURE.md) sets the exit criterion of phase 0 at a
first display **under 100 ms** and declares the criterion unmeasured.
[PERFORMANCE](PERFORMANCE.md), which **is authoritative on the numeric budgets**,
sets **300 ms** after the server's first response, and declares the budget
confirmed for SQLite (2.6 ms measured).

The two values may not have the same counting origin — one starts from
launching the query, the other from the server's first response. That is
precisely what makes the gap unusable: **no regression can be
arbitrated** as long as the threshold and its starting point are not unique.

**What is not settled.** Which value, counted from which instant. Once
decided, a single document carries the number and the other points to it.

### 3. The status of ADRs: review of 2026-09-24

**Settled on 2026-09-24.** Criterion: an ADR becomes `accepted` when its decision,
**as written**, is implemented and held by at least one test. ADR-0009
carries `superseded` ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).

| ADR | Status | Test that holds it, or reason for keeping it |
|---|---|---|
| [0021](adr/0021-marqueur-d-arret.md) | accepted | `a_session_left_open_and_silent_is_an_abnormal_shutdown` (`oxyn-store/src/sessions.rs`), `an_abandoned_session_is_reported_as_abnormal` (`oxyn-desktop/src/backend/recovery.rs`) |
| [0023](adr/0023-fournisseurs-declares-et-provenance.md) | accepted | `no_url_carrying_credentials_reaches_the_disk` (`oxyn-store/src/providers.rs`), `an_unresolved_endpoint_gets_no_benefit_of_the_doubt` (`oxyn-llm/src/reach.rs`) |
| [0026](adr/0026-agents-externes-acp.md) | accepted | `an_external_agent_never_serves_a_local_connection` (`oxyn-ai/src/privacy.rs`), `the_table_has_no_secret_column` (`oxyn-store/src/agents/tests.rs`) |
| [0027](adr/0027-porte-unique-pour-les-deux-destinations.md) | accepted | `under_local_no_prompt_is_composed` (`oxyn-ai/src/external/prompt/tests.rs`), `the_local_tier_refuses_before_even_launching_the_process` (`oxyn-ai/src/external/tests.rs`) |
| [0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md) | accepted | `a_preview_without_request_composes_neither_where_nor_order_by` (both drivers), `no_page_is_offered_without_a_total_order` (`oxyn-desktop/src/ipc/metadata.rs`) |
| [0029](adr/0029-interface-tauri-shadcn.md) | accepted | `check_dependency_graph` of `.claude/verifier_socle.py` (through `make socle`), `a_window_crosses_batches_and_stays_bounded` (`oxyn-desktop/src/backend/results.rs`). The measurement campaign is its **reconsideration** condition, not a prerequisite |
| [0031](adr/0031-validation-des-reponses-ipc.md) | accepted | "rejects what the grid could not draw, rather than letting it through" (`apps/desktop/src/lib/ipc/types.test.ts`), "degrades an unknown ending instead of refusing the event" (`ai.test.ts`) |
| [0032](adr/0032-agent-externe-confine-au-lancement.md) | accepted | `claude_agent_keeps_no_tool_of_its_own` (`oxyn-ai/src/external/confine.rs`), `a_confined_agent_is_put_in_its_mode_first_and_cut_off_as_soon_as_it_leaves` (`external/session/tests.rs`) |
| [0033](adr/0033-couches-de-configuration-codex.md) | accepted | `every_readable_layer_is_switched_off_by_name_each_once` (`oxyn-ai/src/external/confine/codex_layers.rs`) |
| [0034](adr/0034-echantillon-pour-toute-destination.md) | proposed | **code/ADR deviation**: its § 2 says that the columns "join no statement", whereas since `d6cf80b` they enter the `PreviewRelation` through `PreviewShape::columns`, quoted by the driver ([Phase 3](#phase-3--the-ai-workspace)). The rest is held (`an_external_agent_receives_only_the_columns_the_user_ticked_and_only_after`); the sentence must be clarified before accepting |
| [0005](adr/0005-wasm-plugins.md) | proposed | WIT and component instantiation return "phase 4" (`oxyn-plugin/src/host.rs`) |
| [0007](adr/0007-driver-sidecar.md) | proposed | no sidecar, postponed to phase 4 |
| [0020](adr/0020-apercu-trie-filtre-parcouru.md) | proposed | its default order on the primary key is contradicted by the code and by ADR-0028, which clarifies it |
| [0022](adr/0022-rafraichissement-automatique.md) | proposed | a DDL does not reread the visible preview, whereas the ADR plans it; `RefreshSignal::of` has no test |
| [0024](adr/0024-autosauvegarde-au-repos-de-frappe.md) | proposed | implemented (`DRAFT_IDLE_MS`), but neither the delay nor the three immediate writes are tested |
| [0025](adr/0025-proposition-de-changement-de-schema.md) | proposed | the Rust composer is tested, but the gesture is not mounted: `useSchemaProposal` has no caller |
| [0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) | proposed | **code/ADR deviation**: § 2 bis promises one write per question, `WriteGate` applies "one pending request at a time" (`nothing_runs_while_a_request_waits_even_from_an_earlier_question`) — to be decided |

ADR-0035 was already `accepted`; ADR-0036, written the same day, was not reviewed.

**Since the review.** [ADR-0040](adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md)
becomes `accepted` on 2026-09-25, once its code was merged (PR #59), according to the
same criterion: `a_forced_exit_records_the_close_and_is_not_offered_recovery` and
`a_forced_exit_over_a_pending_write_is_bounded_and_leaves_the_close_unwritten`
(`oxyn-desktop/src/backend/recovery.rs`) hold it.

**What remains reported, without being fixed here.** Several accepted ADRs still cite
`oxyn-app` or GPUI as the place of implementation (0021, 0023, 0026, 0029):
their decision holds, their text has aged, and an accepted ADR is not rewritten.
The two `compile_fail` doctests of ADR-0027 (`prompt/tests.rs`) live in a
`#[cfg(test)]` module that rustdoc does not go through: they never run.

What follows is the state found on 2026-09-15, kept.

Only six ADRs then had the `accepted` status (0008, 0010, 0014, 0015, 0016,
0017); the twenty others remained `proposed`, whereas the decision of most
of them was implemented and proven by tests.

**Why it is not cosmetic.**
[.claude/rules/documentation.md](../.claude/rules/documentation.md) rests
the protection "an **accepted** ADR is not rewritten" on this status. As long as
everything is `proposed`, this protection applies **nowhere**. It is not
theoretical: it is through successive rewriting that [ADR-0026](adr/0026-agents-externes-acp.md)
ended up carrying, on the same page, a claim and its opposite about
the `agent-client-protocol` dependency.

**What was not settled** — and is since, see the table above.
Which ADRs become `accepted`, and according to which criterion — the decision taken, or the
decision implemented and measured. Moving to the
`accepted` status locks rewriting: it is a governance decision, not
a sweep of fields to be done in bulk without review.

### 4. The conversation budget is written, but nothing applies it

Found on **2026-09-23**. [PERFORMANCE](PERFORMANCE.md) sets the disk budget of
the assistant's history — 200 threads, 90 days of inactivity, 32 MiB of
transcript per workspace — "applied by `Conversations::prune`".
`Conversations::prune` and `RetentionPolicy::default()` exist in
`oxyn-store` and are proven by its tests, but **no caller outside these
tests**: the application never prunes. The budget therefore does not hold, and that is
precisely the growth it was meant to prevent.

**Fixed on 2026-09-23.** `Backend::prune_conversations` applies
`RetentionPolicy::default()` once per launch, on the blocking pool, without
delaying opening. Pruning happens at launch rather than after each exchange: the
budget bounds months of use, not a session, and a deletion in the middle of a
conversation could take away the thread the user is reading.

**Settled on 2026-09-24, done on 2026-09-25:** the user is warned.
The pruning report is kept in memory by the backend (`ai_pruned_history`)
and the "Conversations" list displays a note that says how many threads were
removed and according to which rule, with the numbers of `RetentionPolicy`
([UX-SPEC](UX-SPEC.md#a-conversation-stays-in-the-workspace-not-what-was-shown-to-it)).
The `info` log remains. Held by
`the_launch_prune_is_transmitted_with_its_rule`.

## License: what remains before going public

[ADR-0044](adr/0044-licence-gpl-et-contrat-apache.md) has been in place since
2026-09-25: licenses, `NOTICE`, `CLA.md`, `CONTRIBUTING.md`, third-party notices
in the application. Four gestures remain that are not files of the repository,
or that are only done once:

- **enable CLA signing** when the repository goes public. The planned
  choice is CLA Assistant (<https://cla-assistant.io>) on `so-keyldzn/oxyn`,
  pointing to `CLA.md`. `CONTRIBUTING.md` already announces the bot. As long as it
  is not active, no external contribution is merged;
- **offer the source with any distributed binary** (GPLv3, § 6). The public
  repository is enough for that. Before then, no binary leaves the holder's circle;
- **have `CLA.md` reviewed by a lawyer**, at the same time as the transfer of
  rights to the company. `NOTICE`, the `copyright` field of
  `crates/oxyn-desktop/tauri.conf.json` and § 1 of the CLA then change
  holder;
- **publish the WIT interfaces under Apache-2.0** in phase 4, in a
  directory or a crate that carries that license alone.

## What does not belong here

Decisions. A phase that needs a decision writes an ADR
([`/adr`](../.claude/commands/adr.md)); it does not settle it in this file.
