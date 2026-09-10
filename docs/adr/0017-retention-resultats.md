# ADR-0017 — Borner les résultats conservés qui n'ont plus de lecteur

**Statut :** accepté · **Date :** 2026-09-10

## Contexte

La bibliothèque peut rouvrir un `ResultId` sans exécuter de SQL, mais le registre
de l'exécuteur conservait tous les tampons jusqu'à la fin du processus. La borne
individuelle de 256 Mio n'empêche pas une accumulation de nombreux résultats.

## Décision

Le registre conserve au plus 16 résultats sans lecteur, avec des plafonds cumulés
de 256 Mio de lots résidents/cache décodé et 1 Gio de débordement IPC pour cette
catégorie. Les plus anciens sont évincés en premier. Une référence détenue par
une vue, une exécution ou un export protège son tampon de cette éviction.

Le contrôle intervient après exécution et périodiquement dans le backend. Les
objets retirés sont détruits hors du verrou du registre et hors du thread UI.
Les résultats consultés restent protégés ; les budgets ne sont donc pas un
plafond global de mémoire pour toutes les vues ouvertes. L'index et les
allocations temporaires restent à mesurer selon PERFORMANCE.

Une référence d'historique évincée reste visible mais devient indisponible.
Elle n'est jamais recréée par rejeu de SQL. Fermer sa vue ne change pas les
données du serveur.

## Conséquences

- **+** Une longue session de requêtes successives ne conserve pas tous ses
  résultats inutilisés.
- **+** Un export ou une vue ouverte garde ses données et ses pages locales.
- **−** Un ancien résultat peut expirer avant la fermeture de l'application.
- **−** Les résultats tenus par des vues restent à la charge de leurs lecteurs.

**Coût de sortie :** changer le registre et ses points de contrôle, sans changer
les buffers Arrow, le SQL ni la persistance des références d'historique.

**Reconsidérer si** une rétention sur disque entre redémarrages est demandée,
ou si des mesures imposent un budget global partagé entre vues actives.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Garder tous les résultats | Croissance sans borne du registre |
| Évincer une vue encore ouverte | Ses lectures de pages et exports deviennent indisponibles |
| Relancer une requête pour recréer le résultat | Change les données observées et peut répéter une écriture |
