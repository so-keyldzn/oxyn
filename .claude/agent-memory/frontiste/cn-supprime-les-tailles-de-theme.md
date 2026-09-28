---
name: cn-supprime-les-tailles-de-theme
description: "cn() reads `text-reading`/`text-caption` as a color and drops them when the same list carries text-muted-foreground: write text-[length:var(--reading-text)]"
metadata:
  type: feedback
---

In a `className`, do not write `text-reading` or `text-caption`: write
`text-[length:var(--reading-text)]` / `text-[length:var(--reading-caption)]`.

**Why:** `cn` (the `cn` package, a tailwind-merge compatible engine) only knows
Tailwind's default size scale. A homemade theme token like `--text-caption` is
not part of it: `cn` puts `text-caption` in the **color** group and drops it as
soon as the same list carries a real color. Checked at runtime:

```
cn("text-caption text-muted-foreground")                        → "text-muted-foreground"
cn("text-[length:var(--reading-caption)] text-muted-foreground") → both survive
cn("text-xs text-[length:var(--reading-caption)]")               → the length wins
```

The failure is **silent**: the text simply inherits the parent's size, so the
"Comfortable" preset seems to move only the line height. Nothing fails in types,
lint or axe.

**How to apply:** everywhere a density size goes through `cn` or through the
`className` of a `src/components/ui` component (which does `cn(variants,
className)`) — so in practice everywhere. `@apply text-reading` in
`styles.css` stays correct: Tailwind resolves it, not `cn`. The same caution
applies to any future token outside the default scale (`--text-*`, and tokens
ambiguous between color and size). The trap is documented next to the
definition of the tokens in `apps/desktop/src/styles.css`.

To prove it: a story that reads `getComputedStyle(el).fontSize` under
`data-density="comfortable"` fails as long as the class is swallowed.
