---
name: menu-rend-le-focus
description: A Base UI menu gives focus back to its trigger after the closing animation, overriding a focus given by an item's handler
metadata:
  type: feedback
---

A menu item (context or dropdown) whose handler focuses another field loses that
focus: at the end of the closing animation, Base UI gives it back to the
trigger. A `setTimeout(0)` is not enough.

**Why:** focus is returned when the popup unmounts, after the animation, so
after the handler.

**How to apply:** defer the action until the `Root`'s
`onOpenChangeComplete(false)` (a "pending" ref read on close). Check in a story:
`toHaveFocus()` still true after ~300 ms.
