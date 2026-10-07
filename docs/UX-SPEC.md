# Interface behaviors

> **Authority**: what the interface does in the situations where the behavior
> is decided, not guessed. This document does not talk about appearance.

Invariants concerned: [I-02](../CLAUDE.md#i-02), [I-05](../CLAUDE.md#i-05).

## States of a view

Every view that depends on a remote operation has **five** states, and all five
are designed before being coded. The one that gets forgotten is always the same
— the empty one — and it is the first a new user sees.

| State | What it shows |
|---|---|
| Initial | before any action; explains what to do |
| In progress | progress and **a way to cancel**; never a mere freeze |
| Populated | the result |
| Empty | a legitimately empty result; visibly distinct from an error |
| Error | what failed, whether it is retryable, and the next action |

## What updates by itself

A view that shows stale data without saying so is a silent defect: the user
reads an earlier state and does not know it. After an execution **that
succeeded**, on the connection concerned:

- a DDL makes the catalog explorer reload;
- a DDL or a write makes the **visible preview** reload, keeping the requested
  predicate, sort and page — the reload shows the same rows, up to date, not a
  return to the beginning;
- any execution makes the library reload, if it is open.

Nothing reloads after an **error**: a reload that follows a failure hides the
failure. Nothing reloads on another connection, nor on a tab that is not on
screen — it will reload when the user comes back to it. And the user's
statement is **never** replayed: what is re-issued is the preview read that Oxyn
composes, bounded and read-only.

`Refresh` stays, and keeps its meaning: force a reload even when nothing changed
on Oxyn's side — for example when someone else wrote to the database
([ADR-0022](adr/0022-rafraichissement-automatique.md)).

## What is never optimistic

Optimistic display — showing the result before the server confirms — is
forbidden for any operation that writes. It is acceptable for what is purely
local (collapsing a node, reordering tabs).

**Concrete failure:** the interface shows the row as updated, the server rejects
it because of a constraint, and the error message is missed. The user leaves
convinced their fix is saved.

## Destructive operations

A confirmation clicked by reflex protects nobody. On a connection marked
`production` ([SECURITY](SECURITY.md#connection-marking)), the
confirmation of a write or a DDL **names the connection** and shows the exact
SQL. The default button is never the destructive action.

A review only contains the operations of a single connection. A transaction
open on another connection has its own view, naming its connection and its
environment; its actions do not appear in the current confirmation.

The catalog's `Drop…`, `Truncate…` and `Rename…` apply this rule in an in-place
review, described in
[Destructive operations from the catalog](#destructive-operations-from-the-catalog).

## Cancellation

Every operation exceeding the 300 ms budget
([PERFORMANCE](PERFORMANCE.md#interaction-budgets)) is cancellable, and the
cancellation reaches the server
([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-it-exposes-cancellation-and-cancellation-really-cuts)).
A "Cancel" button that only abandons the display is a lie: it leaves a query
running and a connection taken.

**When the session does not declare `SERVER_SIDE_CANCEL`**, three outcomes
remain possible, and only one is acceptable:

| Outcome | Why it is rejected, or kept |
|---|---|
| Promise anyway | that is the lie above |
| Hide the button | the operation becomes unstoppable, which the 300 ms budget forbids |
| **Keep the button and withdraw the promise** | kept: cutting the stream remains useful, and the caveat says what the button does not do |

The caveat **accompanies** the button, it does not replace it, and it only
asserts what the missing flag proves: that no cancellation goes out to a
server. It does not conclude that the statement keeps running — SQLite does not
declare this capability for lack of a server, and `sqlite3_interrupt` does stop
the statement ([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-it-exposes-cancellation-and-cancellation-really-cuts)).

## What is exported is what is displayed

An export is only offered on a **whole** result. A buffer still open is refused
by `ExportOptions`; a buffer **closed but truncated** — stopped by the row limit
or by the memory budget ([I-06](../CLAUDE.md#i-06)) — is not, and that is the
dangerous case: it has finished loading, so nothing on screen distinguishes it
from a complete result.

**Concrete failure:** `SELECT * FROM commandes` over fifty million rows, the
buffer stops at two million, the user exports, and leaves with a CSV they
believe is the table. Nothing in the file says forty-eight million are missing.

A result whose columns share a name — `SELECT 1 AS id, 2 AS id`, or
`SELECT *` over a join — is **refused in JSON and JSON Lines**, before the
destination is touched. A JSON object identifies its fields by name, so one of
the two values would be lost to any ordinary reader. The refusal names the
shared columns and asks for distinct aliases. CSV, TSV and Arrow IPC keep every
column by position and stay available. Names are compared exactly: `id` and
`ID` are two keys.

A format the product cannot write yet is shown as **unavailable**, not absent:
offering it would make the write fail after the file is chosen, leaving an empty
file on disk; hiding it would suggest the product will never have it.

A table preview uses the label `Export preview…` and announces the number of
rows included. A complete result of the preview query limited to 200 rows can
be exported; it does not represent the whole table. An interrupted buffer, still
in progress or truncated by the receive budget, remains non-exportable. Moving
the action into a menu does not change this rule.

## Errors are addressed to a professional

Oxyn's audience reads PostgreSQL messages. An error message shows the server's
message — code included — and not a reassuring paraphrase. What is added around
it is what the server does not say: which connection, which query, whether it
is retryable.

What never appears in a message: a connection credential, a bound value
([I-03](../CLAUDE.md#i-03)).

## State lives in the workspace, and it is readable

A tab, an unexecuted query, a connection: what is restored at restart is written
in an open, documented format ([I-11](../CLAUDE.md#i-11)), readable without
Oxyn.

## Restoring after an abrupt stop

**An abnormal stop is observed, not guessed.** Oxyn records its shutdown when
it is clean, and leaves a heartbeat while it works. At the next launch, a
session left without a shutdown record and whose heartbeat has aged is an
abnormal stop; everything else is not
([ADR-0021](adr/0021-marqueur-d-arret.md)).

The recovery screen therefore does **not** appear after an ordinary shutdown,
even if working copies remain: the consoles open at shutdown come back **in
their window, offline**, without selection, without reconnection or execution,
like the editors resumed by "Selective restore at startup"; they also stay in
the library ([ADR-0043](adr/0043-multi-fenetre.md)). A screen shown at every startup stops being read, and it must be read on the day a
write was really interrupted.

At restart after an abnormal stop, Oxyn presents the recoverable local tabs and
drafts. The user chooses the items to restore or starts with an empty
workspace. Without a selection, the restore action is disabled. The selection
uses checkboxes, keyboard-accessible with their item name. The final button
announces the number of selected items; checking or unchecking an item does
not restore it immediately.
Restoring stays offline: it reconnects no session and re-runs no query. A
restored object tab keeps its location, without loading its data before an
explicit reconnection.

An interrupted write may have an unknown outcome. Recovery keeps this warning
and asks to inspect the server state after reconnecting; it never retries the
write ([I-13](../CLAUDE.md#i-13)).
The warning ends when the user marks the write reconciled, in the library's
History tab, on the "Needs inspection" row: a confirmation names the connection
and quotes the query, and its default button keeps the warning. A write still
in progress in this launch cannot be acknowledged: its outcome is not known.
The recovery screen, which precedes any connection, states in text where the
acknowledgment happens, without acknowledging anything itself. Only the human
acknowledges — an agent has inspected nothing, and the `PolicyGate` refuses
it. The acknowledgment is dated in the local state, touches no database and
replays nothing; the write remains viewable, marked "Reconciled".

## Updates

**An update never interrupts work.** Oxyn checks about a minute after launch,
then every 24 hours, on the stable channel only, and downloads in the
background ([ADR-0051](adr/0051-automatic-updates-from-github-releases.md)). A
downloaded update installs **when the user quits**; Oxyn never restarts on its
own. Until then the window shows `Update ready` in the status bar, or in the
home screen's title bar — no toast after the first one, no screen at startup.
After seven days, the indicator gains a border and the popover says how long
ago the update was downloaded; it adds nothing else. Below 1200 px it stays, as
an icon with its label for assistive technology: it is a pending action, not
secondary information.

**A background failure is silent; a requested one is not.** A failed automatic
check (no network, server error) shows nothing outside Settings ▸ Updates,
where the last result stays readable with its message. `Check for updates…` (in
the Oxyn menu on macOS, in Help on Windows and Linux) opens Settings ▸ Updates
and starts a check. That view has the five states of a view; the check and the
download can be cancelled, and cancelling stops the transfer. A failed
signature verification is always shown: the update is discarded, nothing is
installed, and the release page is offered so the user can download Oxyn
manually.

**`Restart now` is the ordered quit, followed by a relaunch.** With nothing
running, it does not ask. If a query is running or an export is in progress, a
confirmation names them, says the query is cancelled on the server and the
export stops and its destination keeps its previous content, and focuses
`Later`. An open transaction then holds the restart like the exit ("Windows"),
with `Cancel` focused; cancelling keeps the update for the next quit. The
drafts are written and the shutdown is recorded: the consoles come back in
their windows, offline. A failed installation is said once at the next launch,
and Settings keeps it.

**Settings ▸ Updates applies to the computer, not to the workspace**, and says
so. It shows the version, the last check and the switch `Download and install
updates automatically`. Turning it off stops a download and discards an update
that is not installed yet; `Check now` stays available. A failed save leaves
the setting applied for the session and offers `Save again`. Where Oxyn does
not install its own updates, the view says why and offers no action: `Updates
are managed by your package manager` (.deb and .rpm), `Updates are turned off
by your administrator`, `Updates are turned off in development builds`. A check
sends only the version, the operating system and the architecture.

**After an update**, the first window to open says once `Oxyn updated to
1.4.0`, with `What's new`, which opens the release page in the system browser.
Release notes are shown as plain text.

## Home screen

Before any connection, the screen carries a single-row title bar: the Oxyn
brand and its two title lines on the left, the window actions on the right —
resume the local working copies, and go back to the workspace left open when
there is one. These actions belong to the bar; none is overlaid on the screen,
where it would cover the brand. An action with no object — going back to a
workspace that does not exist — is not shown disabled: it is absent. The brand
is unique: a single logo image, never two variants side by side.

## Editing a saved connection

The edit form shows the non-secret parameters again, never a secret: a secret
field left empty keeps what the keyring holds, and says so ("Stored in the
keyring"). As soon as a non-secret parameter departs from the saved value, that
field stops claiming to be stored and announces that it must be entered again
("Connection settings changed — enter the password again"): the saved secret
will be forgotten on save
([SECURITY](SECURITY.md#a-secret-does-not-follow-its-connection-elsewhere)). Going
back to the saved value restores the stored state. The announcement happens
before saving, not after: the user who fixes a typo in the host learns they
lose the password at the moment they can still cancel.

## Common workspace structure

The structure chosen is the **dense workbench** of the Figma page
[22 · Database workspace](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=189-1163)
(decided on 2026-09-07, [ADR-0011](adr/0011-structure-commune-workspace.md)).
It combines a side explorer, connection context, object and console tabs,
object sub-tabs, work area and status bar. A side inspector can complete this
area. An 88 px page header is not added to this structure.

The sidebar stays **inset with collapse to an icon rail** on every screen, as
in the Figma page `01 · Sidebar`. Open at 280 px or collapsed at 64 px, it keeps
the same component, the same global navigation and a content adapted to the
connection. The work panel stays in its inset with an 8 px margin. The
workbench does not create a second family of sidebars and collapsing never
hides the navigation entirely.

The screens of Figma pages 04 to 09 reuse this common structure. Their titles
and subtitles identify the scenarios in the tabs and the context; they do not
prescribe a second navigation model. Page 01 defines the sidebar component and
page 22 the reference structure of the workbench. The repository's
authoritative documents prevail over any board for business behaviors.

### Permanent landmarks

The environment is shown in the top bar by an **outlined pill**, carrying the
full label `PRODUCTION` for production. The connection name stays visible next
to this landmark and in every write review; color alone never carries this
information.

The AI tier uses a common form in the top bars: `Metadata · Cloud` for a
remote provider at the Metadata tier, `Metadata · Local` for a provider
resolved on the machine. The suffix follows the **measured location** of the
destination that would receive the question — the one chosen in the panel,
otherwise the default one —, never an assumption. Without a usable
destination, for a provider whose address could not be resolved or for an
external agent, of which Oxyn cannot see where it sends, the label has **no
suffix**: `Metadata`. At the `Local` tier, which only admits a local provider,
the label stays `Local`: the suffix would repeat the tier. Outside the top bars,
in a list where nothing else says "AI", the badge keeps the form
`AI · Metadata`. The provider's name appears in the context and privacy
details, not in this label. The other tiers and the blocked states remain
explicitly distinguished; this presentation does not change the rules of
[AI-PROVIDERS](AI-PROVIDERS.md).

In the confirmation of a write in production, `Cancel` receives the visible
initial focus. The action that names the connection has a destructive style and
is not focused. Enter alone does not validate the write.

### Reduced width

Below **1200 px of window width**, the compact layout applies: sidebar collapsed
to **64 px**, row inspector closed and secondary actions grouped in a menu. The
grid content keeps its horizontal scrolling; no data column is removed.
`Columns` and `Export preview…` appear only in `Actions` at this width, with no
duplicate in the bar. `Read-only preview` stays on one line.

The `Data` and `Structure` sub-tabs stay visible; `Indexes`, `Constraints`,
`Relations` and `DDL` move into the `More` menu. The connection, the
environment and the AI tier stay visible in the top bar. The status bar keeps
connection and execution state; secondary information such as the time zone
and saving moves to the details. The update indicator is the exception: it
stays, as an icon with a tooltip and its full label for assistive technology,
because it is a pending action ("Updates").

The `Inspect row` action opens the inspector on demand as an overlay panel,
without shrinking the grid further. At normal width, the inspector's handle
moves with the mouse or with the left/right arrows once focused; Home restores
280 px. The bounds of 240 to 480 px are defined in ADR-0013.
The width is saved at the end of the drag, not at every frame. When going back to a width of at least
1200 px, the wide layout preferences are restored. Width changes trigger
neither a query nor a connection change.

### DDL inspection

The definition panel is read-only. It distinguishes cancellable loading,
available definition, error and missing capability; the stored/reconstructed
provenance and the limits stay visible. The Copy DDL button copies the displayed
text. Open DDL in console creates a separate document and keeps the existing
consoles; no execution is triggered by this preparation. A text kept after a
failed refresh is flagged as potentially stale, including when copied into a
console.

In the wide layout, the definition accompanies the metadata views in a panel
initially 424 px wide. Its 8 px handle works with the mouse and the keyboard
(320–640 px, Home: 424 px). This width stays within the open workspace. In the
compact layout, the DDL sub-tab presents the definition over the available
width; changing the window width does not re-run its read.

### Readability and grid height

The grid takes the available height; its status and its export scope stay at
the bottom of its area. The number of rows drawn on a board does not define a
limit of the application's viewport.

Two reading presets are offered in the preferences: `Compact` (main text
13 px, secondary mentions 11 px, data rows 24 px) and `Comfortable` (14 px,
12 px and 28 px). The choice is independent of the theme and of the window
width threshold; it re-runs no query. The result bar also allows changing the
text size. The choice is saved in the workspace with the theme and the panel
preferences, according to [ADR-0013](adr/0013-preferences-workspace.md). A save
error leaves the setting applied locally and shows an explicit retry action; it
never announces a successful save.

A missing value uses the `null` token and a marker, `∅ NULL` by default: the
missing value marker of Settings → Formats, a preference saved according to
[ADR-0013](adr/0013-preferences-workspace.md), replaces it in the grid cells
and in `Inspect full value`, for a result already shown as for the next one,
without running anything again. The marker is drawn only: the value stays an
absent value, and neither the SQL nor an exported file contains it. A string
containing the text `NULL` or the marker itself, or the SQL expression `NULL`
of a default value in Structure, keeps its text or code treatment.

AI context attachments keep their name on a single line, with a trailing
ellipsis if needed. The full name remains viewable in the attached sources and
the review of the outgoing context; removal stays local.

## Menus, shortcuts and gestures

The action registry behind these menus is decided by
[ADR-0041](adr/0041-registre-d-actions-menus-et-raccourcis.md); the review of
destructive operations by
[ADR-0042](adr/0042-revue-sur-place-des-operations-destructrices.md); the
windows by [ADR-0043](adr/0043-multi-fenetre.md). This section says what the
user sees; it redefines no behavior already written elsewhere in this document,
it points to it.

### One action, one label, one shortcut

Every action has **a single label and a single shortcut**, the same in the menu
bar, the right click, the palette and the shortcut sheet. A menu entry triggers
exactly what the button of the same name triggers: same command, same review,
same cancellation. A label that changes from one surface to another suggests
two actions; a displayed shortcut that is not the one that works is worse than
no shortcut.

The same key can carry two actions **in two different areas**: the area that
has the focus decides, the most specific winning over the global level (`⌘/`
and `⌘F`, below). Never two actions in the same area, and the displayed menu
label follows the active area.

An entry that applies to the surface but cannot be used now — nothing
selected, an execution in progress, an AI tier that refuses — is **greyed out,
with its reason**, readable on hover as with the keyboard. It is only absent in
the cases this document already fixes: a capability the source does not declare
("Filter, sort, page through", "Session context of a console"), no AI
destination declared ("The AI workspace only exists if it has been
configured"). One case specific to menus is added: destructive entries do not
exist for an agent, which has no menu to open and to which no tool offers them.

### Menu bar

On macOS, the menu bar is **native**. On Windows and Linux, it has the same
content, as the first row of the page, below the system title bar: `Alt`
reveals the mnemonics, `F10` gives it the focus, and below 1200 px — the
threshold of "Reduced width" — it collapses into a single menu button. `Ctrl`
replaces `⌘` there in all the shortcuts of this section, except `⌘⌥B`, which
becomes `Ctrl+Shift+B` there: `Ctrl+Alt` is the `AltGr` key there, and would
steal a character.

| Menu | Entries |
|---|---|
| `Oxyn` (macOS) | `About Oxyn`, `Check for updates…`, `Settings…` `⌘,`, `Hide Oxyn`, `Quit Oxyn` `⌘Q` |
| `File` | `New console` `⌘T`, `New window`, `New connection…`, `Open from library…`, `Open Recent ▸`, `Save` `⌘S`, `Save as…`, `Close tab` `⌘W`, `Export…`, and on Windows and Linux `Settings…` `Ctrl+,` then `Exit` `Ctrl+Q` |
| `Edit` | `Undo`, `Redo`, `Cut`, `Copy`, `Paste`, `Select All`, `Find` `⌘F` |
| `View` | `Toggle sidebar` `⌘B`, `Toggle side panel` `⌘⌥B`, `Assistant`, `Text size ▸`, `Enter full screen`, `Theme ▸` |
| `Query` | `Run` `⌘↵`, `Run all` `⌘⇧↵`, `Explain`, `Cancel` `Esc`, `Format` |
| `Window` | the open windows, and the system's window entries |
| `Help` | `Documentation`, `Keyboard shortcuts` `⌘/`, and on Windows and Linux `Check for updates…` |

What these entries do is written elsewhere, and the menu adds nothing to it:

- `Quit Oxyn` and `⌘Q` on macOS, `File ▸ Exit` and `Ctrl+Q` on Windows and
  Linux, do exactly what closing the last window does
  ([ADR-0038](adr/0038-un-plantage-s-annonce-une-fois.md)), for all windows at
  once: an open transaction is resolved first ("Windows", below), then the
  drafts are written, the shutdown is recorded, and the consoles are not closed
  one by one — they come back in their window, as "Restoring after an abrupt
  stop" says. The Dock's `Quit` and the macOS logout let neither the last
  keystrokes be written nor a transaction be resolved, which is rolled back;
  they record the shutdown
  ([ADR-0040](adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md));
- `Close tab` is the `⌘W` of "Independent consoles"; no entry closes the window
  with `⌘W`;
- `Undo` and `Redo` undo a local edit — text, layout —, never a write on the
  server, which cannot be undone afterwards ("What is never optimistic");
- `Find` searches in the area that has the focus: the editor, or the result;
- `Run`, `Run all`, `Explain` and `Cancel` are those of the console bar
  ("Scope of Run in a SQL console", "Explain", "Cancellation"): greyed out in
  the same cases as their buttons;
- `Text size` chooses one of the presets of "Readability and grid height", the
  same choice as the preferences and the result bar, saved the same way;
  `Theme` likewise chooses between `Light`, `Dark` and `System`, and the bar
  checks the choice in effect;
- `Settings…` is on Windows and Linux in `File`, just above `Exit` (decided on
  2026-09-25): these platforms have no application menu;
- `Assistant` opens the assistant in the side panel; with no AI destination
  declared, the entry does not exist;
- `Export…` follows "What is exported is what is displayed", including for a
  truncated result.

### Context menus

The right click opens the menu **of what is under the pointer**. Its entries
are actions of the registry: same label and same shortcut as elsewhere.

| Surface | Entries |
|---|---|
| Catalog | `Open data`, `View structure`, `View DDL`, `New console on this schema`, `Copy qualified name`, `Copy as ▸` (`Quoted name`, `SELECT *`, `INSERT template`, `DDL`), `Refresh this level`, `Collapse all`, `Pin to question`, then `Rename…`, `Truncate…`, `Drop…` |
| Saved connection | `Connect` or `Disconnect`, `New console`, `Refresh catalog`, `Edit…`, `Duplicate`, `Change environment…`, `Copy connection`, `Delete…` |
| Tab | `Close`, `Close others`, `Close to the right`, `Close all`, `Duplicate`, `Rename…`, `Open in new window`, `Reveal in library` |
| Grid cell or selection | `Copy value`, `Copy rows as ▸` (`TSV`, `CSV`, `JSON`, `Markdown`, `INSERT`, `IN list`), `Inspect full value`, `Filter by this value`, `Exclude this value`, `Is NULL`, `Sort ▸`, `Hide column`, `Open referenced row`, `Send to assistant` |
| Column header | `Sort ▸`, `Filter…`, `Hide`, `Freeze`, `Autosize`, `Copy name`, `Copy values` |
| SQL editor | `Cut`, `Copy`, `Paste`, `Run selection`, `Run statement`, `Explain`, `Format`, `Toggle comment`, `Open object under cursor`, `Ask assistant about selection` |
| Library | `Open`, `Rename…`, `Duplicate`, `Reveal in Finder` (`Reveal in Explorer` on Windows), `Copy path`, `Delete…` |
| Assistant, message | `Copy answer`, `Copy as Markdown`, `Regenerate answer`; on a question: `Edit question` |
| Assistant, code block | `Copy code`, `Open in console` |
| Assistant, mention | `Open object` |
| `erd` diagram | `Open table`, `Copy name`, `Re-layout`, `Export image…` |

What each surface guarantees:

- **Catalog.** Any SQL that a `Copy as` entry composes quotes its identifiers
  through the driver ([I-10](../CLAUDE.md#i-10)); it goes to the clipboard,
  nothing runs. `New console on this schema` opens a console whose session
  context is that schema, and only exists where that context exists ("Session
  context of a console"). The three destructive entries are described below.
- **Saved connection.** `Copy connection` never copies a secret
  ([I-03](../CLAUDE.md#i-03)): the clipboard is one of the six channels.
- **Tab.** Closing several tabs applies to each one the closing rule of a
  console: every console that asks for it is named, and `Cancel` stops the
  series. The middle click closes the tab; `⌘⇧T` reopens the last closed one,
  without running anything. `Duplicate` opens an independent console, with its
  own session.
- **Grid.** `Filter by this value`, `Exclude this value`, `Is NULL` and `Sort`
  go through the preview's filter and sort ("Filter, sort, page through"): the
  predicate Oxyn composes quotes the identifier through the driver and binds the
  value. On a console result, they are greyed out: the SQL written by the user
  is never rewritten. `Hide column` is the same action as `Columns` ("Columns
  and value inspection"). `Send to assistant` only transmits values through the
  approval screen of a sample, under `Sampled`; elsewhere it is greyed out, with
  the tier as the reason ([I-04](../CLAUDE.md#i-04)).
- **Column header.** `Copy values` says what it copies: "N loaded rows", never
  "the column" — the whole column is not in memory, and will not be for a copy
  ([I-06](../CLAUDE.md#i-06)). `Freeze` and `Autosize` are local, like
  visibility.
- **SQL editor.** `Run selection` and `Run statement` are the scope of `⌘↵`
  made explicit ("Scope of Run in a SQL console"); in a read-only editor, they
  do not exist, like the shortcut. `Open object under cursor` resolves the name
  against the catalog already read, like a mention.
- **Assistant.** A code block never offers `Run`: it becomes text in a console,
  and it is the user who runs it ([I-07](../CLAUDE.md#i-07), "A proposal is
  never executed by being one").

### Keyboard

| Shortcut | Effect |
|---|---|
| `⌘K` | the palette: all the actions of the registry, with their shortcut, the unavailable ones greyed out with their reason |
| `⌘P` | open an object by its name, among the objects already loaded — the scope is announced, as in the catalog search |
| `⌘/` | the shortcut sheet |
| `⌃Tab`, `⌃⇧Tab` | the next tab, the previous one ("Independent consoles") |
| `F6`, `⇧F6` | the next area, the previous one: catalog, editor, results, side panel |
| `⇧F10`, `Menu` key | the context menu of the focused element, like the right click |

`⌘J` (back to the editor), `⌘B` (sidebar) and `⌘2` (preview grid) keep the
meaning that "First workspace navigation" and "Filter, sort, page through" give
them; `⌘T`, `⌘W` and `⌘S` the one of "Independent consoles" and "Saving a
console". `⌘1` shows the catalog and `⌘⇧H` the library in the sidebar; `⌘,`
opens the settings. No `⌘1…9` selects a tab.

A shortcut that the SQL editor defines wins **only when the editor has the
focus**: `⌘/` comments the line there, `⌘F` searches the text there; elsewhere,
they keep their global meaning.

Shortcuts hold on a non-QWERTY layout, AZERTY included: what the menus, the
palette and the sheet display is the combination to press on the active
layout, not that of an American keyboard.

### Mouse and drag

- **Double-click** on a cell opens `Inspect full value`; on the edge of a column
  header, it gives the column back its default width. A catalog table already
  opens on click ("Data of a selected table").
- **Dragging** a catalog table to the editor inserts its qualified name, quoted
  by the driver; nothing runs.
- Tabs and columns are **reordered** by dragging. It is local: neither the SQL,
  nor the rows received, nor the column order of an export change, as for their
  visibility.
- **Dropping** a `.sql` file opens it in a console, without running it. A
  `.sqlite` or `.duckdb` file offers a connection, if a registered driver reads
  it; it starts in `production`, like any new connection ("First workspace
  navigation").
- `⇧` and the wheel scroll horizontally.

### What Oxyn does not do, because it is not a browser

The interface runs in a webview; nothing of the webview must show.

- **No page context menu** — `Reload`, `Inspect` and their neighbors exist
  nowhere. A text field without its own menu keeps `Cut`, `Copy` and `Paste`.
- **No reload**: `⌘R` does nothing. A reload would lose the window's unsaved
  state.
- **No page zoom**, neither with the keyboard nor by pinching: the text size
  goes through `View ▸ Text size`. The `erd` diagram keeps its own zoom,
  pinching included, which only affects the diagram.
- **No navigation history**: no key or gesture brings back to a "previous
  page". Going back from the connection screen to the workspace left open is a
  named action, not a history.
- **No spell checking**, no automatic capitalization and no typographic quotes
  in the fields: a table name silently corrected is a wrong name.
- **The text of the interface frame cannot be selected** — labels, tabs, bars.
  What is content stays selectable: SQL, DDL, values, error messages, assistant
  answers.
- **An external link opens in the system browser**, never in Oxyn's window.

### Destructive operations from the catalog

`Drop…`, `Truncate…` and `Rename…` open an **in-place review box** (ADR-0042,
which refines [ADR-0025](adr/0025-proposition-de-changement-de-schema.md)). It
applies "Destructive operations" and adds:

- the **full SQL** that will be sent, composed by Oxyn with its identifiers
  quoted;
- the connection and its environment, **named**;
- on a `production` connection, the button only activates once **the object's
  name is typed**; `Cancel` keeps the initial focus and Enter alone validates
  nothing ("Permanent landmarks"). The typed name protects against a mistake on
  the object; it grants nothing;
- `CASCADE` is **never checked by default**;
- the catalog's **known dependencies** are listed, and the list says it is
  limited to what is known;
- the box says whether, for this engine, DDL is **transactional** or not — that
  is, whether a failure can leave a half-applied state.

What the box submits then goes through the policy, like any SQL. On
`production`, the approval is given in Oxyn's **native dialog**, which names the
connection and quotes the statement
([ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)): no
second screen is inserted. Elsewhere, `Drop…` and `Truncate…` ask for approval
in the same box, and `Rename…` runs.

An entry the engine cannot perform is greyed out with its reason — on SQLite,
`Truncate…`; what is offered follows the capabilities declared by the driver
(ADR-0042). After a successful execution, the catalog reloads ("What updates by
itself"). A timeout is **not replayed**: the box says the server may have
applied it, and offers to refresh the catalog to check
([I-13](../CLAUDE.md#i-13)). These entries are never offered to an agent.

### Windows

`New window` opens an empty window; `Open in new window` moves the chosen tab
there (ADR-0043). **A console lives in a single window** at a time. The menu
bar targets the active window. The windows and their position are restored at
startup; the consoles follow the restore rules of this document ("Restoring
after an abrupt stop", "Selective restore at startup"), window by window,
offline, without reconnection or execution. A tab is not torn off its window
with the mouse: it changes window through `Open in new window`. Closing a
window closes its consoles, after the dialog that names those that ask for it:
a single one for the window, with `Cancel`, which has the focus, and
`Discard and close window`. A console to keep is saved beforehand. Closing the
last one quits Oxyn, on every platform, like `Quit Oxyn` (ADR-0043).

**An open transaction holds back the exit.** Before quitting, and before
closing a window, a dialog lists every console whose transaction is open, or
whose state is not known, with its connection and its environment
([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)). It offers
`Commit`, `Rollback` and `Cancel`; the focus is on `Cancel`, and Enter alone
validates nothing. `Commit` and `Rollback` are sent as if the user had typed
them in each console, and the exit only continues when every session says it
no longer has a transaction. A `Commit` refused by the server is shown and
holds back the exit; a `Commit` whose outcome is unknown is never replayed
([I-13](../CLAUDE.md#i-13)). `Cancel` leaves the application, its windows and
its transactions intact. Without an open transaction, no dialog opens.

## Columns and value inspection

`Columns` sets the local visibility of the columns, without changing their
Arrow indices, the rows received or the SQL. The menu announces the number of
visible columns and allows showing them all again. This visibility does not
project the export: the exported result keeps all its columns, which the menu
states explicitly. The control stays accessible in `Actions` at compact width.

The inspector follows the selected row and presents its fields read-only. The
arrows select a field; Enter opens its full value. Without a selected row, it
explains how to start. Reading a missing page uses the existing result, never a
new query.

`Inspect full value` opens a read-only view naming connection, column and row
number. The source representation is provided in pages of at most 16 KiB, at
UTF-8 boundaries; the counter shows the rendered text bytes, not the native
storage size. Previous and Next navigate this representation. A missing value,
an empty value and the text `NULL` remain visibly distinct. Closing during
loading cancels it; an old response does not reopen the view. The error state
keeps a way to close and explicitly resume the inspection, without a new SQL
execution.

## First workspace navigation

The sidebar follows the [Oxyn design](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=13-291).
It measures 280 px open and 64 px collapsed. Its collapse button and `⌘B` share
the same action; `⌘J` brings back to the SQL editor. The icons come from
Hugeicons Stroke Rounded. The light and dark variants share the same dimensions
and behaviors.

The tree presents the objects of the connection actually open. Expanding a node
requests the corresponding tier through the command bus; no demo table replaces
a missing response. The search covers the objects already loaded and announces
this scope. Unavailable features do not trigger a simulated query.

A **virtual table** (SQLite `CREATE VIRTUAL TABLE … USING <module>`) is not
listed as an ordinary table: its row carries `virtual · <module>`, the module's
name as the schema declares it. When the connection does not provide that
module, the mark becomes `<module> not loaded`, in the warning color, and its
hint says the rows cannot be read here. Availability is asked of the engine
(`pragma_module_list`), for every module alike: `fts5`, built in, shows as
virtual; a module an extension adds shows as loaded the day Oxyn loads it, with
no case written for it. When the engine cannot say, the mark stays
`virtual · <module>` and the hint says availability is unknown. The tables a
virtual table stores its data in — its **shadow tables** — carry `shadow`, and
their hint names the virtual table they belong to: SQLite's own word when the
module is loaded, the `<virtual table>_` naming convention only when it is not.
The module name comes from the database: it is shown as text and never joins a
query Oxyn composes.

The form only offers the registered drivers. Every new connection starts in
`production` until explicitly changed. During a connection change, the old
workspace stays accessible; a **new** connection copies the SQL text of the
active console into its first editor, announces it and does not run it, and the
original console keeps it. The connections opened during the session join the
list immediately.

**Open workspaces stay connected**
([ADR-0046](adr/0046-workspaces-retenus-restent-connectes.md)). Switching from A
to B closes nothing: A keeps its consoles, their texts and their edit history,
its results, its sessions, their transactions and their context. Choosing A
again on the home screen gives back its workspace as it was, without
reconnection and without running SQL. Only `Disconnect`, closing a console and
quitting Oxyn close sessions; `Disconnect` first writes the drafts, then closes
all the sessions of the connection and stops its conversations. A hidden
workspace receives no shortcut, no dialog, nor the text meant for "the active
console".

In the home screen list, a connection whose workspace is retained carries the
`Open` label. If one of its consoles reported an open transaction, or an
unknown state on a session that declares transactions (last value received,
[ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)), the row says so
**in plain words** and names the connection: "Transaction open in a console of
*billing*", "Transaction state unknown in a console of *billing*". Its locks are
held while the user works elsewhere; a color alone would not say it.

A window keeps **at most eight** workspaces. A ninth connection is refused
before any connection attempt — "8 connections are open in this window.
Disconnect one before opening another." — and no workspace is closed
automatically to make room for it: closing could roll back a transaction
without the dialog that announces it. An open connection cannot be deleted from
the settings; disconnect it first. Editing an open connection applies at once to
its workspace, visible or hidden.

The results of a hidden workspace are not pinned beyond the budgets of
[PERFORMANCE](PERFORMANCE.md#memory-budgets): a result evicted during the
absence reads "expired" on return; it is never re-executed.

The form's status bar names the connection being prepared and distinguishes
`Not tested`, the test result and the saving. It does not announce `Connected`
for an untested connection; another open session keeps its own explicit
context.

The preview of a confirmation keeps the full SQL and allows scrolling it with
the mouse as well as with the arrows, Page Up/Page Down and Home/End. Enter
alone never validates a write; Escape declines.

## Data of a selected table

Selecting a table or a SQL view opens the **Data** tab and reads at most
**200 rows**. The driver builds and quotes the qualified identifier; the bus
applies the connection's policy and enforces a read-only query. On PostgreSQL,
the catalog designates the session's database, while only the schema and the
table join the qualified SQL name.

This preview has its own grid and its own cancellation. It replaces neither the
SQL draft nor the editor's results. A new selection cancels the previous
preview; its late responses are ignored. **Refresh data** explicitly reloads
the rows; no automatic refresh follows an error. Loading, empty result, failure
and cancellation are distinct. Without a requested sort, the order of the rows
is not guaranteed, and the preview never counts the whole table.

A failed read of a virtual table whose module the connection does not load is
**explained**, not shown raw: "This table is provided by the SQLite extension
`<module>`, which Oxyn does not load. Its data is stored in …", followed by its
shadow tables as links that select them. The driver's message stays one click
away, under `Driver message`, word for word. The failure is permanent
([DRIVER-CONTRACT §4](DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)):
nothing offers to run it again, and nothing retries it. Any other failure of the
same table is shown as it came.

### Filter, sort, page through

The bar below the data actions carries a field preceded by `WHERE` and an
`Apply` button, then a `Sort` button (Figma `190:1618`). These controls only
exist for a source that declares it can filter or order a preview; elsewhere
they are absent, not disabled.

The `WHERE` field receives a **predicate the user writes**, sent as is: neither
parsed, nor rewritten, nor completed. An empty field filters nothing.

An invalid predicate fails in two ways, and the preview does not confuse them.
A predicate that Oxyn cannot read as a condition — `id >< 3` — is refused
**before anything is sent**: what is not classified counts as a write, and
this caution is what protects. The message then says it in those terms, and
points to the predicate's syntax; announcing it as a read-only defect would send
the user looking for a missing privilege where there is a typo. A well-formed
predicate that is wrong for this table — a column that does not exist — goes to
the server and comes back with **its** message, code included.

The preview can therefore now fail for a syntax reason, which was not the case
before. It can however write nothing: the final text is reclassified, the
session is held read-only by the server, and the row bound applies.

When a read that **changed** the filter, the sort or the page is refused, the
line under the bar says, in the warning color, that the rows of the previous
shape were not kept and that the text stays where it can be fixed. A first read,
or a refresh of the shape in force, that fails had no previous shape: that line
is not shown, and the failure below says all there is.

`Sort` chooses columns, not an expression: Oxyn composes this part of the
query, so it answers for it. A column unknown to the relation is refused before
sending.

The next page is a **new execution**, not a scroll: scrolling through the rows
already received never triggers a query. It is only offered if the order is
total — the requested sort, completed by a unique key that the catalog
declares. Without a known key, the preview stays on its first page and says
why: an `OFFSET` on an uncertain order would show the same row twice and omit
another one, without signaling anything. Between two pages, the server's data
may have changed; the preview does not claim to be a snapshot
([ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md)).

In this read-only preview, `Edit rows…` is disabled. A tooltip, also
keyboard-accessible through its help trigger, explains that editing requires an
editable view, a session allowing writes and the appropriate capabilities. A
write in production remains subject to the review defined above; the disabled
control offers no workaround.

**Structure** loads the columns through the existing catalog and shows them in
a virtualized list. `⌘2` gives the focus to the preview grid and Escape cancels
its loading. The session's capabilities and the nature of the object determine
the availability of Data.


## Local query browsing

The library distinguishes history, saved queries and references to recent
results. Opening this library and selecting a row replaces neither the draft
nor the result of the current console. The full text is shown separately,
read-only: navigation, selection and copying are possible; typing, pasting,
native composition and `⌘Enter` neither modify nor run this text.

Search filters apply before pagination. An old response does not replace a
more recent search or selection. Cancellation is explicit and stays distinct
from an empty list. A modified working copy can be viewed without modifying the
named copy. A result reference in the history does not prove its availability
after restart; reopening it must check retention without replaying the query. A
write without a certain outcome keeps its reconciliation warning.


## Scope of Run in a SQL console

`⌘Enter` runs the explicit selection when there is one; otherwise, only the
statement under the cursor is submitted. The position is computed in UTF-8
bytes from the editor's Unicode cursor. A trailing semicolon stays attached to
its statement; code following at the same location takes priority, but a
comment does not tip over to a write further on. Whitespace without a statement
sends no command.

PostgreSQL bodies between dollar quotes and SQLite triggers with their CASE/END
blocks stay whole. If the text is incomplete or its boundaries are ambiguous,
the console asks for an explicit selection. The splitting modifies neither the
document's text nor the classification and review rules. A multi-statement
selection keeps the limits declared by the session: it is not silently turned
into successive executions.
A read-only editor announces selection/copy, without an execution shortcut.

The console bar presents Run, Stop and Explain as three distinct controls. The
one that has nothing to do is dimmed **and** inert: Run during an execution,
Stop outside of one. A single button whose meaning toggles would be one missed
click away from restarting what the user wanted to stop. Stop reaches the real
cancellation, all the way to the server.

## Explain

Explain describes the current statement; it never runs it. The prefix is put on
a single statement: a batch refuses the scope, so does a statement that already
starts with `EXPLAIN`, and a dialect without a text plan says so rather than
sending a syntax the server would reject. `ANALYZE` is never added: it would
actually run the analyzed query, deletion included. The console's draft stays
the one the user wrote, and the plan comes back as an ordinary result, without
a write review.

Ordinary by its path, not by its presentation: **the result area says it is a
plan**. Above the rows, as long as they are displayed, an `Execution plan`
mention specifies that this is how the server would run the statement, and not
its data, and that the statement itself was not run. Someone who launches
Explain, steps away and comes back reads the grid, not the button they pressed;
without the mention, they would read a plan as data. The mention disappears at
the next launch that is not an Explain.

## Session context of a console

The bar's selector reads `<connection> / <schema>`: the name given by the user,
then the place where the session resolves the names a statement does not
qualify. It only exists for a source that can carry this context; elsewhere,
schemas are qualified in the SQL, and no disabled control suggests it is
possible.

As long as nothing has been declared, the bar says the server placed the
session at opening — it does not name a schema Oxyn did not ask for. The choice
crosses the network: meanwhile the bar announces the target, reminds where the
session **still** resolves, and offers a cancellation that reaches the server.
The display only changes on the response, and shows what the session reports,
never what was requested. A refusal shows the server's message as is, says that
nothing moved, and says whether it is worth retrying.

The context belongs to the console, like its session: switching tabs shows the
one of that tab, and a console does not inherit from its neighbor. Going back to
the server's default is always offered. The SQL written by the user is never
rewritten: the context changes what the server resolves, not the submitted text
([ADR-0019](adr/0019-contexte-de-session.md)).

The catalog explorer **does not follow** this context: it shows a qualified
tree. The console can therefore work in `analytics` while the sidebar shows
`public`; it is a visible gap, preferred to a catalog that moves without being
asked to.

### Open transaction

Next to the selector, the bar says whether the session holds an open
transaction, in text: `Transaction open`. On a session that declares
transactions and could not say — an abandoned execution, a session that is
closing —, it says `Transaction state unknown`: nothing asserts "no
transaction" without the session having observed it. Without an open
transaction, or on a session that cannot hold one, the bar says nothing: no
dimmed marker suggests a transaction is possible where it is not.

The state is the one the session reports at the end of each execution —
success, failure or stop —, never the one deduced from the submitted text: a
Stop or an error can close the transaction automatically. Nothing optimistic:
the display changes on that report, not on the submission of a `BEGIN` or a
`COMMIT`, and keeps the last observed value during an execution. Oxyn offers no
`Commit` or `Rollback` button: the user types `COMMIT` or `ROLLBACK`
([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)).

## Bound values of a console

A console carries its own bound values, shown by the `Parameters` control and
its panel. They only exist in the window: they are neither saved with the
query, nor written to the history or the log, nor copied into an error message,
and the input field refuses copying as well as native text extraction
([I-03](../CLAUDE.md#i-03)).

They go into the execution request, **never** into the SQL text: nothing is
concatenated or substituted, and the SQL written by the user stays intact
([I-10](../CLAUDE.md#i-10)). Each console has its own; opening a neighboring
console does not inherit them.

Each value declares its type. A new row is `NULL` and its input is closed until
a type is chosen. A value that its type cannot convert stops the submission
before anything is sent: the panel opens on the faulty row, and the message
names the position and the expected type, never the typed text. The product's
bounds are 128 values and 1 MiB of cumulative text.

## Independent consoles

A new console keeps the chosen connection and establishes its own session
before any execution. Opening is cancellable; a response that became obsolete
creates no tab and its session is released. Switching tabs keeps the text, the
result, the page reading, the confirmation and the export of each console. A
confirmation of a hidden tab stays attached to that tab.

`⌘T` opens a console, `Ctrl+Tab` and `Ctrl+Shift+Tab` go through the consoles,
and `⌘W` only targets the active console when the SQL panel is shown. Closing
an empty, idle console is immediate. If it contains unsaved SQL or an
operation, a dialog names the console, explains what will be abandoned and puts
the focus on Cancel. A console whose transaction is open, or whose state is
unknown on a session that declares transactions, does not close without this
dialog either: it then also names the connection and says the transaction will
be **rolled back**, its uncommitted writes lost — even if the text is saved.
Closing replays no query and
does not cancel the operations of another console. Closing the last one leaves
an empty state with the open action, while the catalog stays available.

The window also keeps the connection workspaces already open, connected
("First workspace navigation"). Selecting a connection already present makes
its workspace and its consoles visible, without replacing their texts. A copy
to a new connection is announced and runs nothing; the original console stays
available.


## Saving a console

Explicit saving covers the whole document and its name, not only the executed
selection. `⌘S` and the save button send the same local command. The name is
bounded to 256 UTF-8 bytes: a replacement that is too long is refused without
cutting the text or disturbing native composition.

The editor stays usable during a save. The acknowledgment covers the requested
text: more recent changes stay flagged as unsaved. Only one save is in flight
per console at a time, with a way to cancel it. If the store contains a
concurrent version, the new save must create another identity and preserve the
existing version.

The close dialog offers save then close, discard changes, or cancel. It waits
for the save acknowledgment then the close one before removing the tab.
Cancelling the save prevents its late response from closing the console. During
the local close, text and name stay readable but not editable; cancelling it
keeps the view if the write had not already finished. The named copy stays in
the library after changes are discarded.


## Opening from the library

Opening a copy names the destination connection before the action. It creates
an independent console and runs nothing. Its origin indication notably reminds
whether the text comes from an agent history entry. A write whose outcome
requires a reconciliation stays viewable but does not offer this editable
opening.

Resuming a saved query on its original connection opens the working text and
keeps its identity and its storage language. It does not replace a draft with
the named copy: if the console already exists, it becomes visible with its
current changes. On another connection, the user opens a copy or first selects
the original connection to resume the document. A cancelled session opening
creates no late console.

A storage conflict or an unreadable copy during saving is handled without
overwriting the existing document. Closing a console in conflict leaves its
stored record intact; saving under a new identity keeps both versions.


## Draft autosave

Text and name changes save the working copy locally, without modifying the
named copy and without running SQL. An empty working name stays recoverable;
only saving a named copy requires a name. A new prefilled copy — from History,
a saved query or the previous connection — is saved like an edit, without
waiting for it to be modified. An
edit out of bounds is flagged as unsaved, and a response about an earlier state
cannot announce that state as recovered.

The last pending draft replaces the intermediate drafts. Named saves keep their
snapshot, and closing keeps its own cancellation. If a close is cancelled before
it is committed, its pending draft takes its place again. If it completes, no
old write reopens the document. A conflict with a revision modified elsewhere
stops the queue and keeps the editor's text for a separate copy decision.


## Selective restore at startup

When open working copies are available, the recovery screen presents them in
pages with selection checkboxes. The selection does not load all the bodies:
only the displayed document is read. Continuing without restoring leaves the
stored copies intact. It is possible to go back to the selection to add other
documents without replacing the editors already resumed.

The resumed editors are offline: text, name, edit undo, autosave and named save
work, while execution waits for a connection. Choosing the connection is a
separate gesture. The original connection resumes the document; another one
produces a copy and keeps the initial document. Attaching does not launch the
SQL and keeps the editor entity. Cancelling the connection choice leaves the
draft offline.

The screen **says what it observed**. When the shutdown marker reveals an
abandoned session, it announces that Oxyn did not close normally; opened on
demand from the library, it does not announce it — that would assert a crash
that did not happen ([ADR-0021](adr/0021-marqueur-d-arret.md)). It reminds,
conditionally, that an interrupted write requires an inspection of the server,
and replays nothing to perform that inspection.


## Retained results

The history and Recent results allow opening a buffer that is still retained.
This opening establishes no session and submits no SQL. The grid and the export
use the existing result, including its pages spilled to disk. An incomplete,
truncated result, or one associated with an uncertain outcome, stays viewable
but cannot be presented as a complete export.

An expired reference shows an explicit unavailability. Going back to the
library does not cut an export in progress; a result being exported must be
finished or cancelled before its view is replaced or closed. Retention without
a reader is bounded by ADR-0017, and does not imply keeping it across restarts.
No evicted result is recreated by replaying the query.

## The AI workspace only exists if it has been configured

As long as no destination is declared — neither provider, nor external agent —,
`Ask AI` **is not shown**, no privacy badge appears, and nothing suggests that
a feature is missing. An external agent declared alone is enough to make the
entry exist. Oxyn is a complete client without AI (ADR-0006). Access to the
configuration goes through the settings, never through a call to action in the
connection bar: a greyed-out button that invites to configure is an
advertisement, not a feature.

The first saved destination makes the entry appear **without a restart**, and
removing the last one makes it disappear — closing the panel if it was open.
This happens **without a new variant of the execution bus**: the view that
writes the declaration knows the fate of its write and reloads the list itself.
Publishing an event there would do what [ADR-0022](adr/0022-rafraichissement-automatique.md)
refuses — a second place where one must remember to publish
([ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md)).

### The tier is read before speaking, not after

The privacy badge and `Ask AI` sit next to each other in the connection bar and
are read together. The badge announces the tier of **the current connection**
— never an application setting — and it changes when the connection changes,
even with a conversation open. This tier is set in the connection form, field
`AI privacy`, which shows what would go out before anything is saved; its
safeguards are written in
[AI-PROVIDERS](AI-PROVIDERS.md#setting-the-tier).

On a connection at `Local` where no declared provider is local, `Ask AI` stays
**visible and disabled**, with the reason: this tier only admits a provider
resolved on the machine, and an external agent cannot serve it either, since
it cannot be known where it sends
([AI-PROVIDERS](AI-PROVIDERS.md#an-external-agent-the-reach-is-not-unknown-it-is-unknowable)).
Making the entry disappear would read as a defect, and ADR-0006 requires that
an unavailable feature explain itself. The same goes for a session that does
not speak SQL: the assistant only writes SQL, and the entry says so.

### Who answers is chosen in the panel

The panel header carries a selector, "Who answers", which arranges the
destinations in two groups: the providers, then the external agents. A
destination that the connection's tier refuses stays there, disabled, with its
reason. Without a choice from the user, the question goes to the **first usable
provider** in declaration order, and to an agent only if there is none: a
provider is the only destination whose scope Oxyn can check (ADR-0026). The
choice holds as long as the destination stays offered; once removed, the
question falls back on this default.

The selector is inert while an answer is being written. Changing destination
between two questions does not transmit the previous exchanges: the panel says
so with a line (`destinationChanged`), since the earlier answers were not those
of the new recipient.

A chosen agent comes with two permanent mentions: "Oxyn cannot see where it
sends the question", and, for an agent that Oxyn does not confine, that it
**cannot prevent it from launching commands or modifying files on the machine
on its own** ([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)). For
a confined agent, the agent's mode selector does not exist: Oxyn keeps it in
its strictest mode, and a dangerous mode must not be selectable on screen. A
non-confined agent keeps the modes and options it declares.

An endpoint that Oxyn could not resolve counts as remote. The configuration
shows it as "unresolved", not as "local": doubt does not benefit sending.

### The agent is chosen per conversation

The panel header carries a second selector, the **agent** — SQL, Schema,
and the agents the user added
([ADR-0049](adr/0049-agents-declared-as-markdown-files.md), accepted). It
lists only the agents offered for the connection's dialect and the chosen
destination (`ai_list_agents`); a new conversation starts with the SQL
agent.

* **Choosing another agent opens a new conversation.** The current one
  stays in the "Conversations" list, unchanged, with its agent: an agent's
  prompt never changes under a written history. The selector is inert while
  an answer is being written.
* **A user agent is marked as such** in the selector, next to its name
  ("user agent"), and its description says it comes from the user's
  `agents/` folder. A shipped agent carries no mark.
* **An invalid user agent is listed, not hidden**: greyed out, not
  selectable, with the error the backend returns — file name and line,
  never the file's content. One invalid file does not remove the others
  from the list.
* **A resumed conversation runs with its recorded agent.** When that agent
  no longer exists — its file deleted or now invalid —, the conversation
  reopens with the SQL agent, and a line above the next question says so,
  naming the missing agent ("The agent “Name” no longer exists; this
  conversation continues with the SQL agent."). The panel never lets a
  conversation run under another prompt without saying it.
* Changing **destination** in "Who answers" keeps the conversation and its
  agent; what changes is the part of the prompt written for the recipient,
  and the `destinationChanged` line already says the new recipient starts
  without the previous exchanges. A destination the conversation's agent is
  not offered for (its `recipients` excludes it) stays in "Who answers",
  disabled, with its reason, like a destination the tier refuses.

### The panel shows what is happening, including when nothing arrives

The panel follows the five states of a view. What distinguishes them here:

* **in progress**: the answer is written as the stream flows, and the
  cancellation stays offered the whole time — not only between two turns;
* **catalog reading**: when Oxyn reads from the server the structure the
  question needs and the cache does not have
  ([AI-PROVIDERS](AI-PROVIDERS.md#what-the-ai-sees-of-the-schema-and-when-it-is-read)),
  the exchange shows the step, a spinner and "Reading the catalog…
  metadata only, no row is read." — before the first read, never when nothing
  is missing. It closes **in place** on its summary: objects described, lists
  read, and, when relevant, the stop at the deadline or with the question, the
  failed reads, and what is not loaded yet, with "The assistant was told." A
  step, not a `status` region: the panel announces its state once, elsewhere.
  Five seconds at most, and cancellation stops it like the rest of the
  question. `describe_schema` and `refresh_catalog` show the same step under
  their call;
* **tool call**: each command requested by the agent is shown **before** its
  result, with its name and the targeted connection. An agent that works in
  silence for eight turns is indistinguishable from a stuck agent;
* **error**: what the user reads is the server's whole message; what the model
  received may be less, depending on the tier, and the panel says so when the
  two differ. Hiding the gap would make a poorly informed answer look like a
  wrong answer;
* **cut-off answer**: an answer stopped by the provider's token ceiling says so
  on its last line. A truncated answer that does not announce it reads as a
  wrong answer, and the user corrects a model that had not finished speaking;
* **turn ceiling reached**: said as such, with the number of turns. It is
  neither a success nor a failure.

**What "the cancellation stays offered" means exactly.** The button stays
active and the request is taken into account immediately; its *effect* depends
on the provider. The loop rereads the request between two stream events and
never interrupts a read in progress: a future dropped in the middle of a frame
would leave the decoder out of sync, and the repository refuses that risk
(`.claude/rules/rust.md`, § Async). If the provider goes silent, the
cancellation waits for it to speak again. It is the `LlmProvider` contract that
makes it effective — every provider honors the cancellation token in its
stream — and a provider that did not would make the cancellation inoperative
without anything else signaling it.

### The rows of an agent query are read under its call

When an agent — internal provider or external agent through MCP — runs
`execute_query` and the command finishes, **the user sees the returned rows**
under the tool call, in a compact grid. The model, for its part, only receives
their shape — number of rows and batches, truncation — and nothing of what the
grid shows ([ADR-0006](adr/0006-ai-privacy-tiers.md),
[ADR-0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) §4); the grid's
footer recalls it ("shown to you only; the model got the count").

* **Same path as the console.** The `ResultId` retained by the scheduler comes
  up in the end-of-call event, never in what goes to the model. The grid opens
  the result through `OpenRetainedResult`, which checks that it belongs to the
  conversation's connection, then reads pages bounded by `read_result_page`;
  the cells are formatted in Rust. A result produced on a connection other than
  the conversation's is not offered.
* **Bounded.** The grid reaches at most the first 100 rows, shows eight before
  scrolling, and says "First 100 rows of N" when there are more.
  `Open all rows` opens the whole result in a result tab of the workspace — the
  retained buffer, **never a re-execution**. This tab does not offer opening the
  statement in a console: it only enters one with its provenance, from the
  agent's answer.
* **States.** Reading in progress; populated; **empty** ("The query returned no
  rows.", without an alert); error (the backend's whole message, `Try again`
  only if it is declared retryable); **expired** ("Result no longer
  available", without offering anything that re-runs the query).
* **Chart.** The footer offers `Chart`, which toggles between the grid and a
  chart, when the columns include at least one numeric column (integer, float,
  decimal) and something to draw it with: a date (`Date*`, `Timestamp`) or a
  category (text, boolean) for the axis; failing that, two numeric columns, or
  a single row. Otherwise the button **does not exist**. Four series at most.
  The chart reads the same bounded page as the grid (100 rows at most). The
  numbers are read back from the cells formatted by Rust, and only if they are
  unambiguous (digits, decimal point, exponent, thousands separator U+00A0); a
  column that contains anything else — a cut value, `NaN`, text — is left out
  **and named** under the chart. If none remains, it says so ("Nothing to
  chart").
  * **Shapes.** The whole shadcn charts gallery is offered: area (simple,
    stacked, 100 % stacked, step, gradient), bars (vertical, horizontal,
    grouped, stacked, with values), line (straight, curved, step, with dots),
    sectors (pie, donut, donut with total), radar, radial bars; plus two shapes
    outside the gallery written in its style, the scatter plot and the key
    figure. A **single** dropdown menu, which starts on `Auto`: closed, it only
    shows the drawn shape (icon of its family and name, `Auto · …` when Oxyn
    chose); open, it lists `Auto` then the possible shapes, grouped by family.
  * **Refusals.** A shape the rows would draw badly **is not offered**, and a
    family with no possible shape disappears with them. Refused: a sector
    or a radial bar on dates, with more than one series, a single category or
    more than five (the palette has five colors), a `NULL` or negative value,
    and for the sector a zero value or a repeated category; a line or an area
    on categories (it would draw a trend they do not have) or on a single
    point; a stack with a single series, a negative or `NULL` value, and at
    100 % a row summing to zero; vertical bars with several series (those are
    grouped bars) and grouped ones with a single series; values written on more
    than twelve bars; a radar outside 3 to 8 categories, with a `NULL` or a
    negative; a scatter without two numeric columns or two rows carrying both;
    a key figure on more than one row; any axis shape when there is no axis.
    The user's choice only holds as long as the rows allow it; otherwise `Auto`
    takes over.
  * **Auto**, in this order: a single row gives **key figures**, Rust's exact
    text; without an axis, two numbers give a **scatter**; on dates, one series
    gives an **area**, several give **lines**; on categories, a positive series
    of 2 to 5 distinct parts gives a **donut**, more than twelve categories or
    a label longer than fourteen characters give **horizontal bars**, otherwise
    **vertical bars**, grouped if there are several series. The radar is never
    chosen alone: it puts all the columns on the same radial scale, and the
    columns of a query rarely share a unit.
  * **Order and computation.** The rows are drawn in the order returned by the
    query, except the sector, read from the largest part to the smallest.
    Nothing is aggregated in the webview, with three exceptions, all specific
    to the chosen shape and stated in the caption: the donut's total (the sum
    of the at most five parts it shows), the parts of a 100 % stack, and the
    top of a stack, which is a sum.
  * **Accessibility.** `accessibilityLayer` on each chart, a legend as soon as
    there are several series or parts, a figure caption that says what is
    drawn, and the current shape announced ("Drawn as Donut, chosen by Oxyn").
    The colors are the fixed palette `--chart-1` to `--chart-5`, with a
    contrast of at least 3:1 against the background in both themes (WCAG
    1.4.11); no data enters the keys or the colors.

**Lifetime.** An agent's result is a result retained without a reader: it lives
until retention evicts it according to the bounds of
[ADR-0017](adr/0017-retention-resultats.md) — at most 16 results without a
reader, the oldest leaving first —, until the conversation is deleted — which
releases the results of its calls —, or until the end of the process. Nothing
of it is written to the workspace file. A conversation reopened from the
workspace therefore shows no grid: under a call that had returned rows, it says
"Result no longer available: the workspace keeps the statement, never its
rows".

### A sample is approved column by column, in a single screen

Under `Sampled`, row values go to an agent — provider or external agent — only
after the user has checked, in the approval screen, the columns that go out.
The screen opens in two ways, and it is **the same one**
([ADR-0034](adr/0034-echantillon-pour-toute-destination.md),
[AI-PROVIDERS](AI-PROVIDERS.md#approved-sample)):

* **the user pins** an object ("Pin to question" in the catalog menu), then
  sends their question: the screen opens before sending;
* **the agent asks** (`request_sample`) while it answers: the screen opens
  above the panel, and the call waits. Title "An agent asks for a row
  sample", then, before any other line, **who** asks: "Claude Code
  asked for it, not you, and waits for your answer. Cancel declines." Only
  the columns the agent named are offered — all of them if it names none.

What does not change depending on the trigger, and what the stories hold:

* **nothing is checked** at opening, **nothing checks everything**, nothing is
  remembered for the next time;
* **Cancel has the focus**, comes first at every width, and **Enter sends
  nothing**;
* the button says what goes out and where: "Send 2 of 4 columns to Claude Code · to an
  unresolved address";
* **no value** appears in the screen: it approves columns, not a preview;
* outside `Sampled`, the screen only shows the refusal, without a checkbox or a
  send button.

Selecting columns asks the backend to open the native confirmation of
[ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md), naming
the recorded recipient and the approved read before any row is read. Refusing
that dialog consumes the request; a pinned question is not sent. If another
native confirmation is already open, the request remains available and the
selection screen allows another decision after reporting the error.

**An agent's request closes by itself** when it is decided, expires (five
minutes) or the question stops: the agent then receives "the user declined". A
user's pin goes first: the agent's request waits behind it. The tool call reads
in the thread like the others; its report only states counters ("sent 5 rows
of 1 column you approved"), never a column name or a value.

### Naming an object with `@`

In the question field, `@` opens above the field the list of the connection's
objects: tables, views, collections, columns (`table.colonne`) and saved
queries of this connection or of none. What follows the `@` filters it. It
comes from the local catalog and the workspace library, **never from a
model**: it is a name completion, a deterministic result
(`.claude/rules/ia.md`). It must answer within 100 ms at most
([PERFORMANCE](PERFORMANCE.md)); it is bounded to twelve objects.

* the list opens **above** the field, and flips below only if space is lacking
  there; it keeps 8 px from the edge, never exceeds the field's width or the
  window, and its height is `min(18rem, available space)`, with internal
  scrolling. The active row stays visible when the keyboard moves it. The
  stories hold it at 360 px wide and in a short window;
* **a single active row**: the mouse and the keyboard move the same highlight,
  without a distinct hover style;
* the columns (`table.colonne`) come from the fields the catalog search has
  already found (`matchedFields`), three per relation at most. The list loads
  no description to offer some: what is cached is shown first, the rest
  completes;
* each row carries the icon of its kind and, on the right, its schema — enough
  to tell two `orders` apart;
* **a lone `@` already shows objects**: those of the catalog tree that the
  sidebar has read (the same cache), in tree order, without the system
  objects, then the most recent saved queries. The panel prepares these two
  sources when it opens; the first `@` waits for neither the backend nor a
  delay, and the library's 60 ms wait only applies between two keystrokes;
* **the first `@` on a connection lists what the tree has not expanded**: the
  tables and views of the schemas never listed, within the bounds of a
  question ([AI-PROVIDERS](AI-PROVIDERS.md#what-the-ai-sees-of-the-schema-and-when-it-is-read)), then the
  list rereads the cache. Meanwhile, what is already known stays displayed,
  marked as completing; a failure is not announced and leaves what is loaded.
  Opening the panel, for its part, reads nothing on the server;
* the list has its states: a skeleton "Loading…" row as long as no response has
  arrived, "No matching object" **only** when a response said so, "Type to
  search tables, views, columns and queries" when the tree is not readable, and
  the backend's error as it arrives. During a search, what is displayed stays
  displayed: the list does not flicker at every letter;
* the focus **stays in the field**: the arrows move the highlight, which the
  field designates through `aria-activedescendant`;
* **Enter or Tab chooses the object and sends nothing**; **Escape closes the
  list and stops nothing** — it is the next Escape, list closed, that stops an
  answer in progress. The stories hold these two;
* the chosen object becomes a **chip** `@orders` in the text. Backspace erases
  it whole: a label without its address does not exist;
* the chip reads **like a word**: at the text size, on its baseline, centered
  within 1 px on the neighboring text, never taller than the line. Inserting or
  erasing a chip changes the height of no line, the caret stays level on either
  side, and a chip at the end of a line moves whole to the next one. The
  stories measure it, in the field as in the thread, in light and dark.

The chip carries an **address** — the catalog's, and the column name if there
is one —, never a text the backend would reparse. The question keeps the
readable label; the backend checks the address against its cache. Sixteen
objects at most per question.

On sending, the named objects are **imposed** on the context, before what the
question makes the search find, for **every** destination
([AI-PROVIDERS](AI-PROVIDERS.md#mentions)). Naming is not sending values: the
sample pin above stays a separate gesture. The header says, after the question,
what did not go out: "1 mentioned object(s) not found", "2
mentioned object(s) named only, over the budget".

In the thread, the sent question shows its mentions with **the same chip** as
the field: same component, same icon per kind, same colors. The chips come from
what the question **carries**, saved with it — never from rereading the text:
an `@maison` typed by hand stays text. A table, view, collection or column chip
is a button that opens the object. A reread conversation rechecks each address
against the cache: an object the cache knows to be gone is shown dimmed, with
"not found", and opens nothing. Without proof of its disappearance, a chip
stays normal. The label comes from the catalog: it is hostile input, rendered
as text.

### An `erd` block is drawn from the catalog, not from the answer

A fenced code block of language `erd` in an answer lists table names, one per
line, possibly qualified `schema.table`, using the connection dialect's
identifier quoting (for example `"Ventes"."T1.totaux"` for PostgreSQL/SQLite,
or `` `Ventes`.`T1.totaux` `` for MySQL/MariaDB). Once the block is **closed**, the panel draws it as a
diagram; while the answer is being written, it stays text — a diagram redrawn
at every arriving name would read as a defect.

* **Names are requests, not facts.** Each one is looked up by `search_catalog`
  among the objects Oxyn has already read: the exact spelling first, then
  case-insensitive if a single table matches. A name that designates two tables
  is not decided ("matches auth.users, public.users: not drawn until
  qualified"); a name that cannot be found is **said**, never drawn from its
  spelling ("Not found among the objects Oxyn has read"). The search does not
  introspect: a table of a schema never expanded is not found, and the message
  hints at it.
* **What is drawn comes from the backend.** Columns, primary key and foreign
  key columns are read by the same commands as the object view
  (`relation_facets`, then `refresh_relation_facet` for a facet never read);
  the edges are the foreign keys. To the named tables are added their **direct
  neighbors** through a key, outgoing or incoming, and the keys between two
  drawn tables. No model is consulted, and the same catalog gives the same
  diagram (dagre layout, left to right: the table that carries the key before
  the one it references).
* **Bounded.** A block is read up to 20 names, a diagram stops at 40 tables and
  a table shows 12 columns, keys first; each surplus is counted and said. At
  most four reads are in flight at once.
* **Navigable.** Pan with the mouse, zoom with the buttons, pinching or the
  keyboard (arrows, `+`, `-`, `0` to fit) when the diagram has the focus. The
  wheel scrolls the conversation, not the diagram. A click on a table name
  opens it in the workspace, as from the catalog; nothing runs. The keys are
  also written as a list for a screen reader.
* **States.** Reading in progress; drawn; nothing to draw (all names not
  found); error — the backend's whole message, without paraphrase, with
  `Try again`, which rereads without writing anything. A metadata read that
  would require an approval is refused and said, as in the object view.
* `Show source` shows the block's text as the model wrote it.

For a model to know how to emit this block, the system prompt must describe it;
without that, it only appears if the user asks for it.

### The code and diagrams of an answer stay text all the way

A model answer is hostile input
([SECURITY](SECURITY.md#input-surface)): nothing it writes becomes markup in
the page.

* **Highlighting.** A **closed** code block is highlighted by shiki, rendered
  as React tokens (`<span>` and text) — never as HTML. Languages are loaded on
  demand, at the first block that names them: SQL and its dialects, JSON,
  JavaScript, TypeScript, JSX, TSX, Python, shell, YAML, TOML, XML/HTML, diff,
  Rust, Go; a block without a language is read as SQL. Another language, a
  block of more than 50,000 characters or a grammar that fails stay plain
  text. The colors are those of the SQL editor, as CSS variables: they follow
  the light or dark theme. While the answer is being written, the open block
  stays plain text; once highlighted, a block is not recomputed (memoized by
  content). `Copy code` and `Open in console` are unchanged.
* **mermaid.** A **closed** `mermaid` block is drawn; mermaid is only loaded at
  that moment. The drawing is shown through an image
  (`<img src="data:image/svg+xml;base64,…">`), which runs nothing, loads
  nothing and receives no event. mermaid runs with `securityLevel:
  "strict"`, `htmlLabels: false`, without automatic rendering, with 20,000
  characters and 500 edges at most; these settings are declared `secure`,
  which the diagram cannot modify. Its configuration lines — `%%{init}%%`
  directives and `---` header — are removed before rendering, and the block
  says so. `Show source` shows the text. An invalid diagram shows mermaid's
  error, readable, with its source. The drawing follows the application's theme
  at render time, and is redrawn if it changes.

### What the panel shows of an external agent

An external agent works with its own tools, which are not Oxyn `Command`s. The
panel shows what it can of them **without copying anything from the
machine**:

* the text of the answer and the reasoning the agent streams;
* each step of its own tools, reduced to **its kind** — read, edit, search… —
  and to its state: pending, in progress, completed, failed. The title the
  agent composes for the step is never shown, nor its content: it can quote a
  path of the machine. A kind the agent does not specify is shown as "tool",
  never as an error. The step is drawn as the agent's work, distinct from an
  Oxyn tool call, which went through the bus. When the agent calls an Oxyn
  tool, its adapter also announces the call as a step: this step is not drawn,
  since Oxyn's card already shows the call and is authoritative. It is
  recognized by an exact name among the tools announced to the agent, read in
  structured fields and never in the title. An unrecognized step stays drawn:
  hiding one that is not Oxyn's would hide what the agent did
  ([RESEARCH-NOTES](RESEARCH-NOTES.md#what-acp-adapters-say-about-an-mcp-call--re-read-on-2026-09-24)).
  A call that Oxyn refuses before the bus therefore has its own trace: the line
  "refused before the bus, nothing ran", with the reason in Oxyn's words, never
  the agent's. This applies to a connection that became `local`, an unreadable
  tier, the answer's call ceiling reached, or a call Oxyn could not translate.
  The tool is only named there if it is one of those announced to the agent.
  Without a question in progress, nothing is shown, and nothing runs;
* its **plan**, replaced whole at each send and never merged with the previous
  one: the protocol sends neither a diff nor a step identifier. An unknown
  priority is shown as "medium", an unknown state as "pending" — never
  "completed" on an assumption;
* the occupancy of its context window, and its cost when it gives it;
* each request to act on the machine that Oxyn refused, without a button to
  grant it
  ([AI-PROVIDERS](AI-PROVIDERS.md#what-the-agent-itself-asks-of-the-machine)).

The user messages echoed back by the agent, its available commands and the
session information are not relayed; a variant this version does not know is
nothing, rather than something other than what it is.

**This refines [ADR-0026](adr/0026-agents-externes-acp.md)**, which only
relayed text fragments on the grounds that a plan or a tool step displayed "as
its own" would suggest work authorized by Oxyn. The risk is held otherwise: the
marking distinguishes the agent's work from Oxyn's, and nothing it shows can
quote the machine. Hiding it fell back into the state this document refuses: an
agent that works in silence is indistinguishable from a stuck agent.

### A conversation stays in the workspace, not what was shown to it

A conversation belongs to its connection. Closing the connection stops what its
conversations were running and removes them from the window; they stay **in the
workspace file** (tables `ai_conversations`, `ai_conversation_turns` and
`ai_conversation_nodes`) and are reopened from the panel's "Conversations"
list, which shows the 64 most recent ones of the connection. At the next
launch, the panel opens on a new conversation; the previous ones wait in this
list.

What is kept, exchange by exchange: the question, the destination (label and
model, never a key), the privacy tier **under which this exchange took place**,
and its outcome; for the answer, the text the user read, the reasoning, a
**rendering** of each Oxyn tool call — name, SQL statement, outcome, displayed
message — and the declared tokens. The sibling versions created by a
regeneration or an edit are kept with their tree.

What never is, for lack of a column to write it in: the arguments of a tool
call, a query result, a bound value, a key. The steps and the plan of an
external agent are not kept either.

**An exchange that received a row sample only keeps its question**, its
counters — rows and columns, never their names — and its outcome. Neither the
answer, nor the reasoning, nor a tool call, nor an error message: each can
quote the values sent, and the workspace file is one of the six channels of
[I-03](../CLAUDE.md#i-03). The file refuses it itself, and marking the exchange
erases what had already been written for it
([AI-PROVIDERS](AI-PROVIDERS.md#approved-sample)). Reopened, this exchange
announces it instead of showing a blank.

**Reopening does not give the model its memory back.** The conversation is
shown in full, but the next question starts from a fresh context, and a line
says so (`restarted`): what had been sent to the model was not written, and a
replayed transcript without the tool results would make it believe in a
context that is no longer there. The kept reasoning stays on disk and is not
shown on reopening.

Deleting a conversation removes it from the file with all its exchanges, in the
same transaction; what went out to a recipient stays recorded in `ai_egress`
([SECURITY](SECURITY.md#what-goes-out-to-an-ai-recipient-leaves-a-trace)).
Deleting a connection does not erase its conversations, which keep the
connection's name: what the user asked does not disappear with the tool that
was used to ask it. The connection's delete box **says so beforehand**: its
conversations stay in the workspace, under its name, and stay listed,
read-only, in the assistant panel. If writing a conversation fails, the
question goes out anyway, and the panel says this thread is only kept in the
window.

At launch, Oxyn prunes the assistant history to its budget
([PERFORMANCE](PERFORMANCE.md): 200 conversations, 90 days of inactivity,
32 MiB of transcript). When pruning has removed at least one conversation, the
"Conversations" list says so, **for the whole launch**, with a sober note above
the threads: how many were removed, and the rule, with the numbers the backend
applied — never a copy written in the interface. A thread gone without a word
reads as a defect; a thread removed by a stated rule reads as the rule. The
note offers no action: what is pruned is deleted, and nothing brings it back.
With nothing pruned, nothing is shown.

The conversations of a deleted connection are listed in the "Conversations"
list, under a collapsed section
**From deleted connections**, which only exists if at least one remains. Each
row gives the title, the name the connection had, the date of last activity
and the number of exchanges. The list is **read-only**: no row opens, is
renamed or is deleted, and nothing from it goes to a model — the connection
that gave these exchanges their privacy tier no longer exists. It covers the
whole workspace, 64 conversations at most, and is reread each time the history
is shown. An identifier from this list does not allow deleting the thread on
behalf of another connection: the deletion carries the connection in its
condition. These conversations remain subject to the launch pruning, like the
others. The content of the exchanges cannot be reread there yet, and the
assistant panel only exists on an open connection: with no connection at all,
the list is not accessible.

### A proposal is never executed by being one

A query proposed by an agent arrives in a console as **text**, and nothing
more. It is run by the user, with the same button and the same path as what
they write themselves. There is no "automatic execution" mode, and `Explain` on
a proposal follows the same rule: `EXPLAIN ANALYZE` actually runs what it
analyzes.

When an agent submits a command itself, it goes through the `PolicyGate` with
`Actor::Agent`. A write on a `production` connection is **refused** to it, not
submitted to confirmation: a confirmation ends up being clicked (I-02). The
user sees the refusal in the conversation, with its reason.

### What an agent wrote stays marked

A document whose text comes from an agent proposal carries its provenance: the
agent, the model and the date. The mark is visible on the tab and in the
library.

Two rules govern it, and the second is the one that gets broken without
noticing:

* **it appears as soon as an agent writes**, including in a document the user
  had started themselves;
* **it never disappears afterwards.** Rewriting the whole text by hand does not
  erase it, and an ordinary autosave — which knows nothing of the text's origin
  — does not erase it either. A missing provenance means "nothing new to
  write", never "nobody".

What the mark asserts is therefore "an agent wrote this text", not "it wrote it
entirely" nor "it is the last one to have touched it". Claiming the latter
would require following every keystroke, which Oxyn does not do and does not
have to do.

It does not follow a copy-paste: the clipboard carries no metadata. A text
copied by hand into another document therefore arrives there without a mark,
and it is an accepted limit of ADR-0023, not a defect to work around.

### Provider and agent configuration

The AI screen of the settings declares two kinds of destinations and shows them
in **one** list: the providers, with their key, and the external agents,
with optional environment secrets.

A provider is declared by its family, its endpoint, its model and, if the
endpoint requires it, a key. The key goes to the system keyring; it is neither
shown again, nor copied into an error message, nor exported with the workspace
(I-03). What the screen shows after saving is "configured" or "missing", never
the value.

**The default model is chosen in the provider's own list, or typed.** As soon
as the form knows the family, the endpoint and — unless the endpoint is local
or the declaration being edited keeps its stored key — a key, it asks the
provider for its models, after a pause in typing. The list is the provider's
and in the provider's order: Oxyn carries no list of models and invents no
"recommended" one. The field is a searchable combobox — on the model's name,
its id and the provider's name; arrows, Enter to choose, Escape to close — that
shows the name, and the id beneath it when they differ. What is saved is the
id.

* **Typing is never blocked**: whatever the list says, "Use "…" as model id"
  keeps what was typed — a custom gateway, a private model, an Azure
  deployment are declared with the id the user knows. It comes first, so
  Enter keeps "vega" rather than the listed model it happens to match; only a
  listed id typed exactly comes first instead. Leaving the field — to save,
  for instance — keeps a typed id that was never picked; leaving it emptied
  keeps the current model.
* **The current model is never replaced in silence**: a model the provider no
  longer lists stays selected, first in the list, marked "Current model —
  unavailable from provider".
* The field has its states: waiting for the endpoint and key, listing, empty
  list, failure. A failure says why in one sentence — key refused, endpoint
  unreachable, endpoint that does not list its models — then the provider's
  short message, never the key nor the URL's credentials. "Refresh models"
  asks again past the backend's cache.
* Changing the family, the endpoint or the key drops the list shown; an answer
  that arrives for an endpoint typed before is ignored, never shown for the
  one typed after.
* "Test connection" lists the models afresh and says "Connected — N models
  available" or "Connection failed — …" next to it, announced politely. It
  saves nothing.

The key typed in the form travels inside the listing request and nowhere else:
not in a log, not in a cache key — the backend caches a list by family and
endpoint only.

The screen shows the local/remote classification **with the time of its
measurement**, because it is recomputed and never persisted: an endpoint
classified local yesterday may resolve elsewhere today. A URL carrying
credentials in its authority is refused on input, not silently cleaned.

The providers are shared by all workspaces; the privacy tier, for its part,
stays attached to each connection (ADR-0023). The same goes for external
agents.

**An external agent is declared in two ways**, and neither is declared without
the user wanting it:

* **through a preset**, for the agents Oxyn knows — Claude Code and Codex
  ([AI-PROVIDERS](AI-PROVIDERS.md#an-external-agent-the-reach-is-not-unknown-it-is-unknowable)).
  **Each time the screen opens**, Oxyn looks in the documented installation
  locations for the launcher, `node` and the agent's program. It is a **disk
  read**, off the interface thread: nothing is launched, nothing is saved. An
  agent installed while Oxyn is running is found through "Detect again". The
  screen shows the proposed command, with the pinned version of the adapter,
  what was found or not, and the `PATH` variable added so that an Oxyn
  launched from the Finder finds Node — never a token. Without a launcher
  found, declaring is impossible: it would save an agent that cannot start.
  The agent's authentication command is shown alongside; it is the agent that
  signs its user in, Oxyn sees no credential;
* **by hand**, through a program, its arguments and its environment, launched
  without a shell. The arguments are typed **one per line**, as the program
  receives them: no quotes, no splitting on spaces
  ([ADR-0026](adr/0026-agents-externes-acp.md)). The environment is typed as
  one `NAME=value` line per variable; a line without a valid name refuses the
  save rather than being ignored. The values, which are often tokens, go out
  once and leave the screen immediately, whether the save succeeds or not; the
  list only shows their names. Each variable has a secret toggle; token-like
  names are always secret. Help states that secrets go to the system keychain
  and other values are stored in clear in the local SQLite state.

In both cases, declaring requires **two confirmations**: the screen's button,
then a **native** dialog box that copies the exact command, environment
included, and recalls that Oxyn cannot see where the agent sends the questions.
Native, because a script injected into the webview can call the declaration
command but not click a window it does not draw
([ADR-0026](adr/0026-agents-externes-acp.md)).

An agent declared **exactly** as the preset proposes — same adapter, same
version, no extra argument — is confined at launch
([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)). Any other agent,
modified preset included, is treated as unknown: the list marks it with a
warning saying Oxyn cannot prevent it from acting alone on the machine, and the
panel repeats it when it is chosen.
