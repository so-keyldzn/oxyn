# ADR-0038 — A crash is announced once, and ⌘Q goes through the orderly shutdown

**Status:** accepted · **Date:** 2026-09-25

**Clarifies:** [ADR-0021](0021-marqueur-d-arret.md), on what happens to an
abandoned session once it has been noticed, and on the closing paths that
record `closed_at`.

**Clarified by:** [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md),
which records the closing of the Dock Quit and of session logout in
`RunEvent::Exit`, and takes up the alternative rejected here.

## Context

The installed application displayed the recovery screen — "Oxyn did not close
normally" — after **every** ordinary close: exactly the defect
[ADR-0021](0021-marqueur-d-arret.md) was meant to remove. The database of a
real workspace, read on 2026-09-25, carried 17 sessions, 16 of them without
`closed_at`. Two causes, independent.

**An abandoned session was abandoned forever.** ADR-0021 defines an abnormal
shutdown as "a previous session without closing, with a stale heartbeat",
without saying what becomes of it after it has been announced. Nothing settled
it: a single crash — here a launch of 2026-09-10 — made every following launch
abnormal, clean closes included.

**⌘Q and the Quit menu never recorded the close.** Tauri's predefined Quit item
sends `terminate:` to the application. tao 0.35.3 handles
`applicationWillTerminate` but not `applicationShouldTerminate`: macOS ends the
event loop with a `RunEvent::Exit`, without `ExitRequested`, and nothing can
hold the exit long enough to flush the drafts and record the close. Only the
window's close button, which goes through `CloseRequested`, recorded it: it is
the only closed session in the database.

## Decision

**The launch that notices an abandonment marks it announced.** Migration 17
adds `app_sessions.reported_at`. `Sessions::begin` fills it on the abandoned
sessions not yet announced, in the transaction that records the new launch;
only an abandoned **and unannounced** session leads to concluding an abnormal
shutdown. `closed_at` stays `NULL`: the row still says that this launch did not
close cleanly. If the announcing launch crashes in turn, it itself becomes an
unannounced abandoned session, and the next one offers recovery again.

**On macOS, Oxyn replaces the predefined Quit with its own.** Tauri's default
menu is rebuilt identically, except the last item of the application menu: an
`oxyn-quit` item (`CmdOrCtrl+Q`) that triggers the close button's orderly
shutdown (`crates/oxyn-desktop/src/commands/recovery.rs`). Elsewhere, Tauri
sets no menu, and closing the window remains the exit.

**Each abandonment leaves a log line.** `info` when the close is recorded;
`warn` when no webview is subscribed, when the draft flush is not confirmed,
when the close is not recorded and when the application exits without the
orderly shutdown. The log is flushed to disk before exiting, in a wait bounded
to one second.

## Consequences

* **+** The recovery screen becomes a signal again: it no longer follows a close
  by ⌘Q, by the menu or by the window button.
* **+** The crash stays readable without Oxyn ([I-11](../../CLAUDE.md#i-11)):
  `closed_at IS NULL` says it, `reported_at` says when it was announced.
* **+** A future diagnosis starts from a log that says how the close went, not
  from a silent log.
* **−** The Dock Quit and macOS session logout still go through `terminate:`:
  the close is not recorded there, and the next launch offers recovery. It is
  the cautious direction, and the log says so.
* **−** Oxyn carries a copy of Tauri's application menu: an evolution of the
  default menu will not reach it by itself.
* **−** One more migration. At the first launch that applies it, the sessions
  already abandoned are announced one last time.

**Exit cost:** a column, a query clause and a menu. Removing the menu gives ⌘Q
back to `terminate:`; removing `reported_at` makes recovery permanent after a
crash.

**Reconsider if** tao exposes `applicationShouldTerminate`: exiting through
`terminate:` would then become holdable, for the Dock as for the menu, and
Oxyn's own Quit would no longer have a reason to exist.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Settle an abandoned session by filling its `closed_at` | Would rewrite a crash as a clean shutdown, in the very table that must tell them apart |
| Delete an abandoned session once announced | Deletes the only trace, readable without Oxyn, that a crash happened |
| Record the close synchronously in `RunEvent::Exit` | Drafts can no longer be flushed there (the webview answers through the main thread we would block): it would mark a clean shutdown on work possibly not written |
