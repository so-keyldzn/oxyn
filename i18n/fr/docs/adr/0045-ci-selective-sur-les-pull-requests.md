<!-- oxyn-translation source="docs/adr/0045-ci-selective-sur-les-pull-requests.md" sha256="60244706b272" -->

> Traduction française de [docs/adr/0045-ci-selective-sur-les-pull-requests.md](../../../../docs/adr/0045-ci-selective-sur-les-pull-requests.md). **La version anglaise fait foi.**

# ADR-0045 — Sur une pull request, la CI saute les jobs dont la zone n'est pas touchée ; sur `main`, tout tourne

**Statut :** accepté · **Date :** 2026-09-25

**Précise :** la règle de [CLAUDE.md](../../CLAUDE.md#ce-qui-est-exécuté) selon
laquelle la CI appelle `make qualite` en jobs parallèles, sans rien ajouter, et
dont `make socle` vérifie qu'elle n'oublie aucune cible. Cette règle reste
vraie ; elle cesse de dire que **chaque** poussée exécute **toutes** les cibles.

## Contexte

Chaque pull request lance aujourd'hui tous les jobs de
[qualite.yml](../../../../.github/workflows/qualite.yml) : `controles`, deux tranches de
`stories`, et `rust`, dont le délai est borné à 75 minutes. Une PR qui ne touche
qu'un document recompile le workspace ; une PR qui ne touche que le front
recompile tout le Rust, et une PR Rust rejoue les stories dans Chromium.

Deux coûts en découlent, constatés le 2026-09-24 et le 2026-09-25 :

* le quota de minutes du compte a été épuisé le 2026-09-24 (la matrice macOS a
  été retirée des PR pour cette raison, voir le commentaire du workflow) ;
* les agents qui travaillent en parallèle attendent le job `rust` pour un
  changement qui n'y touche pas, et le flux des PR s'en trouve ralenti.

L'utilisateur l'a tranché le 2026-09-25 : alléger la machine locale et
accélérer le flux des agents, **sans sur-ingénierie**.

## Décision

**Sur l'événement `pull_request` seulement**, un premier job `zones` exécute
[`script/zones-ci`](../../../../script/zones-ci), qui compare le commit de fusion de
la PR à son premier parent (`git diff --name-only HEAD^1 HEAD`, `fetch-depth: 2`)
et range chaque fichier dans une zone :

| Fichier | Zone |
|---|---|
| `Makefile`, `.github/`, `Cargo.toml` et `Cargo.lock` à la racine, `.cargo/`, `.config/`, `rust-toolchain.toml`, `clippy.toml`, `deny.toml`, `renovate.json5`, `.claude/`, `.agents/`, `script/` | **transversal** : toutes les zones |
| `apps/desktop/` | `front` |
| `crates/`, `drivers/` | `rust` |
| `docs/`, tout autre `*.md` | `docs` |
| tout autre fichier | **transversal** : un fichier que le script ne sait pas ranger déclenche tout |

Le script n'utilise aucune action externe : `git` et Python, déjà présents sur
le runner.

Les jobs en dépendent ainsi :

| Job | Tourne sur une PR si | Ce qu'il saute sinon |
|---|---|---|
| `controles` | toujours | `make front-controles` et l'installation de Node, si `front` n'est pas touchée ; `make socle todo` tourne toujours |
| `stories` | `front` | tout le job |
| `rust` | `rust` ou `front` | `make rust` et son outillage si seule `front` est touchée ; `make front-build` tourne dès que l'une des deux l'est |
| `serveurs` | `rust` | tout le job, y compris `make test-serveurs` contre des services PostgreSQL, PostgreSQL TLS et MySQL jetables |

`rust` tourne pour une PR front parce que c'est lui qui porte `front-build` :
`oxyn-desktop` embarque `apps/desktop/dist` à la compilation, et le build du
front n'a pas d'autre job. La zone `docs` ne commande aucun job : `controles`,
qui vérifie les liens et l'index des ADR, tourne toujours. Elle est affichée
dans le journal du job `zones`, pour qu'on lise ce qui a été décidé.

`serveurs` tourne à côté de `rust` quand Rust change. Il utilise la même cible
de gate, avec des services de base de données jetables, pour exécuter les
suites ignorées qui ne peuvent pas tourner sans serveur.

Un job final **`qualite`** dépend de tous les autres (`if: always()`), et
réussit si et seulement si `zones` a réussi et qu'aucun autre job n'a échoué ni
été annulé — un job `skipped` compte comme réussi. **C'est ce job, et lui seul,
que la protection de branche exigera** : un status check requis sur un job sauté
resterait en attente pour toujours. Le 2026-09-25, la protection de branche est
encore refusée à ce dépôt privé
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-et-livraison-github)) : d'ici là,
`qualite` est le seul check à lire avant de fusionner.

