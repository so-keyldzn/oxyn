---
name: fieldlabel-choice-card-dark-survit-a-cn
description: Recoloring a FieldLabel choice card through className is not enough — its dark:has-data-checked:* classes are not merged by cn and win in dark mode
metadata:
  type: feedback
---

shadcn's "choice card" pattern (`FieldLabel` > `Field` + `RadioGroupItem`)
carries in `field.tsx` two sets of classes for the checked state:
`has-data-checked:border-primary/30 has-data-checked:bg-primary/5` **and**
`dark:has-data-checked:border-primary/20 dark:has-data-checked:bg-primary/10`.

Passing `className="has-data-checked:border-<token>"` replaces the first set
(same group, same modifiers for tailwind-merge) but **not** the second:
`dark:has-data-checked:` is a different list of modifiers, so no conflict is
detected. In the dark theme, the card is tinted `primary` again.

**Why:** migrating a token-colored card to this pattern forces either writing a
manual `dark:` (forbidden by styling.md), or touching `ui/field.tsx`
(forbidden). Neither the typecheck nor axe see it.

**How to apply:** a choice card whose checked color depends on a value
(environment, status) stays a homemade `label`; the migration is a `décision`
to report, not a `visuel` fix. Applies to any `ui/` component that doubles its
state classes under `dark:`.
