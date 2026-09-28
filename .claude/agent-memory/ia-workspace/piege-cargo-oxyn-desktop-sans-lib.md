---
name: piege-cargo-oxyn-desktop-sans-lib
description: `cargo test -p oxyn-desktop --lib` fails ("no library targets"); the desktop backend tests run with `--bins`
metadata:
  type: feedback
---

`oxyn-desktop` has only a binary target: `cargo test -p oxyn-desktop --lib <filter>` returns "no library targets found". Use `cargo test -p oxyn-desktop --bins <filter>`.

**Why:** observed on 2026-09-23 while running a test of `backend/ai/conversation/tests.rs`; the error has nothing to do with the code.

**How to apply:** for any targeted test in `crates/oxyn-desktop`, pass `--bins` directly.
