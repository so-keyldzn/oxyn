# ADR-0014 — Separate drafts, saved queries and history

**Status:** accepted · **Date:** 2026-09-10

## Context

Figma board `47:7638` presents History, Saved queries and Recent results. The
local tables exist, but their lists load the full SQL and documents do not
distinguish a draft from an explicit save. Adding autosave without this
distinction would modify a saved query while the user works on a draft.

## Decision

- Migration 5 keeps existing documents as saved queries. It adds the
  open/deleted state, the draft revision, the save revision and the explicitly
  saved text/title. The format remains made of readable SQLite columns, with no
  GPUI type.
- Draft and save revisions are independent. A recent draft must not cancel an
  earlier explicit save that completes later; the latter does not replace the
  recent draft.
- Closing with discard restores the saved copy, or empties a draft with no saved
  copy. A revision marker prevents an old write from reopening or recreating the
  document. Explicit deletion also removes the texts, while keeping this minimal
  marker.
- Lists go through the bus and remain paginated: at most 200 entries per page,
  SQL summaries limited to 256 characters. A selected document or entry is read
  separately. Editable SQL is bounded to 1 MiB and titles to 256 bytes; exceeding
  them produces an explicit error, never a silent truncation of the opened or
  saved text.
- History keeps its existing scope: the user's local connections, including
  deleted ones. It is not presented as a list necessarily belonging to the active
  workspace alone. Connection, text, date and status filters are explicit.
  Pagination follows descending recording order, with a stable identifier bound.
- A history entry can reference a result still retained. This reference restores
  no session and runs no query. An expired reference produces an explicit
  unavailability.
- Opening a document or historical SQL creates an editing context without
  execution. A copy to another connection is announced and keeps the original.
  The history of an ambiguous write offers inspection, not a replay action.

## Consequences

- **+** Recoverable drafts do not modify the saved library.
- **+** Asynchronous writes cannot restore a closed document.
- **+** Lists do not materialize all the local SQL in memory.
- **−** Each document keeps two text states and two revisions.
- **−** Small deletion markers remain in the store to protect against late
  responses; they do not appear in user lists.
- **−** SQL files exceeding the editing bound need separate handling and are not
  opened partially under their original name.

**Exit cost:** migrate the document columns and the save/close commands. Drivers
and SQL content remain independent.

**Reconsider if** collaborative editing requires content merging, if large
scripts become common use, or if marker retention needs an expiry policy tied to
local sessions.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| A single text for draft and save | Every keystroke modifies the saved copy |
| Immediately delete the row of a closed document | A late response can recreate it |
| Load all queries into the list | Memory proportional to the whole history |
| Replay to reopen a result | Changes the observed data and may repeat a write |
