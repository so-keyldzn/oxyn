---
name: axe-region-defilante-sans-focus
description: A scrolling container with no focusable control inside makes axe fail in error mode (scrollable-region-focusable) — add tabIndex={0}
metadata:
  type: feedback
---

Any element with `overflow-y-auto` / `overflow-x-auto` that contains **no
focusable element** fails the axe rule `scrollable-region-focusable`, and since
`a11y.test` is `error` in `.storybook/preview.ts`, it is the `make front` gate
that falls. The fix is `tabIndex={0}` on the container, plus
`outline-none focus-visible:ring-2 focus-visible:ring-ring/60` so that the focus
shows.

**Why:** an `<ol>` or a `<ul>` bounded by `max-h-*` is only reachable with the
mouse without it; a keyboard user cannot read what overflows. Encountered on a
40-element step list, then on a source list.

**How to apply:** as soon as a height or width is bounded with `overflow-*` in
`src/components/oxyn`. A container that already contains buttons — for example
`AttachmentGroup` — satisfies the rule on its own and does not need `tabIndex`.
Do not confuse: it is the **focusable content** that exempts, not the role nor
the `aria-label`.
