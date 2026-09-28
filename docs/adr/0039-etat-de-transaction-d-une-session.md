# ADR-0039 — A session returns the transaction state it observed, and the console shows only that one

**Status:** accepted · **Date:** 2026-09-25

**Clarifies:** [ADR-0003](0003-driver-capabilities.md), on one point: a
capability says what a session **can do**; nothing yet said what it **is
doing**. This ADR adds a state read to the `Session` trait, the only one the
console needs: is a transaction open?

## Context

Today, a single engine can keep a transaction open in a console: **SQLite**.
Its session declares `Capabilities::TRANSACTIONS` and `MULTIPLE_STATEMENTS`
(`drivers/oxyn-driver-sqlite/src/driver.rs`), and a `BEGIN` typed in the
console is sent as is. **PostgreSQL** does not declare it: a session relies on
a pool and borrows a connection per execution, so the driver refuses `BEGIN`,
`COMMIT`, `ROLLBACK` and their synonyms **before sending**
(`drivers/oxyn-driver-postgres/src/session.rs`, `TRANSACTIONS_REFUSED`;
`transaction_text.rs`), rather than letting a `ROLLBACK` "succeed" on another
connection than the write's. The `Session::begin`, `commit` and `rollback`
methods have no caller outside tests.

A SQLite transaction open in a console is visible nowhere. Three consequences,
all silent:

