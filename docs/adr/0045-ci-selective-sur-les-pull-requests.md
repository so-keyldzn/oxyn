# ADR-0045 — Sur une pull request, la CI saute les jobs dont la zone n'est pas touchée ; sur `main`, tout tourne

**Statut :** accepté · **Date :** 2026-09-25

**Précise :** la règle de [CLAUDE.md](../../CLAUDE.md#ce-qui-est-exécuté) selon
laquelle la CI appelle `make qualite` en jobs parallèles, sans rien ajouter, et
dont `make socle` vérifie qu'elle n'oublie aucune cible. Cette règle reste
vraie ; elle cesse de dire que **chaque** poussée exécute **toutes** les cibles.

## Contexte

Chaque pull request lance aujourd'hui tous les jobs de
[qualite.yml](../../.github/workflows/qualite.yml) : `controles`, deux tranches de
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
[`script/zones-ci`](../../script/zones-ci), qui compare le commit de fusion de
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

`rust` tourne pour une PR front parce que c'est lui qui porte `front-build` :
`oxyn-desktop` embarque `apps/desktop/dist` à la compilation, et le build du
front n'a pas d'autre job. La zone `docs` ne commande aucun job : `controles`,
qui vérifie les liens et l'index des ADR, tourne toujours. Elle est affichée
dans le journal du job `zones`, pour qu'on lise ce qui a été décidé.

Un job final **`qualite`** dépend de tous les autres (`if: always()`), et
réussit si et seulement si `zones` a réussi et qu'aucun autre job n'a échoué ni
été annulé — un job `skipped` compte comme réussi. **C'est ce job, et lui seul,
que la protection de branche exigera** : un status check requis sur un job sauté
resterait en attente pour toujours. Le 2026-09-25, la protection de branche est
encore refusée à ce dépôt privé
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-et-livraison-github)) : d'ici là,
`qualite` est le seul check à lire avant de fusionner.

**Sur `push` vers `main` et sur `workflow_dispatch`, tout tourne toujours.**
`script/zones-ci` renvoie toutes les zones hors de l'événement `pull_request`,
et la matrice macOS s'y ajoute comme avant. C'est le filet : ce qu'une PR aurait
sauté à tort est rattrapé à la fusion.

`make socle` continue de vérifier que la réunion des `run: make …` du workflow
couvre `make qualite`. Il vérifie en plus que tout job qui appelle `make` figure
dans les `needs` du job `qualite` : sans quoi son échec ne bloquerait pas la
fusion.

En local, [`make verif-rapide`](../../Makefile) applique la même idée au
travail courant — ne vérifier que ce qui a changé depuis `origin/main` — sans
changer ce qui fait foi : **`make qualite` reste la porte**.

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
| Filtrer aussi sur `main` | supprime le seul endroit où tout est vérifié ensemble ; la moindre erreur de rangement deviendrait invisible |
| Laisser la CI telle quelle et n'alléger que le local | ne répond pas à l'épuisement du quota, ni à l'attente des agents sur le job `rust` |
