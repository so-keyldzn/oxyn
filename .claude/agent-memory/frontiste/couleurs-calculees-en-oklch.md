---
name: couleurs-calculees-en-oklch
description: The theme is in OKLCH since 2026-09-23; getComputedStyle returns oklch(...), so a story that reads a color with an rgb() regex fails — go through a canvas
metadata:
  type: feedback
---

A story that measures a contrast **never** reads a computed color with an
`rgba?\(` regex: paint it on a 1×1 canvas and read back `getImageData`, like
`paint()` in `theme-contrast.stories.tsx`.

**Why:** `styles.css` is written in OKLCH since the 2026-09-23 redesign, and
Chromium then returns `getComputedStyle(el).borderTopColor` in the form
`oklch(0.545 0.01 60)`. `assistant-sample-approval.stories.tsx` parsed
`rgb()`: its two contrast stories failed on "Unreadable colour", while the real
contrast passed. Fortunately the parser failed loudly; a silent fallback would
have measured 1:1 or anything.

**How to apply:** any new color measurement in a story goes through a canvas
with the `#010203` sentinel (an unreadable color keeps the sentinel and makes
the story fail). And a status tint is also checked **on its own tint**: `ui/`
writes `bg-destructive/10` in light, `/20` in dark, `/30` on hover — that is
where axe found 4.4:1, not on the bare surfaces.
