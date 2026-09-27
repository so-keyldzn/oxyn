# ADR-0019 — A declared session context, never set silently

**Status:** accepted · **Date:** 2026-09-10

**Clarifies:** [ADR-0015](0015-consoles-independantes.md), on what a session
carries besides its transaction.

## Context

The console bar of the mockup (`191:2003`, 260 px) shows
`commerce-prod / public`: the connection **and** the schema the console works
in. Nothing in the product carries that second half today.

The state found in the code:

- the `Session` trait (`crates/oxyn-driver/src/traits.rs:119`) has no context
  method; `Driver::connect` receives a `ConnectionConfig` that carries neither a
  schema nor a `search_path`;
- no capability of `Capabilities` (`crates/oxyn-core/src/capabilities.rs:37`)
  describes a modifiable context; `SCHEMAS` only says "the source exposes named
  schemas", which is an introspection capability;
- `CatalogPath` (`crates/oxyn-catalog/src/path.rs:229`) already designates a
  schema through its `namespace` level, but per catalog node, never as a state;
- the PostgreSQL driver sets no `SET` (`session.rs`), and the SQLite driver
  explicitly documents its refusal of any implicit `PRAGMA`
  (`session.rs:28-35`).

Two constraints bound the solution. First, a contract prohibition:
"modifying the server's session state without declaring it" is in the refusal
table of [DRIVER-CONTRACT](../DRIVER-CONTRACT.md), because an invisible
`SET search_path` changes the meaning of the user's next queries. Second, the
isolation of [ADR-0015](0015-consoles-independantes.md): each console has its
session, but **the catalog and the table preview share the "preferred"
session** of the connection (`crates/oxyn-exec/src/executor.rs`,
`SessionSlot::read_catalog`). A context set on that session would therefore
also move what the explorer shows.

## Decision

The session context is a **declared operation**, not a side effect.

