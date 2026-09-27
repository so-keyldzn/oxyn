# ADR-0020 — Preview: a sort Oxyn composes, a predicate the user writes, a deterministic page

**Status:** proposed · **Date:** 2026-09-10

**Clarifies:** [ADR-0012](0012-lecture-pages-resultats.md), on what distinguishes
a result page from a table page.

> **Clarified by [ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)
> on one point, and it matters.** The remedy proposed here — "without a
> requested sort, the driver orders by the primary key alone" — **was not
> implemented**. The argument behind it is kept: an `OFFSET` over a
> non-guaranteed order duplicates and omits rows silently. What the code does
> instead is **stricter** — no imposed order, and no page offered as long as
> the order is not total.
>
> Read ADR-0028 **before** "fixing" `pagination_from` or composing a default
> `ORDER BY`: the two passages of this ADR that describe an imposed sort are
> stale, and following them would reintroduce the silent failure this ADR
> exists to prevent.

## Context

Selecting a table opens the Data tab and reads at most 200 rows
(`PREVIEW_ROWS`, `crates/oxyn-app/src/workspace/preview.rs`). The driver
composes `SELECT … FROM … LIMIT n`, quotes the identifiers, and the executor
enforces read-only. [UX-SPEC](../UX-SPEC.md#data-of-a-selected-table) is
explicit about what is missing: "the order of the rows is not guaranteed and
the preview does not count the whole table".

Three facts found in the code bound the solution.

**The grid has no sort.** `crates/oxyn-ui/src/data_grid.rs` reads the
`RecordBatch`es in their arrival order; its only column settings are
visibility and width. There is therefore nothing to reconcile between an
existing client sort and a server sort — but also nothing to build on.

**`ReadResultPage` does not paginate what one thinks.** [ADR-0012](0012-lecture-pages-resultats.md)
says so: "it is a local read […] it does not contact the server, does not
compose SQL and re-executes nothing". It rereads an Arrow batch **already
received** and spilled to disk. Paginating a *table* is the opposite
operation: a new execution, with an `OFFSET` or a cursor. Confusing the two
would give either a "next page" button that shows the same rows again, or a
scroll that relaunches queries — which the grid rule forbids.

**`oxyn-core` can neither name `CatalogPath` nor compose SQL.**
`Command::PreviewRelation` already carries its levels as `Option<String>` for
that reason.

Finally, the catalog knows which columns form the primary key
(`Relation::primary_key`, `crates/oxyn-catalog/src/model.rs:712`). That is what
makes honest pagination possible.

## Decision

**The two halves of the request do not look alike, and the mockup says so.**
The Figma survey of 2026-09-10 shows, under the Data bar, a
"Filter toolbar" (`190:1618`, 1272 × 32) made of a field `272:10667` carrying
the literal prefix **`WHERE`**, a 1048 px input area and an `Apply` button,
then an 84 px `Sort` button. The filter is therefore a **predicate the user
writes**, not a column/operator/value builder.

This is not a breach of [I-10](../../CLAUDE.md#i-10), it is its letter: what the
invariant forbids is Oxyn **concatenating a received identifier**; it also says
that "the SQL *the user writes* is sent as is — that is the feature". A
predicate typed by a professional belongs to the second category, like the
text of a console.

**The sort, for its part, stays structured.** `PreviewSort { column, descending }`:
the column is an identifier the driver quotes. Oxyn composes that fragment, so
Oxyn answers for what it contains. A column the relation does not declare is
refused rather than passed to the server.

**`PreviewShape` carries all three: `sort`, `predicate: Option<String>` and
`offset`.** An empty or whitespace-only predicate means "no filter" — composing
a `WHERE` without a condition would produce a syntax error where the user
believes they cleared everything. It is the only normalization applied to their
text.

**The predicate is not an open door for all that.** The final text is
reclassified by `oxyn-query` and refused if it becomes mutating — the executor
already does so for every preview —, the session is held read-only **by the
server**, and the row bound applies. A `;` followed by a write gets past none
of these three.

**Two capabilities, `PREVIEW_SORT` and `PREVIEW_FILTER`.** An engine that does
not declare them does not show these controls
([ADR-0003](0003-driver-capabilities.md)). This is not a theoretical
precaution: the product also targets key-value families, where ordering a read
makes no sense.

**A next page is an execution, and it is only offered if the order is
deterministic.** An `OFFSET` over a non-guaranteed order returns duplicate rows
and omits others, with nothing signaling it — it is the silent failure this ADR
refuses. Therefore:

- if the user requested a sort, the driver **completes it** with the primary
  key declared in the catalog, to break ties;
- without a requested sort, the driver orders by the primary key alone;
- if the relation has no known unique key, **pagination is not offered** and
  the interface says why. The preview stays bounded to its first page, which
  it already is today.

**`ReadResultPage` stays unchanged, and the two notions meet nowhere.**
Scrolling the received rows never triggers a query; asking for the table's next
page is an explicit action, which produces a new result with its own identity.

## Consequences

- **+** A preview becomes usable on a real table: finding a row no longer
  requires writing SQL in the console.
- **+** The predicate is SQL, so it says everything SQL says:
  `a IS NOT NULL AND (b > c)` can be written, where three menus could not have
  expressed it.
- **−** This predicate is also SQL the user can write wrong. The server's error
  message will tell them — its audience reads it — but the preview no longer
  has the property "cannot fail for a syntax reason".
- **+** Pagination does not lie: it exists when it is correct, and its absence
  is explained.
- **−** Each page is an execution: it costs the server, and the data may have
  changed between two pages. The interface must say so rather than suggest a
  snapshot.
- **−** `OFFSET` is linear: the hundredth page costs a hundred times the first.
  It is not a flaw of Oxyn, but Oxyn is what will make it visible.
- **−** Two capabilities and one more signature in a contract that
  [PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md) will have to carry in phase 4.
- **−** A default sort imposed on the primary key changes what the user sees
  first compared to today. It is an arbitrary order replaced by a
  deterministic order, but it is a visible change.

**Exit cost:** remove a shape field, two capabilities and one translation per
driver. Nothing is persisted in a workspace format — a preview's sort and
filter do not survive closing the tab, which this ADR does not seek to change —,
which bounds the cost to code.

**Reconsider if** a driver cannot express a stable `OFFSET` and imposes an
opaque cursor, or if measurement shows that `OFFSET` pagination is unusable on
real table sizes. The second case would lead to keyset pagination
(`WHERE key > last seen value`), which is faster but assumes exactly what the
present decision already requires: a unique order.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Sort and filter in the grid, in memory | Would only cover the rows already read: the user would think they search the table and would search 200 rows |
| A structured filter builder — column, operator, bound value | Safer on paper, but it is not what the mockup draws, and Oxyn's audience writes SQL all day. A builder would force them to express in three menus what they type in five seconds, and could not say `a IS NOT NULL AND (b > c)` |
| Let Oxyn compose a `WHERE` from values it concatenates | There, yes, the invariant applies: it would be SQL composed by the product from received data ([I-10](../../CLAUDE.md#i-10)) |
| Reuse `ReadResultPage` for the next page | It rereads an already received buffer; it does not contact the server and therefore cannot return rows that were never read |
| Paginate without a deterministic order | `OFFSET` without a stable `ORDER BY` duplicates and omits rows without signaling anything — a wrong result that looks right |
| A single capability for sort and filter | An engine may know how to order without knowing how to filter, and the reverse; a single flag would force refusing both for lacking only one |
