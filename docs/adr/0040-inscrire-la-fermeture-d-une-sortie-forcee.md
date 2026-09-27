# ADR-0040 — An exit that macOS does not let us hold back records its closing

**Status:** accepted · **Date:** 2026-09-25

**Clarifies:** [ADR-0038](0038-un-plantage-s-annonce-une-fois.md), on the
alternative "record the closing in `RunEvent::Exit`", which it rejected.

## Context

[ADR-0038](0038-un-plantage-s-annonce-une-fois.md) routes ⌘Q and the Quit menu
through the orderly shutdown. It leaves one limit: **the Dock's Quit and the
macOS session logout** still send `terminate:` to the application. The closing
is not recorded there, and the next launch offers recovery after an ordinary
exit. It is the very defect ADR-0038 fixed, on one more path.

The only point where this path can be held back is `applicationShouldTerminate:`,
on the application delegate. tao does not register it, neither the pinned
version (0.35.3) nor the latest published one (0.37.0), and Tauri 2.11.5 adds
nothing. tao only handles `applicationWillTerminate:`, which produces
`RunEvent::Exit` on the main thread. macOS ends the process as soon as this
callback returns.
Dated sources in [RESEARCH-NOTES](../RESEARCH-NOTES.md#tauri-interface-and-front-end).

Adding the method to tao's delegate would require `unsafe` in `oxyn-desktop`,
which the workspace refuses ([SECURITY](../SECURITY.md#unsafe-policy)).

ADR-0038 rejected recording in `RunEvent::Exit` for one reason: drafts can no
longer be flushed there, because the webview answers through the main thread
that this callback occupies. Yet the orderly shutdown already accepts this
case: if the webview does not confirm the flush within 2 s, the closing is
recorded anyway, because a draft is written at most 250 ms after the last
keystroke ([ADR-0024](0024-autosauvegarde-au-repos-de-frappe.md)). What
`RunEvent::Exit` loses is therefore what the orderly shutdown already
tolerates: at most the last 250 milliseconds of typing.

## Decision

**In `RunEvent::Exit`, if the orderly shutdown has not taken place, Oxyn
records the closing after the local writes, in a wait bounded to 2 s.**

- `Backend::close_on_forced_exit` (`crates/oxyn-desktop/src/backend/recovery.rs`)
  marks the shutdown as started, then launches on the runtime the same
  `close_session_after_local_writes` as the orderly shutdown. Reading and
  writing the store happen on the blocking pool.
- The main thread waits for this task to finish on a channel, at most
  `FORCED_EXIT_GRACE` (2 s, `crates/oxyn-desktop/src/commands/recovery.rs`). The
  window is already gone: this wait freezes nothing
  ([I-05](../../CLAUDE.md#i-05)), like the log flush already done at the same
  place.
- Drafts are not flushed. No request is sent to the webview.
- If a local write is still in progress when the wait ends, the closing **may
  remain unrecorded**. The task is not cancelled: it records it if the write
  finishes before macOS kills the process, and always after it. Otherwise, the
  next launch offers recovery, and the log says so (`warn`).
- Statements running on servers are not cancelled on this path. Nothing
  changes on that point: they were not before.

## Consequences

* **+** The Dock's Quit and session logout no longer cause recovery to be
  offered after an ordinary exit.
* **+** No `unsafe`, no dependency on a private tao behavior: the fix relies
  only on `RunEvent::Exit`, which Tauri documents.
* **+** ADR-0021's rule holds: the closing is never written over a local write
  in progress.
* **−** On this path, the last 250 milliseconds of typing can be lost without
  recovery being offered. It is the orderly shutdown's tolerance for a silent
  webview, extended to a case where the webview is not even asked.
* **−** The main thread can wait up to 2 s on top of the log's second. macOS
  may force the end of a slower session logout. In that case, the closing is
  not recorded, which is the cautious direction.
* **−** A running server statement is not cancelled: it continues until the
  server notices the disconnection.
* **−** A write is only counted at the start of its Tauri command. A write
  sent from the webview but not yet started escapes the wait, for a few
  milliseconds. The orderly shutdown has the same window.

**Exit cost:** one method and one branch of `on_run_event`. Removing them
returns this path to the state described by ADR-0038: closing not recorded,
recovery offered.

**Reconsider if** tao registers `applicationShouldTerminate:`. Exiting through
`terminate:` would then become holdable and would go through the full orderly
shutdown, drafts flushed. This path and Oxyn's own Quit would have no reason
to exist anymore.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Add `applicationShouldTerminate:` to tao's delegate class (`class_addMethod`) | Requires `unsafe` in `oxyn-desktop`, and lifting that refusal for a single call is a disproportionate architecture decision. The fix would also depend on the name of a private tao class |
| Keep the limit (recovery offered after the Dock's Quit) | The recovery screen stops being a signal for whoever quits from the Dock, i.e. the defect ADR-0038 fixed |
| Record the closing without waiting for local writes | Would mark a clean shutdown over a write that may not have happened, which ADR-0021 forbids |
| Wait without a bound | A blocked write would hold back the macOS session logout, which would end up killing the process |
