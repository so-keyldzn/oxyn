---
name: piege-nextest-absent
description: cargo-nextest is not installed on the dev machine; the gate's tests step falls back to cargo test
metadata:
  type: reference
---

`cargo nextest` answers "no such command" on the dev machine (observed on
2026-09-24). The Makefile detects it and falls back to
`cargo test --workspace --all-features`, which also runs the doctests.

**How to apply:** to reproduce the tests step on targeted crates, run
`cargo test -p <crate> --all-features` (and `--bins` for `oxyn-desktop`, see
[[piege-cargo-oxyn-desktop-sans-lib]]), not `cargo nextest run`.
