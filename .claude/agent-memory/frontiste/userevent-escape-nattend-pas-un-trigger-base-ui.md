---
name: userevent-escape-nattend-pas-un-trigger-base-ui
description: In a Storybook play, userEvent.keyboard("{Escape}") with focus on a Button that is a Base UI TooltipTrigger triggers no React onKeyDown, while a real key works
metadata:
  type: feedback
---

A `play` that does `button.focus()` then `userEvent.keyboard("{Escape}")` **does
not trigger any React `onKeyDown`** when that button is a Base UI
`TooltipTrigger` (`<TooltipTrigger render={<Button …/>}>`). `toHaveFocus()`
passes, no tooltip is open, and yet an ancestor's handler is never called. The
same key sent by the real keyboard (Playwright `keyboard.press("Escape")` on the
same story) works.

**Why:** two hours lost believing in a bug of my Escape handler on the Oxyn
workspace; the component had to be instrumented with a `console.log` and the
story runner compared with a real browser to see that the event was not sent.
From an `<input>` or a `role=tab`, `userEvent.keyboard` works just fine.

**How to apply:** to prove a global keyboard shortcut in a story, put the focus
on an **ordinary** element (field, tab) rather than on a wrapped Base UI
trigger. If the story really has to start from a button with a tooltip, check
first in a real browser before concluding the code is wrong — and instrument the
component rather than the test.
