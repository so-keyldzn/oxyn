---
name: stories-instables-sous-charge
description: "Under load (several agents), stories that read a Base UI animation frame fail intermittently: wrap in waitFor, and rerun before concluding to a regression"
metadata:
  type: feedback
---

A story that reads the state of a Base UI dialog or popup **right after it
opens** fails intermittently when the machine is loaded. Wrap the assertion in
`waitFor`, and rerun the file alone before concluding to a regression.

**Why:** Base UI surfaces enter an opacity transition. `toBeVisible()` read on
the frame where the element is still transparent fails. Under load — local CI,
or several agents each running Vitest/Playwright — the frame read changes, so
the failure shows up in a batch and disappears in an isolated run. Also
observed: `Failed to fetch dynamically imported module` and
`Port 63315 is already in use` when two runs overlap, and an axe
`aria-activedescendant` violation pointing to an id of a popup being mounted.
None of these three is a component defect.

**How to apply:** before "fixing" a component for a story failure, rerun
`pnpm exec vitest run --project storybook <the file>` alone. If it passes, fix
the **story** (add `waitFor`) and not the component. Add `NO_COLOR=1` so that
`grep` patterns find the `FAIL` lines in the output.
