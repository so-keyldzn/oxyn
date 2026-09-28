---
name: lexical-dans-les-stories
description: Testing a Lexical field in a story — userEvent.type works, toHaveValue does not, and the typeahead menu rewrites its ARIA at every keystroke
metadata:
  type: feedback
---

A Lexical field (`ContentEditable`) in a story:

* `userEvent.type` and `userEvent.keyboard` from `storybook/test` **work**
  (Lexical does receive `beforeinput`); no need to look for a real keyboard;
* `toHaveValue` returns `undefined` on a `contenteditable`: compare
  `field.textContent` or `toHaveTextContent`;
* an IME composition is simulated with
  `fireEvent.keyDown(field, { key: "Enter", isComposing: true })`;
* the `LexicalTypeaheadMenuPlugin` menu is **portaled into `document.body`**,
  not into `canvasElement`: `within(document.body).findByRole("listbox", …)`.

The most expensive trap: the menu's anchor is detached then reattached **at
every keystroke**, and Lexical puts `aria-label="Typeahead menu"` back in an
effect that runs after the children's. A `setAttribute` in an effect therefore
only holds for one keystroke; a `MutationObserver` on the attribute is needed.
For the class, the `anchorClassName` prop is enough. It also leaves
`aria-activedescendant` on `typeahead-item-0` when the list empties (axe
`aria-valid-attr-value`), and a list without options fails with
`aria-required-children`.

The shape that passes axe in `error`: Lexical's anchor is forced to
`role="presentation"` (without its `aria-label` or its `id`), and it is **the
scrolling container** that carries `role="listbox"` and `tabIndex={0}`. A
`tabIndex={-1}` on a scrolling div *inside* the listbox makes
`aria-required-children` fail, and a `tabIndex={-1}` on the options is not
enough for `scrollable-region-focusable`. The status rows (empty, searching,
error) are `role="option" aria-disabled`. The sourced detail is in
`docs/RESEARCH-NOTES.md`, section "Interface Tauri et front".

**Why:** each `vitest --project storybook` round trip costs ~15 s, and the
"Unable to find role=listbox" failure does not say that the name changed.

**How to apply:** for any component that mounts a `LexicalComposer`, and any
future typeahead list (`/` commands, for example). See
[[pieges-de-la-porte-front]].
