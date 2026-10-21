---
name: piege-verif-rapide-sans-build-front
description: In a fresh worktree, `make verif-rapide` stops at "oxyn-desktop embeds apps/desktop/dist/client, which is missing" before any Rust step
metadata:
  type: reference
---

`make verif-rapide` refuses to check `oxyn-desktop` when `apps/desktop/dist/client` is absent — the case of every new worktree (observed 2026-10-04). `cargo check/clippy/test -p oxyn-desktop` themselves build fine without it.

**How to apply:** either run `make front-build` first (pnpm, slow), or reproduce the Rust steps by hand on the touched crates — `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test -p <crate> --all-features` (`--bins` for oxyn-desktop, see [[piege-cargo-oxyn-desktop-sans-lib]]), `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps`, `python3 .claude/verifier_socle.py` — and let CI run the full gate.
