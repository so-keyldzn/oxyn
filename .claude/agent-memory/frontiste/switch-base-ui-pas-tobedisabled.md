---
name: switch-base-ui-pas-tobedisabled
description: A Base UI Switch is a span with aria-disabled — toBeDisabled() fails in a play; assert aria-disabled="true"
metadata:
  type: feedback
---

In a story `play`, `expect(getByRole("switch")).toBeDisabled()` fails on a disabled `Switch` from `components/ui/switch`: Base UI renders a `<span role="switch" aria-disabled="true" data-disabled>`, and jest-dom's `toBeDisabled` only reads the `disabled` attribute of form elements.

**Why:** seen on 2026-10-07 writing `semantic-ranking-settings.stories.tsx`; two stories failed although the switch was visibly inert.

**How to apply:** assert `toHaveAttribute("aria-disabled", "true")` on a switch. A `Button` disabled the normal way still passes `toBeDisabled()` (unless `focusableWhenDisabled`). Related: [[pieges-de-la-porte-front]].
