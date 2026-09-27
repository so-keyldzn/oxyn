---
name: piege-base-ui-initialfocus-a-l-ouverture
description: Base UI AlertDialog — `initialFocus` only acts on opening; a body remounted under an already open dialog loses focus
metadata:
  type: feedback
---

The `initialFocus` of an `AlertDialogContent` (Base UI) only applies **on opening**.
If the dialog stays open and its content is remounted (a `key` change, one
request following another), focus stays on an element removed from the DOM, and
"Cancel" does not take it back. Nothing announces it: the opening stories stay green.

**Why:** observed on 2026-09-24 on the sample screen: an agent request waiting
behind the user's pin appeared without Cancel having focus. A `play` story that
chains two requests shows it; a story with a single request does not.

**How to apply:** any dialog whose content can change without closing gives
focus back itself when the body mounts (`useEffect(() => ref.current?.focus(), [ref])`),
and its story chains two contents before checking `toHaveFocus`.
