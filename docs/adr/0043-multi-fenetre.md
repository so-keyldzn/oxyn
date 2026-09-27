# ADR-0043 — Several windows in a single process, each owning its consoles and its sessions

**Status:** proposed · **Date:** 2026-09-25

**Clarifies:** [ADR-0015](0015-consoles-independantes.md), on the following
point: a session belongs to a window, and two windows open on the same
connection share no session, including that of the catalog and the preview.

**Clarifies:** [ADR-0013](0013-preferences-workspace.md), on the following
point: `object_location` becomes a window state, and layout settings only
apply to the other windows at their next opening.

**Clarifies:** [ADR-0021](0021-marqueur-d-arret.md), on three points: the
clean shutdown waits for the drafts of **all** windows to be flushed; after an
ordinary closing, the open working copies no longer remain only "accessible
through the library": they come back, offline, in their window — the recovery
screen, for its part, stays reserved for abnormal shutdown; and an open
transaction holds back the orderly shutdown for the time of a decision ("Open
transaction on exit").

**Clarifies:** [ADR-0038](0038-un-plantage-s-annonce-une-fois.md), on the
following point: `⌘Q` still triggers the orderly shutdown, but a step precedes
it, which the user can cancel when a transaction is open. Oxyn's Quit, its
shortcut and the orderly shutdown itself do not change.

**Clarifies:** [ADR-0039](0039-etat-de-transaction-d-une-session.md)
(proposed), which left closings outside the console out of its scope and added
no `Commit` or `Rollback`: quitting and closing a window offer them, on the
state it makes observed. This part of the decision assumes ADR-0039 accepted.

**Clarifies:** [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md),
on one point: an exit it records rolls back open transactions, and the log
says so.

**Clarifies:** [ADR-0029](0029-interface-tauri-shadcn.md), on the following
point: the webview's capability covers windows through a label pattern, with
unchanged permissions.

## Context

The user put multi-window in the V1 scope on 2026-09-25. The concrete need is
to work on two screens at once: a `staging` connection on the left,
`production` on the right, or two consoles of the same database side by side.

**The code assumes a single webview everywhere.** Surveyed on 2026-09-25:

| Where | What assumes a single window | What a second window would cause |
|---|---|---|
| `crates/oxyn-desktop/tauri.conf.json` | a `main` window, 1280 × 820, minimum 960 × 600, `titleBarStyle: Overlay`, `hiddenTitle` | — |
| `capabilities/main.json` | `"windows": ["main"]` | a window with another label has **no** permission: neither moving by the bar, nor file picker |
| `commands.rs`, `subscribe_events`; `commands/metadata.rs`, `subscribe_refresh_signals` | a **process-static** generation counter (`commands/subscriptions.rs`): every new subscription ends the previous one, so that a reload leaves no orphan task | the second window that subscribes **silently cuts** the first one's execution stream and catalog signals |
| `backend/recovery.rs`, `LocalWork::shutdown` | a single `Option<Channel<ShutdownSignal>>` | the draft flush request only reaches the last subscribed window; the others lose their last 250 milliseconds of typing ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)) |
| `commands/recovery.rs`, `on_run_event` | every `CloseRequested` is held back and launches the orderly shutdown, then `app.exit(0)`; Oxyn's Quit (`oxyn-quit`, [ADR-0038](0038-un-plantage-s-annonce-une-fois.md)) and an unexpected `ExitRequested` lead there too; `RunEvent::Exit` records the closing of an exit macOS does not let us hold back ([ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) | closing **any** window quits the application |
| `backend/settings/location.rs`, `write_object_location` | a single `object_location` in the workspace preferences, carrying its connection; `workspace-screen.tsx` reopens it as a location when the window opens that connection | two windows that navigate overwrite each other's saved object tab; both reopen the same one at the next launch |
| `backend/confirm.rs`, `NativeDialog` and `Confirmations` ([ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md)) | the critical dialog has no parent window, and only one is open for the whole process | — (correct with several windows, provided it says which one it answers: see "Closing") |
| `backend.rs`, `disconnect` | `Command::Disconnect { connection }` closes all the connection's sessions, and `release_agents(connection)` all its agents | a window that switches connection closes the sessions and stops the agents of another window open on the same database |
| `backend/ai/conversation.rs`, `ai_open_thread` | a live conversation receives the `Channel` of whoever opens it, and replaces the previous one (`threads.rs`, `ThreadState::channel`) | opening the same conversation in a second window steals its stream |
| `apps/desktop/src/features/session.ts` | `open`, `restored`, `recoveryOffered`: a single session state per webview; the reload key in `sessionStorage` | each window believes it is the only one to have offered recovery |
| `features/consoles/draft-registry.ts` | `flushAllDrafts` flushes the drafts **of its** webview | — (correct per window, provided each window is called) |

None of these assumptions fails loudly. A second window opened without a
decision would produce a silent first window, then a closing that quits
everything.

