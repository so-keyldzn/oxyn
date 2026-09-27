# ADR-0042 — Drop, Truncate and Rename run from an in-place review, like ordinary user SQL

**Status:** proposed · **Date:** 2026-09-25

**Clarifies:** [ADR-0025](0025-proposition-de-changement-de-schema.md), on the
following point: a gesture composed by Oxyn never executed anything. For the
three entries `Drop…`, `Truncate…` and `Rename…` of the catalog's context menu,
the composed text **can** be executed, from a review box and without going
through a console. The rest of ADR-0025 stays in force: `Propose change…`
still composes a fully commented template, opened in a console, and its scope
still excludes `DROP TABLE`.

**Complements:** [ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md),
on an approval path it could not name: `run_object_operation` approves a held
command without going through `decide`. When the approval is critical in the
sense of its § 1, family 1 — a write or a DDL on a `production` connection —,
it goes through the same check as `decide`, hence through the native dialog,
and the webview's box grants none.

## Context

The catalog's context menu (ADR-0041) carries `Rename…`, `Truncate…` and
`Drop…`. The user settled that they are part of V1, and that they run from a
review box — the shadcn `alert-dialog` component — rather than by opening a
console. Through ADR-0025's path, a `DROP TABLE` would cost four gestures: open
the console, uncomment, `Run`, approve. It would also leave behind an
autosaved document for a statement played once.

What the code imposes, surveyed on 2026-09-25:

* **The policy already demands an approval.** `DefaultPolicy::authorize`
  (`oxyn-core/src/policy.rs`) returns `RequireApproval` for any non-null
  `MutationRisk`, whatever the actor or the environment. `oxyn-query` classifies
  `DROP …` as `Ddl` + `MutationRisk::DropObject`, `TRUNCATE` as `Ddl` +
  `MutationRisk::Truncate`, and `ALTER TABLE … RENAME` as `Ddl` without risk.
  The latter is therefore `Allow` for a human outside production, and subject
  to approval in production.
* **Approval already has its screen, and its guarantee lies elsewhere.**
  `ApprovalDialog` (`apps/desktop/src/components/oxyn/approval-dialog.tsx`) is
  an `alert-dialog`: `Cancel` has the initial focus and Enter alone does not
  approve. It is the screen of a command held in the console and on the
  connection screen; its consent calls `decide`. Since
  [ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md),
  `Backend::decide` first calls `confirm_held`
  (`crates/oxyn-desktop/src/backend/confirm.rs`): for a mutating command of an
  `Actor::Human` that the gate holds as `production` at approval time, nothing
  runs without the native dialog, composed in Rust; otherwise `confirm_held`
  returns `NotCritical` and the webview's consent suffices. What an XSS can do
  with a webview button, it can do with a direct call to the Tauri command: on
  `production`, only the confirmation drawn by the host is a guarantee.
* **Invalidation is already there.** After a successful `Execute` of `Ddl`
  intent, the executor calls `invalidate_catalog` then publishes
  `Event::CatalogUpdated` (`oxyn-exec/src/executor.rs`), and the subscribers of
  [ADR-0022](0022-rafraichissement-automatique.md) reread the tree. None of
  this happens on the error path.
* **Three possible sessions, none suitable as is.** The initial session is
  reserved for the catalog and the preview
  ([ADR-0015](0015-consoles-independantes.md)). Its introspection reads hold
  the session lock until they end (`oxyn-exec/src/sessions.rs`): a `DROP`
  waiting there for a table lock would freeze the connection's whole tree until
  the 30 s timeout. A console's session carries the transaction the user may
  have opened. A `DROP` joining it would be undone by its `ROLLBACK`, or would
  hold its lock until its `COMMIT`.
* **Quoting exists, and one of its styles does not quote.**
  `oxyn_catalog::quote_identifier` with `QuoteStyle::Bare` returns the name as
  is. `CatalogPath::qualify_sql` and `QuoteStyle::for_dialect` never choose
  that style.
* **Capabilities do not say what the box must say.** No `Capabilities` flag
  indicates whether DDL is transactional, whether the engine refuses to drop an
  object others depend on, or whether `TRUNCATE` exists.

