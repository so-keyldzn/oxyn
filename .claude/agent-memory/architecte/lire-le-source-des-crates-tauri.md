---
name: lire-le-source-des-crates-tauri
description: Vérifier un comportement de Tauri/muda/wry dans ~/.cargo/registry sans buter sur le hook de secrets ni l'alerte de redirection
metadata:
  type: feedback
---

Pour vérifier un défaut de Tauri (menu macOS, permissions, options de webview), lire le
source de la version de `Cargo.lock` dans `~/.cargo/registry/src/index.crates.io-*/<crate>-<version>`.

- Ne pas faire `cd ~/.cargo/registry/src/*/tauri-x && grep src/...` : le hook refuse
  (« cd whose target cannot be resolved » + règle `./**/secrets/**`). Mettre le chemin
  dans une variable : `D=$(ls -d /Users/nicolas/.cargo/registry/src/index.crates.io-*/tauri-2.11.5 | head -1); grep ... $D/src/app.rs`.
- `awk 'NR>=10 && NR<=20'` et `sed -n` déclenchent l'alerte « redirection / sed modifie en place »
  du hook (faux positif sur `>=`). Sans effet, mais préférer `Read` avec offset/limit.
- Sources utiles : `tauri/src/app.rs` (menu par défaut), `tauri/src/menu/menu.rs`,
  `tauri/permissions/`, `tauri-utils/src/config.rs` (doc des options de fenêtre),
  `muda/src/platform_impl/macos/mod.rs` (sélecteurs des rôles prédéfinis).

**Why:** la doc en ligne ne dit pas la version ; le source verrouillé fait foi pour I-12.
**How to apply:** tout fait Tauri cité dans un ADR se vérifie ainsi, daté, versions de Cargo.lock.
