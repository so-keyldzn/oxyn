---
name: verifier-un-nom-dicone-hugeicons
description: A Hugeicons icon name is not checked with require() — grep the .d.ts of dist/types
metadata:
  type: feedback
---

To know whether `FooIcon` exists in `@hugeicons/core-free-icons`, grep the types
file:

```bash
grep -oE "\b[A-Za-z0-9_]+Icon\b" \
  node_modules/@hugeicons/core-free-icons/dist/types/index.d.ts | sort -u
```

**Why:** `node -e 'require("@hugeicons/core-free-icons")'` returns an object
where **no** name is present (the package is ESM), so every name tested this way
looks missing — including those the repository already uses. Time is lost looking
for a replacement for an icon that exists. The package exposes ~6,700 names, and
the numbered families are tricky: `CircleIcon` exists, `Circle01Icon` does not.

**How to apply:** before writing an `import { … } from
"@hugeicons/core-free-icons"` with a name not seen elsewhere in `src/`.
`lucide-react` is forbidden by [front.md], so there is no fallback.
