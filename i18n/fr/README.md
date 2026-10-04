<!-- oxyn-translation source="README.md" sha256="627c6f188e28" -->

> Traduction française de [README.md](../../README.md). **La version anglaise fait foi.**

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

## Interface et démonstrations

![Console SQL Oxyn et résultats de chiffre d’affaires issus d’une base SQLite fictive](../../assets/demo/screenshots/sql-workspace.jpg)

Consultez la [galerie de captures et les vidéos de démonstration](assets/demo/README.md) :

- [Explorer une base](../../assets/demo/videos/explore-database.mp4) : parcourir les
  lignes, inspecter les colonnes et lire la définition de la table.
- [Exécuter une requête de chiffre d’affaires](../../assets/demo/videos/run-query.mp4) :
  exécuter du SQL et inspecter les résultats.

Enregistrements réalisés dans **Oxyn 0.0.3 pour macOS**, avec un workspace
temporaire et des [données fictives reproductibles](../../assets/demo/seed.sql).

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
parallèles ([.github/workflows/qualite.yml](../../.github/workflows/qualite.yml)), le hook `Stop`
le rappelle, la définition de « terminé » s'y adosse. Un contrôle ajouté
ailleurs est un contrôle optionnel.

Elle enchaîne : cohérence du socle de pilotage, `TODO` datés, format, clippy,
tests, documentation. `make aide` liste les autres cibles.

## Où se trouve la vérité

| | |
|---|---|
| [docs/](docs/README.md) | le domaine — vision, architecture, contrats, sécurité, budgets, ADR |
| [CLAUDE.md](CLAUDE.md) | la carte du dépôt et les treize invariants |
| [.claude/](claude/README.md) | la manière de travailler — règles, commandes, agents, hooks |
| [AGENTS.md](AGENTS.md) | l'entrée équivalente pour Codex |
| [i18n/fr/](../README.md) | les miroirs français des documents anglais |

**En cas de contradiction entre le code et un document de `docs/`, c'est un
bug** : le signaler, ne pas trancher seul.

## Langue

Le dépôt s'écrit en **anglais** : code, identifiants, commentaires, messages
d'erreur, `///`, documentation, ADR, messages de commit et pull requests
([ADR-0047](docs/adr/0047-english-as-the-repository-language.md)).
[`i18n/fr/`](../README.md) porte des miroirs français ; **l'anglais fait foi**, et
`make qualite` refuse un miroir plus ancien que son original.

## Licence

Copyright 2026 Nicolas Boromée.

L'application est sous **GPL-3.0-or-later** ([LICENSE-GPL](../../LICENSE-GPL)). Les
quatre crates qu'un driver tiers doit lier (`oxyn-core`, `oxyn-catalog`,
`oxyn-data` et `oxyn-driver`) sont sous **Apache-2.0**
([LICENSE-APACHE](../../LICENSE-APACHE)). Un auteur de driver ou de plugin choisit
donc sa propre licence. [NOTICE](../../NOTICE) dit quelle licence couvre quelle
partie, et [ADR-0044](docs/adr/0044-licence-gpl-et-contrat-apache.md) dit
pourquoi.

Tout le code de ce dépôt s'utilise sans compte ni abonnement. Ce qui se paiera,
ce sont des services optionnels rattachés à un compte, comme l'IA hébergée ou la
synchronisation.

Une contribution externe demande d'accepter le [CLA](../../CLA.md) : voir
[CONTRIBUTING](CONTRIBUTING.md).
