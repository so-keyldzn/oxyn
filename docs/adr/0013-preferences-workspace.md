# ADR-0013 — Persist reading preferences in the workspace

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0004](0004-command-bus.md), for local settings.

## Context

The mockup offers Compact and Comfortable, independently of theme and width.
The existing reading settings disappear on close. The local store has a migrated
SQLite schema, while GPUI types must stay in the interface. Rapid changes must
not be rewritten out of order by asynchronous tasks.

## Decision

- `oxyn-core` carries `WorkspacePreferences`, `ReadingDensity` and
  `PreferencesSnapshot`, with no GPUI type. The JSON format version 1 contains
  `appearance`, `reading_density`, `sidebar_collapsed`, `inspector_open`,
  `inspector_width`, `null_text`, `group_thousands` and `object_location` —
  the restored object location, added on 2026-09-10. On 2026-09-15, for the
  Tauri interface: `follow_system_appearance` (a boolean rather than a `system`
  variant of `appearance`, which an earlier binary would refuse at startup),
  `binary_display` (an unknown value reads as `hex`) and `cell_max_chars`
  (64 to 16,384, never without truncation).
- **A field is added without changing the version, and an unknown field is
  ignored.** Preferences are read at application **startup**: refusing an unknown
  field would prevent an earlier version of Oxyn from launching after a newer one
  wrote the same file — rolling back is something users do. `version` carries
  incompatibility, explicitly checked, and it alone. The price is that a
  misspelled field name reads as its default instead of failing; it is the
  cheaper of the two errors for a file Oxyn writes itself.
- SQLite migration 4 adds `workspace_preferences`, tied to the workspace, with
  `revision`, a JSON `payload` and `updated_at`. Previous migrations remain
  unchanged. A missing row gives the default values.
- Reads and writes go through `ReadWorkspacePreferences` and
  `WriteWorkspacePreferences`. Bootstrapping reads the snapshot once off the UI
  thread. These commands contact no remote database. Writing interface settings
  is reserved to `Actor::Human`; an agent does not change its user's interface.
  The refusal goes through the `PolicyGate` and the log.
- The revision increases on every local change. An older write never replaces a
  more recent revision. The same revision with different content signals a
  conflict, without overwriting the file. The interface distinguishes the local
  application from the confirmed save and offers an explicit retry on failure.
  Closing the last window waits for committed writes before requesting shutdown.
  GPUI's native hook also receives this wait, but its own 100 ms delay is not
  enough to guarantee every shutdown path of the system; those paths need their
  own manual test.
- Data is validated before writing and after reading: known version, revision in
  the positive SQLite range, absence label limited to 64 bytes, inspector width
  between 240 and 480 px. These two width bounds are implementation safeguards;
  the initial width of 280 px comes from Figma, the bounds do not.
- Panels keep their wide preference when switching to compact. Their adaptation
  to the viewport produces no preference write and no query. The inspector handle
  is usable with the mouse and the keyboard; it saves the chosen width after the
  gesture.

## Consequences

- **+** The same settings come back after a restart and a connection change.
- **+** The format remains readable with SQLite and an ordinary JSON reader.
- **+** A slow task cannot overwrite a more recent setting.
- **−** A second instance that read the same revision can cause an explicit
  conflict; the interface must allow saving again after re-reading.
- **−** Local saving has its own error states and its own completion wait.

**Exit cost:** migrate a small settings table and replace the two commands;
connections, secrets and result data are independent.

**Reconsider if** preferences become specific to a document, if several windows
ask for diverging settings, or if a synchronization between machines needs a
field merge rather than a snapshot revision.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Serialize `Theme` | Brings GPUI types and computed colors into the domain |
| Write a file from the view | Blocks rendering and bypasses the bus |
| Save only on close | Loses the settings on an abnormal shutdown |
| Accept all writes in their order of arrival | A slow response can restore an earlier preference |
