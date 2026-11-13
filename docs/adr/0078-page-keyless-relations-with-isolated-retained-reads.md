# ADR-0078 — Page keyless relations through isolated retained reads

**Status:** accepted · **Date:** 2026-11-13

**Refines:** [ADR-0069](0069-turso-driver-hrana-over-http.md), [ADR-0068](0068-a-preview-comes-in-key-order.md) and [ADR-0077](0077-retain-preview-streams-for-native-pagination.md). A missing trustworthy key does not restrict a queryable relation to one page when its driver can retain a bounded read.

## Context

Views already reach the Data tab, but PostgreSQL, MySQL, SQLite, DuckDB, Turso and MongoDB still prepare independent, limited reads. Ordinary views, materialized views, keyless tables and MongoDB views/time-series therefore cannot advance. Declared keys are insufficient where NULLs, inheritance or informational constraints invalidate uniqueness.

Retaining an ordinary SQL cursor on the catalog session would block its worker, connection mutex or pool. Polling transaction state after a page can also wait behind that cursor. Opening a sibling from the latest saved settings is unsafe: settings may have changed while the original session still targets another database.

Turso's current pipeline response is a whole JSON document. Hrana 3 also defines a newline-delimited cursor response; its protocol and cleanup constraints are recorded in [RESEARCH-NOTES](../RESEARCH-NOTES.md).

## Decision

`PreviewRelation` remains the policy, audit and cancellation boundary. The frontend continues to use the existing page sizes and opaque result continuations. `PreviewShape::continuable` defaults to false, so samples retain bounded independent requests. Data browsing opts in. `Relation::preview_key` proves actual uniqueness separately from the cost guarantee `key_order_preview`; independent pages use only that proof. Other supported record-bearing relations use one retained read, including requested sorts with ties. No synthetic key, COUNT query or unordered OFFSET is introduced.

A prepared preview distinguishes bounded execution, retained execution with an independent protocol cursor, and retained execution on a dedicated session. The latter uses a driver-owned `Session::open_preview_session` operation: it preserves the source session's opening target, credentials, access options and applicable namespace context, rather than resolving edited saved settings. The driver refuses a session-local namespace it cannot faithfully reproduce; it never substitutes another database or a homonymous object. Routines needing arguments or capable of effects remain outside automatic data previews.

The executor reserves capacity before opening a retained read: at most 16 active or opening reads. Dedicated sessions are internal resources, not consoles or preferred catalog sessions. Continuations retain their logical owner; running-statement cancellation targets the actual execution session. Post-page transaction-state polling is skipped for these read-only previews; normal console guarantees are unchanged.

A registry-owned task holds each dedicated session through opening and closing. It drops the cursor before awaiting session close, retains capacity until cleanup completes, and is joined on session/connection shutdown. EOF releases the remote resource while previously read pages remain readable. Error, abandonment, last-result release, expiry and disconnect cancel and close it. The existing five-minute lifetime and Arrow history bounds remain in force. Authorized writes release retained previews on their connection before execution so an idle SQLite preview does not block that application's write. External writers can still encounter SQLite read locks: no implicit WAL or isolation change is made.

Turso's retained path uses Hrana 3 cursor framing with byte-bounded lines and batches, stable Arrow schema and explicit errors for incompatible later values. Stream batons stay in Rust and receive bounded cleanup. Credentials are confined to the configured origin, with one explicit routing exception: a configured HTTPS Turso Cloud endpoint may route to another HTTPS port-443 host under `.turso.io`. Custom endpoints cannot change origin; credentials, query strings and fragments in routing URLs are refused. HTTP redirects stay disabled. The existing bounded pipeline path remains available for ordinary requests.

Pagination availability belongs to the displayed result and session. A stale availability response cannot attach to replacement rows; availability errors are visible and explicitly retryable. Refresh and shape changes restart a retained read; Previous reads retained history. The UI states that a read stays open between pages and does not claim a universal snapshot.

## Object coverage

| Driver | Pageable record sources | Limits and non-record objects |
|---|---|---|
| PostgreSQL protocol | Tables, partitioned/foreign tables, views, materialized views and system relations; retained when key validity/cost is unproven, including inheritance and Redshift informational keys | Session-local temporary namespaces cannot be forked |
| MySQL protocol (including MariaDB) | Tables, views and system relations; independent proven keys, otherwise a private retained cursor | No automatic routine invocation |
| SQLite | Tables, views, loaded virtual tables and schema relations; private retained cursor when keys are nullable/unproven | File attachments are reopened read-only; temporary objects and attached memory state keep a bounded preview |
| DuckDB | Tables, views, attached catalogs and real `system` views | User temporary state keeps a bounded preview; built-in internal views are reproducible |
| Turso/libSQL | Hrana 3 tables and views through `/v3/cursor` | Older pipeline-only endpoints keep bounded previews, with independent pages only for a proven rowid alias |
| MongoDB | Collections, views (including missing/duplicate `_id`) and time-series | One native cursor; no invented unique key |
| ClickHouse | Tables, views, materialized views, dictionaries and system relations | Existing streaming cursor, no claim that its sorting key is unique |
| Elasticsearch/OpenSearch | Indices, aliases and data streams | Existing PIT/version and native-order restrictions remain explicit |
| Redis protocol | Hashes, sets, lists, sorted sets, streams, vector sets, time-series and TopK members | Strings, JSON and Bloom/cuckoo/count-min/t-digest summaries set `single_page_preview`: no pager |

Functions, procedures, sequences and other metadata-only objects keep their metadata views. Console, history and assistant results page their existing result buffer without re-execution. Unsupported continuation shows only a bounded preview size control and its reason; `Pagination::NotApplicable` hides the pager for scalars/summaries.

## Consequences

* **+** Record-bearing objects share a pager without inventing uniqueness or repeating arbitrary SQL.
* **+** Catalog exploration and unrelated tabs do not wait behind a suspended preview cursor.
* **+** Edited connection settings cannot silently redirect an existing preview to another target.
* **−** Retained reads consume bounded connections/cursors, snapshots and sometimes read locks until released.
* **−** A quota, expiry or a write can require refreshing before further navigation. Session-local state that cannot be reproduced remains an explicit limitation, never a fallback to another object.
* **−** A requested server sort can still require scanning the source before returning rows.

**Exit cost:** remove the isolated-session preparation mode and cleanup owner, and replace each driver's retained composer/transport. IPC paging and stored workspace formats do not change.

**Reconsider if** measurements show the connection or lock cost exceeds these bounds, or engines expose a continuation that releases its connection safely between pages.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Unordered OFFSET on views | Independent executions can skip or repeat records |
| Sort every column to manufacture an order | Duplicate rows still tie, and opening a preview may sort the whole relation |
| Retain the catalog session's SQL cursor | Blocks metadata, refresh or transaction-state reads |
| Cancel a preview whenever the catalog is expanded | Makes unrelated exploration silently destroy pagination |
| Reopen from current saved settings | Can read a new destination under an old catalog |
| Materialize a view before showing page one | Adds unrequested work and violates bounded streaming |
| Automatically invoke functions/procedures | Arguments and effects require a separate user action |
