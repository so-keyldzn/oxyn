---
name: spinner-porte-role-status
description: the Spinner of components/ui carries role="status" + aria-label="Loading" — every visible spinner is one more live region
metadata:
  type: feedback
---

`Spinner` (`src/components/ui/spinner.tsx`, generated) renders `role="status"`
and `aria-label="Loading"`. A panel that wants **a single** live region (the
assistant's panel) counts as many more as there are visible spinners. And a
`getAllByRole("status")` counts those too.

**Why:** discovered while bringing the assistant's panel back to a single
`status` region. Spinners in a `MarkerIcon` are already hidden (`aria-hidden`
on the parent), but not those of a `Badge`.

**How to apply:** next to a label that already states the state, pass
`aria-hidden` to the `Spinner`. In a story, target the region through a
`data-slot` rather than counting the `status`es. See also
[[spinner-change-le-nom-du-bouton]].
