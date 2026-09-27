---
name: piege-cargo-fmt-portee-crate
description: cargo fmt -p reformats the whole crate, including the files another agent is currently writing
metadata:
  type: feedback
---

`cargo fmt -p <crate>` has no file granularity: it rewrites the **whole** crate.
When a task has an exclusive scope and another agent works in the same crate,
this touches its files in progress.

**Why:** seen in session — formatting `-p oxyn` while another agent was writing
`workspace/library.rs`; formatting is idempotent so no harm done, but the diff
goes beyond the announced scope and muddles the review.

**How to apply:** when the scope is exclusive, prefer
`rustfmt --edition 2024 <files>` on the touched files only, or check
`find <crate> -name '*.rs' -mmin -N` before/after to know what moved.
Corollary: `cargo clippy`/`cargo test` on a shared crate will fail
intermittently on the other agent's work — wait with
`until cargo check -p <crate>; do sleep 10; done` rather than "repairing" its
code.
