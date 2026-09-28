---
name: classes-d-enfant-posees-par-le-parent
description: base-nova components style their children from the parent (`*:data-[slot=…]:`, `[&_svg]:size-4`) — a className on the child can be redundant or losing, and cn does not see it
metadata:
  type: feedback
---

Several generated components set the child's class **on the parent**:
`Alert` `destructive` carries `*:data-[slot=alert-description]:text-destructive/90`,
`SidebarMenuButton` carries `[&_svg]:size-4` (descendant, not only `>svg`).
`cn` only merges within a single list: it never sees the conflict between the
parent and the child.

Consequences on review:

- an icon nested at any depth in a `SidebarMenuButton` (the logo tile as a
  `div`) already has `size-4`: the `className="size-4"` is redundant, removing
  it is `mécanique`;
- a `text-foreground` passed to an `AlertDescription` under
  `variant="destructive"` competes with the parent's rule, of stronger
  specificity: it probably has no effect. Do not remove it as "mécanique"
  without having measured the rendering; it is a doubt to report.

**Why:** observed on 2026-09-23 during a compliance pass on `src/features`;
reading only the child's `className` leads to wrongly conclude to an override
that wins, or to a necessary `size-*`.

**How to apply:** before classifying a `size-*` or a color on a component's
child, grep the parent component in `ui/` for `[&_`, `[&>` and `*:`.
