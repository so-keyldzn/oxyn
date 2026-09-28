---
name: puce-inline-ligne-de-base
description: An inline-flex chip whose first child is an SVG icon takes the icon's bottom as baseline and floats above the text; mark only the text `self-baseline`
metadata:
  type: feedback
---

An `inline-flex items-center` chip placed in a line of text, with an icon as
first child, **is not aligned**: a flex container's baseline is its first
child's, and an SVG has none, so its bottom edge is used. The chip rises by 1 to
1.5 px, enlarges the line, and the caret jumps.

What holds: the chip at the text's size (no `text-xs`), `align-baseline`, the
name alone as `self-baseline` (it becomes the container's baseline), the icon as
`self-center` at `size-[1em]`, `leading-[1.25]` and a transparent border on all
variants so that the dashed variant keeps the same box.

**Why:** user feedback on the real window; `toBeVisible` saw nothing. A box
measurement (the chip's center against the `Range` rect of the neighboring text,
and the block's height against a clone where each chip becomes a 0-height
inline-block) fails with the old style at 1.06 px and 1.3 px.

**How to apply:** for any inline pill in text (mentions, badges in a bubble).
Check that a measurement story fails without the fix before relying on it. See
[[lexical-dans-les-stories]].
