# ADR-0016 — Serialize a document's writes in a bounded queue

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0014](0014-documents-et-historique.md), for autosave and
concurrency between instances.

## Context

Sending a 1 MiB text on every keystroke with no queue bound would pile up copies
while the store is blocked. Making the save wait on the UI thread would violate
I-05. An autosave must neither modify the named copy, nor replace the work of
another editor that opened the same document.

## Decision

A backend-side `DocumentWriter` serializes a console's writes: one active
operation, at most one latest pending draft, one named save and one close.
Intermediate drafts are replaced by the most recent one. An explicit save keeps
its snapshot until it is processed. Closing withdraws uncommitted drafts and
forbids new submissions.

Writes use an expected revision, checked in the SQLite transaction before
mutation. A conflict or an unreadable state stops the queue; the view keeps its
text and offers a separate copy. No user SQL is executed. The old commands
without an expected revision remain readable for compatibility, without being
used by autosave.

The close counter belongs to the backend and covers the whole active queue, not
only its current operation. The view disappearing therefore does not lose the
latest draft already submitted. A document closed before its first write receives
a marker without text so as to refuse any late write.

## Consequences

- **+** Pending memory is bounded per document.
- **+** Drafts remain distinct from explicitly saved copies.
- **+** The revision check prevents a stale editor from overwriting another.
- **−** A concurrency error requires a copy or retry decision.
- **−** Closing and saves share a local scheduler.

**Exit cost:** replace the queue and its acknowledgements in `oxyn-app` and the
revision check in `oxyn-store`, without changing the drivers or Arrow.

**Reconsider if** collaborative editing requires merging, or if scripts must
exceed the 1 MiB bound decided in ADR-0014.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| One task carrying the text on every keystroke | Unbounded queue of copies |
| A delay held only by the view | Last keystroke lost if the view disappears before it fires |
| Increasing revision without an expected revision | A stale editor can end up exceeding another's counter |
