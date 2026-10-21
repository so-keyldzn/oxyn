---
name: verif-rapide-worktree-sans-front
description: make verif-rapide fails in a fresh worktree on a missing apps/desktop/dist/client as soon as Cargo.toml changes; for a docs-only change, make socle is the conclusive check
metadata:
  type: feedback
---

In a fresh worktree, `make verif-rapide` stops with "oxyn-desktop embeds
apps/desktop/dist/client, which is missing: run `make front-build`" as soon as
a workspace-wide file (root `Cargo.toml`) differs from origin/main — even when
that change belongs to another agent working in the same worktree.

**Why:** a modified root manifest makes it check the whole workspace, and
`oxyn-desktop` embeds the front-end build at compile time.

**How to apply:** for a docs/ADR-only commit, `make socle` (dead links,
mirror sha256, manifests) is what proves the change; report the Rust part
of `verif-rapide` as blocked by the environment rather than running
`make front-build` for someone else's crate.
