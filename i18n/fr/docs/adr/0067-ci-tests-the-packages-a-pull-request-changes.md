<!-- oxyn-translation source="docs/adr/0067-ci-tests-the-packages-a-pull-request-changes.md" sha256="e2773ba442ad" -->

> Traduction française de [docs/adr/0067-ci-tests-the-packages-a-pull-request-changes.md](../../../../docs/adr/0067-ci-tests-the-packages-a-pull-request-changes.md). **La version anglaise fait foi.**

# ADR-0067 — Sur une pull request, la CI ne vérifie que les paquets Rust qu'elle modifie et ceux qui en dépendent

**Statut :** accepté · **Date :** 2026-10-24

**Précise :** [ADR-0045](0045-ci-selective-sur-les-pull-requests.md), sur un
point : à l'intérieur de la zone Rust, une pull request ne vérifie plus tout
l'espace de travail. Il revient sur l'alternative qu'ADR-0045 avait écartée,
« calculer les crates dépendantes et ne tester qu'elles en CI ». Tout le reste
d'ADR-0045 reste en vigueur : les zones, l'exécution complète sur `main`, le
job agrégat `qualite`.

## Contexte

Sous ADR-0045, une pull request n'est sélective que **par zone**. Tout fichier
sous `crates/` ou `drivers/` lance la totalité de `make rust` — `cargo fmt`,
clippy `--workspace`, nextest `--workspace --all-features`, les tests de
documentation, `cargo doc`, `cargo deny`, `package-contracts` — sur macOS et
Linux, plus le job `serveurs`, qui rejoue toutes les suites contre un serveur
de base de données (PostgreSQL, PostgreSQL TLS, MySQL, ClickHouse ; MongoDB,
Redis et Elasticsearch arrivent).

Les 2026-10-23 et 2026-10-24, avec six pull requests de pilotes ouvertes en
même temps, le job `rust` macOS a pris plus de 60 minutes par exécution, et la
file des runners est devenue le goulot du flux. Une pull request qui ne touche
que le pilote ClickHouse recompilait et retestait le pilote SQLite, l'espace
de travail IA et l'hôte de plugins, dont aucun ne peut observer le changement.

Le mainteneur a énoncé la règle deux fois : **« on ne teste que les parties
qu'on code, pas tout le code »**.

