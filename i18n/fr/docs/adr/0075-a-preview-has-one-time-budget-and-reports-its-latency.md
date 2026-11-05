<!-- oxyn-translation source="docs/adr/0075-a-preview-has-one-time-budget-and-reports-its-latency.md" sha256="588ab5d83575" -->

> Traduction française de [docs/adr/0075-a-preview-has-one-time-budget-and-reports-its-latency.md](../../../../docs/adr/0075-a-preview-has-one-time-budget-and-reports-its-latency.md). **La version anglaise fait foi.**

# ADR-0075 — Un aperçu a un seul budget de temps pour se préparer et s'exécuter, et rapporte la latence que l'utilisateur a attendue

**Statut :** accepté · **Date :** 2026-11-05

## Contexte

Un rapport QA interne (2026-10-31) a observé trois défauts liés sur les aperçus,
qui partagent une session par connexion :

1. le délai de 30 s d'une exécution partait de l'appel du driver
   (`ExecLimits::timeout`, armé dans `run_statement`), mais un aperçu commence
   par **préparer** sa requête — attendre la session, décrire la relation — et
   cela se passait avant que le délai existe : une préparation bloquée attendait
   sans borne, et une préparation de 20 s était suivie de 30 s d'exécution
   complètes (50 s au total, reproduit sur une horloge en pause) ;
2. le pied de page affichait `ExecStats::total_time`, que le driver mesure à
   partir de l'envoi de l'instruction : après une longue attente de la session,
   une lecture affichait `0 ms` ;
3. une lecture automatique devenue invisible gardait la session occupée (côté
   interface, ticket #357 ; pas de décision ici).

## Décision

1. **Un budget, une horloge.** L'horloge d'une commande part quand l'exécuteur la
   commence (`started`). Un aperçu tire sa préparation et son exécution sur la
   même échéance : `ExecLimits::DEFAULT_TIMEOUT` comptée depuis `started`. Une
   préparation de 20 s en laisse 10. Les 30 s ne sont pas relevées, et le délai
   propre de l'exécution est plafonné à ce budget (`min`), jamais prolongé.
2. **La préparation est annulée au budget**, par son jeton, et échoue avec
   `OxynError::Timeout`, comme une exécution.
3. **Limite connue.** Le budget borne le *travail*, pas le *nettoyage* : après
   l'annulation, l'exécuteur attend que le driver lâche prise, comme pour
   `execute_within`, car abandonner le futur laisserait l'instruction ou la
   session occupée (DRIVER-CONTRACT §2). Un driver qui ne répond plus à son
   annulation fait attendre l'appel au-delà du budget.
4. **Deux durées, jamais mélangées.** `ExecStats::total_time` reste la durée SQL.
   Un nouveau `ExecTimings { prepare, start, stream, cleanup, total }`
   (`oxyn-core`), porté par `Outcome::Executed`, dit où est passé le temps de la
   commande ; `total` est ce que l'utilisateur a attendu. Le délai jusqu'à la
   première ligne n'est pas une phase à part : un driver qui calcule le premier
   lot dans `execute` le compte dans `start`, les autres dans `stream`.
5. **Là où l'utilisateur le ressent.** `CommandOutcome::Executed` porte
   `latencyMs` (= `timings.total`) à côté de `elapsedMs` (= durée SQL). Le pied
   de page de l'aperçu affiche `latencyMs` ; les consoles gardent `elapsedMs`
   tant que leur propre rapport ne demande pas autre chose. Les phases sont
   journalisées au niveau `debug`, sans aucun texte d'instruction (I-03).

## Conséquences

- Un aperçu se termine dans les 30 s de son départ, ou seul le nettoyage reste
  à attendre.
- Ajouter un champ à `Outcome::Executed` et au miroir IPC
  (`src/lib/ipc/types.ts`) change les deux côtés dans le même commit.

## À reconsidérer si

- l'annulation d'une préparation par un driver survit au budget en pratique : le
  nettoyage aurait alors besoin d'une borne propre, comme `WRITE_SETTLE_GRACE`
  borne une écriture arrêtée.
