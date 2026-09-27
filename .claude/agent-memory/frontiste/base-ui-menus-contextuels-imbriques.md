---
name: base-ui-menus-contextuels-imbriques
description: Base UI ContextMenu.Trigger stops the propagation of contextmenu — nested menus (code block in an answer, chip in a question) open at the deepest level, with no extra code
metadata:
  type: reference
---

Base UI's `ContextMenu.Trigger` (1.8) calls `stopEvent` (preventDefault + stopPropagation) in its `onContextMenu`: a trigger nested in another wins, the parent does not open. An `onContextMenu` passed on the `render` element is merged and runs anyway (the anchor is set there).

**Why:** avoids writing a "what is under the pointer" detection by hand for nested surfaces.

**How to apply:** wrap each surface in its own `<ContextMenu>`; no manual `stopPropagation`. Neighboring trap: adding a story to a component exempted in `script/verifier-stories` makes the check fail ("stale exemption") — render the component through the parent's story already named in the exemption, or remove the exemption.
