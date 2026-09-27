# Mockup index for implementation

Survey state: 2026-09-07, before the move to Tauri. **This document is
historical**: the interface is now written with shadcn/ui on Base UI
in `apps/desktop` ([ADR-0029](adr/0029-interface-tauri-shadcn.md)), and its
tokens live in `apps/desktop/src/styles.css`. It reads as the survey of
what the mockup said at its date, not as the source of the screens to ship.
This index locates the frames; behaviors are authoritative in
[UX-SPEC](UX-SPEC.md), and the delivery scope in
[IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md).

## Shared reference

Page 22 carries the workbench adopted by
[ADR-0011](adr/0011-structure-commune-workspace.md). The sidebar is always
inset, collapsible to icons, according to page 01. The screens of pages 04 to 09 have
been migrated to this structure, keeping their identifiers and contents.

| Frame | Figma node |
|---|---|
| Table, populated state | [190:1163](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=190-1163) |
| SQL console, populated state | [191:1521](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=191-1521) |
| Object structure | [193:1814](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=193-1814) |
| Compact width, 1024 px, dark | [215:3279](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=215-3279) |
| Compact width, 1024 px, light | [235:10352](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=235-10352) |
| Table with collapsed sidebar | [233:9139](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=233-9139) |
| Palette open in the workbench | [229:6949](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=229-6949) |
| Constraints | [229:7637](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=229-7637) |
| Relationships | [229:32690](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=229-32690) |

## Console bar

Surveyed on the Figma server on 2026-09-10. Frame `191:1958` measures 1272 × 32 and
sits at (12, 12) in the console.

| Element | Node | x | Width |
|---|---|---|---|
| Run / Stop / Explain group | [273:37036](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=273-37036) | 0 | 286 (three segments of 96) |
| Parameters · N | [191:1993](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=191-1993) | 294 | 144 |
| Read-only notice | `191:2002` | 446 | 558 |
| Connection / schema selector | [191:2003](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=191-2003) | 1012 | 260 |

The segment that does not apply is dimmed: on the frame, it is Stop, because nothing
is running. The selector shows `commerce-prod / public` — an example, not a
real connection; what it changes is settled by
[ADR-0019](adr/0019-contexte-de-session.md).

## Connection bar

Surveyed on the Figma server on 2026-09-10, in `190:1163`. The bar measures
1296 × 48 (`190:1543`).

| Element | Node | x | Width |
|---|---|---|---|
| `Toggle sidebar · ⌘B` | `221:4945` | 12 | 32 |
| Context `commerce-prod / commerce / public` | `190:1544` | 52 | 868 |
| Environment badge | `190:1545` | 928 | 108 |
| Privacy badge | `190:1547` | 1044 | 136 |
| `Ask AI` | `190:1549` | 1188 | 96 |

The entry to the AI workspace is therefore **in this bar**, next to the badge that
announces the privacy tier — the two are read together. On the
recovery frame (`232:9100`), `Ask AI` and this badge are marked
`hidden`: the entry is conditional, it does not exist when there is nothing to
ask. No tier is displayed as long as no provider is configured
([ADR-0006](adr/0006-ai-privacy-tiers.md)).

Surveyed on the server on 2026-09-10, the button `190:1549` measures 96 × 32, placed at
(1188, 8): 16 × 16 icon at x = 16, 40 × 20 label at x = 40. Same template as
the badges preceding it.

### What the mockup does not show

**There is no frame for the conversation panel or the provider
configuration screen.** The mockup stops at the entry point. What
exists are the building blocks: page 23 (`Input Group`, input with an integrated
action), page 26 (`Message`, user / assistant / tool variants),
page 27 (`Attachment`) and the provider logos of page 02 (`244:1402`).

