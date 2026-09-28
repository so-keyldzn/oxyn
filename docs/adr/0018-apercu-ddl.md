# ADR-0018 — Inspected DDL remains metadata, prepared separately from its execution

**Status:** accepted · **Date:** 2026-09-10

## Context

Mockup `193:2433` shows a 424 px DDL panel, the associated indexes, Copy DDL and
Open DDL in console. A creation text is neither a data read nor an authorization
to apply it. SQLite keeps its declarations; PostgreSQL mainly exposes their
components and rendering functions. A definition can also exceed the 1 MiB limit
of editable documents.

## Decision

`CatalogProvider::relation_definition` is a cancellable read, conditioned on
`OBJECT_DEFINITION`. It goes through `CatalogRefreshScope::Definition`.
`RelationDefinition` carries SQL, provenance and scope notes. The driver and the
bus validate the size before atomic publication; an error or a cancellation keeps
the previous cache. The Debug rendering omits the SQL.

SQLite reads object, indexes and triggers in a single cursor over `sqlite_schema`
and qualifies their declaration names. PostgreSQL rebuilds the statements from a
consistent catalog read. Objects that cannot be rebuilt are explicitly refused; no
Figma example replaces a real response. The scope is the creation of the object
and its associated elements, without data, privileges or external dependencies.
Provenance and limits remain visible.

The preview uses a read-only editor, replaced on every new definition so as not
to accumulate old texts in the undo history. Its loading has an identity and a
cancellation independent of the other metadata tabs. Responses for an object
that was left are ignored.

Copy DDL copies the displayed text. Open DDL in console creates a new console
through the existing document copy path; it preserves existing consoles and
executes nothing. Any later execution goes through the normal policy and its
confirmations. After a failed refresh, the previous text is marked as possibly
stale.

The preview is bounded to 1 MiB, its notes to 32 entries and 64 KiB. Each
connection's cache keeps at most 16 definitions and 16 MiB cumulated of SQL and
notes. Old definitions, invalidated ones included, are evicted without touching
the other metadata; the newly published definition is kept. The 8 px handle
resizes the panel between 320 and 640 px; Home restores 424 px. The width is
kept in the open workspace. Compact mode gives access to the DDL through its
sub-tab, with no new read on a mere resize.

## Consequences

- Viewing and preparing the SQL introduce no execution path parallel to the
  bus.
- A text that is too large is refused instead of being partially copied.
- The PostgreSQL rebuild requires validations against the engine; it does not
  constitute a complete database export.
- The panel width is not yet a setting persisted between launches.

**Exit cost:** changing the public format requires an evolution of the contract,
the cache and the reader; the SQL remains recoverable as text. The editor and the
geometry are confined to `oxyn-app`/`oxyn-ui`.

**Reconsider if** generation becomes a complete schema export, if a definition
must be modified in place, or if the document cap changes.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Copy the mockup's example SQL | Does not describe the connected database |
| Replace the active editor | Risk of losing the user's draft |
| Execute when the console opens | Confuses preparation and authorization |
| Give a partial rebuild without saying so | Presents an omission as a complete definition |

The external sources and checks are in [RESEARCH-NOTES](../RESEARCH-NOTES.md).