**What Tauri allows, checked in the sources** of the `Cargo.lock` versions —
`tauri` 2.11.5, `tauri-utils` 2.9.3, `tauri-runtime-wry` 2.11.4 —, unpacked in
the local registry, on 2026-09-25 ([I-12](../../CLAUDE.md#i-12)):

- a capability's `windows` field accepts a glob pattern, `admin-*` for example
  (`tauri-utils`, `src/acl/capability.rs`, doc of `Capability::windows`);
- a command can receive the `Webview` or the `WebviewWindow` that invokes it:
  the identity comes from the runtime, not from the content sent by the
  JavaScript (`tauri`, implementations of `CommandArg` in `src/webview/mod.rs`
  and `src/webview/webview_window.rs`);
- a `Channel` delivers to the webview that created it, and to it alone
  (`src/ipc/channel.rs`, `from_callback_fn(webview, …)`);
- the menu event listener is global: it is called "for any menu event, whether
  it is coming from this window, another window or from the tray icon menu"
  (`src/webview/webview_window.rs`, doc of `on_menu_event`);
- creating a window "deadlocks when used in a synchronous command or event
  handlers" on Windows (same file, doc of `WebviewWindowBuilder::new`);
- when the last window is destroyed, the runtime emits
  `RunEvent::ExitRequested { code: None }`, which the application can prevent
  (`tauri-runtime-wry`, `src/lib.rs`, handling of
  `TaoWindowEvent::Destroyed`); `RunEvent::Reopen` only exists on macOS
  (`tauri`, `src/app.rs`);
- a window declared with `"create": false` serves as a template for
  `WebviewWindowBuilder::from_config`, under another label (`tauri-utils`,
  `src/config.rs`, field `WindowConfig::create`).

These facts are recorded in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#menus-windows-and-destructive-ddl--check-of-2026-09-25).

Two decisions written in parallel bound this one: ADR-0041 (action registry;
native menu bar in Rust on macOS, web bar in each window on Windows and Linux)
and ADR-0042 (in-place destructive review, in the view that requested it). Four
others, from the same day, set what it does not change: the native dialog of
critical decisions
([ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md)), the
shutdown paths ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md),
[ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) and the
transaction state of a console
([ADR-0039](0039-etat-de-transaction-d-une-session.md), proposed).

## Decision

### One process, several windows, one identity per window

- All windows live in the `oxyn-desktop` process, on the same `Backend`, the
  same `Executor` and the same Tokio runtime.
- A window has a persistent identity, `WindowKey` (a UUID), declared in
  `crates/oxyn-desktop/src/backend/windows.rs`. Its Tauri label derives from
  it: `workspace-<uuid without dashes>`. **The front end never chooses a
  label**: no IPC command receives one.
- The `main` window of `tauri.conf.json` becomes a template, label `workspace`
  and `"create": false`. All windows, the first included, are built by
  `WebviewWindowBuilder::from_config` under their `workspace-…` label:
  dimensions, minimum, `titleBarStyle` and `hiddenTitle` stay written in a
  single place.
- **At most 16 windows.** It is a guard bound against a repeated gesture or a
  compromised webview, not a measurement: the memory cost of an additional
  webview is not measured. The 17th request is refused with a message that
  says so.
- `Backend` holds a registry, `WindowRegistry`, the only source of truth on
  what a window owns. An IPC command that targets a console, a session, a
  command in progress, a result or a conversation receives the calling
  `Webview`; `commands/` derives the `WindowKey` from it, and `Backend`
  refuses what this window does not own ("This console belongs to another
  window"). Workspace-level commands — drivers, saved connections,
  preferences, library, history, AI settings — have no owner.

### What a window owns, and what is shared

**A console belongs to a single window at a time.** Likewise an assistant
conversation, the external agent that serves it, a destructive review in
progress (ADR-0042), a pending confirmation, an export under way.

**A connection open in two windows is two independent connection
workspaces.** Each establishes its own sessions through the bus: the session
reserved for the catalog and the preview, then one per console
([ADR-0015](0015-consoles-independantes.md)). No session is shared between
windows, for the reason that ruled out sharing between consoles: a commit, a
transaction rollback or a session context change
([ADR-0019](0019-contexte-de-session.md)) decided in one window cannot happen
in the other.

Shared, because they belong to the workspace and not to a view: the
connection's saved configuration, **its environment marking and its privacy
tier** ([I-02](../../CLAUDE.md#i-02), [I-04](../../CLAUDE.md#i-04) — the tier
stays attached to the connection, never to the window), the catalog cache, the
preferences, the library, the history, the retained results
([ADR-0017](0017-retention-resultats.md)), the provider and agent
declarations.

Consequences on existing paths:

- **A window no longer emits `Command::Disconnect`.** It closes its own
  sessions through `Command::CloseSession` and stops its own conversations.
  `Backend` emits `Disconnect` itself when the last window holding the
  connection releases it: it sees all windows, a window sees only one.
  `release_agents` becomes per window.
- **Deleting a saved connection** that another window holds is refused: "This
  connection is open in another window. Close it there first."
- **Modifying a connection** — marking, tier, read-only — applies immediately
  to all windows: the `PolicyGate` already reads the configuration saved per
  connection. Each window that holds it receives
  `WindowSignal::ConnectionChanged` and rereads what it displays; a stale tier
  marker in a window would be a false marker.
- **Opening what another window owns** — a document from the library, a
  conversation from the list — neither duplicates nor steals it: the owning
  window comes to the front, on that tab. `ai_open_thread` no longer replaces
  the `Channel` of a conversation another window owns.

### Moving a console

V1 offers one moving gesture: **`Open in new window`**, on a tab.
`New window` opens an empty window, on the connection choice screen; no
connection is opened automatically. The shortcuts of these actions are held by
ADR-0041's registry.

Moving a console **transfers its session**, without closing or reopening it:

1. the source window flushes the console's draft (autosave,
   [ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md));
2. `Backend` creates the window, records the console, its session and its
   result in the target's name, then prepares a `ConsoleHandoff`: document
   identifier, connection, session, result reference, bound values;
3. the target adopts the `ConsoleHandoff`, declares itself reader of the
   result, **then** the source releases its view: in between, the result always
   has a reader, and [ADR-0017](0017-retention-resultats.md)'s retention does
   not evict it;
4. the target establishes its own catalog session. The console is usable
   immediately: its session followed it.

Nothing is re-executed: the grid rereads the pages of the same `ResultBuffer`
([I-06](../../CLAUDE.md#i-06)).

Moving is **refused** — the entry is greyed out, with its reason — as long as
the console has an execution in progress, a pending confirmation, an open
destructive review (ADR-0042) or an export under way. A command's outcome
comes back as the reply to the `invoke` of the webview that emitted it: moved
in flight, this outcome would arrive in a window that no longer holds the
console.

**Bound values follow the console, in memory only.** The `ConsoleHandoff`
does not derive `Debug`, is never written, and is consumed once; if the target
closes before adopting it, it is abandoned with them
([I-03](../../CLAUDE.md#i-03)). **The editor's undo history does not
follow**: it lives in the source's CodeMirror instance.

A moved object tab reopens the same location in the target, which loads it as
a selection would: it is a read through the bus, not a transfer.

**Outside V1**: tearing off a tab by dragging it out of its window, and moving
a console to an **existing** window. The second gesture reuses the same
`ConsoleHandoff` and changes no format; it only requires a target-choice
interface, which the current implementation makes tricky — a window holds
only one connection at a time there (`session.ts`, `open`).

### Actions target the focused window

- `Backend` follows focus through `WindowEvent::Focused`:
  `WindowRegistry::focused` is the last window to have received it, and
  becomes empty again when it closes.
- **On macOS**, the menu bar is global to the process (ADR-0041). A
  window-scoped action is sent to the window that has focus **at the moment
  the menu event is received**, through its own channel
  (`WindowSignal::Action`), never to the others. Application-scoped actions —
  `New window`, `Settings…`, `Quit Oxyn` — have no target. Without a focused
  window, window-scoped actions are greyed out.
- **The enabled or greyed state follows focus.** Each webview declares the
  state of its actions to `Backend` when it changes (`report_action_state`,
  identifiers of ADR-0041's registry; an unknown identifier is refused).
  `Backend` keeps each window's state and only applies to the native menu that
  of the focused window, at each focus change and each declaration by it.
- **The window that receives an action rechecks it** before executing it, and
  ignores one that is no longer available. The menu state can be one
  declaration behind a focus change; it is never conclusive.
- **On Windows and Linux**, the bar lives in each webview: the action leaves
  from the window that triggers it, and no routing is needed.

### IPC: nothing is broadcast

- **One subscription per window and per stream.** `subscribe_events`,
  `subscribe_refresh_signals`, `subscribe_shutdown` and the new
  `subscribe_window` are indexed by `WindowKey`. The logic of
  `commands/subscriptions.rs` is kept, but its generation counter becomes
  **per window**: a window's reload ends its previous subscription, and that
  one alone.
- **An execution event goes to the window that owns the command.** The
  registry records the owner of a `CommandId` **before** dispatch, so that no
  event precedes it; a window's subscription task only forwards the events of
  its commands. The filter is in Rust: a window never receives another's
  stream to sort it itself. An agent command's owner is the window of its
  conversation.
- **A catalog signal goes to the windows that hold the connection**, not to
  the others. A change of preferences or of a saved connection goes to each
  window, on its own channel, as a notice
  (`WindowSignal::PreferencesChanged { revision }`,
  `WindowSignal::ConnectionChanged`); the window rereads through the bus.
- **The six emission methods of `tauri::Emitter`** — `emit`, `emit_to`,
  `emit_filter` and their `emit_str*` variants, surveyed in `src/lib.rs` of
  `tauri` 2.11.5 on 2026-09-25 — **are forbidden** in `clippy.toml`: they are
  the broadcast path, and the front end does not have the permission to listen
  to an event anyway (`capabilities/main.json` grants nothing of
  `core:event`).
- Every new message — `invoke` reply or `Channel` message: `WindowSignal`,
  `ConsoleHandoff` as seen from the front end, window state — has its schema
  in `apps/desktop/src/lib/ipc/` and is validated on entry, by `call` or by
  the `Channel` guard of `client.ts`
  ([ADR-0031](0031-validation-des-reponses-ipc.md)). In the other direction,
  `commands/` parses what a webview declares before handing it to `Backend`.

### Capabilities: a pattern, not a permission

The `capabilities/main.json` file keeps **exactly** its three permissions —
`core:window:allow-start-dragging`, `core:window:allow-internal-toggle-maximize`,
`dialog:allow-open` — and its `windows` field becomes `["workspace-*"]`. No
window or webview creation permission is granted to the webview: **a window
only opens through an Oxyn command**, executed in Rust, bounded to 16, and
reviewed like any IPC surface change
([SECURITY](../SECURITY.md#input-surface), surface 5). An XSS in a window
can therefore at worst open windows up to the bound; it gains no webview API.

### Persistence: two tables of the workspace file

A SQLite migration of the store — number 18 if no other precedes it: 17, from
[ADR-0038](0038-un-plantage-s-annonce-une-fois.md), is the last one on
2026-09-25 — adds two tables, readable with any SQLite client and without Oxyn
([I-11](../../CLAUDE.md#i-11)) — columns rather than JSON, because the
ownership constraint must be held by the file itself:

```sql
CREATE TABLE workspace_windows (
    id               TEXT PRIMARY KEY NOT NULL,  -- WindowKey
    workspace_id     TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    app_session_id   TEXT NOT NULL,              -- last launch that wrote it
    ordinal          INTEGER NOT NULL,           -- restore order
    x                REAL,                       -- logical pixels; NULL = system placement
    y                REAL,
    width            REAL NOT NULL,
    height           REAL NOT NULL,
    maximized        INTEGER NOT NULL,
    object_location  TEXT,                       -- same shape as in the preferences
    active_document  TEXT REFERENCES documents(id) ON DELETE SET NULL,
    revision         INTEGER NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;

CREATE TABLE workspace_window_consoles (
    window_id    TEXT NOT NULL REFERENCES workspace_windows(id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    PRIMARY KEY (window_id, document_id),
    UNIQUE (document_id)
) STRICT;
```

`UNIQUE (document_id)` is the "one console, one window" invariant written in
the file: a write that would violate it fails instead of producing two rival
windows on the same document.

- **Writes go through the bus**, like preferences
  ([ADR-0013](0013-preferences-workspace.md)): `Command::WriteWindowLayout`,
  reserved to `Actor::Human` by the default policy, counted by
  `is_local_write` so that shutdown waits for it, executed on the blocking pool
  ([ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)).
  An agent opens, closes or arranges no window.
- **When writing happens.** Ownership — opening, closing or moving a console,
  active tab — is written immediately. Geometry is written one second after
  the last `Moved` or `Resized`, and on closing: a burst of resizing produces
  only one write.
- **Two instances on the same file** ([ADR-0021](0021-marqueur-d-arret.md)
  provides for it): a launch only adopts the rows whose `app_session_id`
  designates a finished session — closed or abandoned in ADR-0021's sense — and
  rewrites them in its name. The windows of a live instance are not taken over
  by the other.
- **The file is a hostile input**
  ([SECURITY](../SECURITY.md#input-surface), surface 3). On read: at most 16
  windows and 256 consoles per window, the surplus ignored and logged, its
  documents left in the library; a width or height below the template's
  minimum is brought back to the minimum; a window whose rectangle intersects
  no available screen is repositioned by the system at the template's size; an
  unreadable `object_location` counts as absent.
- `object_location` leaves the preferences for the window. Today, it is a
  preferences field carrying its connection, written by
  `write_object_location` and reopened as a location, without a read, when the
  window opens that connection. This behavior does not change: it becomes that
  of each window, on its own row. At the first launch after the migration, the
  first window takes the preferences' value; afterwards, the preferences field
  is no longer written, and stays readable by an earlier version, in line with
  ADR-0013's rule on fields.
- `--temporary-workspace` holds these tables in its in-memory store, like the
  rest: nothing is written.

### Restoration and recovery, per window

- **After an ordinary closing**, each window comes back at its place and its
  size. The recovery screen does not display: it stays reserved for abnormal
  shutdown ([ADR-0021](0021-marqueur-d-arret.md)).
- **Its consoles come back with it, offline** — settled by the user on
  2026-09-25, for `⌘Q`, `File ▸ Exit` and closing the last window; likewise
  after the Dock's Quit, whose closing ADR-0040 records. Text, name and
  autosave, without reconnection or execution, through the mechanism that
  already reopens this way the editors chosen on the recovery screen
  (`features/session.ts`, `restored`;
  `components/oxyn/offline-console-bar.tsx`) and following the rules of
  [selective restoration](../UX-SPEC.md#selective-restore-at-startup):
  attaching to a connection stays a separate gesture. No selection is asked
  for: that is what distinguishes an ordinary closing from an abnormal
  shutdown. Until now, these copies could only be found in the library
  (ADR-0021); they stay there too.
- **After an abnormal shutdown**, the windows come back at their place and
  each offers the recovery selection **of its own consoles** and of its object
  tab, as the recovery screen does today for the single window. A console not
  kept leaves the window and stays intact in the library. The interrupted
  write warning ([I-13](../../CLAUDE.md#i-13)) concerns the history, not a
  window: it displays once, in the first restored window. The "offered once
  per launch" finding (`recoveryOffered`) and the object tab refusal
  (`objectPlaceDeclined`) move from `session.ts` to the registry, per window.
- An open working copy that no window claims — written by an earlier version,
  or detached by an unconfirmed closing (below) — comes back in the first
  window.
- **Autosave stays per console**
  ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)); what is added per
  window is the layout row, written as above.

### Closing

**Closing a window that is not the last one closes its consoles.**
`CloseRequested` is held back; the window receives
`WindowSignal::CloseRequested` and applies the closing rules of a console
([ADR-0015](0015-consoles-independantes.md)) to all of its own, in **a single
dialog** that names the window, lists the consoles carrying unsaved SQL or an
operation, and puts focus on `Cancel`. A console with an open transaction is
resolved there as on exit ("Open transaction on exit", below), for this
window's sessions only. Once confirmed, the closing flushes this window's
drafts, closes its documents, its sessions and its conversations, removes its
layout row, then destroys the window. An operation in progress there is
cancelled as by `⌘W`; a write whose outcome becomes unknown stays reported,
never replayed ([I-13](../../CLAUDE.md#i-13)).

**The critical dialog stays the process's, not a window's.** The native
dialog of [ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md)
keeps its absence of a parent window: the analysis of its § 3 — the button
Enter answers, under `CFUserNotificationDisplayAlert` — applies to this case.
With a parent window, `rfd` switches to another mechanism, an `NSAlert` sheet
(`rfd` 0.16.0, `src/backend/macos/message_dialog.rs`, `show` and
`show_async`, read on 2026-09-25), whose behavior regarding this analysis is
not checked. What identifies the decision is what the backend writes: the
connection, its environment, its address and the statement — not the window,
which has no name. A single critical dialog is open for the whole process; a
critical decision coming from another window meanwhile is refused without
being consumed, as its § 3 provides.

**A pending approval in a window being closed is rejected** when the closing
is confirmed, like a confirmation of a closed tab. The plugin cannot close a
native dialog: if it stayed on screen, its answer approves nothing anymore,
since `confirm_held` only confirms the decision it showed and it is no longer
pending.

A webview that does not answer within the flush delay of `backend/recovery.rs`
(`FLUSH_GRACE`, 2 s) does not prevent the closing; its consoles are then
**not** closed: their documents stay open, detached from any window, and the
next launch treats them as a working copy no window claims ("Restoration and
recovery, per window"). It is the cautious reading of a closing nobody
confirmed, the same as ADR-0021.

"The last one" is decided in `WindowRegistry`, under its lock: a window whose
closing is confirmed no longer counts, a window whose dialog is open still
counts.

**Closing the last window quits the application, on all platforms, macOS
included** — settled by the user on 2026-09-25, for the reasons below. It is
the orderly shutdown already in place: draft flush, waiting for submitted
local writes
([ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md)),
recording the closing ([ADR-0021](0021-marqueur-d-arret.md)), cancelling the
sessions. It does not close the consoles: their documents stay open in the
store, and come back in their window at the next launch.

The exit paths:

| Exit | Path | With several windows |
|---|---|---|
| Closing the last window | `CloseRequested`, transaction resolution, orderly shutdown | unchanged |
| `⌘Q`, `Quit Oxyn` (macOS) | `oxyn-quit` item ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md)), transaction resolution, orderly shutdown | whatever the number of windows; none is closed one by one |
| `File ▸ Exit` (Windows, Linux) | `request_exit` command (ADR-0041), same path as `⌘Q` | same |
| Unexpected `ExitRequested` | held back, same path as `⌘Q` | unchanged |
| Dock's Quit, macOS session logout | `RunEvent::Exit`, closing recorded after local writes, drafts not flushed ([ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md)); transactions rolled back, and logged | no window is asked; each can lose its last 250 milliseconds of typing |

The first four go through a single Rust function, the one `on_run_event`
calls today (`request_exit`, `commands/recovery.rs`); none bypasses it.

In the orderly shutdown, the flush is requested from **all** windows at the
same time, and `FLUSH_GRACE` bounds the whole, not each window; the closing is
only recorded once all webviews have answered, or the delay has elapsed. A
native dialog open at shutdown time counts as a refusal (ADR-0037 § 3).
`RunEvent::Reopen` is not handled: there is no windowless state.

On Windows and Linux, `File ▸ Exit` quits all windows in one gesture, like
`⌘Q`: none is closed one by one, and all give back their consoles at the next
launch.

### Open transaction on exit

Settled by the user on 2026-09-25: **an open transaction holds back the exit,
and the user chooses `Commit`, `Rollback` or `Cancel`.** Everything that
follows assumes [ADR-0039](0039-etat-de-transaction-d-une-session.md)
accepted: without an observed state, there is nothing to list, and the exit
rolls back as today.

**When.** Before the orderly shutdown, and before `begin_shutdown`: as long as
a transaction is not resolved, nothing is flushed, nothing is recorded, and
`Cancel` leaves the application intact. For closing a window that is not the
last one, the same step concerns its sessions only.

**What is listed.** The backend lists each console session that declares
`TRANSACTIONS`, with the **last state the executor observed** for it: the one
`Outcome::Connected` carries when the console opens, then that of each
`Event::TransactionState` published at the end of an execution
([ADR-0039](0039-etat-de-transaction-d-une-session.md) § 3 and § 4). A session
with a console statement in progress — sent by `run_console` or approved by
`decide` — counts as `Unknown` until its reply, whatever event arrives
meanwhile; events missed by a lagging subscriber make all sessions `Unknown`.
Before reading, the backend applies the events already published, waiting at
most `FLUSH_GRACE` (2 s); beyond, all count as `Unknown`. An `Open` or
`Unknown` session is listed. None: the orderly shutdown starts immediately, as
today. The front end does not provide the list: it could have a stale one, and
an XSS could empty it.

*Implementation note, 2026-09-25:* the proposed text had the state read by
`Session::transaction_state` from the bridge. It was a driver call outside the
bus, the one ADR-0039 § 4 already removed in the name of
[I-01](../../CLAUDE.md#i-01): the exit reads the state the executor observed
and published, without asking the session anything
(`crates/oxyn-desktop/src/backend/exit.rs`). The signal carries the session
and not the document, which the backend does not know: the window names the
console from its tabs, and the log of an exit without acknowledgment names the
connection and the session.

**Where the dialog lives: in the webview.** The backend sends to each window
concerned, through its shutdown channel (`subscribe_shutdown`, per window), the
signal `ShutdownSignal::ResolveTransactions`, carrying the consoles to resolve
— document, connection, environment, state. The window comes to the front and
opens an `alert-dialog` that names, for each console, the console, the
connection and its environment, and the state (`Transaction open`, or
`Transaction state unknown`), with three actions: `Commit`, `Rollback`,
`Cancel`, focus on `Cancel`, Enter alone validating nothing. `Commit` and
`Rollback` apply to this window's whole list; a console-by-console decision
remains possible beforehand, from each console, by typing `COMMIT` or
`ROLLBACK`.

Why not ADR-0037's native dialog: `Commit` there falls under no family. It
writes nothing the gate has not already let through — `oxyn-query` classifies
`COMMIT` and `ROLLBACK` as reads (`crates/oxyn-query/src/classify.rs`,
transaction control), and each write of the transaction was held and, on
`production`, confirmed in the native dialog at the time of its execution. A
script that wanted to commit a transaction does not need this dialog: it types
`COMMIT` in the console through `run_console`, which it already can. The exit
dialog therefore opens no new capability, and stays where one reads and
chooses.

**What the buttons do.** `Commit` and `Rollback` emit, for each listed
session, a `Command::Execute` carrying `COMMIT` or `ROLLBACK`, under
`Actor::Human`, through `run_console`'s path — the same as if the user had
typed it: gate, `audit_journal`, `query_history`
([I-01](../../CLAUDE.md#i-01)). No `Command` variant is added, and the
`Session::commit` and `rollback` methods stay without a caller. The window
waits for each session's `Event::TransactionState` (ADR-0039 § 3):

* all `Idle` — the window calls `request_exit` again, which rereads the state;
  the orderly shutdown starts when no window has a transaction anymore;
* an error — the server's message displays, the exit does not continue, and
  the dialog stays open on the unresolved sessions. A `COMMIT` whose outcome
  is ambiguous is **never** replayed ([I-13](../../CLAUDE.md#i-13)): it is the
  state observed afterwards that says whether it took;

  *Implementation note, 2026-09-25:* statements are sent one by one and the
  first error stops the series. A `COMMIT` that failed, or whose reply did not
  arrive, is no longer offered for that session: only `Rollback` and `Cancel`
  remain, and `COMMIT` typed in the console remains possible after `Cancel`.
  `run_console` only returns after the state is published: the window calls
  `request_exit` again as soon as each statement has replied, and it is the
  backend's reread that decides;
* `Cancel`, in any window — the window calls `cancel_exit`, and the exit is
  abandoned for all: the other windows close their dialog
  (`ShutdownSignal::ExitCancelled`), and nothing has been flushed or recorded.

**A silent webview does not hold back the exit.** Each window acknowledges the
signal through `shutdown_acknowledged`, within `FLUSH_GRACE` (2 s). Without an
acknowledgment — frozen webview, reloaded, without a subscription —, the exit
continues for that window as today: its transactions are rolled back by
closing the sessions, and the log says so (`warn`, connection and console
named, never the SQL). Once the acknowledgment is received, the wait has no
bound: it is the user who decides, as in front of a console's closing dialog.

**Two more IPC commands, without arguments**: `shutdown_acknowledged` and
`cancel_exit`, on the model of `shutdown_flushed`. They carry no data and only
act on an exit already requested; outside that case, they do nothing. What an
XSS gets from them: holding back or cancelling an exit the user requested — a
nuisance, not data nor a write
([SECURITY](../SECURITY.md#input-surface), point 5).

**The Dock's Quit and the macOS session logout** cannot wait (ADR-0040): open
transactions are rolled back there by the end of the process.
`close_on_forced_exit` adds one `warn` log line per session listed at that
moment — read without waiting for what is in progress, hence possibly
`Unknown` —, and nothing else changes on this path.

> **Why we depart from the macOS convention.** It keeps the application alive
> when its last window closes, and reopens one on a click on the Dock icon
> (`RunEvent::Reopen`). V1 does not follow it:
>
> - the orderly shutdown is already the path of closing the last window, and
>   that of `⌘Q` since ADR-0038. A windowless state would require a second
>   regime: a closing of the last window that stops nothing, then a shutdown
>   without a webview to flush;
> - a "zero window" state would force deciding what it keeps: server sessions
>   open without any view to show them, or their closing — hence a closing of
>   the last window that closes its consoles, and a next launch that no longer
>   gives them back. The Dock's Quit, the only remaining exit from that state
>   with `⌘Q`, moreover cancels no statement in progress (ADR-0040);
> - nothing, in V1, works without a window: no background export, no
>   monitoring, no autonomous agent.
>
> The price: a macOS user who closes the window expecting the application to
> stay in the Dock sees it quit, and relaunches. This price is bounded by
> restoration: they find their windows at their place, with their consoles,
> offline.
>
> **Reconsider if** an activity must survive closing the windows (long export,
> scheduled [automatic refresh](0022-rafraichissement-automatique.md), agent
> working alone), if the measured cold start of the Tauri interface exceeds
> the 1 s budget of
> [PERFORMANCE](../PERFORMANCE.md#interaction-budgets), or if the user
> reports the gap as a defect.

### Threads

Nothing, when a window opens, blocks the main thread
([I-05](../../CLAUDE.md#i-05)):

- `open_window` and moving are **`async`** commands: never synchronous,
  because of the deadlock documented on Windows and because a synchronous
  command runs on the main thread
  ([ARCHITECTURE](../ARCHITECTURE.md#le-modèle-de-threads));
- writing the layout goes through the bus and the blocking pool; building the
  window waits neither for this write nor for the catalog session, which opens
  afterwards, cancellable;
- handling `Moved`, `Resized`, `Focused` and `CloseRequested` in
  `on_run_event` only updates the in-memory registry and launches a task; it
  writes nothing and waits for no lock held by a task;
- the layout is read by `Backend::open`, **before** the event loop — the
  exception already admitted for assembling the backend, bounded to 16 windows
  and 256 consoles per window. At startup, the window that had focus is built
  first; the others follow, each in its task, without delaying the first.

### Budgets

No new budget is invented; those of
[PERFORMANCE](../PERFORMANCE.md#interaction-budgets) apply. `New window` and
`Open in new window` show feedback under **100 ms** — the window frame — and a
usable window under **1 s**, the cold opening budget. At startup, this 1 s
budget applies to the first window, not to the whole. None of these values is
measured on the Tauri webview to date.

### Implementation notes, 2026-09-25 (batch 7, first part)

Batch 7 ships in two pull requests: the first makes the windows live —
registry, ownership, subscriptions, menu, closing and exit —, the second
brings the persisted layout, per-window restoration and `Open in new window`.
What the first clarified, while writing the code:

- **The assistant belongs to one window per connection, not per
  conversation.** The code holds the assistant's state per connection
  (`backend/ai`: conversations, pending agent, requested samples). Making it
  specific to each window would have redone that whole module. The registry
  therefore holds `assistants : ConnectionId → WindowKey`. The first window
  that uses it takes it; another window open on the same connection receives
  "The assistant of this connection is open in another window". The owning
  window only comes to the front on opening a conversation, not on every
  refusal: a script looping on a refusal would otherwise steal another
  window's keyboard. A window only takes the assistant of a connection it
  holds, as it only opens a console on a connection it holds. The rule stays
  stricter than the text: no conversation is shared. It will be to reconsider
  if two assistants on the same database, in two windows, are requested.
- **`report_action_state` is `set_menu_state`**, the command that existed,
  now per window. It already refused an unknown identifier. Focus stays on the
  last window that received it when the application goes behind another one,
  and only empties when that window closes. Without a focused window, the bar
  keeps only `New window`, `Settings…` and `Quit Oxyn` enabled. An
  application action, without a target, then goes to the first window.
- **Closing a non-last window happens in two steps**, in this order. First,
  its transactions. `ExitTransactionsDialog`'s dialog is reused and carries
  `scope: window`: `Commit` and `Rollback` there call `close_window` again,
  not `request_exit`. Then, if consoles remain that would lose work, **a
  single** dialog (`CloseWindowDialog`) lists them. It names the window by its
  connections, since it has no name, and offers only `Cancel`, which has
  focus, and `Discard and close window`. A console to keep is saved
  beforehand, from the console. Without a console at risk, the window closes
  without a dialog, like a console without work. `shutdown_acknowledged` and
  `cancel_exit` serve both steps. Added: `subscribe_window`, `close_window`
  and `confirm_window_close`, without arguments, and `open_window`.
- **The events of an agent command go to no window**: no webview read them,
  the assistant follows its agent through its conversation's `Channel`. A
  decision or a result no window received — an agent's, which the assistant
  displays — goes back to the first window that uses it. Their identifiers
  cannot be guessed (UUID v7).
- **A document belongs to the window whose console writes it**, from its
  first save, and only consoles save. A console of another window can no
  longer write, close or delete it. It remains to bring the owning window to
  the front when this document is reopened from another window's library: it
  is the second part, with console moving.
- **Only the window built at launch offers the recovery screen**
  (`recovery_status`). A window opened afterwards did not see the shutdown
  that preceded it, and two recovery screens would offer the same copies
  twice.
- **`PreferencesChanged` rereads everything, except the layout**: theme,
  density and cell format follow immediately. The window's sidebar and
  inspector stay as they are until its next opening (§ ADR-0013, above).
- **A webview that does not acknowledge its closing** has its sessions
  closed, hence its transactions rolled back, with one `warn` line per
  transaction. Its documents stay open.

### Implementation notes, 2026-09-26 (batch 7, layout and restoration)

- **Migration 18 is the one written above**, with an index
  `workspace_windows_order (workspace_id, ordinal)`.
  `oxyn-store/src/windows.rs` writes it (`save`, `remove`) and reads it back
  (`adopt`). `WindowLayout` and `WindowLayoutChange` live in `oxyn-core`,
  without a Tauri type ([I-08](../../CLAUDE.md#i-08)). The row names the
  launch that writes it: the executor receives the launch's `AppSessionId`
  (`with_app_session`) and passes it to the store. `WindowLayout::validate`
  refuses, before the file, a size or a position that is not a finite number,
  more than 256 consoles, a duplicate console or a foreground console that is
  not one of them.
- **A never-saved console is not written**: its `documents` row does not
  exist yet, and the foreign key would refuse it. It joins the layout at its
  first save. A console that another row lists leaves it
  (`ON CONFLICT (document_id)`); it is the registry that prevents a window
  from taking another's console, by removing from its report any document
  another window writes.
- **Reading adopts in one transaction**, bounded to 64 rows read and 257
  consoles per window. Each column is read without failing: an edited file may
  have lost `STRICT`, and a mistyped value only costs its row or its column. A
  row whose identifier cannot be read back, and those beyond 16, are removed
  from the file; consoles beyond 256 too, logged, their documents left in the
  library. A console whose document is closed or deleted does not come back. A
  size that is not a finite number counts as `0`, brought back to the
  template's minimum by the desktop; an out-of-bounds position counts as
  absent.
- **The rectangle is in logical pixels**: outer position and inner size,
  divided by the window's scale. It is compared to the usable area of each
  connected screen, each at its own scale. A window minimized to the Dock
  keeps the rectangle it had. It is written when the window is built, one
  second after the last `Moved` or `Resized`, and before the orderly shutdown,
  which waits for this write like the other local writes. The Dock's Quit does
  not do it: the last stabilized rectangle is authoritative.
- **Ownership comes from the webview**: `report_window_consoles` carries the
  documents of the consoles of all the window's workspaces, displayed or
  retained, in tab order, and that of the foreground. The front end only sends
  a changed list, once per render pass; Rust does not write an unchanged list.
  Rust removes what another window writes or what its row lists: the consoles
  of a restored window belong to it from launch, before any save, and the
  orphan copies to the first window as soon as it receives them.
- **`restored_consoles`** returns the copies a window reopens: its own, in tab
  order, then — for the launch's first window, once — the open copies no
  window claims, read in the library in pages of 200, sixteen at most; beyond,
  they stay in the library. After an ordinary closing, `routes/index.tsx`
  hands them to `restoreWorkingCopies`, without a question. After an abnormal
  shutdown, each window's recovery screen offers only them, and those not
  chosen immediately leave the window's row. A webview reload gives them back
  too: they are the consoles it held.
- **`recoveryOffered` and `objectPlaceDeclined` stay in `session.ts`**: that
  store belongs to the webview, hence already to the window. Moving them to
  the registry would have added nothing. All windows built at launch see the
  abnormal shutdown; only the first reports the interrupted write.
- **`read_object_location` and `write_object_location` target the calling
  window.** The first window built when the file holds none takes the
  preferences' value; the preferences field is no longer written.
- **The row is removed** when a non-last window closes, whether by
  confirmation or because its webview does not answer. The exit removes
  nothing: it is what it must give back.
- **The policy refuses `WriteWindowLayout` to an agent**, in `policy.rs`, next
  to the refusal of preferences. The test is on the desktop side
  (`backend/windows/layout/tests.rs`).

### Implementation notes, 2026-09-26 (batch 7, `Open in new window`)

- **The move is prepared before the window.** `open_in_new_window` reserves
  the target, transfers the console to it and opens its catalog session, then
  only builds the window: its webview finds the `ConsoleHandoff` as soon as it
  asks for it (`take_console_handoff`, once). A window that fails to build
  gives the console back to the source, session and transaction included.
- **Session and document change window under a single lock**
  (`WindowRegistry::hand_over`): no command sees the console belonging to
  nobody, or to both. The source keeps the connection and its catalog.
- **The result** is counted for the target before the source unmounts its
  console, which then releases its own view. The target rereads the columns; a
  result expired meanwhile is not shown, and its view is released.
- **Refusal.** The menu greys out the entry, with its reason, during an
  execution, a confirmation, an export or a save of the console, and on an
  offline console. The backend on its side refuses everything another window
  owns, a console whose statement is running and a console whose statement
  waits for a confirmation. It **marks** the console "moving" under the same
  lock that observes it is idle: until the end of the move, `run_console` and
  `decide` are refused there, so that no statement leaves while the catalog
  session opens. An open destructive review holds its dialog in the
  foreground: the tab menu is not reachable.
- **Everything that can fail precedes the transfer**: configuration, catalog
  session, description of both sessions. After it, only writing the two
  layout rows remains, and its failure is logged without undoing the move: a
  console transferred then abandoned would close its session, and its
  transaction with it. The console leaves the source's row for the target's.
- **The assistant stays with the window that holds it**, on the connection;
  `ai_ask` now checks that the named session is the calling window's, so that
  the source's assistant does not work in the moved session.
- **A handoff unreadable by the front end is reported**, not swallowed: the
  window keeps the session until it closes.
- **The target restores nothing else**: neither recovery screen nor copies.
  Its first console is the moved console, with the note "Moved from another
  window. Nothing was executed."

## Consequences

* **+** Two connections, or two consoles, can be viewed side by side, on two
  screens.
* **+** No session is shared between windows: what ADR-0015 guarantees between
  consoles applies between windows, without a new rule.
* **+** Ownership is held in three places that cross-check: the registry
  refuses a command outside its window, the Rust filter only delivers its
  events, and the file refuses a console in two windows.
* **+** The webview's surface does not widen: the same three permissions, and
  window creation stays in Rust, bounded.
* **+** Closing then relaunching gives the windows back at their place, with
  their consoles offline; that is what makes bearable a closing that quits.
* **+** Quitting no longer silently throws away an open transaction, except
  through the Dock, which macOS does not let us hold back — and the log then
  says so.
* **+** No new shutdown path: those of ADR-0021, 0038 and 0040 remain the
  only ones, with one more draft flush per window.
* **−** Almost the whole IPC surface changes signature: each command that
  targets a console, a session, a command, a result or a conversation receives
  the calling `Webview` and undergoes an ownership check. It is the bulk of the
  work, and an omission is hard to see: a command without a check works, from
  any window.
* **−** Each window open on a connection consumes at least **two** server
  sessions there — catalog and first console. On a server close to its
  connection limit, the second window can fail to connect where the first
  succeeded.
* **−** The memory cost of an additional webview is not measured; the bound of
  16 is a guard, not a demonstrated capacity.
* **−** Moving a console loses its editor's undo history.
* **−** The sidebar and the inspector stay common preferences: collapsing the
  bar in one window does not collapse the others right away, but the following
  ones will open collapsed. It is ADR-0013's reconsideration condition that
  starts to come true.
* **−** Closing a window closes its consoles, closing the last one keeps them:
  an asymmetry to learn, the same as browsers'.
* **−** On macOS, Oxyn departs from the platform convention.
* **−** `⌘Q` may no longer quit: `Cancel` in front of an open transaction
  leaves the application open. And a failed `Commit` holds back the exit until
  the user chooses `Rollback` or `Cancel`.
* **−** The transaction step adds up to 2 s to the exit when a session is slow
  to give its state, and as much when a webview does not acknowledge.
* **−** On PostgreSQL, nothing is listed as long as the driver does not
  declare `TRANSACTIONS`: the step only serves SQLite today.
* **−** A critical dialog open for one window blocks those of the others until
  it closes — refusal without consumption, to ask again.
* **−** One more migration, hence one more format to carry, and two tables to
  validate like any hostile input.

**Exit cost:** going back to a single window requires removing the registry
(`backend/windows.rs`), the per-window gestures and subscriptions, and
restoring the fixed label in the capability. The `Webview` parameter added to
the commands can stay, inert. What cannot be undone: the migration — a version
without multi-window would ignore the two tables, without deleting them. The
cost is bounded by what the registry concentrates: no core crate knows windows
exist ([I-08](../../CLAUDE.md#i-08)); only `Command::WriteWindowLayout` and the
layout type enter `oxyn-core` and `oxyn-store`, without a Tauri type.

**Reconsider if** actual use exceeds 16 windows; if an additional idle window,
measured, costs more than 150 MiB of resident memory (a product threshold, not
a measurement); if the double cost in server sessions is reported as a defect,
in which case an **explicit and visible** sharing of the catalog session would
have to be decided, never a silent sharing; or if the tear-off-by-dragging
gesture is requested — it would require checking what the webviews of the three
platforms report of a drag leaving the window.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Single window, tabs only | Does not answer the need set by the user on 2026-09-25: two connections or two consoles visible at the same time, on two screens. Split-screen tabs in one window do not extend to a second monitor |
| One process per window | Each process would have its backend: result budgets ([ADR-0017](0017-retention-resultats.md)) and catalog cache (1,024 scopes, 50,000 objects) multiplied by the number of windows; no movable console, a session not crossing a process; preference revision conflicts between instances, already noted as a negative consequence of ADR-0013; one shutdown marker per process; one icon per process on macOS |
| Read-only secondary windows | Two kinds of windows, hence two versions of each gesture. Read-only belongs to the connection (`ConnectionConfig::read_only`) and is decided at the `PolicyGate`, not by a view: a "read" window that offers `Explain` executes `EXPLAIN ANALYZE`, hence the query ([I-07](../../CLAUDE.md#i-07)) |
| Share a connection's sessions between windows | Rejected between consoles by [ADR-0015](0015-consoles-independantes.md) for the same reason: a transaction or a session context decided in one view changes the other. Sharing only the catalog session would additionally tie the lifetime of two windows together |
| Broadcast events to all webviews (`emit`) and filter in the front end | Each window would serialize and receive the stream of all the others; the filter, reproduced in each webview, would be the only safeguard of an ownership the backend already knows; and `listen` would require a `core:event` permission the capability does not grant |
| Keep `main` for the first window and a pattern for the following ones | Two naming regimes for a single kind of window; the first would stop being restorable like the others |
| Let the front end create its windows through Tauri's JavaScript API | Requires granting the webview a window or webview creation permission: an XSS would gain an API, without a bound |
| A third-party window-state plugin | It would hold its own persistence outside the workspace file — a second source of state, not versioned with the workspace, which `--temporary-workspace` would not contain. *Exact behavior to check in its sources before reopening the question* |
| Move a console by closing it then reopening it in the target | Closes its session: transaction and session context lost, result to re-execute — exactly what [I-06](../../CLAUDE.md#i-06) and [I-13](../../CLAUDE.md#i-13) forbid doing silently |
