# ADR-0066 — A session classifies the risk of the non-SQL requests it classifies as writes

**Status:** accepted · **Date:** 2026-10-23

**Refines:** [ADR-0064](0064-a-session-classifies-non-sql-requests.md), on the
following point: "`Some(intent)` replaces `Unknown`, with risk `None`". A
proven write, DDL or grant now takes the risk its session answers. ADR-0064
stays in force as written for everything else — the intent, the SQL boundary,
the absence of I/O, transaction control and context change.

## Context

For SQL, `oxyn-query` computes a `MutationRisk` alongside the intent:
`UnboundedUpdate`, `UnboundedDelete`, `Truncate`, `DropObject`, the two `COPY`
forms. The `PolicyGate` checks the risk before anything else, and a risk asks
for an approval **whatever the actor and the environment**
(`crates/oxyn-core/src/policy.rs`): `DELETE FROM users` on staging waits for a
human, with the reason "DELETE without a WHERE clause: every row".

ADR-0064 let a non-SQL session prove an intent, and fixed the risk of a proven
intent at `MutationRisk::None` — it listed that as a negative consequence: "a
non-SQL write is never flagged as an unbounded delete, only as a write". The
review of the MongoDB driver (pull request #330) gave that consequence its
concrete form. The driver's classifier knows that
`{"delete":"users","deletes":[{"q":{},"limit":0}]}` deletes every document and
that `{"drop":"users"}` drops the collection, but has no way to say so:

* on staging or development, a human's collection-wide delete runs on one click,
  where `DELETE FROM users` asks for an approval;
* an agent's collection-wide delete on staging is asked for under the generic
  "an agent is requesting a write operation" instead of the reason that says
  how much is touched;
* on production the confirmation names the connection, but not the scope.

Redis (`FLUSHALL`, `FLUSHDB`) and Elasticsearch (`DELETE /index`,
`_delete_by_query` with `match_all`) have the same need, and their drivers are
being written now.

## Decision

`trait Session` (`crates/oxyn-driver/src/traits.rs`) gains a second
synchronous method with a default body, sibling of `classify_intent`:

```rust
fn classify_risk(&self, request: &ExecRequest) -> Option<MutationRisk> {
    let _ = request;
    None
}
```

* `oxyn-exec` consults it in `reclassified_request`
  (`crates/oxyn-exec/src/executor.rs`), through `SessionSlot::classify_risk`
  which takes the slot's lock with `try_read` like `classify_intent`. It does so
  **only** for a request whose language has no SQL dialect, and **only** when
  `classify_intent` answered `Write`, `Ddl` or `Grant`. A proven `Read` is never
  given a risk: the session could only invent it. A request left `Unknown`, or
  answered `Unknown`, keeps its current handling — it already counts as
  mutating.
* The answer replaces `MutationRisk::None`, the risk ADR-0064 gives a proven
  intent and the one `oxyn-query` gives any non-SQL text. It can therefore only
  raise the risk: `None`, or `Some(MutationRisk::None)`, leaves the request
  exactly as before this method existed.
* The `PolicyGate`, the approval flow and the journal are unchanged. The risk
  travels in the `ExecRequest` like a SQL risk: the gate reads it, the journal
  records it, and an approved command leaves with it.
* The method is **conservative in the other direction** from `classify_intent`:
  the risk is answered whenever the request may touch every document, key or
  index of its target, not only when that is proven. A wrong `Read` bypasses the
  production guard; a missed risk bypasses an approval. Both are silent.
* The rule for implementers lives in
  [DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-it-declares-its-capabilities-per-session-and-simulates-nothing).

The variants of `MutationRisk` are reused as they are: an empty delete filter is
`UnboundedDelete`, a drop of a collection or an index is `DropObject`, a flush
of every key is the driver's to map onto the closest variant. No new variant is
added by this decision.

## Consequences

* **+** A non-SQL collection-wide delete or drop asks for the approval its SQL
  equivalent asks for, on every environment, for a human and for an agent.
* **+** No existing session changes: the default answers `None`, which is the
  behavior of ADR-0064. The change to `oxyn-driver`, the Apache-2.0 crate third
  parties link ([ADR-0044](0044-licence-gpl-et-contrat-apache.md)), is not
  breaking.
* **+** The decision stays in the gate: the session supplies a fact, through the
  same type SQL uses, and nothing downstream needs to know where it came from.
* **−** The approval of a non-SQL destructive request now rests on the driver's
  word. A driver that misses a form — a filter that matches everything without
  being empty, `{"q":{"_id":{"$exists":true}}}` — lets it run as a plain write,
  and no test of `oxyn-exec` can notice.
* **−** The reasons the gate shows are SQL-worded
  (`MutationRisk::reason`): a MongoDB delete is announced as "DELETE without a
  WHERE clause: every row". It is understandable, not exact.
* **−** A session closing between the two calls answers an intent and no risk;
  on that window, a destructive write is asked for as a plain write. It only
  happens while the console is being closed, and the write still goes through
  the gate.

**Exit cost:** low. Removing the method means deleting a default method from
the trait, its overrides in the non-SQL drivers, `SessionSlot::classify_risk`
and the match in `reclassified_request`; non-SQL writes would go back to having
no risk. Third parties that override it would break at compile time.

**Reconsider if** a non-SQL language needs to say something `MutationRisk`
cannot — a flush, a server-side script, a reindex — and a driver starts mapping
it onto a variant whose reason says something false; or if `classify_intent`
and `classify_risk` are found disagreeing in a driver often enough that one
method returning both would be the safer shape.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Change `classify_intent` to return a `(StatementIntent, MutationRisk)` pair, or a full `Classification` | Breaks every override written against ADR-0064, in a published trait, for a fact half the requests never carry; a sibling method with a default costs nothing to sessions that have no risk to tell |
| Consult the risk for every non-SQL request, reads included | A read with a risk counts as mutating (`ExecRequest::is_mutating`): a session answering a risk on a read would turn it into a write, which is `classify_intent`'s question, and two methods would then decide the intent |
| Let the session lower a risk the executor computed | The executor computes none for a non-SQL request today; allowing a lowering would let a future reader of non-SQL text be overruled by the less careful of the two |
| Add non-SQL variants to `MutationRisk` now (`FlushAll`, `DropIndex`) | `MutationRisk` is in `oxyn-core`, whose reasons are shown and journaled; a variant is worth adding when a driver needs it and can name its reason, not ahead of the drivers |
| Teach the gate the destructive forms of each language | The gate would parse MongoDB, Redis and Elasticsearch requests, which ADR-0064 rejected for the intent for the same reason: only the session speaks the language |
