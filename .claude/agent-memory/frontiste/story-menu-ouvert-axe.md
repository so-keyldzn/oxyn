---
name: story-menu-ouvert-axe
description: A story that ends with a Base UI menu still open or closing fails on axe (aria-hidden-focus of the focus guards)
metadata:
  type: feedback
---

A story that clicks a context menu (or submenu) item and ends right away fails
in `make front` on `aria-hidden-focus`: axe runs after the `play` and finds the
menu's `span[data-base-ui-focus-guard]` still mounted.

**Why:** the Base UI menu unmounts after its closing animation; axe runs before.

**How to apply:** end every `play` that opens a menu with
`await waitFor(() => expect(within(document.body).queryByRole("menu")).toBeNull())`,
including after a click that closes the menu by itself. Between two successive
openings, also wait for the closing, otherwise `Escape` targets the old popup.
