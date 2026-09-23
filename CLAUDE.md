# Oxyn

Workspace natif et haute performance pour explorer, interroger et gérer tout
type de base de données — relationnelle, analytique, NoSQL, vectorielle, graphe.
Des agents IA y aident à comprendre les schémas et les requêtes, **aux côtés**
de l'utilisateur, jamais à sa place. Public : professionnels des données, qui
lisent les messages d'erreur de PostgreSQL.

**Langue.** Code, identifiants, commentaires, messages d'erreur, `///` : en
**anglais**. Documentation, ADR, messages de commit, échanges : en **français**.
La frontière est celle du code source : ce qu'un contributeur international doit
lire est en anglais.

**L'état du dépôt n'est pas ici** : il est injecté à chaque session par
`.claude/hooks/contexte_session.py`. Ce fichier ne contient rien de périssable.

## La documentation fait autorité

| Document | Fait autorité sur |
|---|---|
| [docs/VISION.md](docs/VISION.md) | le périmètre du produit et ses principes |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | crates, dépendances, command bus, threads |
| [docs/DRIVER-CONTRACT.md](docs/DRIVER-CONTRACT.md) | ce que tout driver garantit |
| [docs/AI-PROVIDERS.md](docs/AI-PROVIDERS.md) | ce qui traverse la frontière IA |
| [docs/PLUGIN-CONTRACT.md](docs/PLUGIN-CONTRACT.md) | ce qu'un plugin peut faire |
| [docs/SECURITY.md](docs/SECURITY.md) | secrets, connexions, surface d'entrée, `unsafe` |
| [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | les budgets chiffrés |
| [docs/UX-SPEC.md](docs/UX-SPEC.md) | les comportements d'interface |
| [docs/RESEARCH-NOTES.md](docs/RESEARCH-NOTES.md) | toute version externe, sourcée et datée |
| [docs/IMPLEMENTATION-PLAN.md](docs/IMPLEMENTATION-PLAN.md) | les phases et leurs portes de sortie |
| [docs/adr/](docs/adr/) | les décisions coûteuses à défaire |

**En cas de contradiction entre le code et un de ces documents, c'est un bug :
signaler, ne pas trancher seul.**

## Stack

Vérifié le 2026-09-05, interface le 2026-09-15 — [détail et sources](docs/RESEARCH-NOTES.md).

| | Version | À savoir |
|---|---|---|
| Rust | épinglé par `rust-toolchain.toml` ([ADR-0008](docs/adr/0008-chaine-outils-rust.md)) | la machine de dev est en `1.89.0`, la stable en `1.98.1` |
| Édition | **2024** | exige Rust ≥ 1.85 |
| Interface | **Tauri 2** + TanStack Start en **mode SPA** + shadcn/ui sur **Base UI** ([ADR-0029](docs/adr/0029-interface-tauri-shadcn.md)) | `apps/desktop` (pnpm) servi par `crates/oxyn-desktop` ; aucun serveur, le backend est Rust |
| Tests de composants | Storybook 10 + `addon-vitest` + `addon-a11y` | une story est un test, axe compris ; Vitest reste en **4** |
| Résultats | Apache Arrow ([ADR-0002](docs/adr/0002-arrow-result-model.md)) | `RecordBatch` de bout en bout |

Aucune version ne s'écrit de mémoire : voir [I-12](#i-12).

## Commandes

| | |
|---|---|
| `make qualite` | **la** porte de qualité : socle, TODO datés, front (format, lint, types, stories, build), format, clippy, tests, doc. Rien n'est terminé sans elle, et la CI n'appelle qu'elle |
| `make desktop-dev` | l'application Tauri avec rechargement à chaud, sur un workspace temporaire |
| `script/nouvelle-crate` | crée une crate déjà conforme — voir le piège des règles plus bas |
| [`/plan`](.claude/commands/plan.md) [`/implementer`](.claude/commands/implementer.md) [`/relire`](.claude/commands/relire.md) | le cycle courant |
| [`/driver`](.claude/commands/driver.md) [`/commande`](.claude/commands/commande.md) [`/ecran`](.claude/commands/ecran.md) [`/conformite-shadcn`](.claude/commands/conformite-shadcn.md) | les gestes qui ont un contrat à respecter |
| [`/adr`](.claude/commands/adr.md) [`/versions`](.claude/commands/versions.md) [`/benchmark`](.claude/commands/benchmark.md) [`/securite`](.claude/commands/securite.md) | les gestes rares et faciles à rater |

Liste complète : [.claude/README.md](.claude/README.md).

## Invariants

Treize interdits. Leur violation est **silencieuse** : rien n'échoue au moment de
la faute. Chacun renvoie au document qui le fonde.

<a id="i-01"></a>**I-01 — Rien ne contourne le command bus.** L'UI, un agent et un
plugin émettent des `Command` ; aucun n'appelle un driver. Un second chemin
d'exécution, une fois créé, n'est jamais audité comme le premier — et c'est
celui-là que l'IA empruntera. → [ADR-0004](docs/adr/0004-command-bus.md)

<a id="i-02"></a>**I-02 — Aucune écriture sur une connexion `production` sans
confirmation qui nomme la connexion.** Pour un `Actor::Agent`, c'est un refus,
pas une confirmation renforcée : une confirmation finit par être cliquée. Une
connexion sans environnement renseigné vaut `production`, jamais l'inverse.
→ [SECURITY](docs/SECURITY.md#marquage-des-connexions)

<a id="i-03"></a>**I-03 — Aucun secret dans un journal, une erreur affichée, un
rapport de plantage, un fichier de workspace, une invite IA ou le presse-papiers.**
Les six canaux comptent ; il suffit d'en oublier un. Corollaire vérifiable :
**aucun `#[derive(Debug)]` sur un type portant un secret** — c'est le
`tracing::debug!("{cfg:?}")` ajouté six mois plus tard qui fuit.
→ [SECURITY](docs/SECURITY.md#secrets)

<a id="i-04"></a>**I-04 — Rien ne rejoint une invite IA hors du point de passage
unique qui applique le niveau de la connexion.** Le niveau est attaché à la
connexion, pas à la session ni au fournisseur : sinon un réglage pris sur une
base de test s'applique à la base client ouverte trois jours plus tard.
→ [AI-PROVIDERS](docs/AI-PROVIDERS.md) · [ADR-0006](docs/adr/0006-ai-privacy-tiers.md)

<a id="i-05"></a>**I-05 — Aucun I/O, aucun réseau, aucun `block_on` sur le thread
UI.** Une requête de 30 s y fige toute la fenêtre ; l'utilisateur conclut au
plantage et tue le processus, perdant son travail non sauvegardé.
→ [ARCHITECTURE](docs/ARCHITECTURE.md#le-modèle-de-threads)

<a id="i-06"></a>**I-06 — Aucun résultat n'est matérialisé en entier.**
`RecordBatch` en flux, `ResultBuffer` borné, débordement sur disque. Un
`SELECT *` sur 50 millions de lignes déclenche l'OOM killer : sur macOS le
processus meurt sans trace, après un simple clic sur une table.
→ [ADR-0002](docs/adr/0002-arrow-result-model.md) · [PERFORMANCE](docs/PERFORMANCE.md#budgets-de-mémoire)

<a id="i-07"></a>**I-07 — Aucune sortie de modèle n'est exécutée directement.**
Elle devient une `Command` portant `Actor::Agent` et traverse le `PolicyGate`.
Y compris ce qui « ne fait que lire » : `EXPLAIN ANALYZE` exécute réellement la
requête qu'il analyse, `DELETE` compris.
→ [AI-PROVIDERS](docs/AI-PROVIDERS.md#ce-quon-fait-des-réponses)

<a id="i-08"></a>**I-08 — Aucune crate hors `oxyn-desktop` ne dépend de `tauri`.**
Un type de toolkit importé dans `oxyn-core` « juste pour un champ » supprime
définitivement la possibilité d'une CLI, des tests sans écran, et d'un changement
d'interface — celui que [ADR-0029](docs/adr/0029-interface-tauri-shadcn.md) a
fait en retirant GPUI sans toucher au cœur.
→ [ARCHITECTURE](docs/ARCHITECTURE.md#le-sens-des-dépendances)

<a id="i-09"></a>**I-09 — Aucun `unwrap`, `expect`, `panic!`, `unreachable!`,
indexation de tranche ni `as` débordant sur un chemin atteignable depuis une
réponse serveur.** Un serveur renvoie ce qu'il veut : un type inconnu, un `NULL`
là où le schéma l'interdit, un encodage invalide. La panique tue l'application.
→ [DRIVER-CONTRACT](docs/DRIVER-CONTRACT.md#1-il-ne-panique-jamais-sur-une-entrée-venue-du-serveur)

<a id="i-10"></a>**I-10 — Le SQL qu'Oxyn compose ne concatène jamais un
identifiant reçu.** Citation par le driver, valeurs liées. Le SQL que
*l'utilisateur écrit* part tel quel — c'est la fonctionnalité. Une table nommée
`"users"; DROP TABLE audit; --` est légale dans PostgreSQL : un aperçu construit
par concaténation exécute la suppression au clic.
→ [DRIVER-CONTRACT](docs/DRIVER-CONTRACT.md#6-il-échappe-tout-identifiant-quil-compose)

<a id="i-11"></a>**I-11 — Aucun format de persistance fermé.** Ce qu'Oxyn écrit
— workspace, session, export — est lisible sans Oxyn. « Open by default » n'est
pas une posture : un utilisateur qui ne peut pas récupérer son travail sans le
produit est captif.
→ [VISION](docs/VISION.md)

<a id="i-12"></a>**I-12 — Aucune version, aucune limite externe recopiée de
mémoire.** Vérifiée au registre, datée dans
[RESEARCH-NOTES](docs/RESEARCH-NOTES.md), re-vérifiée par [`/versions`](.claude/commands/versions.md).
Une valeur plausible et fausse ne se voit ni à la compilation, ni aux tests, ni
en revue.

<a id="i-13"></a>**I-13 — Une erreur ambiguë ne se retente jamais.** Un délai
dépassé côté client pendant une écriture n'est pas une erreur transitoire : le
serveur a peut-être appliqué. Rejouer crée un doublon dans les données de
l'utilisateur, sans message d'erreur nulle part.
→ [DRIVER-CONTRACT](docs/DRIVER-CONTRACT.md#4-il-distingue-trois-familles-derreurs-et-il-les-classe)

## Organisation du code

L'arborescence et le sens des dépendances font autorité dans
[ARCHITECTURE](docs/ARCHITECTURE.md#le-découpage). Ce qui vaut partout :

- **une crate porte un sujet.** `utils`, `common`, `helpers`, `misc` sont
  interdits : un nom fourre-tout est un découpage raté qui devient le point de
  couplage universel ;
- **un driver par protocole, pas par produit** — Redshift ≡ PostgreSQL
  ([ADR-0003](docs/adr/0003-driver-capabilities.md)) ;
- **pas de code mort, pas de code commenté « au cas où »** : git s'en souvient ;
- **pas de `TODO` sans date** ni sans nom de ce qui le débloque ;
- **pas d'abstraction pour un seul appelant** — un trait à une seule
  implémentation qui n'est pas une frontière est une indirection, pas un
  découplage ;
- **un commentaire dit *pourquoi*.** Ce que fait le code, le code le dit ;
- **seuil de vigilance** : au-delà d'environ 400 lignes, un fichier porte
  probablement deux sujets. C'est un signal à regarder, pas une règle mécanique.

## Règles chargées à la demande

`.claude/rules/*.md` porte les conventions d'un répertoire. Elles se chargent
quand Claude **lit** un fichier correspondant à leur `paths:`.

| Règle | `paths:` |
|---|---|
| [rust.md](.claude/rules/rust.md) | `**/*.rs` |
| [drivers.md](.claude/rules/drivers.md) | `drivers/oxyn-driver-*/**`, `crates/oxyn-driver/**` |
| [front.md](.claude/rules/front.md) | `apps/desktop/**`, `crates/oxyn-desktop/**` |
| [ia.md](.claude/rules/ia.md) | `crates/oxyn-ai/**`, `crates/oxyn-llm/**` |
| [tests.md](.claude/rules/tests.md) | `**/*_tests.rs`, `**/tests.rs`, `**/tests/**`, `**/benches/**` |
| [documentation.md](.claude/rules/documentation.md) | `docs/**/*.md`, `*.md`, `.claude/**/*.md` |
| [manifestes.md](.claude/rules/manifestes.md) | `**/Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `Makefile`, `deny.toml`, `clippy.toml`, `.cargo/config.toml`, `.config/nextest.toml`, `renovate.json5`, `script/*`, workflows |

> **Le piège à connaître.** Une règle `paths:` se charge quand Claude *lit* un
> fichier correspondant, **pas quand il en crée un**. Le premier fichier d'un
> répertoire neuf s'écrit donc sans sa règle, et un manifeste sans
> `[lints] workspace = true` ne fait rien échouer. Deux choses compensent, et
> aucune n'est une option de confort : `script/nouvelle-crate`, qui écrit un
> squelette déjà conforme, et les commandes de `.claude/commands/`, qui
> chargent la procédure explicitement — `/driver`, `/commande`, `/ecran`.

## Ce qui est exécuté

`.claude/hooks/` **refuse** ce que ce fichier ne peut que demander : écritures
interdites, contournements par le shell, format de commit, injection de l'état
réel au démarrage. Un hook n'est pas un rappel, c'est un mur. Détail et
protocole : [.claude/hooks/README.md](.claude/hooks/README.md).

Les hooks ne valent que pour une session Claude. Ce qui vaut pour **tout le
monde**, humain compris, passe par `make qualite` — appelée par la CI
([.github/workflows/qualite.yml](.github/workflows/qualite.yml)), qui n'ajoute
aucun contrôle de son côté :

| Ce qui refuse | L'invariant tenu |
|---|---|
| [clippy.toml](clippy.toml) | les chemins d'appel interdits ([I-03](#i-03), [I-05](#i-05), [I-09](#i-09)) |
| `.claude/verifier_socle.py` | `tauri*` hors d'`oxyn-desktop`, `gpui` où que ce soit, un manifeste hors du workspace ([I-08](#i-08)) |
| `make front` | un composant sans story passante, une violation d'accessibilité, un type ou un lint faux dans `apps/desktop` |
| `script/verifier-todo` | une marque de travail restant sans échéance |
| [renovate.json5](renovate.json5) | une version recopiée de mémoire ([I-12](#i-12)) |
