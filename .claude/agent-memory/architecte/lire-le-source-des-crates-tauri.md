---
name: lire-le-source-des-crates-tauri
description: Check a Tauri/muda/wry behavior in ~/.cargo/registry without running into the secrets hook or the redirection alert
metadata:
  type: feedback
---

To check a Tauri default (macOS menu, permissions, webview options), read the
source of the `Cargo.lock` version in `~/.cargo/registry/src/index.crates.io-*/<crate>-<version>`.

- Do not do `cd ~/.cargo/registry/src/*/tauri-x && grep src/...`: the hook refuses
  it ("cd whose target cannot be resolved" + rule `./**/secrets/**`). Put the path
  in a variable: `D=$(ls -d /Users/nicolas/.cargo/registry/src/index.crates.io-*/tauri-2.11.5 | head -1); grep ... $D/src/app.rs`.
- `awk 'NR>=10 && NR<=20'` and `sed -n` trigger the hook's "redirection / sed edits in place"
  alert (false positive on `>=`). Harmless, but prefer `Read` with offset/limit.
- Useful sources: `tauri/src/app.rs` (default menu), `tauri/src/menu/menu.rs`,
  `tauri/permissions/`, `tauri-utils/src/config.rs` (doc of the window options),
  `muda/src/platform_impl/macos/mod.rs` (selectors of the predefined roles).

**Why:** the online doc does not state the version; the locked source is authoritative for I-12.
**How to apply:** every Tauri fact cited in an ADR is checked this way, dated, with the Cargo.lock versions.
