---
name: select-base-ui-listbox-sans-nom-axe
description: A story that opens a Base UI Select fails axe (aria-input-field-name) if the list is still there at the end of the play
metadata:
  type: feedback
---

Base UI 1.x does not name the `role="listbox"` of `Select` (neither `aria-label`
nor `aria-labelledby`), and `SelectContent` passes nothing to
`SelectPrimitive.List`. axe runs **after** the `play`: if the list is still
mounted (closing animation included), the story fails with
`aria-input-field-name`.

**Why:** encountered on the stories of the assistant's chart, 2026-09-24; the
`assistant-agent-settings` stories already worked around it this way.

**How to apply:** end every `play` that opens a Select with
`await waitFor(() => expect(within(document.body).queryByRole("listbox")).toBeNull())`.
Do not touch `src/components/ui/select.tsx` (generated).
