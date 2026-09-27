---
name: piege-rustfmt-commentaire-tableau
description: rustfmt glues a one-line comment placed before a short array element onto the end of the previous element's line — the comment changes owner
metadata:
  type: feedback
---

In an array of short elements (`["PATH", "HOME", …]`), rustfmt switches to the
"mixed" layout: a **single-line** `//` comment placed above an element is moved
up to the end of the line of the **previous** element
(`"LANG", // Where a launcher unpacks…` instead of introducing `"TMPDIR"`). An
end-of-line comment on the last element is worse: rustfmt merges the last two
elements onto one line.

**Why:** that is how a comment of `ALWAYS_PASSED` (spawn.rs) ended up attached
to the wrong variable, and a review flagged it. Nothing fails: the format is
"correct", the meaning is wrong.

**How to apply:** any comment above a short array element spans **at least two
lines** (rustfmt does not move a multi-line block). Check with
`rustfmt --edition 2024 --check <file>` after the edit, not only at the end.
