---
name: pieges-context-menu-et-stories
description: Tooling traps of Base UI ContextMenu (select-none, refusing to open) and Storybook (vi missing, animated dialog, pointer coords)
metadata:
  type: feedback
---

Traps seen while writing the context menus (batch 3, 2026-09-25).

- `ContextMenuTrigger` (shadcn on Base UI) adds `select-none` to the trigger: around an editable field (CodeMirror), pass `className="select-text"` to the trigger, otherwise text selection inherits `user-select: none`.
- To refuse opening on an area without a target (empty tab strip), make `ContextMenu` controlled and read the target in a **ref** set by the child's `onContextMenu`: `onOpenChange` is called in the same event, before the React state is committed.
- `storybook/test` does not export `vi`: read the calls through a module-level `fn()` (`mock.calls`).
- A Base UI `Dialog` opens animated: `toBeVisible()` right after `findByRole` fails; wrap it in `waitFor`.
- `userEvent.pointer({ keys: "[MouseRight]", coords })` carries coordinates: get them through a `Range` on the text node to target a word in CodeMirror (`posAtCoords`).

**Why:** each one cost a test round trip; none is visible when reading the code.
**How to apply:** for any new context menu surface or story that opens a menu or a dialog.
