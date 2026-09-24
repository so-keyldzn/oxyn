---
paths:
  - "**/Cargo.toml"
  - "Cargo.lock"
  - "rust-toolchain.toml"
  - "Makefile"
  - "deny.toml"
  - "clippy.toml"
  - ".cargo/config.toml"
  - ".config/nextest.toml"
  - "renovate.json5"
  - ".github/workflows/*.yml"
  - "script/*"
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
il ne tourne pas : la CI l'appelle
([.github/workflows/qualite.yml](../../.github/workflows/qualite.yml)), le hook
`Stop` le rappelle, la définition de « terminé » s'y adosse. Ajouter un contrôle
ailleurs, c'est le rendre optionnel.

**La CI n'ajoute aucun contrôle.** Elle appelle les cibles de `make qualite`,
réparties en jobs parallèles, et rien d'autre. Un contrôle qui n'existerait que
dans le fichier de workflow serait irreproductible en local : on découvrirait son
existence en le voyant échouer.

Le découpage a le risque inverse : une cible ajoutée à `qualite` et oubliée dans
le workflow ne tournerait jamais en CI. `make socle` le refuse
(`controler_couverture_ci`). Une nouvelle cible de la porte s'ajoute donc aux
deux endroits dans le même commit.

## Où vivent les interdits mécanisables

| Fichier | Ce qu'il refuse |
|---|---|
| [clippy.toml](../../clippy.toml) | les chemins d'appel interdits — `disallowed-methods`, avec la raison et le remplacement |
| [.cargo/config.toml](../../.cargo/config.toml) | rien ; il impose les drapeaux qui doivent valoir pour tout le monde, dont la cible macOS |
| [.config/nextest.toml](../../.config/nextest.toml) | un test qui pend, et deux tests qui se partagent un serveur |
| [renovate.json5](../../renovate.json5) | une version recopiée de mémoire — c'est [I-12](../../CLAUDE.md#i-12) mécanisé |

Un invariant qui se ramène à un chemin d'appel appartient à `clippy.toml`, pas à
une relecture. `clippy.toml` vaut pour **tout** le workspace : un interdit qui ne
doit valoir que pour une crate n'y a pas sa place.

## Une crate ne se crée pas à la main

`script/nouvelle-crate <nom> "<description>"`. La raison est écrite dans
[CLAUDE.md](../../CLAUDE.md) : une règle `paths:` se charge quand un fichier est
**lu**, pas quand il est créé. Un manifeste écrit de mémoire oublie
`[lints] workspace = true`, et la crate échappe alors à tous les lints du dépôt
sans que rien n'échoue. `make socle` le rattrape après coup ; le script l'évite.
