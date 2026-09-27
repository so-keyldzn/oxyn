---
name: piege-scan-litteraux-francais
description: A line-by-line grep misses two thirds of a crate's French messages — `\` continuations and short strings without accents
metadata:
  type: feedback
---

To inventory user-facing text in Rust, a line-by-line `grep` on accents **is not
enough**. Two blind spots, each made me miss messages:

- a string broken by a `\` continuation at the end of a line: the literal spans
  several lines, a single-line `"..."` regex does not see it. The whole file has
  to be read with `re.S`;
- a short string **without accents**: `"nom trop long"`, `"le genre est vide"`,
  `"{} lignes / {} octets"`. A second pass on a lexicon of frequent French words
  is needed, not only on `[àâçéèêë…]`.

**Why:** each pass found about fifteen messages the other missed; sticking to
the first leaves French in production without anything failing.

**How to apply:** for any inventory of literals (translation, secret leak,
message audit), write the scan in the scratchpad with both passes, cut at the
first `\n#[cfg(test)]\nmod ` and **do not filter** multi-line results on display
— hiding them is how I lost them the first time. Then check with `cargo test`,
which flushes out tests comparing a message.
