---
name: tabslist-line-souligne-coupe
description: A shadcn TabsList variant="line" with overflow-x-auto loses its active tab underline, and an h-* passed in className does not win against the component's default
metadata:
  type: feedback
---

Two traps of the generated `TabsList` (`components/ui/tabs.tsx`, Base UI), which
show up together and break no test:

**1. `variant="line"` draws the active tab marker with `after:bottom-[-5px]`,
so *outside* the `TabsList` box.** Adding `overflow-x-auto` to scroll a tab bar
also makes the vertical axis scrollable (CSS: if one axis is not `visible`,
`visible` becomes `auto`) and **cuts off the underline**. The active tab then
has no visible marker at all — neither background nor border, the `line` variant
setting them to `transparent`. The `TabsList` must be given a height that leaves
the ~8 px under the trigger (e.g. fill the 48 px bar rather than staying at
32 px).

**2. An `h-8` on the trigger exceeds the content box of a `TabsList` in
`h-8 p-[3px]`** (26 useful px): combined with point 1, the row can slide
vertically under the pointer. Check `list.scrollHeight <= list.clientHeight`.

**Why:** found on the Oxyn workspace tab bar; the active tab could only be told
apart by the text weight, and nobody had seen it because no story looks at the
geometry of the `::after`.

**How to apply:** as soon as `overflow-x-auto` is put on a
`TabsList variant="line"`, write a story that asserts
`trigger.bottom + 5 + 2 < list.bottom` **and**
`list.scrollHeight <= list.clientHeight`.

**The tailwind-merge corollary:** the default comes from the cva under the
`group-data-horizontal/tabs:h-8` variant. A bare `h-12` in `className` does not
replace it — tailwind-merge treats them as two different keys and CSS
specificity (`.group[data-orientation=horizontal] .h-8`, 0-2-1) wins over `.h-12`
(0-1-0). The override must **carry the same variant**:
`group-data-horizontal/tabs:h-12`. Applies to any utility the generated
component already sets under a variant.

See [[userevent-escape-nattend-pas-un-trigger-base-ui]].
