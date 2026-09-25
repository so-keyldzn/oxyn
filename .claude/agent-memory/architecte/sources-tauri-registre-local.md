---
name: sources-tauri-registre-local
description: Sourcer un comportement Tauri (I-12) en lisant les sources dépaquetées du registre Cargo local, et le piège du hook sur `cd` vers un chemin glob
metadata:
  type: reference
---

Pour sourcer un comportement de Tauri sans le citer de mémoire (I-12), lire les
sources dépaquetées dans `~/.cargo/registry/src/index.crates.io-<hash>/tauri-<version>`,
à la version relevée dans `Cargo.lock` (`grep -A1 '^name = "tauri"$' Cargo.lock`).
Les docstrings y portent les limites de plateforme (ex. blocage sous Windows à
la création de fenêtre depuis une commande synchrone) ; `tauri-runtime-wry`
porte la sémantique réelle de la boucle d'événements.

**Piège d'outillage :** `cd ~/.cargo/registry/src/*/tauri-…` puis `grep src` est
refusé par un hook (« a deny rule (./**/secrets/**) … a cd whose target cannot
be resolved »). Résoudre d'abord le chemin avec `ls -d`, puis passer des chemins
absolus en variable, sans `cd`.
