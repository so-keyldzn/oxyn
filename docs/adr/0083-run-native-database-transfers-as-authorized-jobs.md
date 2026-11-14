# ADR-0083 — Run native database transfers as authorized, owned jobs

**Status:** proposed · **Date:** 2026-11-13

**Refines:** [ADR-0004](0004-command-bus.md) on native transfer commands and
[ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md) on
owned I/O and audit outcomes. This proposal defines a boundary, not an
authorization to run a tool or an assertion that authentication is solved.

## Context

Issue [#374](https://github.com/so-keyldzn/oxyn/issues/374) asks for PostgreSQL
native backup/restore. The current `Export` command writes retained results;
it does not export a database. A raw Tauri process launcher would create a
second execution path outside connection policy, cancellation and audit.

A transfer can outlive its panel, emit sensitive diagnostics, create a partial
file or change a database before a client error. Authentication to an external
executable also crosses a boundary absent from ordinary driver connections.
The [research journeys](../implementation-plan/schema-research-journeys.md)
separate export success from restore success and unknown outcomes.

## Decision

`ExportDatabase` and `RestoreDatabase` are distinct typed `oxyn-core`
commands. `oxyn-exec` owns policy, authorization rechecks, audit, connection
identity and job lifecycle. The first revision is human-only for both commands,
including refusal at the gate rather than merely hiding an AI tool.
Database read-only protection and production confirmation remain governed by
[SECURITY](../SECURITY.md); restoration is explicitly mutating.

A new `oxyn-transfer` crate owns the native-tool boundary: a narrow
`TransferRunner` interface, validated transfer requests/events and a PostgreSQL
adapter. It performs no policy decision and has no dependency on Tauri,
`oxyn-store` or `oxyn-secrets`. The executor depends on that boundary and calls
it only after authorization; `oxyn-desktop` supplies the runner and native
dialogs. PostgreSQL argument construction and process ownership live in focused
modules of this crate, not in the UI or the network driver's SQL execution path.
No general-purpose shell-launch service is exposed.

An executable is a human-configured, host-owned selection. Its identity and
supported invocation are checked before use; a path offered by the webview does
not by itself grant execution. Commands name a host selection, target connection
and structured options, not an executable plus arbitrary arguments.
The reviewed request fixes the actual destination, source/destination files,
options and tool identity; a material change requires a new review.
The review identifies the archive's source and the known effects of the chosen
options. It does not certify the archive's contents as safe or claim to enumerate
every effect; native restore remains an explicit privileged user operation.

The adapter invokes the executable without a shell, with an explicitly built
environment and no ambient connection-service configuration. Network target
and transport protection match the reviewed connection. Options capable of
redirecting execution to an archive-specified target are refused until that
actual target can be independently authorized. A shared wire protocol is not
proof that a PostgreSQL variant supports these tools.

Authentication is fail-closed. This ADR authorizes neither a password in process
arguments, nor an inherited environment, nor writing a temporary plaintext
password file. It does not select a password transport or rely on ambient user
credential files. An authentication mode is offered only after its platform
implementation proves destination binding, secret confinement and cleanup.
The unresolved transport is an implementation entry gate in the plan, not a
silent fallback. No new exception to SECURITY is granted here.

`oxyn-exec` retains ownership through child startup, output draining,
cancellation, termination and reap. Closing the progress tab requests
cancellation and retains the job until cleanup settles; hiding it does not
cancel. Closing the owning window participates in the existing shutdown review.
Cleanup covers descendants through a platform-specific implementation; dropping
a child handle is not a proof of termination. No automatic resume or retry of
a restore follows timeout, process loss or restart.

Tool output is untrusted. Bounded diagnostics strip terminal controls and
redact sensitive content before display, storage or logging; raw SQL/data echoed
by an error is not forwarded. If safe extraction is unavailable, report a
structured failure without raw stderr. Progress describes observed stages or
counts, not a guessed completion percentage.

Export writes an owned temporary destination and publishes the final file only
after process success and finalization. Existing destinations require explicit
overwrite consent; interruption cannot label a partial file successful.
Restore records refused, failed-before-start, completed, partial and unknown
outcomes distinctly. An interrupted write remains unreconciled until inspected.
Affected metadata and previews become stale after any restore that may have
changed the target, not only after a reported success.

The persistent job record contains identity, reviewed non-secret options,
timestamps and sanitized outcome; it carries no credential or raw process log.
After restart, unfinished jobs are shown as interrupted, never replayed.

## Consequences

* **+** Native tools remain subject to the same execution authority as other writes.
* **+** Partial files, ambiguous restores and cleanup failures remain observable.
* **−** Authentication and process-tree cleanup require platform-specific proof;
  unsupported modes remain unavailable.
* **−** Structured options offer less flexibility than an unrestricted terminal,
  and safely bounded diagnostics may provide less detail than raw tool output.

**Exit cost:** remove the runner and adapter, retain readable historical job
records and disable the two commands. A replacement adapter can reuse policy,
audit and UI contracts. This introduces one focused crate boundary, but no
external process-management dependency is selected by this ADR.

**Reconsider if** no useful authentication mode can meet SECURITY on supported
platforms, or a native library can supply equivalent format compatibility and
safer cancellation. Any security exception or new library requires its own
reviewed decision before use.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Spawn tools directly from a Tauri command | Bypasses the command bus, policy and audit |
| Reuse retained-result export as database backup | Confuses a result buffer with a database-wide operation |
| Expose free-form command flags | Can redirect the target or change effects outside the reviewed request |
| Treat process exit or cancellation as rollback | Does not establish what the destination applied |
| Write a temporary password file as an implementation detail | Conflicts with the current prohibition on plaintext secrets on disk |
| Share a generic process launcher with AI providers | Couples unrelated trust boundaries and encourages overly broad authority |
