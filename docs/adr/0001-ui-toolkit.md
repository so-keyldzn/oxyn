# ADR-0001 — Toolkit UI : GPUI, avec isolation stricte

**Statut :** remplacé · **Date :** 2026-09-05
**Remplacé par :** [ADR-0029](0029-interface-tauri-shadcn.md), le 2026-09-15. Ce
qui suit est conservé tel qu'il a été décidé : c'est la raison pour laquelle GPUI
paraissait le bon choix, et ce qui a changé depuis se lit dans l'ADR-0029.

## Contexte
« Native first, blazing fast » exclut Electron et Tauri. En Rust pur, deux options
crédibles : egui (immediate mode, stable, cross-platform) et GPUI (retenu, moteur de
Zed, excellent sur le texte). Les deux surfaces critiques d'Oxyn sont l'éditeur de
requêtes et la grille de résultats.

## Décision
Adopter **GPUI**. Interdire toute dépendance au toolkit hors de `oxyn-ui` et `oxyn-app`.

> **Précisé par [ADR-0009](0009-source-dependance-gpui.md).** La rédaction initiale
> parlait d'« épingler un commit précis », supposant qu'il faudrait vendorer le monorepo
> Zed. C'est faux : `gpui` est publié sur crates.io. La consommation depuis le registre,
> en version exacte, est tranchée par l'ADR-0009.

## Justification
Le coût dominant du projet est l'éditeur de code de qualité professionnelle. GPUI fournit
un moteur de texte et des primitives d'édition éprouvées ; egui obligerait à les
reconstruire, ce qui dépasse le coût de l'instabilité d'API de GPUI.

## Conséquences
* **+** Rendu de texte et latence au niveau de Zed ; layout flexbox familier.
* **+** *Vérifié le 2026-09-05* : une fenêtre GPUI 0.2.2 utilisant `Application::new().run`,
  `cx.open_window` et `impl Render` compile sur macOS 26.2 / arm64 / Rust 1.89 en 1 min 52.
  Le doute sur la faisabilité même de la dépendance est levé.
* **−** Documentation pauvre, lecture du code de Zed souvent nécessaire ; publications
  espacées sur crates.io (cf. ADR-0009).
* **−** Windows fragile → macOS et Linux d'abord, assumé.
* La règle d'isolation garde la décision réversible : basculer vers egui reste une
  réécriture de deux crates jusqu'à la fin de la phase 1.

## Alternatives écartées
* **egui** — retenu comme plan de repli, pas comme cible.
* **Iced** — bon modèle, moteur de texte insuffisant pour un éditeur de code.
* **Slint / Dioxus** — DSL ou VDOM ; pas d'avantage décisif ici.
