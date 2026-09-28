---
name: eprouver-un-schema-hors-du-depot
description: A Node script in the scratchpad does not resolve `zod` — pnpm isolates node_modules; import by absolute path from .pnpm
metadata:
  type: feedback
---

To test a zod schema at runtime without creating a file in `apps/desktop`, an
`.mjs` placed in the scratchpad fails with `ERR_MODULE_NOT_FOUND: zod`: Node
resolves bare imports from the importing file's directory, and pnpm exposes
nothing outside `apps/desktop/node_modules`.

What works:

```bash
node --input-type=module -e '
const { z } = await import("<abs>/apps/desktop/node_modules/.pnpm/zod@<v>/node_modules/zod/index.js")
…'
```

The path is found with `node -e "console.log(require.resolve(\"zod\"))"` run
**from `apps/desktop`** (it returns the `.cjs`; take `index.js` next to it for
ESM). The version in the path changes at every upgrade: read it again, do not
copy it.

**Why:** `make front` and `vitest` cannot run while several agents convert files
in parallel, and `tsc` says nothing about a schema's runtime behavior (what it
accepts, what it refuses, the path named in the error). This probe gives the
answer without writing anything in the repository.

**How to apply:** when a doubt is about what zod *does* (a `z.custom` that would
make a key optional, a `discriminatedUnion` on the wrong tag, an `.int()` that
would refuse a `u64`), probe before shipping rather than reasoning.
