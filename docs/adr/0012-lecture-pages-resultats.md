# ADR-0012 — Lire les pages de résultats hors du rendu et borner leur cache en octets

**Statut :** proposé · **Date :** 2026-09-10

**Précise :** [ADR-0002](0002-arrow-result-model.md), sur la relecture et son
budget ; [ADR-0004](0004-command-bus.md), sur la commande locale de lecture.

## Contexte

`DataGrid` refuse correctement les entrées-sorties au rendu, mais ne déclenche
aucune relecture des lots débordés. Ils restent dessinés sous forme de points
de suspension. `SpillCache` conserve quatre lots sans borne en octets, en plus
des lots résidents. Cette limite ne borne pas la mémoire lorsque la taille des
lots varie. Le budget décidé par PERFORMANCE reste de 256 Mio par résultat.

Le code de débordement emploie déjà des flux Arrow IPC autonomes, lus avec
`StreamReader`. Il n'utilise pas de projection mémoire. La présente décision
rend cette divergence explicite sans introduire de `unsafe`.

## Décision

- `ReadResultPage` porte connexion, résultat et numéro de lot. C'est une lecture
  locale soumise au même bus et à la même politique pour humain et agent.
  L'exécuteur vérifie que le résultat appartient à la connexion. Elle ne contacte
  pas le serveur, ne compose pas de SQL et ne réexécute rien.
- La vue émet une demande lorsqu'un lot visible manque. La lecture et le
  décodage se déroulent hors thread UI. Le retour est corrélé au résultat et à
  la génération de grille ; une réponse ancienne ne remplace pas les données
  courantes. Annulation et erreur sont visibles, sans reprise automatique après
  une erreur.
- Le budget de rétention de `ResultBuffer` comprend les lots résidents et le
  cache de relecture. Lorsque le débordement est autorisé, un quart est réservé
  au cache et trois quarts aux lots initiaux. Sans débordement, tout le budget
  reste disponible pour les lots initiaux. Le cache évince par usage et compte
  les octets Arrow et les entrées, pas seulement les lots.
- Une lecture positionnée conserve le format Arrow IPC existant. Sa copie de
  décodage et les références temporairement détenues par les lecteurs ne sont
  pas de la rétention du cache. Elles doivent être mesurées séparément ; une
  borne du cache ne prouve pas une borne RSS du processus.
- Un lot qui dépasse seul le budget de relecture n'est pas conservé dans ce
  cache. La vue reçoit une erreur explicite plutôt qu'une boucle de chargement
  ou une croissance sans plafond. L'export continue de lire les lots en flux.

## Conséquences

- **+** Les lignes débordées deviennent consultables sans accès disque au rendu.
- **+** Le budget de rétention couvre réellement les deux catégories de lots.
- **+** La commande ne transporte aucun contenu de cellule vers le journal ou
  vers le modèle ; le résultat reste dans le tampon partagé.
- **−** Les lots initiaux débordent plus tôt, au profit de la relecture.
- **−** La relecture positionnée ajoute une copie temporaire ; elle ne bénéficie
  pas de la projection mémoire initialement décrite par ADR-0002.
- **−** Un budget personnalisé inférieur à la taille d'un seul lot peut empêcher
  son affichage, même si son export reste possible.

**Coût de sortie :** remplacer le stockage de pages dans `oxyn-data` et son
chargement dans `oxyn-app`, puis reprendre les tests de mémoire et d'annulation.
Le protocole driver, les données Arrow et le SQL utilisateur ne changent pas.

**Reconsidérer si** les mesures montrent une copie dominante, une pression
mémoire excessive lors de lectures concurrentes ou une éviction qui empêche
un viewport normal de se stabiliser. Une projection mémoire éventuelle exige
sa revue `unsafe` et ne s'introduit pas comme une simple optimisation.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Lire `batch()` directement pendant le rendu | Bloque le thread UI sur le disque et le décodage |
| Relancer la requête au défilement | Change le résultat et peut répéter des effets serveur |
| Ajouter un cache UI sans partager le budget | Double la rétention et disperse son contrôle |
| Considérer quatre lots comme une borne mémoire | La taille d'un lot n'est pas constante |