Ce qui rend le calcul sûr aujourd'hui, là où ADR-0045 le jugeait difficile :
le graphe entre membres de l'espace de travail est entièrement local. `cargo
metadata --no-deps --format-version 1` liste, pour chaque membre, chaque
dépendance qu'il **déclare** — normale, de dev et de build, pour toute
plateforme, optionnelles comprises — avec le `path` de celles qui sont des
membres. Il ne lit que les manifestes : ni registre, ni réseau, ni
téléchargement, ni compilation ; il répond en 0,1 s environ sur cet espace de
travail (mesuré le 2026-10-24 avec la chaîne d'outils épinglée).

## Décision

**Sur l'événement `pull_request` seulement**, `script/zones-ci` calcule, après
les zones, l'ensemble des **paquets Rust** à vérifier, et l'écrit dans la
sortie `rust_packages` du job `zones` (noms séparés par des espaces) :

1. chaque fichier modifié est attribué : un fichier sous `crates/` ou
   `drivers/` au paquet de l'espace de travail dont le répertoire le contient
   (le plus profond) ; un fichier sous `apps/desktop/` à `oxyn-desktop`, qui
   compile le front en lui (`tauri::generate_context!`, `include_str!` de
   `apps/desktop/src/lib/actions/actions.json`) ; un fichier de documentation
   (la zone `docs` d'ADR-0045) à rien ;
2. l'ensemble est fermé par les **dépendances inverses** entre membres, lues
   dans `cargo metadata --no-deps` : chaque arête déclarée compte,
   dépendances de dev comprises — les tests d'`oxyn-exec` utilisent le pilote
   PostgreSQL, donc un changement PostgreSQL vérifie `oxyn-exec` ;
3. le diff est lu par `git diff --name-only --no-renames` : un fichier déplacé
   d'un paquet à un autre compte pour les deux.

**Sûreté : une valeur vide veut dire tous les paquets**, exactement le
comportement d'ADR-0045. La valeur est vide quand :

* un fichier est transversal ou non classé selon ADR-0045 — `Cargo.toml` et
  `Cargo.lock` à la racine, `.cargo/`, `.config/`, `rust-toolchain.toml`,
  `clippy.toml`, `deny.toml`, le `Makefile`, `script/`, `.github/`, tout
  fichier inconnu ;
* un fichier sous `crates/` ou `drivers/` n'appartient à aucun paquet du
  commit de fusion — une crate supprimée, un fichier isolé ;
* `cargo metadata` échoue ou rend quelque chose d'illisible ;
* la fermeture atteint de toute façon tous les paquets ;
* l'événement n'est pas `pull_request` : un push sur `main` et
  `workflow_dispatch` vérifient toujours tout.

Le `Makefile` lit une variable optionnelle, **`OXYN_PACKAGES`**. Vide ou non
définie, chaque commande est octet pour octet la précédente : `make qualite`
et `make rust` ne changent pas en local. Définie, elle restreint :

| Cible | Avec `OXYN_PACKAGES="a b"` |
|---|---|
| `lint` | `cargo clippy -p a -p b --all-targets --all-features` |
| `test` | `cargo nextest run -p a -p b --all-features`, puis `cargo test --doc -p a -p b` ; les tests de documentation sont sautés, et la cible le dit, quand aucun paquet choisi n'a de `src/lib.rs` — `cargo test --doc` refuse une sélection sans bibliothèque, ce qui est le cas d'`oxyn-desktop` seul |
| `doc` | `cargo doc -p a -p b --no-deps --all-features` |
| `test-serveurs` | `cargo nextest run -p <les paquets choisis parmi SERVER_PACKAGES>` avec le même filtre ; rien quand aucun n'est choisi |
| `format`, `deny`, `package-contracts`, `socle`, `todo`, `versions` | inchangées : elles sont bon marché, ou ne raisonnent pas paquet par paquet |

Le workflow passe la valeur comme variable d'environnement aux étapes
existantes `make rust` et `make test-serveurs` ; il n'appelle aucune nouvelle
commande, donc la vérification de `make socle` selon laquelle la CI couvre
`make qualite` et n'appelle rien d'autre tient toujours. Le job `zones` installe
la chaîne d'outils épinglée pour lancer `cargo metadata`.

**Suites serveur.** `SERVER_PACKAGES`, à côté de la cible `test-serveurs`,
nomme les paquets qu'atteint son filtre nextest (`oxyn-driver-postgres`,
`oxyn-driver-mysql`, `oxyn-driver-clickhouse`, `oxyn-exec`, `oxyn-desktop`) ;
un test de `script/test_zones_ci.py` garde la liste et le filtre égaux.
`script/zones-ci` lit cette liste et écrit `serveurs=false` quand la sélection
n'en contient aucun ; le job `serveurs` ne tourne alors pas. Quand il tourne,
seules les suites choisies sont compilées et exécutées : une pull request
ClickHouse ne rejoue plus les suites PostgreSQL et MySQL.

Le job agrégat `qualite` et sa logique `if: always()` ne changent pas : un job
sauté compte toujours comme réussi. Une pull request qui ne touche que le front
ou la documentation garde le comportement d'ADR-0045.

Rejoué sur les commits de fusion de `main` le 2026-10-24 (`git log
--first-parent origin/main`) :

| Pull request | Ce qu'elle modifiait | Sélection |
|---|---|---|
| #294 | `oxyn-driver-postgres` | `oxyn-driver-postgres`, `oxyn-exec`, `oxyn-desktop` |
| #302 | `oxyn-ai`, `oxyn-desktop`, front | `oxyn-ai`, `oxyn-desktop` |
| #332 | `oxyn-driver`, `oxyn-exec` | 8 paquets sur 17 : les pilotes, `oxyn-exec`, `oxyn-plugin`, `oxyn-desktop` |
| #299 | le pilote ClickHouse, `oxyn-core`, `Cargo.toml`, `.config/nextest.toml` | tous |
| #328 | `qualite.yml` | tous |
| #329, #331 | front seulement | aucun : `make rust` ne tourne pas (ADR-0045) |

## Conséquences

* **+** une pull request sur un pilote feuille vérifie deux paquets — le pilote
  et `oxyn-desktop` — au lieu de dix-sept, et ne rejoue que sa propre suite
  serveur.
* **+** tout doute se résout en tout l'espace de travail : un graphe absent, un
  fichier non attribué et une sortie perdue donnent tous une valeur vide, et
  vide veut dire tout.
* **+** en local rien ne change : sans `OXYN_PACKAGES`, `make qualite` est le
  même portail, et `make socle` vérifie toujours que la CI le couvre.
* **−** `-p` n'unifie que les features des paquets choisis. Un paquet qui
  compile dans l'espace de travail grâce à une feature qu'un autre membre
  active peut échouer seul sur sa pull request — un faux rouge, qui est de
  toute façon une correction à faire. L'inverse, une combinaison qui n'échoue
  que sous l'unification de tout l'espace de travail, n'est pas vu avant
  `main`.
* **−** un fichier qu'une crate lit hors de son propre répertoire à l'exécution
  échappe à l'attribution. Au 2026-10-24, les seules lectures de ce genre sont
  celles d'`oxyn-desktop`, dans `apps/desktop/`, et elles sont attribuées
  explicitement ; une nouvelle doit être ajoutée à `script/zones-ci`, sinon
  elle n'est rattrapée que sur `main`.
* **−** `oxyn-desktop` dépend de toutes les crates, donc presque toute pull
  request Rust lance encore le job `serveurs`, avec au moins sa suite
  d'annulation : le gain y porte sur les suites sautées, pas sur le job.
* **−** le job `zones` installe la chaîne d'outils sur chaque pull request,
  documentation comprise : environ une demi-minute sur un job qui prenait
  quelques secondes.

**Coût de sortie :** faible. Retirer les sorties `rust_packages` et
`serveurs`, les deux entrées d'environnement `OXYN_PACKAGES` du workflow et la
variable du `Makefile` rétablit le comportement d'ADR-0045 ; la sélection de
paquets de `script/zones-ci` et son test sont supprimés. Rien hors de la CI
n'en dépend.

**À reconsidérer si** `main` passe au rouge après une pull request verte à
cause d'un paquet que cette sélection a sauté plus d'une fois par mois — il
faut alors élargir l'attribution, ou revenir à tout l'espace de travail pour la
zone Rust.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Le graphe résolu de `cargo metadata` (sans `--no-deps`) | il faut l'index du registre et les sources de chaque dépendance, sur un job de cinq minutes, pour des arêtes entre membres que les dépendances déclarées donnent déjà — et plus largement, optionnelles et autres plateformes comprises |
| Analyser les `Cargo.toml` en Python | une seconde implémentation de `workspace = true`, des renommages et des tables de cible, qui s'écarte de Cargo sans bruit |
| Restreindre aussi `format` et `deny` | `cargo fmt --all` prend quelques secondes, et `cargo deny` raisonne sur tout le graphe de dépendances, pas sur un paquet |
| Ne lancer que les paquets modifiés, sans leurs dépendants | un trait modifié casse ses implémentations dans d'autres crates ; c'est exactement l'échec qu'une pull request doit montrer |
| Un outil tiers de « crates affectées » | un binaire épinglé de plus dans la CI pour un calcul d'une quarantaine de lignes sur `cargo metadata` |
| Restreindre aussi `main` | supprime la seule exécution qui vérifie tout ensemble ; le filet de sécurité d'ADR-0045 est la raison pour laquelle une sélection sur les pull requests est acceptable |
