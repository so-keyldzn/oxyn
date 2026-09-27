---
name: perimetre-cargo-fmt
description: "On oxyn, use cargo fmt -p on your own crates only rather than cargo fmt --all: other agents edit the repository at the same time"
metadata:
  type: feedback
---

Use `cargo fmt -p <crate>` and `cargo fmt -p <crate> --check` on the crates of
your scope only, never `cargo fmt --all`.

**Why:** Nicolas has several agents work in parallel on disjoint crates of the
same uncommitted tree. `cargo fmt --all` reformats another agent's files,
including a file left mid-edit, and the resulting diff belongs to nobody. On
2026-09-10, `cargo fmt --all --check` failed on `crates/oxyn-app/src/root.rs`
while that file was out of scope — so the global check says nothing useful about
your own work.

**How to apply:** check afterwards with
`find crates drivers -name '*.rs' -not -path '*/target/*' -newermt '<time>'`
if there is any doubt about what a command touched. The global gate
(`make qualite`) is launched by Nicolas, not by the agent.
