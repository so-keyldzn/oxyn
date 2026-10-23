# ADR-0064 — A session classifies the requests of a non-SQL language

**Status:** accepted · **Date:** 2026-10-22

**Refines:** [ADR-0003](0003-driver-capabilities.md), on the following point:
"queries carry an explicit `QueryLanguage`; SQL is only one case among others"
also holds for the classification that guards production. ADR-0003 stays in
force as written.

## Context

`oxyn-exec` replaces the intent a caller declares with the one read from the
text, before the `PolicyGate` decides
([ARCHITECTURE §8](../ARCHITECTURE.md#8-security-and-safeguards)). That reading
is `oxyn_query::reclassify`, called in two places: `reclassified()` for every
`Command::Execute`, and the preview path for the request a driver composes for
`Command::PreviewRelation` (`crates/oxyn-exec/src/executor.rs`).

`oxyn-query` parses SQL only. For any other `QueryLanguage` —
`RedisCommand`, `MongoQuery`, `SearchDsl` and the rest —
`classify_language` returns `Unknown`, which counts as mutating. Three drivers
are being written in parallel for these languages (Redis, MongoDB,
Elasticsearch), and with today's code each of them gets, on a production
connection or one without an environment:

* a named confirmation asked of the human for a plain `GET`;
* a refusal for every agent request, reads included (I-02, I-07);
* every preview refused, whatever the actor and the environment — and with a
  message that blames the syntax of the preview filter, since the preview path
  reads an `Unknown` request as a filter typo.

A driver for these engines is therefore unusable on production and cannot
offer a preview at all. What can tell a read from a write in these languages is
the engine, or the driver that speaks its protocol: a command table read once
at connect time, for example, which no reading of the text from outside the
driver can replace.

## Decision

`trait Session` (`crates/oxyn-driver/src/traits.rs`) gains one synchronous
method with a default body:

```rust
fn classify_intent(&self, request: &ExecRequest) -> Option<StatementIntent> {
    let _ = request;
    None
}
```

* `oxyn-exec` consults it **only** when `request.language.sql_dialect()` is
  `None`. `oxyn-query` stays the sole authority on SQL: a SQL request never
  reaches the session.
* It is consulted after `oxyn_query::reclassify`, in both places: in
  `reclassified()` through the session registry — only if the session belongs
  to the command's connection — and in the preview path on the slot already in
  hand. `SessionSlot::classify_intent` takes the slot's lock with `try_read`
  and answers `None` if the session is closing or the lock is held: nothing
  waits on the dispatch path (I-05).
* `Some(intent)` replaces `Unknown`, with risk `None`, `transaction_control` and
  `context_change` false. A driver whose language has such statements refuses
  them in its console, or carries a context change through `set_context`.
* `None` keeps `Unknown`. It is the default, so postgres, mysql, sqlite and any
  third-party driver compile and behave as before: the change to `oxyn-driver`,
  the Apache-2.0 crate third parties link
  ([ADR-0044](0044-licence-gpl-et-contrat-apache.md)), is not breaking.
* The method performs no I/O and takes no lock that can block. It answers from
  what the session learned at connect time.
* It is **conservative**: `Some(Read)` only for a request proven read-only;
  `None` whenever unsure.

The rule for implementers lives in
[DRIVER-CONTRACT §5](../DRIVER-CONTRACT.md#5-it-declares-its-capabilities-per-session-and-simulates-nothing).
A read classified this way follows every rule of a SQL read: on production, or
on a connection marked read-only, it leaves with `limits.read_only` set, and
the session holds that bound or refuses.

## Consequences

* **+** A non-SQL driver can serve reads on production without a confirmation
  per `GET`, an agent can read there, and previews can run.
* **+** No existing driver changes: the default is the old behavior, and SQL
  takes exactly the path it took.
* **+** The decision stays in the executor and the gate: the session only
  supplies a fact, and the journal records the intent the gate saw.
* **−** The production guard of a non-SQL language now rests on a driver's
  word. A driver that over-claims `Read` — a command it does not know answered
  as a read, a `FLUSHALL` taken for a `GET` — bypasses the named confirmation
  and the agent refusal, with no test of `oxyn-exec` able to notice. The
  contract says so in plain words, and every implementation must prove its
  `Read` answers against the engine's own command table.
* **−** The answer has no risk: a non-SQL write is never flagged as an
  unbounded delete, only as a write. The confirmation still names the
  connection; it does not say how much is touched.
* **−** A session closing, or whose lock a closing holds, answers `None` and
  sends a read to confirmation. That is the cautious side, and it only happens
  while the console is being closed.

**Exit cost:** low. Removing the method means deleting a default method from
the trait, its overrides in the non-SQL drivers, and the branch in
`reclassified_request`; non-SQL requests would then go back to `Unknown`
everywhere. Third parties that override it would break at compile time — the
one cost a published trait adds.

**Reconsider if** `oxyn-query` learns to read a non-SQL language itself — the
executor would then have two readers of the same text and should keep one — or
if a driver is found answering `Read` for a request that writes: the answer
would then need a second check, by the engine itself, before the read leaves.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep `Unknown` for every non-SQL request, driver-only | Safe, but makes every non-SQL driver unusable on production — a confirmation per `GET`, every agent read refused — and makes previews impossible; users would mark production connections as staging to work, which removes the guard entirely |
| Teach `oxyn-query` each non-SQL language | `oxyn-query` would need the command set of every engine, version and extension, which only the connected server knows for sure; the session is the one place that talks to it |
| An `async` method that asks the server | Puts a round trip on the dispatch path before every decision, and a lock across an `await` there (I-05); what a session needs to classify can be read once, at connect time, which is when it already evaluates its capabilities (ADR-0003) |
| Let the session also answer for SQL | Two readers of SQL would disagree, and the less careful one would win on the request it misreads; `oxyn-query` holds the tested traps (`EXPLAIN ANALYZE DELETE`, `WITH … DELETE`) |
| Return a full `Classification` (risk, transaction control, context change) | No non-SQL driver needs it today; the flags drive SQL-specific paths (transaction verbs, `USE`) that these languages handle through their own console refusals and `set_context` |
