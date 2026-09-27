---
name: sources-tauri-registre-local
description: Source a Tauri behavior (I-12) by reading the unpacked sources of the local Cargo registry, and the hook trap on `cd` to a glob path
metadata:
  type: reference
---

To source a Tauri behavior without citing it from memory (I-12), read the
unpacked sources in `~/.cargo/registry/src/index.crates.io-<hash>/tauri-<version>`,
at the version found in `Cargo.lock` (`grep -A1 '^name = "tauri"$' Cargo.lock`).
The docstrings carry the platform limits there (e.g. a deadlock on Windows when
creating a window from a synchronous command); `tauri-runtime-wry` carries the
real semantics of the event loop.

**Tooling trap:** `cd ~/.cargo/registry/src/*/tauri-…` then `grep src` is
refused by a hook ("a deny rule (./**/secrets/**) … a cd whose target cannot
be resolved"). Resolve the path first with `ls -d`, then pass absolute paths in
a variable, without `cd`.
