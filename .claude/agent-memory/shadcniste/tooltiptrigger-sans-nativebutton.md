---
name: tooltiptrigger-sans-nativebutton
description: Base UI 1.8 TooltipTrigger has no nativeButton prop — a render={<span />} is not a gap, contrary to what base-vs-radix.md suggests
metadata:
  type: feedback
---

`base-vs-radix.md` lists `TooltipTrigger` among the `render` triggers and says
to add `nativeButton={false}` when `render` is not a button. For
`TooltipTrigger`, that is wrong: in `@base-ui/react` 1.8,
`tooltip/trigger/TooltipTrigger.d.ts` exposes **no** `nativeButton` prop (it
does not use `useButton`). A `<TooltipTrigger render={<span className="inline-flex" />}>`
around a disabled control is correct as is; adding the prop would break the
typecheck.

**Why:** found on 2026-09-23 on a disabled tab wrapped in a `span` so that its
tooltip stays visible; the skill's rule would have produced a "fix" that does
not compile.

**How to apply:** before reporting a missing `nativeButton`, grep the trigger's
`.d.ts` in `node_modules/.pnpm/@base-ui+react@*/…/<component>/trigger/`.
Same caution as [[base-ui-select-faux-positifs]].
