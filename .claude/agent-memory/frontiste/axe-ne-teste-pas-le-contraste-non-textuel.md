---
name: axe-ne-teste-pas-le-contraste-non-textuel
description: Green stories ≠ checked contrast — axe ignores WCAG 1.4.11 and Storybook renders dark by default; measure through a canvas, never with a regex on rgba()
metadata:
  type: feedback
---

A story that passes axe in `error` mode proves **only** the text contrast, and
**only in the rendered theme**. Two blind spots:

1. **Storybook's default theme here is dark** (`initialGlobals: {
   theme: "dark" }` in `.storybook/preview.tsx`). Without a story carrying
   `globals: { theme: "light" }`, light mode is never checked.
2. **axe does not test non-text contrast** (WCAG 1.4.11): a checkbox border, a
   field outline, a switch track, a focus ring.

**The guard now exists for theme tokens**:
`src/components/oxyn/theme-contrast.stories.tsx` measures `--input` and `--ring`
in both themes (fixed on 2026-09-16 after measurements at 1.45:1). A token
defect is written **there**, once, not in a screen's story. It does not cover
everything: the switch, for example, is not in it (track at 2.96:1 in dark,
saved by a thumb at 15.51:1).

**How to measure — the expensive lesson:** reuse that story's `paint` method
(paint the color on a 1×1 canvas above the background, read the pixel back). It
natively reads `oklab`, `color-mix` and alpha, and fails loudly on an unreadable
color. A homemade regex parser got it wrong twice: `rgba(0, 0, 0, 0)` read as
**opaque black** (false success on a light background, false failure on a dark
one), and `oklab(… / 0.8)` unreadable. And do not infer a color from
`styles.css` without measuring it: `--input` had changed right under my eyes.

Also measure **what actually identifies the control**: for an unchecked switch,
the border, the track **and** the thumb — otherwise a false defect is reported.

**Why:** the question asked was "review in dark"; the real gaps were light mode
and non-text, and my first measurement was wrong because it ignored alpha.

**How to apply:** before asserting a contrast, measure it in the browser with
the canvas method, in each theme, and test the test by mutation. See
[[axe-region-defilante-sans-focus]], [[pieges-de-la-porte-front]].
