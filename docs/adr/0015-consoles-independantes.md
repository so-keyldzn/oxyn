# ADR-0015 — Give each console its own controller and session

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0014](0014-documents-et-historique.md), for the contexts
opened from the Figma tabs and library.

## Context

The workspace currently has a single editor, a single execution and a single
console result. Its callbacks write directly into these fields. Changing only
the displayed text, or swapping these fields on every tab change, would make one
tab's responses land in another. Reusing the same session for all consoles
would also share their transactions.

## Decision

- `QueryConsole`, in `oxyn-app`, owns the editing, grid, status, confirmation and
  export entities, as well as their commands and cancellation tokens. Its
  callbacks stay attached to this entity, even when it is hidden.
- `Workspace` keeps the consoles and selects their views; the entity references
  used for drawing are references to the chosen controller, never copies of an
  execution's state. Catalog, table preview, library and preferences stay at the
  workspace level.
- A new console establishes a session through the bus before accepting an
  execution. It keeps the same connection configuration, but receives a real
  session identifier. The gesture is cancellable and runs no user SQL. Sessions
  created for consoles are closed when the consoles are closed; the initial
  session remains available to the catalog and the preview.
- A tab change re-runs, cancels and replaces no query. The value inspection
  attached to the tab being left is closed and cancelled. A pending confirmation
  on a hidden tab stays attached to that tab.
- Closing a console that carries a draft or an operation requires an explicit
  choice. Ambiguous writes have no implicit replay action.

## Consequences

- **+** Results, cancellations, exports and confirmations stay correlated with
  their console, regardless of focus.
- **+** Two new consoles do not share their server transaction.
- **−** Each additional console consumes a session and its resources.
- **−** Closing must coordinate draft, operations and session release; removing
  only a view is not enough.

**Exit cost:** merge the interface controllers and redefine the closing and
session contracts, without changing the Arrow format or the drivers.

**Reconsider if** a driver imposes a single session, or if an explicit shared
transaction mode becomes necessary. That mode will have to be visible and cannot
be inferred from a tab change alone.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Swap the content of a single editor and grid | Asynchronous responses attached to the wrong tab |
| Duplicate the whole workspace per console | Duplicates catalog, preferences and library, which belong to the window |
| Silently share a session between consoles | A transaction commit or rollback affects another tab |
