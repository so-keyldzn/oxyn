---
name: piege-nextest-absent
description: cargo-nextest n'est pas installé sur la machine de dev ; l'étape tests de la porte retombe sur cargo test
metadata:
  type: reference
---

`cargo nextest` répond « no such command » sur la machine de dev (constaté le
2026-09-24). Le Makefile le détecte et retombe sur
`cargo test --workspace --all-features`, qui exécute aussi les doctests.

**How to apply:** pour reproduire l'étape tests sur des crates ciblées, lancer
`cargo test -p <crate> --all-features` (et `--bins` pour `oxyn-desktop`, voir
[[piege-cargo-oxyn-desktop-sans-lib]]), pas `cargo nextest run`.
