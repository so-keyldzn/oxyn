# ADR-0028 — A preview imposes no order, and offers no page as long as the order is not total

**Status:** accepted · **Date:** 2026-09-15

**Clarifies:** [ADR-0020](0020-apercu-trie-filtre-parcouru.md), whose decision
"without a requested sort, the driver orders by the primary key alone" was not
implemented — and for a good reason, left unrecorded until now.

## Context

[ADR-0020](0020-apercu-trie-filtre-parcouru.md) states a real danger and names
it correctly: an `OFFSET` applied to a non-guaranteed order **duplicates and
omits rows silently**. Two consecutive pages can show the same row twice and
never show another, without any error appearing. It is the worst kind of
flaw — the displayed data is wrong and nothing says so.

Its answer was to **impose a default order** on the primary key.

The implementation took another path, and the repository ended up with three
documents that did not say the same thing:

* ADR-0020: "without a requested sort, the driver orders by the primary key alone";
* [UX-SPEC](../UX-SPEC.md): "without a requested sort, the order of the rows is
  not guaranteed";
* the code: both drivers compose **no** `ORDER BY` without a request, which the
  test `a_preview_without_request_composes_neither_where_nor_order_by` anchors.

This contradiction was found on 2026-09-15. What made it costly is not the
inconsistency itself: it is that ADR-0020 is the **only** document that
explains *why* pagination exists. Someone reading it concludes that the preview
is deterministic by default, hence that "next page" should always be offered —
and will "fix" the code that deliberately returns `NeedsOrder`. That is,
reintroduce exactly the silent failure ADR-0020 exists to prevent.

## Decision

**ADR-0020's argument is kept; its remedy is replaced.**

1. **No order is imposed.** A first preview is a bounded `SELECT`, without
   `ORDER BY`. The order of the rows is not guaranteed, and
   [UX-SPEC](../UX-SPEC.md) tells the user so.

2. **No page is offered as long as the order is not total.** The page control
   does not exist in two cases, and distinguishes them:
   * `NeedsOrder` — nothing was sorted, a "next page" would be the second
     page of an order the user never saw;
   * `NoUniqueKey` — a sort is requested, but no unique key is known:
     the order cannot be made total, so `OFFSET` remains dangerous.

3. **The order is checked against the shape the displayed rows come from**,
   not against the one being typed. Two consecutive pages only overlap
   correctly if both were composed from the same total order.

### Why this remedy is better than ADR-0020's

| | Imposed order (ADR-0020) | No order, no page (chosen) |
|---|---|---|
| Safety of `OFFSET` | held **if** a primary key exists — otherwise the order is not total and the danger returns | held by construction: without a total order, the control does not exist |
| Cost | a metadata read **on every preview**, to discover the key — see `oxyn-core/src/preview.rs` | none: nothing is read as long as nothing is requested |
| Visible effect | "a visible change" the ADR accepts: the arrival order is replaced by an arbitrary order | none: the preview shows what the engine returns, like a `SELECT` without `ORDER BY` |
| What the user understands | they believe they see a stable order, without knowing which | they see "next page" appear **when they sort**, which teaches the rule |

The last point is the one that carries the decision: the page control that
appears when one sorts **explains** the constraint instead of hiding it.

## Consequences

`ADR-0020` remains the reference on the rest — written predicate, composed
sort, quoted identifiers — and this ADR only touches its point 3. The two must
be read together; that is why this one **clarifies** it rather than
superseding it.

Nothing to change in the code: it already holds this decision. What changes is
that it is now **written**, and that a reader of ADR-0020 is sent here before
"fixing" `pagination_from`.

## Exit cost

**Low, but not zero.** Going back to the imposed order would require: reading
the primary key on every preview in both drivers, composing the default
`ORDER BY`, and revising the tests that anchor the absence of order
(`a_preview_without_request_composes_neither_where_nor_order_by` in each driver,
`a_page_is_offered_only_where_the_order_is_total` on the interface side).

What would cost more is invisible: users would have got used to an
always-present page control, and its conditional disappearance would read as a
regression.

## Reconsideration condition

* A driver returns a primary key **with no additional read cost** — the main
  argument against the imposed order then falls;
* users report that the absence of a page control on an unsorted preview reads
  as a flaw rather than as a rule;
* a target engine offers stable cursor pagination without a declared order,
  in which case neither remedy applies.
