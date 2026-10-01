# Driver review

To go through entirely before considering a driver delivered. The contract is
authoritative: `docs/DRIVER-CONTRACT.md`.

An unchecked box is not a detail to handle later: every point here matches a
failure that will not show in tests.

## Split

- [ ] It really is a new **protocol**, and not a product speaking an already
      implemented protocol (Redshift ≡ PostgreSQL, MariaDB ≡ MySQL, OpenSearch ≡
      Elasticsearch)
- [ ] The crate only depends on `oxyn-core`, `oxyn-driver`, `oxyn-data` and `oxyn-catalog`,
      plus `oxyn-query` only to split what the server cannot prove to be a single
      statement ([ADR-0050](../../docs/adr/0050-mysql-driver-on-mysql-async-prepared-first.md))
- [ ] No dependency on another driver

## Robustness

- [ ] No `unwrap`, `expect`, `panic!`, slice indexing or overflowing `as` on a
      path reachable from a server response
- [ ] Tested against: unknown type, `NULL` on a `NOT NULL` column, out-of-range
      integer, invalid encoding, **response truncated mid-stream**
- [ ] Tested against an object name containing a quote, a semicolon, or text
      imitating an instruction

## Results

- [ ] Produces `RecordBatch`es, never a row representation
- [ ] The batch is bounded **in bytes**, not in number of rows
- [ ] **Streaming test over a volume that would not fit in memory**, with a
      bound on the process memory — without a bound, the test passes by accident
- [ ] If the source is schemaless: inference by sampling is declared as such up
      to the interface, and a field outside the sample produces an explicit
      error, never a silent loss

## Cancellation

- [ ] **Test proving the server-side stop**, checked in the DBMS process view —
      not at the function's return
- [ ] If server-side cancellation is impossible, it is **declared absent** in
      the capabilities, not simulated

## Errors

- [ ] Three distinct classes: transient, permanent, **ambiguous**
- [ ] The class is **data**, not a deduction made from the message
- [ ] A timeout during a write is classified **ambiguous**, never transient
- [ ] The driver never retries on its own

## Capabilities

- [ ] Evaluated **per session**, not per driver
- [ ] Nothing is simulated: not knowing how is declared
- [ ] Explicit `QueryLanguage`
- [ ] If the session declares `TRANSACTIONS`: `transaction_state` is overridden,
      **ordered after** everything submitted, and tested — `Idle` at opening,
      `Open` after an executed `BEGIN` and after `begin`, `Idle` after `COMMIT`,
      `ROLLBACK`, `commit` and `rollback`, and `Idle` after an interruption
      during a write that triggered the automatic rollback
      ([ADR-0039](../../docs/adr/0039-etat-de-transaction-d-une-session.md))

## Types

- [ ] Mapping table **both ways**
- [ ] Documented losses, notably: arbitrary-precision `NUMERIC`, integers beyond
      2^53, spatial types, proprietary types
- [ ] No time zone assigned to a `timestamp` that has none
- [ ] Unknown type returned as raw bytes **with its type identifier**

## Security

- [ ] No bound value nor connection credential in a log
- [ ] No identifier concatenated into SQL composed by Oxyn
- [ ] No `SET`/`USE` changing session state without declaring it
- [ ] No environment variable read, no file written

## Exit gate

- [ ] `make qualite` passes
- [ ] `relecteur-frontiere` and `relecteur-invariants` have nothing blocking
- [ ] `docs/RESEARCH-NOTES.md` up to date if a dependency was added