Both screens are therefore **composed from these building blocks**, and their
geometry is assumed, not surveyed. Behaviors are authoritative in
[UX-SPEC](UX-SPEC.md#the-ai-workspace-only-exists-if-it-has-been-configured), and the
structure in [ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md). To
be resurveyed the day the frames exist.

## Table preview bar

Surveyed on the Figma server on 2026-09-10, in `190:1163`.

| Element | Node | x | Width |
|---|---|---|---|
| Data bar: `Refresh data` | `190:1589` | 0 | 130 |
| `Columns · N` / `Export preview…` group | [272:10687](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=272-10687) | 138 | 252 (two segments of 126.5) |
| Read-only notice | `190:1611` | 398 | 598 |
| `Edit rows…` | `190:1612` | 1004 | 112 |
| `Why unavailable?` | `212:23799` | 1124 | 148 |
| Filter bar: `WHERE` field + `Apply` | [272:10667](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=272-10667) | 0 | 1180 (input 1048, `Apply` 64) |
| `Sort` | `190:1621` | 1188 | 84 |

The filter bar (`190:1618`) measures 1272 × 32 and sits at (12, 92), below the
Data bar. The field carries the literal prefix `WHERE`: it is a predicate
the user writes, not a condition builder — a trade-off recorded in
[ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md).

**Accepted gap**: the mockup draws **no** page control. Its
`Result status` (`190:1860`) only displays "200 rows loaded · 284 ms · Total
count not requested". The pagination decided by ADR-0020 therefore has no
drawn place; it joins this status line, next to the counter, for lack of a
frame that positions it. To revisit if the mockup settles it later.

### The preview footer, word for word

Surveyed on the server on 2026-09-15, by capturing `190:1860` — the name of the
text node is truncated in the metadata and said "not reported" here, which
was wrong. The mockup writes exactly:

> 200 rows loaded · 284 ms · Total count not requested

Oxyn writes "`N` rows shown · `D` ms · Total count not requested". Two gaps,
both deliberate:

| Gap | Why |
|---|---|
| "shown" and not "loaded" | it is the very subject of this line. A preview bounded to 200 rows on a table containing fifty million looks in every respect like a complete preview; "loaded" would suggest everything is there |
| no "Read only" | the Data bar (`190:1611`) already carries "Read-only preview" forty pixels higher, and the mockup writes it only once. Saying it again here would be the duplicate the mockup avoids |

**The duration, for its part, was missing** and has been added: it comes from the mockup, and
it is not decorative — it is what distinguishes a slow database from a preview
that found nothing.

### The record inspector, compared on 2026-09-15

`Record inspector` (`190:1872`) measures 280 × 640 and sits at x = 992, after a
`ResizableHandle` (`190:1869`) of 8 px; the grid takes the remaining 984.
Checked in the code: default width 280 and 8 px handle, keyboard bounds
240–480. `Inspect full value` and `Hide inspector` (256 × 32) are
present, as well as "Selected row · Read only".

The grid metrics also agree: 28 px headers, 24 px
rows — the theme's values. The reading size control displays
"Text · 13 px" like the mockup, and "Text · 14 px" in comfortable reading.

## Comparison of 2026-09-15 — constraints, relationships, recovery, console

Surveyed on the server, frame by frame, against the code. What agrees is not
listed here unless the value is checked elsewhere; what was missing is.

| Frame | Finding | Follow-up |
|---|---|---|
| Constraints `229:7637` | proportions match — metadata 840 + handle 8 + DDL panel 424 = 1272, and 424 is indeed the code's default width. `Refresh structure`, `Copy DDL`, `Propose change…`, `DDL · Read only` and both summaries are there | **`Open Indexes` (`229:8021`) was missing**: the code only had the sentence "listed under Indexes", which sends you looking without leading you there. Added, wired to the tab bar's `Control::Indexes` and conditioned on the capability |
| Relationships `229:32690` | complete: "Incoming relationships", "Selected relationship", "Bounded related-row preview" and `Review related-row query`. The query template quotes its identifiers and is executed by no path | nothing to do |
| Recovery `232:9100` | the choice table, the checkboxes, the interrupted-write sentence and the exits are there. The privacy badge and `Ask AI` are hidden in the frame, as in the code | **the title "A write may have an unknown outcome" was missing**: it is the sentence that carries [I-13](../CLAUDE.md#i-13), and without its title it reads like a footnote between a table and a row of buttons. Added |
| Console bar `191:1958` | composition matches: Run/Stop/Explain group, `Parameters · N`, read-only notice, selector | two label gaps, below |

### Label gaps not closed, and why

| Mockup | Oxyn | Reason |
|---|---|---|
| `Explain` (96 px) | `Explain query` | more explicit out of context, but **wider than the 96 px segment** the mockup draws. The real effect on the layout cannot be measured without pixels: a story measures the layout of a DOM without rendering, and a width assertion would be green there by mistake ([tests.md](../.claude/rules/tests.md#interface-tests)). To be settled at native acceptance |
| `Run   ⌘↵` | `Run · ⌘Enter` | the shortcut spelled out rather than in symbols; same width caveat |
| `Restore selected drafts` | `Restore N selected items` | deliberate and already described above: the button announces its count |
| `Start with an empty workspace` | `Continue without restoring` | says what the gesture does rather than the resulting state; nothing is lost, the drafts stay in the database |

### The library `47:7638`, compared on 2026-09-15

"Workflow / 07 · History & saved queries", page "06 · Administration &
workflows".

> **How it was found, and the mistake this corrects.** This node was
> believed impossible to find: `get_metadata` without `nodeId` only returned **three**
> pages, and this document's index did not cite it. The file actually has
> **thirty** — the list was partial, not the file. The frame was
> found by querying the Plugin API read-only (`use_figma`), which
> enumerates `figma.root.children`. Worth remembering for the next frame one believes
> missing.

Agrees, and closely: the three tabs `History` / `Saved queries` /
`Recent results`; the five columns `Query` (440), `Connection` (300),
`Status` (180), `Duration` (136), `When` (192); the search,
connection, period and status filters; `Open retained result`, under the same name; and the
detail panel, **238 px on both sides**.

**What was missing**: the banner `268:36866`, framed in danger color,
titled "Ambiguous writes are never replayed" and followed by "An expired write may
have reached the server. History offers inspection and reconciliation, never a
retry action." The text existed in Oxyn — at the end of a grey line, after
two other notices. It is now promoted to a banner, on the History tab
only: it is the only one that lists real executions, and the warning
would be pointless where nothing has ever been written. The tail of the sentence was
removed so as not to say the same thing twice.

Naming gaps, accepted:

| Mockup | Oxyn |
|---|---|
| `Open SQL in editor` | `Open copy on <connexion>` and `Resume working query` — two distinct gestures where the mockup draws one, because opening a copy and resuming the original do not have the same consequences |
| `Save query`, `New saved query` | saving happens from the console, not from the library, which is read-only |

### The preferences screen `47:8222`, compared on 2026-09-15

The frame is named "Workflow / 09 · Preferences & plugins". It is the one that
revealed the wrong metric above, through its bar `47:8422`.

**The gap is structural, not cosmetic**: the mockup organizes settings
in **six tabs** — `Appearance`, `Editor`, `Results`, `AI providers`,
`Plugins`, `Diagnostics` — whereas Oxyn stacks them in one continuous vertical
list (appearance, reading comfort, format, model providers, saving
state).

What agrees: the three fields `Theme` / `Sidebar` / `Interface font` are
408 px, the control height 38 px, and the reading comfort block exists
on both sides.

Absent, and **not added** — each of these points is a feature:

| Element | What it would require |
|---|---|
| `Plugins` tab and its table (`47:8480`) | the wasmtime host, which [ADR-0005](adr/0005-wasm-plugins.md) places in phase 4 and whose entry condition — six native drivers shipped — is not met. An empty tab would be worse than no tab |
| `Diagnostics` tab and `Preview diagnostics` | a diagnostic report whose content and destination nothing describes — yet a crash report is one of the six channels of [I-03](../CLAUDE.md#i-03) |
| `Interface font` field | the choice of interface font; the theme carries `ui_family`, but nothing exposes or persists it |
| `Result page size` field | a page size setting; Oxyn currently fixes the preview at 200 rows (`PREVIEW_ROWS`) |

Accepted gap: the mockup carries `Save settings` and `Cancel`. Oxyn **saves
as you go** and announces it ("Preferences saved locally."). A save
button would let a setting be lost by closing the window, which an
immediate save makes impossible.

### The sidebar, compared on 2026-09-15

Capture of `221:4573` — the one used by the workspace frames, not to be
confused with `8:4` on page 01, which is the initial design. The two
are almost identical; `221:4573` adds `AI workspace`.

**The text the work plan called "Persistent workspace" does not
exist**: the mockup writes "Personal workspace" under the Oxyn brand, and it is
exactly what the code displays. Nothing to fix — the expected wording
was miscopied, and checking showed it.

Present under another name, and that is fine:

| Mockup | Oxyn |
|---|---|
| `Documentation` | `Workspace guide` |
| `Settings ⌘,` | `Settings · ⌘,` — and `⌘,` does open the panel, it is tested |
| `Query history` + `Saved queries`, two entries | `History & queries · ⌘⇧H`, a single one — the library carries both tabs |
| `Connections` + `+` | `CONNECTIONS` + `New connection` |

Absent, and **not added**: each of these points is a feature, not a
display adjustment. Inventing them would amount to deciding alone on flows that
neither UX-SPEC nor an ADR describes.

| Element | What it would require |
|---|---|
| `Search anything…  ⌘K` | a global search — objects, queries, history — whose scope and ranking nothing states. `⌘K` is bound to nothing today |
| `AI workspace` | a navigation entry to a full-screen AI panel; today the AI opens through `Ask AI` from the connection bar, conditioned on the connection's tier (ADR-0006) |
| `Favorites` | marking favorite objects, with its persistence |
| `3 connections · 1 active` | a counter: the information exists, but the mockup places it under the tree and Oxyn already states the session state elsewhere. To settle with the next point |
| Footer `Local workspace · Saved on this device` | Oxyn writes `Connected · Current session` in the same place, and the status bar already says `UTC · Saved locally`. Taking the mockup's text would **duplicate** that last notice |

### What the recovery frame still lacks

`Reconnect manually before inspecting` (`232:9539`, 280 × 32) does not exist in
the code. The button assumes a reconnection flow from the recovery screen
that nothing else describes — neither UX-SPEC nor an ADR. **Deliberately not
implemented**: inventing it would amount to deciding alone what it does with an
already open session, and on the screen whose whole point is to replay nothing.

## States in context

| State | Table | SQL console |
|---|---|---|
| Initial | `224:29197` | `226:5719` |
| Running, with cancellation | `224:29657` | `226:30837` |
| Empty | `224:30102` | `226:31227` |
| Non-retryable error in the displayed context | `224:30538` | `226:31615` |
| Missing capability | `224:30974` | `226:32003` |

These variants complement the abstract frames of page 07. They
represent no actually executed query.

## Editing and recovery

| Frame | Node | Status |
|---|---|---|
| Cell being edited, local change, addition and deletion | `231:7954` | Design outside the first read-only workspace |
| DML review without transactions | `231:8531` | Design outside the first read-only workspace |
| Conflict, draft kept | `232:8538` | Design outside the first read-only workspace |
| Recovery after a hard stop | `232:9100` | Draft selection, local offline restoration |
| Three items restored | `282:13489` | Drafts and object location, nothing executed |
| Empty start | `282:11550` | Offline workspace |

The recovery flow has eight selection combinations and their outcomes.
The checkboxes of the first column change the selection; the
`Restore N selected items` button announces its count and stays disabled when
the selection is empty. The vector checkboxes and their cells carry
the same prototype destinations.

The advanced engine views of page 05 and their copies in the prototype
carry a phase 4 label. This label does not prove the availability
of the driver or of the feature.

## Variables and components

The readability fixes of 2026-09-07 keep the collections,
color values and typographic styles that existed before. The four
Attachment states truncate the name on one line; absent data cells
use the `Kind=Null` variant and the `∅ NULL` label.

Frame `223:29197`, page 03, presents the names intended for the code. The color
aliases then took the values of the GPUI interface's `Palette`;
since its removal, the equivalent tokens are those of
`apps/desktop/src/styles.css`.
The dimension variables carry the names of `Metrics`; the workbench
measurements also have explicit names.

The states of the base components are on pages 10 to 20. `ReadOnly`
concerns value controls and cells; an unavailable button or tab
uses `Disabled`. Visible focus is a state of its own.

Pages 23 to 30 add reusable compositions, used in the
business screens:

| Page | Component | Use |
|---|---|---|
| 23 | Input Group | Search, filter conditions and AI input with an integrated action |
| 24 | Button Group | Neighboring actions and pagination |
| 25 | Item | Context sources and compact lists |
| 26 | Message | User, assistant and tool messages |
| 27 | Attachment | Attached context, with local removal in the prototype |
| 28 | Empty | Initial states and empty results |
| 29 | Alert | Errors, refusals and visible warnings |
| 30 | Accordion | Collapsible secondary details |

These compositions follow shadcn conventions and Oxyn variables. Production
and privacy warnings stay visible; a destructive
confirmation is not grouped with its cancellation.

For a complete inventory, read the local collections and variables through
`figma.variables.getLocalVariableCollectionsAsync()` and
`figma.variables.getLocalVariablesAsync()`. An extraction limited to one node
only returns the variables used in that subtree.

## Provider logos

Page 02 contains frame `244:1402` and the
`Brand / Provider / …` components. Fourteen logos were imported from
[SVGL](https://svgl.app/) through its
[source repository](https://github.com/pheralb/svgl/tree/main/static/library)
on 2026-09-07. Their geometries and brand colors are kept.
The light and dark versions follow the workspace's color mode.
Hugeicons remains the reference for actions and navigation.

Select and Combobox expose `Leading icon` and `Show leading icon`. Endpoints
that are merely API-compatible keep a neutral
identity: compatibility does not designate the effective provider.

## Texts and validation limits

### Fixes after analysis

| Subject | References |
|---|---|
| Production confirmation without a nested staging transaction | `47:6811`, copies `62:9534` and `63:26236` |
| Staging transaction panels separated on the canvas | `47:7046`, `62:9562`, `63:26264` |
| Consistent connection form state | `47:1137`, `62:8608`, `63:25091` |
| Grid taking the available height and export explicitly limited to the preview | the ten populated table variants of page 22 |
| Comfortable reading, wide dark / light | `303:13195`, `303:13955` |
| Comfortable reading, 1024 px dark / light | `303:14712`, `303:15336` |
| Readability choice in preferences | `47:8222` ↔ `305:3416`, and both pairs of the page 08 prototype |
| Compact action menu, light theme | `306:14765` |

The four table pairs are linked by the `Text · 13 px` /
`Text · 14 px` control. The preference is represented by navigating between variants,
without real saving in the prototype. The populated grids contain
32 demonstration rows to show the fill and scrolling; the
200-row counter remains scenario data, without an executed query.

The post-fix checks cover the captures of the modified screens,
the dimensions, the 24 selection transitions of the eight recovery states,
the seven restoration destinations and the absence of a restoration action
for the empty selection. The eight text size transitions are present.
The final check of pages 06, 08, 09 and 22 counts 1,178 links with no
missing destination. The 15 attachments of pages 04, 08 and 09 no longer
overflow and keep their 15 local removal actions.
These static checks are not a keyboard acceptance test in
the Figma viewer or in the application.

### Earlier checks and protected texts

The protected texts remain located in `86:4218`, `44:4045`, `190:1163`,
`47:6811`, `57:1547` and `91:3545`. Their meaning is not replaced by a change
of style or structure. The permanent markers follow
[UX-SPEC](UX-SPEC.md#permanent-landmarks).

Mockup checks cover the captures, dimensions, properties
and destinations of the Figma interactions. They do not validate the execution
of the application, the network, a real database or restoration after a crash.

Acceptance prior to these fixes: the Table → Structure → Constraints path
was clicked and visually checked in Figma Desktop. The static check
of pages 08, 09 and 22 then counted 1,099 links to 146 destinations, with no missing
reference. This is not an exhaustive interactive acceptance test. The eight
recovery selections are linked to their outcomes; their full acceptance in
the viewer remains to be done.

The check of pre-existing variables and typographic styles finds
no change. The texts of production reviews, linked filters and
permanent or ambiguous errors are kept, including in the slots
of composed components.