1. **Closing rolls back.** "If an `sqlite3` object is destroyed while a
   transaction is open, the transaction is automatically rolled back"
   ([`sqlite3_close`](https://www.sqlite.org/c3ref/close.html), checked on
   2026-09-25). A console's closing dialog
   ([UX-SPEC](../UX-SPEC.md#independent-consoles)) only triggers for unsaved
   SQL or an operation in progress: closing a tab whose text is saved silently
   throws away uncommitted writes.
2. **The lock stays in place.** An open write transaction keeps the file's
   write lock; a **write** from the neighboring console, which has its own
   session ([ADR-0015](0015-consoles-independantes.md)), fails on
   `database is locked` — immediately, since no `busy_timeout` is set —,
   without anything saying where the lock comes from.
3. **The end can be implicit.** "If certain kinds of errors occur on a
   statement within a multi-statement transaction (errors including
   `SQLITE_FULL`, `SQLITE_IOERR`, `SQLITE_NOMEM`, `SQLITE_BUSY`, and
   `SQLITE_INTERRUPT`) then the transaction might be rolled back automatically.
   The only way to find out whether SQLite automatically rolled back the
   transaction after an error is to use this function"
   ([`sqlite3_get_autocommit`](https://www.sqlite.org/c3ref/get_autocommit.html),
   checked on 2026-09-25). `SQLITE_INTERRUPT` is the **Stop** button. A display
   inferred from the submitted text — "we saw `BEGIN` go by" — would therefore
   lie precisely after a cancellation or an error.

Two facts of the code constrain **where** and **when** the state is read:

* **The SQLite worker thread replies before it has finished.** A task sends its
  reply from inside (`worker.rs`, `call`; `stream.rs`, write without columns),
  then the thread closes the task (`Interrupter::end`). On Stop, `await_reply`
  returns `Cancelled` as soon as the token fires, while the thread is still in
  `sqlite3_step` — that is, **before** the automatic rollback it is about to
  cause. A value stored by the thread and read by the executor at that moment
  is stale; worse, nothing republishes it. The thread does, however, process
  its tasks **in submission order**: a task submitted after another runs after
  it ends.
* **Not every end of execution produces a terminal event.** In `Executor`
  (`crates/oxyn-exec/src/executor.rs`), a failure of `slot.execute` returns the
  error to the caller without publishing `Event::Failed`; only draining
  publishes `Completed` or `Failed`. Yet a SQLite write runs entirely during
  `slot.execute`: its errors, `SQLITE_BUSY` included, and Stop during the write
  go through this path. Finally, an abandoned future publishes `Cancelled` from
  `AbandonGuard::drop` (`abandon.rs`), which does not have the session.

`rusqlite` 0.37.0, the version in `Cargo.lock`, exposes
`Connection::is_autocommit` (`src/lib.rs`, read in the installed sources on
2026-09-25). On the PostgreSQL side, the state is carried by every
`ReadyForQuery` under three values — `Idle`, `Transaction`, `Error` —, but
`sqlx-postgres` 0.9.0 exposes none of it: `PgConnection::in_transaction` is
`pub(crate)` and conflates `Error` with `Idle` (`src/connection/mod.rs`). The
public method `Connection::is_in_transaction` of `sqlx-core` 0.9.0 does not
answer either: it counts the transactions opened **by sqlx**
(`transaction_depth`), and a typed `BEGIN` does not change it.

The question comes from the consoles batch (P30 of the 2026-09-24 audit); the
user asked on 2026-09-25 that it go through an ADR before any code, because it
touches the `Session` boundary trait — hence, eventually, the WIT interface of
plugin drivers ([PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md#what-this-contract-imposes-on-todays-traits)).

## Decision

### 1. A state type in `oxyn-core`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransactionState {
    /// No transaction block: each statement commits on its own.
    Idle,
    /// A transaction block is open on this session.
    Open,
    /// The session does not know, or does not say.
    Unknown,
}
```

It lives in `oxyn-core` because an `Event` carries it (§ 3) and `oxyn-core`
does not depend on `oxyn-driver`. It carries no database value: the
`derive(Debug)` does not contravene [I-03](../../CLAUDE.md#i-03).

`Unknown` is **never** presented as `Idle`. It is the rule of the session
context ([ADR-0019](0019-contexte-de-session.md)): the interface shows what the
session observed, not what is assumed.

### 2. An asynchronous method of the `Session` trait, with a default

```rust
/// The transaction state, once every operation already submitted on this
/// session has ended.
async fn transaction_state(&self, cancel: &CancelToken) -> TransactionState {
    let _ = cancel;
    TransactionState::Unknown
}
```

* **Ordered after what precedes.** The method returns the state observed
  **after** the end of every operation already submitted to the session —
  execution, `begin`, `commit`, `rollback`, `set_context` —, whether it
  succeeded, failed or was interrupted. That is what makes the read correct
  right after a Stop: the automatic rollback is observed, not preempted.
* **Without a network round trip.** It waits for the operations to end, it
  does not query the server. If `cancel` fires before, or if the session can
  no longer answer, it returns `Unknown`: never an error, never `Idle` by
  default.
* **Called once the cursor is released.** The SQLite thread stays taken by a
  stream as long as its cursor lives (`stream.rs`): a read submitted meanwhile
  would wait without bound. The caller drains or releases the cursor first.
* **Never inferred from the submitted text.** The third consequence of the
  context forbids it.
* **SQLite** submits to its worker thread a task that reads
  `Connection::is_autocommit`. The thread's submission order does the rest: the
  task only runs once the previous execution has returned from `sqlite3_step`,
  interruption included. The cost is a local thread hop.
* **PostgreSQL** keeps the `Unknown` default as long as it does not declare
  `TRANSACTIONS`. There is no manual transaction to show, and refusing a
  `BEGIN` already says that each statement commits on its own.
* **Contract.** A session that declares `TRANSACTIONS` overrides the method. No
  common contract suite exists in `oxyn-driver`; the implementation therefore
  adds, to each driver that declares `TRANSACTIONS` — SQLite today —, tests
  that check `Idle` at opening, `Open` after an executed `BEGIN` and after
  `begin`, `Idle` after `COMMIT`, `ROLLBACK`, `commit` and `rollback`, and
  `Idle` after an interruption during a write that triggered the automatic
  rollback. [`revue-driver.md`](../../.claude/checklists/revue-driver.md) makes
  it a requirement for every driver that declares the capability. Same logic
  as the `begin` guard: not knowing is acceptable, letting believe is not
  ([DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-it-declares-its-capabilities-per-session-and-simulates-nothing)).

### 3. The executor publishes the state on every end of execution it controls

The state belongs to the **session**, not to the command:
`Event::TransactionState { session, state }`. The front end stores, for each
session, the last value received; it never infers it from a command's reply.

* **Every exit.** For a session that declares `TRANSACTIONS`, the executor
  calls `transaction_state` at **every** exit of a statement's execution —
  success, draining failure, early failure of `slot.execute`, cancellation —,
  at a single exit point rather than branch by branch, and publishes the event.
  It is failure and cancellation that silently close a SQLite transaction: a
  single forgotten path, and it is that one.
* **Before the terminal.** When the exit produces a terminal event
  (`Completed`, `Failed`, `Cancelled`), the state is published **before** it:
  `Event::is_terminal` promises that after a terminal nothing more arrives for
  this execution.
* **Before `settle`, guard armed.** The call adds an `.await` on the exit path.
  It happens **before** `guard.settle()`: a future abandoned during this wait
  then lets `AbandonGuard` publish `Cancelled`. Placed after, abandonment would
  publish neither `Cancelled` nor the normal event, and the console would stay
  "running". The existing rule — no `.await` between `settle` and sending the
  terminal — applies here as at the exit point of `dispatch`.
* **A clean token.** The call receives neither the execution's child token nor
  the tab's: already fired after a Stop or a timeout, they would make it return
  `Unknown` every time, and the closing dialog would open after every Stop. It
  receives a token bounded by the session's closing.
* **Abandonment.** `AbandonGuard::drop` cannot read the session. A `Cancelled`
  preceded by no `TransactionState` for its execution — an execution's events
  arrive in order — switches the session to `Unknown` on the front-end side.
* **The bridge.** `ExecutionEventKind::of` (`crates/oxyn-desktop/src/ipc.rs`)
  ends with a `_ => return None` that silently throws away any variant it does
  not name: the new variant is named there, and the front end's validation
  schema learns it in the same commit
  ([ADR-0031](0031-validation-des-reponses-ipc.md)).

### 4. The initial state comes from opening

`ConsoleSession` (`crates/oxyn-desktop/src/backend/consoles.rs`) carries the
state read by `transaction_state` when the console opens. A new SQLite
connection is in autocommit, and this is **observed**: the console starts at
`Idle`, without a marker or a superfluous dialog.

The initial state is read **by the executor** during `Command::Connect`, and
carried by `Outcome::Connected`: the bridge calls no driver outside the bus
([I-01](../../CLAUDE.md#i-01)). `console_session` therefore stays synchronous;
it receives the state from the connection reply instead of reading it.

*Implementation note, 2026-09-25:* the proposed text had `console_session`,
made asynchronous, read the state. The invariants review showed that it was a
driver call from `oxyn-desktop`, outside the bus; the read moved into the
executor.

### 5. The console shows it next to its context

* **`Open`**: the console bar carries `Transaction open`, as text, next to the
  `<connection> / <schema>` selector. Color alone does not carry the
  information ([UX-SPEC, permanent markers](../UX-SPEC.md#permanent-landmarks));
  the environment pill stays where it is, in the top bar.
* **`Unknown`** on a session that declares `TRANSACTIONS`: `Transaction state
  unknown`. Nothing says "no transaction" without the session having observed
  it.
* **`Idle`**, or a session without `TRANSACTIONS`: nothing. No greyed-out
  marker suggests a transaction is possible where it is not — same rule as the
  context selector.
* **Nothing optimistic.** The display changes on the event, never on
  submitting a `BEGIN` or a `COMMIT`. During an execution, it keeps the last
  observed value.
* **Closing the console.** A console whose state is `Open`, or `Unknown` on a
  session that declares `TRANSACTIONS`, does not close without the existing
  dialog. It is a **change** to that dialog: today it names the console; it
  then also names the connection, and says that the open transaction will be
  **rolled back**. Focus stays on `Cancel`.

This ADR adds **no** `Commit` or `Rollback` button: they would be writes
emitted by Oxyn, which would go through the bus and the `PolicyGate`
([I-01](../../CLAUDE.md#i-01), [I-02](../../CLAUDE.md#i-02)) and deserve their
own decision. The user commits or rolls back by typing `COMMIT` or `ROLLBACK`,
as today.

### 6. What the implementation updates, in the same commit

[ARCHITECTURE §4.1](../ARCHITECTURE.md) (the list of the trait's methods),
[DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md) (the method and its contract),
[UX-SPEC](../UX-SPEC.md) (§ 5 above, in "Independent consoles" and the console
bar) and `revue-driver.md`. Not before: as long as this ADR is `proposed`,
these documents describe the code as it is.

## Consequences

* **+** An open SQLite transaction is visible in its console, and **closing
  the console** no longer throws it away without warning.
* **+** The displayed state is the one the engine reports after the actual end
  of the operation, including after a Stop or an error that closed the
  transaction.
* **+** The trait stays traversable by WIT: a function that returns an
  enumeration, with no callback or implicit shared state
  ([PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md#what-this-contract-imposes-on-todays-traits)).
  It is asynchronous, like `execute` already.
* **+** The day PostgreSQL pins one connection per session and declares
  `TRANSACTIONS` (the `TODO(phase 1)` of `variant.rs`), the screen is ready:
  only the driver changes.
* **−** The other closing paths still roll back without warning:
  `Command::Disconnect`, releasing a connection workspace, ⌘Q, and the macOS
  menu's Quit, which nothing can hold back
  ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md)). This ADR only covers
  closing a console.
* **−** One more method on the boundary trait, and one more call at every end
  of execution on a transactional session — a thread hop for SQLite. A driver
  that overrides it must respect the "after everything submitted" order; a
  value simply stored by the driver and read without waiting would recreate
  the race described in the context.
* **−** After a Stop, the end of the execution waits for the SQLite thread to
  actually leave `sqlite3_step`: the read token is bounded only by the
  session's closing. A statement that ignored the interruption would leave the
  console "running", without bound, until it is closed.
* **−** The state is that of the **end of the last operation**. A future
  network driver whose connection is killed on the server side would stay
  displayed `Open` until the next execution.
* **−** Today, the effort only serves SQLite; for PostgreSQL, the feature stays
  invisible until pinning.
* **−** PostgreSQL's failed state (`Error` in `ReadyForQuery`: every statement
  is refused until `ROLLBACK`) has no variant. One will be needed, `Aborted`,
  when PostgreSQL declares `TRANSACTIONS`; `#[non_exhaustive]` allows it on
  the Rust side, but `ipc.rs` and the front end's schema will have to learn it
  in the same commit.

**Exit cost:** low. Removing the method, whose default returns `Unknown`,
breaks no driver; removing the event, the `ConsoleSession` field and the
marker touches `oxyn-exec`, `ipc.rs`, the front end's schema and one
component. What bounds this cost: the state serves **no** security decision —
the `PolicyGate` does not consult it.

**Reconsider if** PostgreSQL pins one connection per session: `Aborted` will
then have to be added, and one must check that `sqlx` exposes the state of
`ReadyForQuery` — otherwise, the driver will have to rebuild it, and the
"never inferred from text" argument will come up again. Reconsider also if the
`PolicyGate` must one day take an open transaction into account (for example
to refuse that an `Actor::Agent` write into a transaction opened by the user):
the state would become security data, and a value published for display would
no longer suffice. Reconsider finally if closings outside the console
(disconnection, workspace, ⌘Q) must in turn be held back.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| **Synchronous method reading a value stored by the driver** — the consoles batch's initial proposal | The SQLite thread replies before finishing its task, and Stop returns control during `sqlite3_step`: the value read at the end of an execution is the one from before the automatic rollback, and nothing republishes it. Making it correct would impose `Unknown` during any task in progress, plus a second signal to publish the final state — the ordered asynchronous method gets the same through the thread's queue |
| **State carried by the execution reply** — an `ExecStats` field filled by `Cursor::stats`, hence in `Event::Completed` | Only covers success. `Failed` and `Cancelled` carry no statistics, the early failure of `slot.execute` publishes no event, and it is precisely the error and the Stop that close a SQLite transaction. `begin`, `commit` and `rollback` produce no cursor. Finally, `ExecStats` describes the cost of an execution, not the state of a session: putting it there would carry the console's state into the history and the retained results, where it would be stale when read |
| **State carried by the command's result** — the success `Outcome` and the IPC error reply | The error is an `OxynError` that crosses every crate: attaching a session's state to it mixes two things an error message must not carry. Abandonment produces no result. And the state belongs to the session, which outlives the command: reading it in a command's reply leaves the question of ordering between replies and events, which travel through two distinct channels |
| **Expose nothing as long as PostgreSQL refuses manual transactions** | SQLite already keeps transactions open, and closing a console rolls them back without a word. Waiting for PostgreSQL leaves this defect in place for the only engine it concerns today |
| **Infer the state from the submitted text**, on the front end or in `oxyn-exec` | `sqlite3_get_autocommit` says only the call to the engine reveals an automatic rollback after an error or an interruption. The display would lie at the moment it matters |
| **Query the server** (a query that returns the state) | A network round trip after every execution. For pooled PostgreSQL, the answer would concern a borrowed connection, not the session; for SQLite, there is no server, and the engine gives the answer without a query |
| **`Idle` default rather than `Unknown`** | A driver that implemented nothing would assert that no transaction is open. It is the "pretending" that DRIVER-CONTRACT §5 forbids |
| **Show the state for every session, `Unknown` included** | An "unknown state" marker on every PostgreSQL console, where no manual transaction is possible, teaches nothing and trains users to ignore the marker — the one that will matter on SQLite |
