<!-- oxyn-translation source="README.md" sha256="1ce715f87d78" -->

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

Pour une visite guidée, regardez la [vue d’ensemble de 46 secondes](../../assets/demo/videos/motion/overview.mp4)
et les [démonstrations animées](assets/demo/README.md#démonstrations-animées), une
par fonctionnalité — reconstruites à partir des composants de l’interface, pas
des enregistrements d’écran.

## Ce que le dépôt contient

Un seul workspace Cargo, dont les membres sont `crates/*` (le cœur, l'application
Tauri et son binaire) et `drivers/*` (les implémentations de protocoles), et le
front end de l'application dans `apps/desktop`. Le découpage et le sens des
dépendances font autorité dans [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Télécharger

Les paquets sont joints aux
[releases GitHub](https://github.com/so-keyldzn/oxyn/releases) : un DMG pour
macOS Apple Silicon (13 ou plus récent), et un DEB, un RPM et une AppImage pour
Linux x86_64 et ARM64. macOS Intel et Windows ne sont pas construits
([docs/RELEASE.md](../../docs/RELEASE.md)).

## Construire

L'application est en Tauri 2, avec un front end dans `apps/desktop`
([ADR-0029](docs/adr/0029-interface-tauri-shadcn.md)). Prérequis :

- **Rust** : la chaîne d'outils est épinglée par `rust-toolchain.toml` et rustup
  l'installe au premier `cargo`
  ([ADR-0008](docs/adr/0008-chaine-outils-rust.md)).
- **Node 22** : la version qu'utilise la CI ; le front end accepte `^20.19.0` ou
  `>=22.12.0`.
- **pnpm**, à la version du champ `packageManager` de
  [apps/desktop/package.json](../../apps/desktop/package.json). Le Makefile
  s'arrête avec un message si `pnpm` manque, et installe lui-même les
  dépendances du front end (`pnpm install --frozen-lockfile`).
- **Linux uniquement**, les bibliothèques système que la CI installe avant de
  construire : `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev
  libssl-dev libayatana-appindicator3-dev librsvg2-dev` (noms Debian et Ubuntu,
  tirés des prérequis de Tauri consignés dans
  [RESEARCH-NOTES](docs/RESEARCH-NOTES.md)).
- **macOS uniquement**, les Xcode Command Line Tools (`xcode-select --install`)
  — ou Xcode — dont l'hôte Tauri a besoin pour compiler ; l'image macOS de la CI
  les a déjà, seule une machine neuve révèle l'oubli (prérequis de Tauri
  consignés dans [RESEARCH-NOTES](docs/RESEARCH-NOTES.md)).
- **`python3`**, pour `make desktop PROFIL=release` et pour la porte de qualité.

```sh
make desktop-dev              # l'application avec rechargement à chaud, sur un workspace temporaire
make desktop                  # une construction debug, sans paquet
make desktop PROFIL=release   # la construction optimisée et ses paquets
```

`make desktop PROFIL=release` régénère les mentions des dépendances tierces que
l'application embarque, et refuse de construire sans `cargo-about` installé. Les
paquets atterrissent dans `target/release/bundle`. Le workflow de livraison
([.github/workflows/livraison.yml](../../.github/workflows/livraison.yml)) lance
cette même cible, puis signe et notarise les paquets macOS
([docs/RELEASE.md](../../docs/RELEASE.md)).

## La porte de qualité

```sh
make qualite
```

C'est le **seul** point d'entrée : la CI en appelle les cibles dans des jobs
parallèles ([.github/workflows/qualite.yml](../../.github/workflows/qualite.yml)), le hook `Stop`
le rappelle, la définition de « terminé » s'y adosse. Un contrôle ajouté
ailleurs est un contrôle optionnel.

Elle enchaîne : cohérence du socle de pilotage, `TODO` datés, versions
épinglées, le front end (licences et avis de sécurité npm, format, lint, types,
stories, construction), puis le Rust (format, clippy, tests, documentation,
licences et avis de sécurité des dépendances). `make aide` liste les autres
cibles.

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
