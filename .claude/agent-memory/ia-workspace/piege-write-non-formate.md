---
name: piege-write-non-formate
description: A file created with Write is not passed through the formatter hook; only Edit triggers it, so a new .rs file can fail `cargo fmt --check` in make verif-rapide
metadata:
  type: feedback
---

A Rust file created with the Write tool is **not** reformatted by the
PostToolUse formatter hook; only an Edit triggers it (observed 2026-10-04 on
a new test module). `make verif-rapide` then fails at `cargo fmt --all -- --check`.

**Why:** the hook reports "PostToolUse hook modified … after your edit" on Edit
only; the Write of a new file left long lines unwrapped.

**How to apply:** after writing a new `.rs` file, make any small Edit to it
(the hook then formats the whole file), or expect the fmt stage to flag it and
fix by Edit — never by running a shell formatter or heredoc. See also
[[piege-edit-sequence-u-decodee]].
