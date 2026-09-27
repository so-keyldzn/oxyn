---
name: base-ui-select-faux-positifs
description: base-nova Select — `SelectValue placeholder` is valid in Base UI 1.8 (the skill says otherwise); a SelectItem outside a SelectGroup is a visual defect, not only a semantic one
metadata:
  type: feedback
---

Two readings of the skill to correct on the Base UI `Select`:

1. **`<SelectValue placeholder="…" />` is not a violation.** `base-vs-radix.md`
   says Base requires a `{ value: null }` element in `items`; the `.d.ts` of
   `@base-ui/react` 1.8 (`select/value/SelectValue.d.ts`) does declare a
   `placeholder?: React.ReactNode` prop. Do not "fix" an existing placeholder.
2. **Forgetting `SelectGroup` costs rendering.** In the generated `select.tsx`
   (base-nova), `SelectContent` has **no padding** and it is `SelectGroup` that
   carries `p-1`: items placed directly in the content touch the popup's edge.
   The fix's risk is therefore `visuel`, not `mécanique`. `DropdownMenuGroup`,
   on the other hand, carries no class (the `p-1` is on the content): adding it
   there is mécanique.

And a neighboring trap, on `Marker`: `MarkerIcon` renders `aria-hidden="true"`.
A `Spinner` (which carries `role="status"` and `aria-label`) placed inside
disappears from the accessibility tree; that is intended when the `Marker`
already carries the status as text, not if the spinner is the only announcement.

**Why:** observed on 2026-09-23 during a compliance pass on the assistant; the
placeholder would have been rewritten for nothing, and the padding gap was not
seen.

**How to apply:** before reporting a `Select`, read the `.d.ts` of the installed
version rather than the skill's rule; classify adding a `SelectGroup` as
`visuel`.
