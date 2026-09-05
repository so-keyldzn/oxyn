---
paths:
  - "**/Cargo.toml"
  - "Cargo.lock"
  - "rust-toolchain.toml"
  - "Makefile"
  - "deny.toml"
---

# Manifestes et outillage — conventions

## Aucune version ne s'écrit de mémoire

[I-12](../../CLAUDE.md#i-12). Une version plausible et fausse ne se voit ni à la
compilation, ni aux tests, ni en revue — elle se voit quand quelqu'un essaie de
construire le projet six mois plus tard.

Avant d'ajouter ou de modifier une dépendance : [`/versions`](../commands/versions.md).
La valeur retenue est reportée dans
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md) **dans le même commit**.

## Le workspace centralise

Les dépendances communes vont dans `[workspace.dependencies]`, les crates y font
référence par `{ workspace = true }`. Deux versions du même crate dans le graphe,
c'est deux fois le code compilé et des types incompatibles entre eux — le
message d'erreur qui en résulte est célèbre pour son opacité.

## Les épinglages de GPUI

`gpui 0.2.2` épingle plusieurs dépendances avec `=`, dont `cocoa =0.26.0`,
`cocoa-foundation =0.2.0`, `core-foundation =0.10.0`
([RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md#gpui)).

**Vérifier avant, pas après.** Ajouter une crate qui touche aux API système
macOS peut rendre le graphe insoluble : Cargo échoue au lieu d'unifier, et le
message ne désigne pas GPUI comme responsable.

## `rust-toolchain.toml`

Version **exacte**, avec `rustfmt` et `clippy`
([ADR-0008](../../docs/adr/0008-chaine-outils-rust.md)). Une montée de version
est un commit délibéré, qui met à jour
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md) en même temps.

## Nouvelle dépendance

Elle se justifie en revue : ce qu'elle apporte, et le coût de s'en passer
([SECURITY](../../docs/SECURITY.md#dépendances)). Une crate utilisée à un seul
endroit pour une seule fonction est un candidat à la réécriture, pas une
évidence. Une crate non maintenue sur une frontière externe est un risque à
documenter.

## `make qualite`

C'est la porte de qualité, et le seul point d'entrée. Si un contrôle n'y est pas,
il ne tourne pas : la CI l'appelle, le hook `Stop` le rappelle, la définition de
« terminé » s'y adosse. Ajouter un contrôle ailleurs, c'est le rendre optionnel.
