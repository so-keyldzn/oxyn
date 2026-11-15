# ADR-0083 — Run native database transfers as authorized, owned jobs

**Status:** accepted · **Date:** 2026-11-15

**Refines:** [ADR-0004](0004-command-bus.md) on native transfer commands and
[ADR-0035](0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md) on
owned I/O and audit outcomes. The implementing commit enables only the
restricted, verified mode below; it grants no general process or authentication
authority.

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

## Initial implementation boundary (2026-11-15)

The common job, policy/audit, grant, cancellation, progress, outcome and
invalidation contracts are engine-independent. The initial native adapter is
PostgreSQL; another wire-compatible product is not enabled by its driver ID.
Other engines and unproven authentication/platform modes return explicit
unavailability, through the same UI and status model.

The first authentication mode is a directly configured local PostgreSQL target
with a literal loopback IP, explicitly disabled TLS, and no SSH routing. The
adapter sets `require_auth=none`, disables GSS encryption, clears the inherited
environment, isolates HOME/cwd and redirects password/service configuration to
its own empty/private locations. It never resolves a saved password. A server
requiring authentication is refused, never served by ambient credentials.
This mode is offered only after the disposable authentication tests pass.
Remote/TLS/SSH/password modes and SQL-script restoration remain unavailable
until their destination-bound confinement has separate proof.

Installed 14.18 and 17.4 clients are unsuitable security baselines; dated
[verification notes](../RESEARCH-NOTES.md) record the vendor advisories and the
corrected release selected for isolated validation. Tool selections become
private executable snapshots, validated for role/version and retained until
owned processes finish. They grant neither an arbitrary argument vector nor
use of a replacement at the original path. A snapshot that cannot run with its
installed dependencies is unavailable; no silent original-path fallback occurs.

Unix process ownership uses a fresh process group and `waitid(WNOWAIT)` to
observe the leader without reaping/reusing its PID. Cancellation first gives
PostgreSQL its native signal-handler opportunity to request server cancellation,
then terminates remaining group members before the leader is reaped. All pipes
are drained with byte/time bounds. There is no detached reaper or unbounded
raw log. macOS and Linux share this Unix implementation; platforms without a
process-tree implementation stay unavailable. Local execution evidence and
unverified platforms are recorded in the completion ledger. Application
process loss is an interrupted/unknown job, not proof that a remote write rolled
back or a stored PID still belongs to the same process.

Exports stage in a private directory under a held destination-directory handle.
After successful tool exit and file synchronization, a no-clobber hard link
publishes the file atomically; staging is then removed and directory durability
checked. Existing paths, including symlinks, are refused. Unsupported hard-link
filesystems fail explicitly. No overwrite fallback is permitted.

Every restore requires the existing host-confirmation port, including local
connections. Backend-composed text names the actual destination, input archive,
selected tool and fixed options, without certifying archive safety. Ordinary
webview approval cannot bypass this path. Destination settings, read-only state,
file/tool identities and ownership are rechecked before execution. Any possibly
applied restore invalidates metadata/previews and remains unreconciled on
failure or interruption; it is never retried automatically.

The only native command-construction site is `files/tool.rs::confined_command`:
it immediately clears ambient environment and fixes private HOME/cwd/service
locations. Its statement-local `clippy::disallowed_methods` expectation records
this I-03 boundary. `process.rs::poll_pause` similarly documents its bounded
blocking-worker timer exception to the UI-thread sleep prohibition; it cannot be
called through an async/UI entry point. No global lint refusal is relaxed.
Inspection has a five-second deadline, archive preflight sixty seconds and native
execution twenty-four hours; cancellation retains ownership through cleanup.
A ready plan's authority expires after five minutes. These are manual fixed
bounds, not automatic workload thresholds or a performance certification.

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
