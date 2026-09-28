---
name: wrap-anywhere-dans-une-grille
description: "A long token without spaces in an Alert (grid) widens the whole page despite break-words: use wrap-anywhere"
metadata:
  type: feedback
---

For text coming from the server (error message, identifier) in an `Alert` or any
grid/flex container, write `wrap-anywhere`, not `break-words`.

**Why:** `break-words` (`overflow-wrap: break-word`) breaks on display but does
**not** reduce the min-content width. The generated `Alert` is a
`grid-cols-[auto_1fr]` grid: its text column grows up to the longest word, and a
180-character relation name brought the page to 1,391 px at every width below
1,440. `wrap-anywhere` (`overflow-wrap: anywhere`) lowers the min-content; even
`overflow-auto` on the `<pre>` is not enough, because it is the child's
contribution to the grid that counts. Observed on 2026-09-16.

**How to apply:** every `<pre>`/server message text in `Alert`,
`AlertDescription`, a flex or grid cell. Add `min-w-0` on the grid child. Prove
it with a 420 px story that compares the container's `scrollWidth` and
`clientWidth`.
