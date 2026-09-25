# Oxyn

Workspace natif et haute performance pour explorer, interroger et gérer tout
type de base de données — relationnelle, analytique, NoSQL, vectorielle, graphe.
Des agents IA y aident à comprendre les schémas et les requêtes, **aux côtés**
de l'utilisateur, jamais à sa place.

Public visé : les professionnels des données, ceux qui lisent les messages
d'erreur de PostgreSQL.

Le périmètre et les principes font autorité dans [docs/VISION.md](docs/VISION.md).
L'état d'avancement réel est dans
[docs/IMPLEMENTATION-PLAN.md](docs/IMPLEMENTATION-PLAN.md) — ce README ne le
duplique pas, parce qu'une copie de l'avancement est périmée le lendemain.

## Ce que le dépôt contient

15 crates dans un seul workspace Cargo : `crates/` pour le cœur, l'interface et
le binaire ; `drivers/` pour les implémentations de protocoles. Le découpage et
le sens des dépendances font autorité dans
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Construire

Prérequis : macOS 13 ou plus récent, `python3`, et rien d'autre — la chaîne
d'outils Rust est épinglée par `rust-toolchain.toml` et rustup l'installe seul
au premier `cargo` ([ADR-0008](docs/adr/0008-chaine-outils-rust.md)).

```sh
make app      # assemble target/debug/Oxyn.app
make lancer   # assemble puis ouvre l'application
```

`PROFIL=release` produit le paquet optimisé. Le paquet `.app` n'est pas une
étape de publication : sans lui, macOS traite le binaire comme un accessoire —
ni Dock, ni activation, ni focus clavier correct.

## La porte de qualité

```sh
make qualite
```

C'est le **seul** point d'entrée : la CI en appelle les cibles dans des jobs
parallèles ([.github/workflows/qualite.yml](.github/workflows/qualite.yml)), le hook `Stop`
le rappelle, la définition de « terminé » s'y adosse. Un contrôle ajouté
ailleurs est un contrôle optionnel.

Elle enchaîne : cohérence du socle de pilotage, `TODO` datés, format, clippy,
tests, documentation. `make aide` liste les autres cibles.

## Où se trouve la vérité

| | |
|---|---|
| [docs/](docs/README.md) | le domaine — vision, architecture, contrats, sécurité, budgets, ADR |
| [CLAUDE.md](CLAUDE.md) | la carte du dépôt et les treize invariants |
| [.claude/](.claude/README.md) | la manière de travailler — règles, commandes, agents, hooks |
| [AGENTS.md](AGENTS.md) | l'entrée équivalente pour Codex |

**En cas de contradiction entre le code et un document de `docs/`, c'est un
bug** : le signaler, ne pas trancher seul.

## Langue

Code, identifiants, commentaires, messages d'erreur et `///` sont en **anglais**.
Documentation, ADR, messages de commit et échanges sont en **français**. La
frontière est celle du code source : ce qu'un contributeur international doit
lire est en anglais.

## Licence

Copyright 2026 Nicolas Boromée.

L'application est sous **GPL-3.0-or-later** ([LICENSE-GPL](LICENSE-GPL)). Les
quatre crates qu'un driver tiers doit lier (`oxyn-core`, `oxyn-catalog`,
`oxyn-data` et `oxyn-driver`) sont sous **Apache-2.0**
([LICENSE-APACHE](LICENSE-APACHE)). Un auteur de driver ou de plugin choisit
donc sa propre licence. [NOTICE](NOTICE) dit quelle licence couvre quelle
partie, et [ADR-0044](docs/adr/0044-licence-gpl-et-contrat-apache.md) dit
pourquoi.

Tout le code de ce dépôt s'utilise sans compte ni abonnement. Ce qui se paiera,
ce sont des services optionnels rattachés à un compte, comme l'IA hébergée ou la
synchronisation.

Une contribution externe demande d'accepter le [CLA](CLA.md) : voir
[CONTRIBUTING](CONTRIBUTING.md).
