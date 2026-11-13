# ADR-0079 — Retain temporary previews on their owning session

**Status:** accepted · **Date:** 2026-11-13

**Refines:** [ADR-0078](0078-page-keyless-relations-with-isolated-retained-reads.md), for state that a sibling cannot reproduce, and [ADR-0019](0019-contexte-de-session.md) / [ADR-0050](0050-mysql-driver-on-mysql-async-prepared-first.md) for physical session ownership.

## Context

Temporary relations, private in-memory databases and session-local attachments cannot be copied to an isolated reader without changing what is read. The current bounded fallback loses pagination. Retaining their cursor directly without coordination would block the owning SQL worker or connection indefinitely. The connection-level catalog cache and object tabs also cannot distinguish homonymous objects owned by different console sessions.

## Decision

`PreviewMode::Original` retains a read on its original session. `oxyn-exec` reserves that session exclusively for the cursor's lifetime. Other operations on that session fail promptly with an actionable busy error; cancellation and closure remain available. Other sessions remain usable. EOF, explicit release, cancellation, expiry and connection/session closure release the cursor and reservation. The existing history, lifetime and global cursor budgets apply. An owned cleanup task keeps the reservation and capacity until `finish_original_preview` acknowledges physical cleanup; EOF and explicit `ReleasePreview` wait for that acknowledgement. No client-side whole-result materialization, OFFSET replay or temporary-state copying is introduced.

Ordinary operations hold shared reservations through their actual operation or cursor lifetime. Original previews claim exclusive ownership before waiting for already-running operations; the command’s cancellation and time budget bound that wait. A second original read is refused promptly rather than queued behind an idle cursor. This prevents a catalog read racing the reservation from waiting behind a suspended cursor. Transaction-state queries report unknown while the exclusive read is held; they never assert idle. A connection-level admission guard excludes new preview openings from an authorized mutation’s cleanup snapshot through completion. Openings hold shared admission until their first result is bound. Disconnect seals admission, cancels pending reads and joins cleanup. Shutdown permanently seals the registry; cancelled continuations cannot bind new result aliases.

`Connect` opens a catalog session and `OpenConsoleSession` opens an execution session. Their immutable Catalog/Console/Preview roles are independent of which catalog a connection prefers: another window’s catalog is never mistaken for a console. `RefreshSessionCatalog` validates and reads the actual requested owner. The original session identity travels through catalog refresh, cached metadata, object tabs, preview stores and result continuations. Session catalogs have separate contents and share one per-connection budget of 1,024 scopes and 50,000 objects. Eviction is keyed by owner and scope; a closed owner releases its charges. Replacing the preferred catalog installs a fresh generation, never reuses its contents. Publication and owner closure are ordered. Session-local tabs are ephemeral: restoring a workspace must not replace them with a persistent homonym. The user can select an open console as the explorer's source. Closing it releases its previews and invalidates its object tabs. A source with an active preview cannot move to another window; session use and transfer are coordinated. Ephemeral object tabs offer metadata and data, while operations that open a separate execution session remain in the owning console.

SQL drivers choose an original read when their state is not reproducible. Any session that has accepted arbitrary user SQL also keeps previews on its source: variables and settings can change reads without creating temporary objects. PostgreSQL logical sessions must retain a single physical backend rather than distribute temporary state among pooled backends. Losing that backend ends the session-local context explicitly; continuation cannot reconnect and substitute a new backend. PostgreSQL original previews use a non-holdable cursor with streamed FETCH windows of 200 rows and the existing 1 MiB batch threshold. Normal idle release rolls back only the preview’s own read-only transaction. Cleanup has a ten-second bound; an unproven restoration discards the physical connection and subsequent use refuses lost state. Redshift uses its documented DECLARE syntax and server-side cursor limits. An ordinary PostgreSQL setup error restores the preview transaction before returning the error, preserving temporary data. MySQL temporary columns and indexes are read from SHOW on the source, rather than the persistent homonym in information_schema; an explicit named-table route covers names the server does not enumerate. A confirmed persistent view remains readable with SELECT alone when SHOW CREATE is denied for lack of SHOW VIEW. PostgreSQL/MySQL never silently replace a physical connection after user SQL may have created private state. Drivers without session-local SQL state keep their existing native continuation behavior. Samples remain bounded.

The frontend reads initial metadata before reserving a source, releases the previous cursor before changing shape, and releases a data tab explicitly rather than treating a React unmount as a close. Retained navigation is proved by its result binding even after metadata eviction. Row templates validate both the window’s readable result and its connection.


Turso keeps a separate stream per ordinary statement; explicit temporary-object creation is refused instead of reporting success for state that disappears immediately. This change does not invent persistent Hrana user sessions.

## Consequences

* **+** Temporary data can be paged without changing its owner or replaying the query.
* **+** Two sessions may hold identically named objects without sharing metadata or rows.
* **−** The owning session cannot execute unrelated work while an original read remains open; the interface explains how to release it.
* **−** The five-minute lifetime and retained-history budget still require a refresh when exhausted. A closed source cannot be reconstructed.
* **−** Serial physical ownership reduces concurrency within a PostgreSQL logical session; independent consoles retain separate sessions.

**Exit cost:** remove the exclusive mode and reservations, session-scoped catalog routing and ephemeral tab identity. Existing bounded and dedicated previews remain separate and unchanged.

**Reconsider if** a driver can suspend a physical cursor while safely interleaving other operations without invalidating it, or the session occupancy proves unsuitable in measured use.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Reopen a sibling | Temporary and private-memory state belongs to the original session |
| Leave metadata queued behind an idle cursor | The user cannot distinguish a deadlock from a slow server |
| Cancel the read whenever metadata is requested | Unrelated browsing destroys pagination without an explicit action |
| Share one connection-level cache | Homonymous temporary objects can acquire another session's columns and rows |
| Replay unordered pages | Rows can be skipped or repeated |