What the engines do, and the source of each fact:

| Fact | Source, date |
|---|---|
| PostgreSQL: `DROP TABLE` defaults to `RESTRICT`, which will "refuse to drop the table if any objects depend on it" | [sql-droptable](https://www.postgresql.org/docs/current/sql-droptable.html), version 18, read on 2026-09-25 |
| PostgreSQL: `TRUNCATE` by default refuses a table referenced by a foreign key, and "is transaction-safe" | [sql-truncate](https://www.postgresql.org/docs/current/sql-truncate.html), version 18, read on 2026-09-25 |
| PostgreSQL: DDL is transactional, except creating and dropping a database or tablespace | [PostgreSQL wiki, Transactional DDL](https://wiki.postgresql.org/wiki/Transactional_DDL_in_PostgreSQL:_A_Competitive_Analysis), read on 2026-09-25 |
| Redshift: `TRUNCATE` "commits the transaction in which it is run"; `DROP TABLE` defaults to `RESTRICT` | [r_TRUNCATE](https://docs.aws.amazon.com/redshift/latest/dg/r_TRUNCATE.html), [r_DROP_TABLE](https://docs.aws.amazon.com/redshift/latest/dg/r_DROP_TABLE.html), read on 2026-09-25 |
| MySQL 8.4: `DROP TABLE`, `TRUNCATE TABLE`, `RENAME TABLE` and `ALTER TABLE` cause an implicit commit | [implicit-commit](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html), read on 2026-09-25 |
| SQLite: no `TRUNCATE` statement (`near "TRUNCATE": syntax error`); `DROP TABLE` succeeds while a view references the table; with `foreign_keys` at `0`, it also succeeds while child rows reference it; `BEGIN; DROP TABLE t; ROLLBACK;` gives the table back | observed on 2026-09-25 with the `sqlite3` 3.51.0 client; the driver embeds SQLite 3.50.2 ([RESEARCH-NOTES](../RESEARCH-NOTES.md)) |
| The SQLite driver never sets `PRAGMA foreign_keys`: foreign keys are not checked there by default | `drivers/oxyn-driver-sqlite/src/session.rs`, module header |

## Decision

**The box composes, shows, then submits this text through the path of any
user SQL: `Command::Execute`, under `Actor::Human`, on a session opened for it.
No `Command` variant is created, and no execution path either.**

### What is composed, and where

`crates/oxyn-desktop/src/backend/object_operations.rs` carries
`Backend::review_object_operation(connection, address, operation)`. It is a
read of the catalog cache, without network I/O or `Command`. It returns an
`ObjectOperationReview`: the SQL, the connection's name and environment read
from its `ConnectionConfig`, the capabilities that govern the box's text, and
the known dependencies. `operation` is `Drop`, `Truncate` or
`Rename { new_name }`. The front end composes no SQL: each keystroke in the new
name field asks the backend for the composition again.

* The object's name is qualified as for the preview: schema and table in the
  session's database for PostgreSQL, catalog convention for attached SQLite
  databases. Each segment goes through `quote_identifier` with
  `QuoteStyle::for_dialect(oxyn_query::dialect_for(&config.driver))`, never
  `Bare` ([I-10](../../CLAUDE.md#i-10)). A `Rename`'s new name is quoted the
  same way. The box shows the quoted result, so `Orders` displays as
  `"Orders"`.
* The text is **a single statement, without a comment**. What is displayed is
  what is sent, and what is recorded in `query_history`. ADR-0025's commented
  header served a text meant to travel to a ticket. Here, the trace is the
  history and the journal. A comment would moreover go to the server and its
  logs, with the connection's local name.
* The composed forms are: `DROP TABLE|VIEW|MATERIALIZED VIEW <object>`
  depending on the `RelationKind`, `TRUNCATE TABLE <table>`,
  `ALTER TABLE <table> RENAME TO <name>` and
  `ALTER TABLE <table> RENAME COLUMN <column> TO <name>`. No `IF EXISTS`: an
  object already gone must fail. Otherwise, a stale cache would pass for a
  success.
* **`CASCADE` is never in the default text.** The box offers a `CASCADE`
  checkbox, unchecked at every opening and never remembered, only if the
  session declares `RESTRICT_DEPENDENTS` (see below). Checking it makes the
  backend recompose the text. It also makes typing the object's name mandatory
  **in every environment**, because the actual scope of the drop is no longer
  known in advance.
* A name carrying a control character (Unicode category `Cc`), U+2028,
  U+2029, or a direction control (U+202A–U+202E, U+2066–U+2069) is not
  composed. Escaping these characters, as `visible` does in
  `backend/proposal.rs`, would designate another object than the one executed.
  The entry is greyed out with the reason
  `This name holds control characters: write the statement in a console.`

### Availability: by capability, never by product name

Three flags are added to `Capabilities` (`oxyn-core/src/capabilities.rs`), in
the free range following `PREVIEW_FILTER` (bits 44 to 46):

| Flag | What it asserts | Declared today by |
|---|---|---|
| `TRUNCATE` | the session accepts the `TRUNCATE` statement | PostgreSQL, Redshift |
| `TRANSACTIONAL_DDL` | every accepted DDL statement, `TRUNCATE` included, applies whole or not at all, and obeys the surrounding transaction | PostgreSQL, SQLite; **not** Redshift, whose `TRUNCATE` commits the transaction |
| `RESTRICT_DEPENDENTS` | without `CASCADE`, the engine refuses `DROP` and `TRUNCATE` as long as another object depends on it | PostgreSQL; **not** Redshift, whose `TRUNCATE` ignores foreign keys; **not** SQLite |

A driver declares none of these flags without the integration test that
proves it against the engine it embeds or reaches. The source table above
justifies the declaration, but does not replace that test.

| Entry | Offered if the catalog session declares | Otherwise, greyed out with |
|---|---|---|
| `Drop…` | `DDL`, on a table, a view or a materialized view | `This connection does not accept schema changes.` |
| `Truncate…` | `DDL` and `TRUNCATE`, on a table | `This database has no TRUNCATE statement.` |
| `Rename…` | `DDL`, on a table or a column | `This connection does not accept schema changes.` |

On SQLite, this gives `Drop…` and `Rename…` offered, and `Truncate…` greyed
out. A read-only connection loses `DDL` and greys out all three entries; the
`PolicyGate` would refuse it anyway.

### What the box shows

The box is an `alert-dialog`. It applies
["Destructive operations"](../UX-SPEC.md#destructive-operations):
`Cancel` has the initial focus, Enter alone validates nothing, and the default
button is never the action. It shows:

1. **the whole SQL**, in a read-only, selectable block;
2. **the connection and its environment**, through `EnvironmentBadge`. An
   environment that is not set is displayed and treated as `production`
   ([I-02](../../CLAUDE.md#i-02),
   [SECURITY](../SECURITY.md#connection-marking));
3. **in `production`**, a field to type the object's name, without automatic
   correction or capitalization. The action button only enables on an exact
   match, case included, with the unqualified name. It is the current name for
   a `Rename`, and the column's name for a column rename. **This input is an
   aid against mistakes, not a guarantee**: it forces reading the name of the
   object targeted by a click in the tree, nothing more. The backend does not
   receive it and does not check it — a script that calls
   `run_object_operation` knows the name and would provide it. On
   `production`, what is a guarantee is the native dialog that follows the
   gate's decision ("What is executed");
4. **the known dependencies**, for a table: the incoming foreign keys, read by
   `refresh_relation_facet` with `RelationFacet::IncomingKeys`, hence
   `RefreshCatalogScope::IncomingForeignKeys`, when the box opens if the cache
   does not have them. The button waits for this read to finish. A failed read
   says so, and is never read as "none". Without `INCOMING_FOREIGN_KEYS`, the
   box writes `Dependents are not reported by this connection.` In every case,
   it writes `Views, routines and triggers that use this object are not listed.`:
   no capability exposes them today ([ADR-0003](0003-driver-capabilities.md));
5. **the engine's safety net**. With `RESTRICT_DEPENDENTS`:
   `Without CASCADE, the server refuses if other objects depend on it.` Without:
   `This database drops the object even if other objects still use it.`;
6. **the transactional nature**. With `TRANSACTIONAL_DDL`:
   `Applied whole or not at all. Once it succeeds it is committed: there is no undo.`
   Without: `This database does not run DDL in a transaction: it cannot be rolled back, and a failure may leave part of it applied.`

### What is executed, and on which session

The action button calls a Tauri command,
`run_object_operation(command_id, connection, operation, sql)`, which
delegates to `Backend::run_object_operation`. It does, in order:

1. it opens a session **specific to the review** through `Command::Connect`,
   the pair `open_console` already uses. This session is in autocommit,
   inherits no session context ([ADR-0019](0019-contexte-de-session.md)) and no
   open transaction. A lock being waited on freezes neither the catalog nor a
   console;
2. it checks on **this** session the required capabilities (`DDL`, and
   `TRUNCATE` for `Truncate`), because capabilities are evaluated per session.
   It also checks that `oxyn_query::split` finds only one statement, and that
   its classification matches the announced operation: `DropObject` for
   `Drop`, `Truncate` for `Truncate`, `Ddl` without risk for `Rename`. It is
   not an authorization, which remains with the `PolicyGate`. This check
   guarantees that the button's label says what is sent;
3. it dispatches `Command::Execute { connection, session, request }` through
   `Backend::run`, hence `Executor::dispatch_as(id, Actor::Human, …)`. The
   request is built as in `Backend::execute`: `ExecRequest::new`, with
   `limits.read_only = config.read_only`. From here, nothing differs from a
   console `Run`. `oxyn-query` reclassifies the text, the `PolicyGate` decides,
   `audit_journal` and `query_history` record the row, and the default timeout
   cancels on the server side;
4. on `NeedsApproval`, it calls `confirm_held`, the very function that
   `Backend::decide` calls before any approval
   ([ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md), § 1
   and § 4). It is this function, and not the box, that says whether the
   decision is critical:
   * **`Confirmed`** — the connection is `production` in the gate's sense, and
     the user confirmed in the native dialog, which the backend composes with
     the intent, the gate's reason and the statement's text. The command is
     approved as by `decide`, and nothing goes back through the webview;
   * **`Refused`** — Cancel, closing, confirmation within the first second,
     deadline: the command is rejected and the decision consumed, as by
     `decide`;
   * **`NotCritical`** — outside `production`, the command stays pending and
     `run_object_operation` returns `NeedsApproval` to the box;
   * **another critical dialog is open** — refusal without consumption
     (ADR-0037 § 3): the command stays pending, `run_object_operation` returns
     `NeedsApproval` with that reason, and `ApprovalDialog`'s consent, which
     goes through `decide`, will ask for the native dialog again;
5. it closes the session through `Command::CloseSession` as soon as the
   outcome is terminal. On `NeedsApproval` returned to the box, it keeps the
   session, indexed by the `CommandId`, and closes it when `decide` returns,
   whether the command is approved, rejected or expired.

**The box grants no approval.** On `NeedsApproval` returned by the backend —
`Drop` and `Truncate` outside `production`, or a critical dialog already open
—, it gives way to `ApprovalDialog` in the same surface, without stacking a
second modal. It displays there the reason and the preview the gate returned
on the reclassified text. An approval given before the gate has spoken would
concern a decision that does not exist yet. On `production`, it is the backend
that chains to the native dialog, after the gate: the webview has no call that
approves a critical command.

The path, by environment:

| Operation | Outside `production` | `production` |
|---|---|---|
| `Rename` | review, then execution: the gate returns `Allow` | review and typed name, then native dialog |
| `Drop`, `Truncate` | review, then `ApprovalDialog` | review and typed name, then native dialog |

Two decisions in every case where the gate holds the command, the second of
which, in `production`, is the only one an XSS cannot take in the user's place.

During submission, `Stop` reaches the session opening as well as the
statement: the dispatch's `CancelToken` is that of `Backend::track`.

### After execution

* **Success.** Invalidation and `CatalogUpdated` come from the executor, as for
  any DDL; the box adds no refresh. It closes, and after a `Drop` the catalog
  selection moves to the parent. Nothing is optimistic
  ([UX-SPEC](../UX-SPEC.md#what-is-never-optimistic)): the box waits for
  the `Outcome` before removing anything from the tree.
* **Permanent error.** The server's message is displayed as is. For a `DROP`
  refused because of dependencies, the box offers `Open in console`, which
  takes ADR-0025's path with the uncommented text. It is the only place where
  the user can write `CASCADE` themselves without the checkbox.
* **Ambiguous error or cancellation after sending.** `OxynError::Timeout` and
  `OutcomeUnknown` fall under `ErrorClass::Ambiguous`. The box writes
  `The server may have applied this. Nothing will be retried. Refresh the catalog to see the current state.`
  It offers `Refresh catalog`, which emits `RefreshCatalogScope` on the parent
  namespace, and no retry button
  ([I-13](../../CLAUDE.md#i-13),
  [DRIVER-CONTRACT §4](../DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).
  Cancellation obtained through `Stop` during the statement receives the same
  text. A cancelled DDL may have been committed before the cancellation
  arrived.
* **Failure to open the session.** No statement was sent. The box says so and
  allows submitting again.

### Trace and provenance

The trace is that of any executed SQL: an `audit_journal` row with
`Actor::Human` and the gate's decision, and a `query_history` row with the
exact text. **No provenance**: no document is created. The text, composed by
Oxyn, is written neither by the user nor by an agent, and that is the rule
ADR-0025 applies to `Propose change…` and to the linked-query template of
`metadata.rs` ([ADR-0023](0023-fournisseurs-declares-et-provenance.md)).
`query_history` does not distinguish a row coming from the box from a row
coming from a console. This is accepted: the user reread and launched this
text.

### Never for an agent

The three entries are human actions of ADR-0041's registry. They appear in no
menu and no list built for an `Actor::Agent`. No `oxyn-ai` tool reaches
`review_object_operation` or `run_object_operation`. A test
`object_operations_are_not_tools`, next to
`le_changement_de_schema_n_est_pas_un_outil` in `oxyn-ai/src/tools.rs`, fails
the day such a tool appears. An agent that wants to drop a table writes SQL
like anyone else. In production, the gate **refuses** it
([I-02](../../CLAUDE.md#i-02), [I-07](../../CLAUDE.md#i-07)). Elsewhere, it
requires an approval. This decision changes nothing in that policy.

### Several windows

The box, its review session and the approval it waits for belong to the
window whose menu opened it (ADR-0043). Closing that window rejects the pending
approval and closes the session. A native dialog left on screen — the plugin
cannot close it (ADR-0037 § 3) — then approves nothing anymore:
`confirm_held` only confirms the decision it showed, and it is no longer
pending. Invalidation concerns the connection: every window that displays it
receives it through `CatalogUpdated`.

## Consequences

* **+** I-01 holds by construction. The text takes `Command::Execute`, and
  everything that protects the user's SQL protects it: reclassification, gate,
  journal, history, cancellation, invalidation.
* **+** A `DROP` costs two decisions, instead of four gestures and a leftover
  document.
* **+** On `production`, approving a `DROP`, a `TRUNCATE` or a rename has no
  path of its own: it is that of any write, through `confirm_held` and
  ADR-0037's native dialog. What an XSS can do with the box is limited to
  opening that dialog.
* **+** What the box asserts comes from capabilities declared and tested per
  driver, not from the product name. Redshift, which speaks the PostgreSQL
  protocol, therefore receives the text that fits it.
* **−** **Two steps** for `Drop` and `Truncate`, even locally: the review, then
  the gate's approval. In production, the name must additionally be typed
  before the native dialog. The second step will seem redundant, and it is the
  one habit will end up clicking through — yet in production, it is the only
  one that is a guarantee.
* **−** Two different approval screens depending on the environment:
  `ApprovalDialog` outside `production`, the native dialog in `production`.
  The second is poor — no highlighting, no dependency list —: what the review
  shows cannot be reread there.
* **−** `run_object_operation` waits for the native dialog, up to its
  five-minute deadline, and keeps its review session open meanwhile.
* **−** Each submission opens a session, hence a handshake with the server,
  TLS included. A session stays open as long as an approval waits, up to the
  approval registry's 5 min timeout.
* **−** The dependency list is partial: incoming foreign keys, not views or
  routines. On SQLite, whose engine refuses nothing, this partial list is the
  only safety net, and the box must say so.
* **−** Three `Capabilities` bits are consumed. Like any flag, they are
  serialized and are never renumbered.
* **−** `query_history` does not know a row comes from the box. Finding "the
  drops made from the menu" requires reading the text.

**Exit cost:** low, and bounded by the absence of a command variant. The
composer is a pure function testable without a database or an interface. The
Tauri command is removed, the box too, and the three entries go back to
ADR-0025's path. Only the three flags survive a rollback, because they are
serialized. They remain true and reusable.

**Reconsider if** a `Drop…` or a `Truncate…` launched from this box is
reported as having targeted the wrong object or the wrong connection: the
review would then not have done its job. Reconsider also if a dependency
introspection capability (views, routines) appears in a driver: the `CASCADE`
checkbox could then announce what it drops, instead of saying it does not
know.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Open a console, like ADR-0025 | Four gestures for a `DROP`, a template to uncomment for an obvious statement, and an autosaved document that stays in the library. It is the heaviness ADR-0025 sets as a reconsideration condition |
| Execute directly on click, without review | Outside production, `Rename` is `Allow`: a missed click would rename without showing anything. For `Drop`, `ApprovalDialog` alone shows neither the dependencies, nor the transactional nature, nor the name input |
| A dedicated command, `Command::DropObject { path }` outside SQL | A second execution path ([I-01](../../CLAUDE.md#i-01)): the gate would need a separate rule, `oxyn-query` would classify nothing, the history would have no SQL, and the driver would compose DDL, which ADR-0025 already rejected. Such a command also maps to an agent tool in one line |
| Execute on the catalog session | Reserved for the catalog and the preview ([ADR-0015](0015-consoles-independantes.md)). A `DROP` waiting for a lock freezes the connection's tree there for 30 s, then ends in an ambiguous error |
| Execute on a console's session | It carries the user's transaction. The `DROP` would be undone by its `ROLLBACK`, or would hold its lock until its `COMMIT` |
| Compose the SQL in the front end | A second quoting implementation, in TypeScript, outside the tests of `oxyn-catalog` ([I-10](../../CLAUDE.md#i-10)) |
| The box itself calls `decide(true)` after its own confirmation | It would approve before the gate returned its decision and its preview. And a call to `decide` outside `ApprovalDialog` creates the precedent an automated flow would reuse |
| In `production`, also give way to `ApprovalDialog` before the native dialog | Three confirmations for a `DROP`: the review, a screen repeating its content, then the dialog. The middle screen adds no guarantee — the webview decides nothing on `production` — and feeds the fatigue that [ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md) names as its first cost |
| Remove the name input in `production` | The native dialog names the connection and quotes the statement, but closes with a click; the input forces reading the object's name beforehand, where the mistake is made — a click on the wrong row of the tree. It is kept for that, and only for that |
| Have the backend check the typed name | A script that calls `run_object_operation` knows the name and would provide it: the check would look like a guarantee without being one |
| Check `CASCADE` automatically when dependencies are known | The known dependencies are only part of what `CASCADE` drops. Pre-checking it means deciding, in the user's place, a scope nobody has seen |
| Add `IF EXISTS` | An object already dropped elsewhere would pass for a success, and would hide that the displayed catalog was stale |
| Grey out entries by product name, for example "SQLite: rename only" | Contrary to [ADR-0003](0003-driver-capabilities.md). SQLite has `DROP TABLE`, and Redshift differs from PostgreSQL on `TRUNCATE` despite the same protocol |
