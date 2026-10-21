---
name: clippy-all-targets-compte-double
description: cargo clippy --all-targets reports every lint of a library module twice (lib build + lib test harness); count with --lib before quoting a number
metadata:
  type: feedback
---

`cargo clippy -p <crate> --all-targets` compiles a library twice — as the
library and as its `cfg(test)` harness — and reports each site once per
build. On 2026-10-07 the generated `model.rs` of `oxyn-embed` showed 138
`unwrap_used` with `--all-targets` for 69 distinct sites with `--lib`.

**Why:** a lint count quoted in an ADR came out doubled and contradicted a
grep of the file.

**How to apply:** before writing a lint count in a document, run
`--lib` (or count distinct primary spans in `--message-format=json`). To
lift an item-level `#[allow]` without touching the shared tree, use a
throwaway `git worktree add --detach` in the scratchpad with
`CARGO_TARGET_DIR` pointing at the existing `target/` (no rebuild of deps).