**Amendement (2026-10-03).** Le dépôt est désormais public. Le ruleset GitHub
[`24429281`](https://github.com/so-keyldzn/oxyn/rules/24429281) exige une pull
request (zéro approbation obligatoire) et le status check agrégé `qualite` de
l’application GitHub Actions `15368` sur `main` ; il interdit aussi la suppression
et les mises à jour non-fast-forward. Les administrateurs du dépôt peuvent
contourner ces règles uniquement lors de la fusion d’une pull request
(`bypass_mode: pull_request`) ; les push directs vers `main` sont refusés à tout
le monde, y compris aux administrateurs.

**Amendement (2026-10-04).** À la demande du mainteneur, le contournement
administrateur passe à `bypass_mode: always` : un administrateur peut pousser
directement sur `main`, si bien qu’un commit dont la CI est rouge peut y arriver
par ce chemin. Tous les autres ont toujours besoin d’une pull request avec un
`qualite` vert, et le run complet sur `main` reste le filet de sécurité.

**Amendement (2026-10-20).** Le dépôt est public (`private: false`,
`visibility: public` d’après `gh api repos/so-keyldzn/oxyn`), et la
documentation de GitHub l’énonce : « Use of the standard GitHub-hosted runners
is free and unlimited on public repositories. »
([référence des runners hébergés par GitHub](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
lue le 2026-10-20, consignée dans
[RESEARCH-NOTES](../RESEARCH-NOTES.md#suivi-de-laudit--vérifié-le-2026-10-20)).
Le quota épuisé le 2026-09-24, quand le dépôt était privé, est ce qui avait
retiré la matrice macOS des pull requests (Contexte ci-dessus) : il ne
s’applique plus. Les jobs `controles`, `stories` et `rust` exécutent donc de
nouveau leur branche `macos-latest` sur les pull requests, comme sur `main`.
Cela remplace la phrase plus bas, « la matrice macOS s'y ajoute comme avant » :
macOS n’est plus quelque chose que seul `main` ajoute.

Rien d’autre ne change. Le job `zones` choisit toujours les zones, un job sauté
compte toujours comme réussi, et le job agrégat `qualite` reste le seul check
que le ruleset exige. Les branches macOS ne sont pas des checks requis : leur
retour ne renomme aucun check requis. `workflow_dispatch` demeure, pour exécuter
toute la porte sur une branche qui n’a pas de pull request ouverte.

**Sur `push` vers `main` et sur `workflow_dispatch`, tout tourne toujours.**
`script/zones-ci` renvoie toutes les zones hors de l'événement `pull_request`,
et la matrice macOS s'y ajoute comme avant. C'est le filet : ce qu'une PR aurait
sauté à tort est rattrapé à la fusion.

**Amendement (2026-10-21).** À la demande du mainteneur, deux restrictions pour
la documentation, constatées le 2026-10-21 : fusionner une PR de documentation
lançait toute la matrice sur `main` (`rust` à lui seul a pris 18 minutes sur
macOS et 11 sur Linux), et modifier la mémoire d'un agent sous `.claude/`
lançait `rust` et les stories sur sa PR.

* **Sur `push` vers `main`**, un filtre `paths` empêche le workflow de démarrer
  quand chaque fichier modifié est sous `docs/` ou est un fichier `*.md` hors de
  `apps/`, `crates/` et `drivers/`. Le Markdown sous ces trois répertoires
  déclenche toujours : les prompts d'agents d'`oxyn-ai` y sont compilés par
  `include_str!`. Un commit qui mêle documentation et autre chose fait tout
  tourner, comme avant. Le filtre reste hors de `pull_request` : un workflow qui
  ne démarre pas laisse le check requis `qualite` en attente pour toujours.
* **Dans `script/zones-ci`**, un fichier `*.md` est rangé en `docs` avant la
  règle transversale : un fichier Markdown de `.claude/`, `.agents/` ou
  `.github/` est lu, jamais compilé ni exécuté. Les répertoires de code gardent
  la priorité sur la règle `.md`, pour la même raison que ci-dessus.

Ce que cela coûte : sur `main`, un commit de documentation seule ne rejoue plus
`make socle`. Il a passé `controles` sur sa PR ; le cas laissé à découvert est
celui de deux PR fusionnées l'une après l'autre, l'une supprimant un fichier
vers lequel pointe la documentation de l'autre. Le commit suivant qui fait
tourner le workflow sur `main` le signale.

`make socle` continue de vérifier que la réunion des `run: make …` du workflow
couvre `make qualite`. Il vérifie en plus que tout job qui appelle `make` figure
dans les `needs` du job `qualite` : sans quoi son échec ne bloquerait pas la
fusion.

En local, [`make verif-rapide`](../../../../Makefile) applique la même idée au
travail courant — ne vérifier que ce qui a changé depuis `origin/main` — sans
changer ce qui fait foi : **`make qualite` reste la porte**.

**Amendement (2026-10-22).** `make test-serveurs` est une cible de `make qualite`
et tourne dans le job Linux `serveurs` quand la zone Rust est touchée. En CI,
l'absence du binaire nextest, une URL de serveur absente ou une suite dépendante
d'une URL sautée font échouer la cible. Hors CI, quand aucune URL de serveur n'est
configurée, la cible affiche explicitement `server suites NOT run` et réussit ;
configurer une URL de serveur est une exécution locale volontaire des suites
serveur et conserve la sémantique d'échec de la CI.

## Conséquences

* **+** une PR de documentation ne lance ni Rust ni stories ; une PR front ne
  lance ni clippy ni les tests Rust ; une PR Rust ne lance pas les stories.
* **+** les status checks requis se réduisent à un seul nom, `qualite`, stable
  quand la matrice ou le découpage des jobs change.
* **+** la règle « la CI n'ajoute aucun contrôle » tient : chaque job appelle
  encore des cibles de `make qualite`, et `make socle` le vérifie toujours.
* **−** une PR peut être verte alors que `make qualite` échouerait sur la même
  arborescence : une PR Rust qui casserait une story n'est pas vue avant `main`.
  Le cas existe — une commande IPC renommée côté Rust sans que le front suive —
  et c'est `main` qui devient rouge, après la fusion.
* **−** le rangement des fichiers est une deuxième description du dépôt, à tenir
  à jour quand un répertoire apparaît. Le repli (« inconnu ⇒ tout ») borne le
  risque : un oubli coûte des minutes, pas un contrôle.
* **−** le jour où la protection de branche devient disponible, elle doit
  exiger `qualite` et aucun autre job : exiger `rust` bloquerait toute PR qui
  ne touche pas au Rust, sur un check qui ne viendra pas.

**Coût de sortie :** faible. Retirer le job `zones`, les `if:` qui le lisent et
le job `qualite` rend le workflow antérieur ; `script/zones-ci` et son test se
suppriment. La protection de branche peut continuer d'exiger `qualite`.

**Reconsidérer si** `main` devient rouge après la fusion d'une PR verte plus
d'une fois par mois à cause d'un job sauté — le rangement est alors trop
optimiste, et il faut soit élargir une zone, soit revenir à tout exécuter.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| `paths` / `paths-ignore` sur le déclencheur `pull_request` | filtre le workflow entier, pas un job ; un workflow non déclenché laisse un status check requis en attente pour toujours |
| Une action tierce de filtrage par chemins | une dépendance externe de plus dans un workflow qui en épingle déjà cinq par empreinte, pour un calcul de trente lignes |
| Calculer les crates dépendantes et ne tester qu'elles en CI | le graphe se calcule mal sans `cargo metadata`, et la CI est justement l'endroit où le workspace entier doit passer ; le gain réel est dans les jobs entiers sautés |
| Filtrer aussi sur `main` | supprime le seul endroit où tout est vérifié ensemble ; la moindre erreur de rangement deviendrait invisible. Restreint le 2026-10-21 aux commits de documentation seule, qui ne compilent rien (amendement ci-dessus) |
| Laisser la CI telle quelle et n'alléger que le local | ne répond pas à l'épuisement du quota, ni à l'attente des agents sur le job `rust` |
