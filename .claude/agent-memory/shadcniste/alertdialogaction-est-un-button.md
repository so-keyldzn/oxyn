---
name: alertdialogaction-est-un-button
description: In base-nova, AlertDialogAction is a plain Button with data-slot, not a Base UI Close; replacing a Button with it does not close the dialog and stays mécanique
metadata:
  type: feedback
---

In the `alert-dialog.tsx` generated in base-nova, `AlertDialogAction` renders
`<Button data-slot="alert-dialog-action" …>`: no `AlertDialog.Close` primitive
behind it, unlike `AlertDialogCancel`. A `<Button variant="destructive">` placed
in `AlertDialogFooter` can therefore be replaced by
`<AlertDialogAction variant="destructive">` without changing anything: same
element, same variants, and closing stays driven by the `onClick`.

**Why:** observed on 2026-09-23; one may hesitate to make the switch for fear
that it closes the dialog before the callback has run (Radix behavior).

**How to apply:** re-read `AlertDialogAction` in `ui/alert-dialog.tsx` (a
registry update can change this), then classify the switch as `mécanique`.

Neighboring tooling trap: `pnpm exec eslint` launched from a subdirectory of
`apps/desktop` fails with "couldn't find any tsconfig.json" on every file; run it
from `apps/desktop`. And under zsh, a variable listing several paths is not
split: pass them one by one.