- The context designates a location through the two already validated levels
  of the catalog, `catalog` and `namespace`, without a relation. It carries no
  SQL text. Like `Command::PreviewRelation`, the command carries them as
  `Option<String>`: `oxyn-core` does not depend on `oxyn-catalog`, and
  reversing that direction for a command type would pay a crate dependency for
  signature comfort ([ARCHITECTURE](../ARCHITECTURE.md#le-sens-des-dépendances)).
  The contract's `SessionContext` type lives in `oxyn-driver`, which already
  knows `CatalogPath`; the executor builds it at the boundary, as it does today
  for the preview.
- `Command::SetSessionContext { connection, session, catalog, namespace }`
  goes through the `PolicyGate` like any other command ([I-01](../../CLAUDE.md#i-01)). The UI
  does not apply it itself and shows nothing before the server's response.
- Its `StatementIntent` is `Read`: it writes no data, and classifying it `Ddl`
  would get it refused on a connection marked read-only, where switching schema
  **to read elsewhere** is precisely the use. On the other hand it is
  **refused to `Actor::Agent`**, like `GRANT` and for the same reason: an agent
  has no legitimate use for moving the context under the human's feet, and the
  effect outlives the command — the `DELETE` the human writes next would hit a
  different schema than the one they think they target
  ([I-07](../../CLAUDE.md#i-07)). It is a refusal, not a confirmation.
- The `Session` trait gains `async fn set_context(&self, context: &SessionContext,
  cancel: &CancelToken) -> Result<()>`, defaulting to `NotSupported`, and
  `fn context(&self) -> Option<SessionContext>`, which returns what the server
  confirmed — a value and not a reference, because an implementation keeps
  its context behind a lock: `set_context` takes `&self`. A `SESSION_CONTEXT` capability declares support; the selector
  does not exist for a driver that does not carry it
  ([ADR-0003](0003-driver-capabilities.md)).
- The PostgreSQL driver implements it with a `SET search_path` whose identifier
  is **quoted by the driver**, never concatenated
  ([I-10](../../CLAUDE.md#i-10)). The action is visible: it appears in the
  status bar and in the history as the operation it is.
- This `SET` is issued **on every execution, on the connection that execution
  borrows**, and not once and for all. The reason is measurable: an Oxyn
  PostgreSQL session is a pool of four connections
  (`MAX_CONNECTIONS`, `drivers/oxyn-driver-postgres/src/options.rs`), and
  `search_path` is a **per-connection** state. A `SET` issued once would hold
  for the connection that received it and for no other: one query in two
  would resolve in another schema, with nothing to signal it. It is the worst
  possible outcome — a control that looks like it works. `execute` already
  holds a single connection from start to finish, which makes the application
  deterministic. Going back to the server default likewise issues a
  `SET search_path TO DEFAULT`: issuing nothing would leave the connection in
  its previous state.
- Symmetrically, **the connection returns to the pool in the state it left
  it**: what was set for an execution is undone before returning it. Without
  that, a console's context would travel with the connection to whatever
  borrows it next — and introspection depends on it, which is not obvious:
  `pg_get_indexdef`, `pg_get_constraintdef`, `pg_get_expr` and `format_type`
  qualify their text **relative to the `search_path`**. The same object would
  be described differently from one read to the next, depending on the
  connection drawn. A connection that cannot be reset to the default is closed
  rather than returned.
- `set_context` **checks the existence** of the location before retaining it,
  with a bound-value query. PostgreSQL silently accepts a `SET search_path` to
  a nonexistent schema; without this check, Oxyn would show a context the
  server does not apply.
- The `catalog` level is refused by PostgreSQL when it designates a database
  other than the connection's: a PostgreSQL session does not change database,
  and pretending otherwise would be the false control this ADR seeks to avoid.
- SQLite does not declare it. Its schemas (`main`, `temp`, attached databases)
  are qualified in the SQL, and `ATTACH` remains a user statement.
- **The user's SQL is never rewritten.** The context changes what the server
  resolves, not the submitted text. No qualification is added to a
  hand-written identifier.
- The session reserved for the catalog and the preview **does not accept** a
  context change: the explorer shows the qualified tree, not a view that
  depends on a state. Changing a console's context therefore does not move the
  catalog.

## Consequences

- **+** The meaning of an unqualified `SELECT` becomes explainable: a single,
  visible operation changed it, and the bar says which.
- **+** The context follows the session, hence the console. Two tabs of the
  same connection can work in two schemas without getting in each other's way.
- **+** No SQL rewriting, hence no class of requalification bugs.
- **−** One more capability and method in a contract that
  [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md) will have to carry in phase 4.
- **−** The console's context and the catalog's scope can diverge: the explorer
  shows `public`, the console works in `analytics`. It is a visible, accepted
  gap, preferable to a catalog that moves under the user's feet.
- **−** The change crosses the network and can fail. It therefore has the five
  states of a remote operation, cancellation included.
- **−** One more round trip per execution while a context is declared, on a
  PostgreSQL session. It is the price of the connection pool. If a measurement
  shows this cost weighs, the alternative is to carry `search_path` as a
  connection option and recreate the pool on change — faster, but unable to
  preserve an open transaction.
- **−** After any writable execution, the PostgreSQL driver resets `search_path` to the default and `standard_conforming_strings` to `on`, which it also enforces at opening (the value the splitter assumes): a `SET` typed in a console does not follow the connection returned to the pool, and a console's schema only goes through this context.

**Exit cost:** remove a command, a capability and two trait methods, plus the
selector. Nothing is persisted in a workspace format as long as the context is
not remembered between launches, which this ADR does not decide — that bounds
the cost to code.

**Reconsider if** a driver can only express its context by requalifying the
text, or if users ask for the catalog to follow the active console's context.
The second case is an interface choice, and will be settled without reopening
the contract.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Issue a `SET search_path` when each session opens, without saying so | Exactly the invisible session state the driver contract refuses |
| Qualify the identifiers of the user's SQL before sending | Requires parsing then rewriting arbitrary SQL; a wrong requalification executes something other than what is written |
| Make the context a property of the **connection** | Two consoles of the same connection would share it: switching schema in one tab would move the other without warning |
| Use the selector only to filter the catalog | Does not answer the question the mockup asks: what an unqualified `SELECT` resolves |
| Open a new session on every context change | Loses the console's transaction and work in progress, for an operation the user believes harmless |
