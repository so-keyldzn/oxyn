# ADR-0068 — A preview comes in key order, wherever the driver vouches that this order is a bounded read

**Status:** accepted · **Date:** 2026-10-26

**Supersedes:** [ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md),
on its decision points 1 ("no order is imposed") and 2 ("no page is offered as
long as the order is not total"), for a relation that has a primary key and
whose driver vouches for it. It **restores** the remedy
[ADR-0020](0020-apercu-trie-filtre-parcouru.md) originally proposed: "without a
requested sort, the driver orders by the primary key alone". ADR-0028 stays the
reference for the rest: no page without a total order (`NoUniqueKey`), and the
order checked against the shape the displayed rows came from.

**Refines:** [ADR-0063](0063-mongodb-driver-database-commands-in-extended-json.md),
on one sentence: a preview of a plain collection is no longer "a `find`
command … with no imposed order" — it is sorted on `_id` when no sort is
requested.

## Context

On 2026-10-26 the user opened a SQLite table, saw 200 rows and no page
control, and asked why there was none. That is, word for word, the second
reconsideration condition of ADR-0028: "users report that the absence of a page
control on an unsorted preview reads as a flaw rather than as a rule".

ADR-0028 kept ADR-0020's argument — an `OFFSET` over a non-guaranteed order
duplicates and omits rows silently — and rejected its remedy for its cost: "a
metadata read on every preview, to discover the key". That cost is real, and it
is not the same everywhere:

| Engine | Metadata round trips per preview, before → after |
|---|---|
| SQLite | one local `PRAGMA table_info`, no server |
| PostgreSQL | plain preview 2 → 2 (the typed-column fetch now also carries the key in index order, `relkind` and `relhassubclass`); sorted or paged 6 → 2; Redshift unchanged |
| MySQL/MariaDB | plain preview 0 → 2 (`describe_relation`: header, then columns, each a prepare/execute/close); counted in the code, not measured on a live server |

The key alone does not make the order cheap. `ORDER BY key LIMIT n` is a
bounded read only where the engine can walk the key in order. A SQLite `vec0`
table declares its key column as a primary key, yet ordering it is served by no
index: SQLite reads every row, vectors included, into a temporary b-tree before
the first one comes out
([RESEARCH-NOTES](../RESEARCH-NOTES.md#default-preview-order--checked-on-2026-10-26)).
Opening such a table for a glance must not pay that.

Nor does a composite key make every column order cheap. On PostgreSQL 17.11,
with `PRIMARY KEY (b, a)` over a table declared `(a, b)`, `ORDER BY a, b
LIMIT 200` ran as a sequential scan and a sort of the whole table, while
`ORDER BY b, a` used the index
([RESEARCH-NOTES](../RESEARCH-NOTES.md#default-preview-order--checked-on-2026-10-26)).

## Decision

**The user chose always-available pagination over a faster first open.** A
preview with no requested sort comes in the order of the primary key, and
offers pages, wherever the driver vouches that this order is a bounded read.

1. **The driver vouches, in the description of the relation.**
   `Relation::key_order_preview: bool` (`oxyn-catalog`, serde default `false`)
   is set by the driver in `describe_relation`.
   `Relation::previews_in_key_order()` is true when that flag holds **and** a
   primary key exists. An absent flag is `false`: a driver that says nothing
   never has its previews ordered behind its back.

2. **One flag, two readers.** With no sort requested, the driver composes
   `ORDER BY <key> ASC` — every key column, quoted — exactly when
   `previews_in_key_order()` holds, and nothing otherwise. The desktop reads
   the same flag to decide whether a page is offered. There is no capability
   bit: a second source of truth would let the order and the page control
   disagree, which is the silent failure itself. The Data tab loads the
   relation's description once when it opens; until it is read, neither a
   notice nor a pager is shown.

   **The key columns come in the index's order, not the table's.**
   `Field::primary_key_rank` (`oxyn-catalog`) carries the engine's rank, and
   `Relation::primary_key()` orders by it first: SQLite from the `pk` column of
   `PRAGMA table_info`, MySQL/MariaDB from `SEQ_IN_INDEX` of the `PRIMARY`
   index in `information_schema.STATISTICS`, PostgreSQL from the index itself
   (`unnest(indkey) WITH ORDINALITY`).

3. **Who vouches, and who does not.**

   | Driver | Vouches for | Why, and what it does not vouch for |
   |---|---|---|
   | SQLite | every table except virtual tables (`PRAGMA table_list` type `virtual`) | the key is the b-tree's order, or a unique index's; a virtual table (`vec0`…) is ordered by reading it whole into a temporary b-tree first (see Context) |
   | PostgreSQL | `relkind` `r` without inheritance children (`relhassubclass` false), and `p` (partitioned) | never views, materialized views or foreign tables. A child table has neither the parent's primary key index nor its uniqueness (checked on 17.11), so ordering an inheritance parent sorts every child row — TimescaleDB hypertables are inheritance parents. An unreadable `relhassubclass` counts as having children |
   | Amazon Redshift | nothing | primary keys are informational and not enforced ([RESEARCH-NOTES](../RESEARCH-NOTES.md#default-preview-order--checked-on-2026-10-26)); ordering is a sort over the whole distributed table |
   | MySQL/MariaDB | `BASE TABLE` on InnoDB, MyISAM or Aria, every key column ranked in the `PRIMARY` index and none a prefix | MEMORY hashes its key, FEDERATED lives elsewhere; a `UNIQUE NOT NULL` index standing in for a missing primary key, which the server reports as `PRI`, is not vouched for |
   | DuckDB | base tables | see the consequence on its cost below |
   | MongoDB | real collections (type `collection`), ordered by `{_id: 1}` | views and time-series collections have no unique key. The plain preview reads only the `listCollections` entry, no document sample |
   | ClickHouse | no, unchanged | no unique key: the sorting key may repeat ([ADR-0060](0060-clickhouse-driver-over-http-rowbinary.md) point 10) — `NoUniqueKey` |
   | Elasticsearch/OpenSearch | no, unchanged | no `OFFSET` beyond the result window, no unique key without a point-in-time cursor ([ADR-0062](0062-elasticsearch-opensearch-rest-json.md)) |
   | Redis | no, unchanged | no sort and no page: keys come from `SCAN` |

   A relation with no key — a view, a keyless table — stays unordered and
   reports `NoUniqueKey`, on every driver.

4. **Four pagination states reach the interface.**
   * `unknown` — the relation is not described yet;
   * `needsOrder` — a key is known but the relation is not ordered by default
     (a virtual table, Redshift): the user sorts to page, paying for the order
     knowingly;
   * `noUniqueKey` — no key can make the order total;
   * `ready { previous, next, firstRow, orderedBy }` — `orderedBy` lists the
     key columns while the default order is in force, and is `null` once the
     user sorted.

   What the interface says in each state is in
   [UX-SPEC](../UX-SPEC.md#filter-sort-page-through); what a driver owes is in
   [DRIVER-CONTRACT §3](../DRIVER-CONTRACT.md#3-it-produces-arrow-recordbatches-streamed).

## Consequences

* **+** A table with a key can be browsed page by page the moment it opens,
  which is what the user expected.
* **+** The first rows are deterministic: the same click shows the same rows,
  and the notice names the order they come in.
* **+** Safety of `OFFSET` is unchanged: a page exists only over a total order,
  the default one included.
* **−** Every preview reads the description: free on SQLite, unchanged for a
  plain preview on PostgreSQL (and cheaper when sorted or paged), two more
  round trips on MySQL/MariaDB — see the table in Context.
* **−** The order users see first changes from "whatever the engine returns"
  to key order. ADR-0020 named that visible change and accepted it; it is
  accepted again here.
* **−** `OFFSET` stays linear: page 100 costs 100 times page 1. An always
  present pager makes deep pages easier to reach, hence that cost easier to
  pay.
* **−** DuckDB has no ordered index scan — its ART index serves constraints
  and highly selective lookups
  ([RESEARCH-NOTES](../RESEARCH-NOTES.md#default-preview-order--checked-on-2026-10-26)):
  its default order is a `TOP_N` over a sequential scan, pruned by zone maps
  through a dynamic filter on the key. On 10 million in-memory rows: 1.9 ms in
  insertion order, about 25 ms shuffled, against 1.8–2 ms without `ORDER BY`;
  at `OFFSET` 5,000,000, about 130 ms, 430–540 ms shuffled. A file-backed
  database was not measured.
* **−** Known gaps. A primary key added with `ALTER TABLE ONLY` on a
  PostgreSQL partitioned parent leaves its index invalid until every
  partition is attached. An empty MongoDB collection samples no field, so its
  description has no `_id` key: no page is offered while the plain preview
  still sorts by `_id`, which is harmless. `system.profile` has no `_id` index,
  a gap that predates this ADR.
* **−** A `needsOrder` relation still pages over its declared key once the
  user sorts. On Redshift that key is not enforced and may repeat; this is
  unchanged from ADR-0028, and remains the user's explicit choice.
* **−** A stale catalog after a DDL could order by a key that no longer
  exists; the invalidation of
  [ADR-0022](0022-rafraichissement-automatique.md) covers it, and nothing new
  was added for it.

**Out of scope, and the follow-ups the user may want:** ClickHouse (a total
order would need an `ORDER BY` over every orderable column), Elasticsearch and
OpenSearch (a point-in-time and `search_after` cursor instead of `OFFSET`),
Redis (paging by the `SCAN` cursor). Each would be its own ADR.

**Exit cost:** low, bounded to code. Going back to ADR-0028 means clearing the
flag in each vouching driver's `describe_relation` — the order and the page
control then disappear together, since they read the same flag — and reverting
the tests that anchor the default order
(`a_preview_comes_in_key_order_only_when_the_driver_vouches_for_it_and_a_key_exists`
in `oxyn-catalog`; `a_vec0_table_is_not_ordered_by_default_but_can_be_sorted`,
`a_composite_key_is_ordered_as_its_index_not_as_its_table` and
`a_preview_without_request_composes_neither_where_nor_order_by_when_no_key_is_known`
in the SQLite driver; `a_plain_preview_is_ordered_exactly_when_the_description_says_so`
in the PostgreSQL driver). Nothing is persisted. What costs more is invisible, and
ADR-0028 already named it: users will get used to an always-present pager, and
its withdrawal would read as a regression.

**Reconsider if:**
* the default order of a vouching driver is measured beyond the first-rows
  budget of [PERFORMANCE](../PERFORMANCE.md#interaction-budgets) on a real
  table — that driver then stops vouching, as virtual tables already do;
* keyset pagination (`WHERE key > last seen value`) lands, which would replace
  `OFFSET` under the same total order and change what a deep page costs;
* an engine gains a stable cursor without a declared order, in which case
  neither the default order nor `OFFSET` is the right tool for it.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep ADR-0028 as it is | Its second reconsideration condition is met: the user reads the missing pager as a flaw |
| Order every relation that declares a key, without the driver's word | A `vec0` table declares one and would read every vector before its first page; Redshift declares keys it does not enforce |
| A capability bit for "orders by default" | Per driver, not per relation: it cannot tell a SQLite table from a SQLite virtual table, and it would be a second source for what the flag already says |
| Order by `rowid` or the engine's physical order where no key exists | Not a total order the catalog can name: a `WITHOUT ROWID` table has no `rowid`, most engines have no equivalent, and a physical order is precisely the order that is not guaranteed |
| Offer pages without an order, warning that rows may repeat | A wrong result that looks right, warned or not — the failure ADR-0020 exists to prevent |
| Keyset pagination now | Also requires a total order, so it does not remove the need for this decision; it changes the cost of a deep page, which is its own decision |
