---
name: passe-visuelle-sans-serveur
description: "Responsive and light/dark contrast check: static Storybook build + a Playwright script that serves and stops; stories alone see neither light mode nor widths"
metadata:
  type: feedback
---

To check widths and contrast in both themes, do not launch `storybook dev`:
build statically and measure with a Playwright script that serves the build
itself on a port chosen by the system, then closes everything.

**Why:** Storybook servers left by agents on 6006/6109 make story suites fail in
`make qualite` with an import error that looks like a regression. And the
stories do not cover what the pass finds: `preview.tsx` renders in **dark**, so
axe only sees a light contrast in stories with `globals: { theme: "light" }`.
Observed on 2026-09-16: the inactive tabs of the generated `TabsTrigger`
(`text-foreground/60`) dropped to 4.27:1 in light in *every* tab list, without
any story failing. See also [[axe-ne-teste-pas-le-contraste-non-textuel]].

**How to apply:**
- `pnpm exec storybook build -o <scratchpad>/sb --quiet`, then a `node` script
  that resolves `playwright` through `createRequire(apps/desktop/package.json)`
  and injects `axe.min.js` from `node_modules/.pnpm/axe-core@*/`.
- Read the build's `index.json` for the list of stories, filter by
  `importPath`. URL: `iframe.html?id=…&viewMode=story&globals=theme:light`.
- Decorators freeze widths (`w-[900px]`): release them with an injected
  stylesheet, otherwise no window width has any effect. They can be **stacked**
  (meta + story): a `scrollWidth` exactly equal to a decorator's width is a false
  positive — check before fixing.
- Each defect found becomes a story that fails without the fix: prove it by
  mutation (neutralize the fix, see the story fail, restore).
- Before concluding, `lsof -nP -iTCP -sTCP:LISTEN`: only kill your own servers;
  6006 may belong to another project of the user.
