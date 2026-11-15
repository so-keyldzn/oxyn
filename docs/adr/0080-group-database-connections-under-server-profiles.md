# ADR-0080 — Group database connections under server profiles without sharing authority

**Status:** accepted · **Date:** 2026-11-13 · **Implemented:** 2026-11-14

**Refines:** [ADR-0019](0019-contexte-de-session.md) on the distinction between
opening another database and changing a console's namespace, and
[ADR-0046](0046-workspaces-retenus-restent-connectes.md) on grouping retained
connections. The multi-base implementation preserves the existing security and
persistence contracts; the [completion ledger](../implementation-plan/completion.md)
records automatic validation and personal acceptance separately.

## Context

Issue [#373](https://github.com/so-keyldzn/oxyn/issues/373) asks to browse databases
under one server. At source snapshot `6647180`, the PostgreSQL driver lists only
its connected database and refuses another database in `SetSessionContext`.
MySQL already lists databases as namespaces. The
[engine matrix](../implementation-plan/schema-research-engines.md) records the
other hierarchies.

`ConnectionId` already scopes policies, catalog caches, results, conversations
and recovery. Replacing it with a server identity would mix unrelated authority.
[SECURITY](../SECURITY.md#a-secret-does-not-follow-its-connection-elsewhere)
also forbids moving a saved secret to an edited destination, database included.
The requirement is less configuration duplication, not broader implicit access.

Interface inspection at `f456ff1` adds an ownership constraint:
[`WorkspaceHost`](../../apps/desktop/src/features/workspace/workspace-host.tsx)
keeps one workspace per open connection, while
[`CatalogSidebar`](../../apps/desktop/src/features/workspace/catalog-sidebar.tsx)
binds its reads and actions to that workspace's `OpenConnection`.
[`useConsoles`](../../apps/desktop/src/features/consoles/use-consoles.ts)
can seed a new workspace from the previous SQL draft, and
[`nameTransfer`](../../apps/desktop/src/features/workspace/name-transfer.ts)
inserts into the editor under the pointer. Merely nesting database names in the
existing tree would leave these actions bound to the wrong owner.

## Decision

`ServerProfile` groups destinations and carries a stable `ServerProfileId`,
display name and validated non-secret connection defaults. It owns no
credential, transaction, result buffer or AI privacy tier.
`oxyn-core` owns this vocabulary; `oxyn-store` persists profiles and membership
in the existing open workspace format.

An opened PostgreSQL database remains an ordinary `ConnectionConfig` with its
own stable `ConnectionId`, full destination, environment, read-only marking,
privacy tier and independent secret reference. The group is not an execution
target. Every command, approval, object path and restored tab resolves to the
child connection before authorization. A database name alone is never an
identity; displayed names can be ambiguous.

The first opening presents the child destination and its protections. New
children use the existing cautious connection defaults rather than inheriting
a sibling's permissive markings. Authentication uses the current explicit
connection flow, with a separate credential entry for each child. There is no
automatic copying of a sibling's secret and no server-wide credential grant in
this revision. A future grant needs its own security decision.

`DiscoverDatabases` is a cancellable metadata command through `oxyn-exec`.
It uses an already authorized seed connection on that server. A new optional
driver/session discovery boundary defaults to `NotSupported`; it reports
validated names and known access hints, never credentials or a promise that
opening will succeed. PostgreSQL owns its discovery query. The host creates
or selects a child through existing connection authorization, then uses
`Connect`; discovery itself creates no child session. Existing
`CatalogProvider::list_catalogs` semantics do not change.

Driver-specific hierarchy stays authoritative: MySQL's namespace switch
remains a session-context operation; SQLite/DuckDB attachments do not become
remote databases. No automatic ATTACH, implicit server connection or all-base
introspection is introduced.

Profile edits change defaults for future children. Applying changes to existing
children is an explicit review of affected destinations, followed by their
ordinary update flow; live sessions are not silently retargeted. Unlinking a
child preserves its ordinary connection and history. Deleting a group does not
delete child connections or credentials implicitly; open work is handled by
the existing close/transaction rules.

Old connections migrate as ungrouped connections with unchanged IDs and secret
references. Membership is additional state, not a rewrite of existing history.
The retained-workspace bounds of ADR-0046 continue to count opened children;
listing databases consumes no retained-workspace slot.

### Local profile command contract

`ListServerProfiles`, `SaveServerProfile`, `RemoveServerProfile` and
`SetServerProfileMembership` operate on the executor's workspace. All four
are refused to an agent, including listing defaults. Their database intent is
`Read`, as for other local settings: saving a group does not execute database
DDL or change a child's authority. Local writes remain audited and owned
through completion on the blocking pool, including during desktop shutdown.

Saving uses an expected revision: absent for creation, the exact prior value
for an update. Revision exhaustion is refused before JavaScript loses integer
precision. Membership uses the observed previous group as its compare-and-set
token. Driver and workspace identities cannot change under an existing group;
membership cannot cross either boundary. Deleting the group removes only
membership, while the child's configuration and secret reference survive.

The core bounds each profile to 32 defaults, 64 bytes per key, 4096 bytes per
value and a 256-byte name. The store additionally bounds encoded defaults to
256 KiB, profiles per workspace to 256 and members per profile to 1024.
These are local-state input bounds, not prefetch or database-session quotas.
Parameter keys are revalidated against declared non-secret driver fields in
the executor, including on read-back; the store separately applies its existing
secret-key guard. Corrupt or excessive state fails explicitly, without returning
partial lists as complete or echoing supplied values.

### Discovery and child creation command contract

The optional discovery boundary lives on the session's `CatalogProvider`, whose
PostgreSQL implementation already owns cancellable, reserved metadata I/O.
`DiscoverDatabases` is a human-only read on its seed connection. Results are
bounded to 1024 names of at most 1024 bytes, refusing excess rather than silently
truncating. `CreateDatabaseConnection` has the same policy as `CreateConnection`:
no agent creation, explicit production approval naming the new destination.
Child persistence and membership are atomic. Strict server-parameter
comparison (excluding database and user) avoids guessing endpoint equivalence or moving a seed credential.
Session opening remains the existing separately authorized `Connect` operation.

### Interface ownership and activation

A common explorer belongs to the window's navigation shell. The content area
retains one workspace per child connection, with that child's tabs, result
buffers, inspector and assistant. The first revision does not merge tabs from
different bases into one tab strip. Activating an already open base reveals its
retained workspace without reopening sessions or changing any draft.

Explorer selection, expansion and activation are separate gestures. One click
selects a node; its disclosure control expands it. Enter, double-click or
`Open workspace` explicitly activates a base; opening one of its objects
activates that base before opening the object. Header, environment, read-only
marker and assistant always describe the active content, not the selected node.
An inactive base can remain connected, running work or holding a transaction.

Navigation has typed targets: profile, discovered database, child connection,
and object with child connection plus full catalog address. Object actions,
search results, menus, refresh, new-console requests and map openings carry that
owner; the host validates it. `ConnectionSummary.location` stays a display
label, never a value parsed to reconstruct a target.

Each database has independent discovery/opening/error state and cancellation.
Opening another base does not freeze the whole explorer. Per-window opening
reservations count toward the existing connection bound; repeated activation
coalesces on the same child. Late completion of a cancelled opening releases
only resources it owns, not a session already retained by another operation.
States distinguish discovered, saved, opening, connected, active, cancelling
and local failures. The ordinary close review handles pending disconnection;
a failed discovery is not an empty server.

### Text routing and context

Within this new grouped navigation, a newly opened base starts with an empty
console. This refines the automatic draft copy described in UX-SPEC's first
workspace navigation; the separate start-screen flow keeps its existing copy
behavior. A future explicit cross-database copy must name source and destination
and create an unexecuted copy; it is not an implicit activation action.

Text insertion requests name their destination connection and console/session
generation; a later change of active base cannot redirect them. Dropping an
object onto another base's editor does not insert automatically: offer
`Open a console on the source database` or Cancel. The same destination
discipline applies to assistant and inspector requests. A namespace selector
changes only the confirmed context of its console; it does not activate a
different PostgreSQL child connection.

### Profile setup, editing and group disconnection

Setup explicitly chooses an already authorized seed connection, or creates
one through the ordinary connection flow before discovery. Saving defaults
alone does not authenticate a profile. Opening a discovered child reviews its
destination and protections and requests its independent credentials.

Profile, base and object menus expose different operations. `Disconnect group…`
shows every affected child, its consoles, unsaved work, running operations and
reported transaction state, including unknown. Unresolved transactions block
completion until the existing per-session review resolves them; no implicit
commit or rollback is introduced. Cancellation of the review closes nothing.
Completed child closes are reported individually if a later close fails.
Closing resources follows existing multi-window ownership, not a global kill.

Disconnecting the active child activates another retained child if available,
otherwise shows a neutral connection view. Profile deletion remains separate
from disconnect, unlink and any database DDL operation.

## Consequences

* **+** Server grouping reuses the connection-based safety and recovery model.
* **+** Existing engine hierarchies and old workspace identities remain usable.
* **−** Opening another authenticated database can require entering its credential.
* **−** A profile edit does not instantly synchronize every child; the review is
  deliberately visible, and independent sessions still consume resources.
* **−** Users switch workspaces to see another base's tabs; a common tab strip
  is deliberately outside this revision. The explorer needs its own navigation
  state and per-child asynchronous operation tracking.

**Exit cost:** remove profile membership and its UI, leaving ordinary child
connections intact. Removing per-base identities instead would require migration
of policies, results, documents and conversation references; that is the costly
alternative this split avoids.

**Reconsider if** measured workflows cannot tolerate per-base credential entry,
or a driver cannot preserve its native hierarchy under this grouping. A shared
credential design must define exact destination authorization, revocation and
live-session behavior before changing this decision.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| One mutable connection for the whole PostgreSQL server | Changing the selected base could change the meaning of pending work and mix policy/cache identities |
| Copy credentials automatically between children | Broadens their destination without the authorization SECURITY requires |
| Open every discovered database | Consumes sessions and performs unrequested work before the user chooses a base |
| Replace every driver's hierarchy with the PostgreSQL model | Regresses engines that already expose multiple namespaces through one connection |
| Rewrite old connections as grouped children during migration | Risks changing history and secret ownership for a visual feature |
| Add database rows while keeping one workspace-bound catalog handler | Menus and asynchronous reads could act through the active base rather than the node's base |
| Merge all bases' tabs immediately | Widens ownership changes across execution, inspector, assistant and close/recovery flows |
| Automatically copy the previous draft during grouped base opening | Makes navigation prefill text for a different destination without an explicit copy gesture |
