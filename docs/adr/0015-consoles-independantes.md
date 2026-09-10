# ADR-0015 — Donner à chaque console son contrôleur et sa session

**Statut :** accepté · **Date :** 2026-09-10

**Précise :** [ADR-0014](0014-documents-et-historique.md), pour les contextes
ouverts depuis les onglets et la bibliothèque Figma.

## Contexte

Le workspace possède actuellement un seul éditeur, une seule exécution et un
seul résultat de console. Ses callbacks écrivent directement dans ces champs.
Changer seulement le texte affiché ou permuter ces champs à chaque changement
d'onglet ferait arriver les réponses d'un onglet dans un autre. Réutiliser la
même session pour toutes les consoles partagerait aussi leurs transactions.

## Décision

- `QueryConsole`, dans `oxyn-app`, possède les entités d'édition, grille, statut,
  confirmation et export, ainsi que leurs commandes et jetons d'annulation.
  Ses callbacks restent attachés à cette entité, même lorsqu'elle est masquée.
- `Workspace` garde les consoles et sélectionne leurs vues ; les références
  d'entités utilisées pour dessiner sont des références au contrôleur choisi,
  jamais des copies de l'état d'une exécution. Catalogue, aperçu de table,
  bibliothèque et préférences restent au niveau du workspace.
- Une nouvelle console établit une session par le bus avant d'accepter une
  exécution. Elle conserve la même configuration de connexion, mais reçoit un
  véritable identifiant de session. Le geste est annulable et ne lance pas de
  SQL utilisateur. Les sessions créées pour les consoles sont fermées lorsqu'on
  les ferme ; la session initiale reste disponible au catalogue et à l'aperçu.
- Un changement d'onglet ne relance, n'annule et ne remplace aucune requête.
  L'inspection de valeur attachée à l'onglet quitté est fermée et annulée.
  Une confirmation en attente sur un onglet masqué reste attachée à cet onglet.
- Fermer une console portant un brouillon ou une opération requiert un choix
  explicite. Les écritures ambiguës n'ont pas d'action de rejeu implicite.

## Conséquences

- **+** Résultats, annulations, exports et confirmations restent corrélés à leur
  console, indépendamment du focus.
- **+** Deux nouvelles consoles ne partagent pas leur transaction serveur.
- **−** Chaque console supplémentaire consomme une session et ses ressources.
- **−** La fermeture doit coordonner brouillon, opérations et libération de
  session ; supprimer seulement une vue ne suffit pas.

**Coût de sortie :** réunir les contrôleurs de l'interface et redéfinir les
contrats de fermeture et de session, sans changer le format Arrow ni les drivers.

**Reconsidérer si** un pilote impose une session unique, ou si un mode explicite
de transaction partagée devient nécessaire. Ce mode devra être visible et ne
pourra pas être déduit du seul changement d'onglet.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Permuter le contenu d'un éditeur et d'une grille uniques | Réponses asynchrones rattachées au mauvais onglet |
| Dupliquer tout le workspace par console | Duplique catalogue, préférences et bibliothèque, qui appartiennent à la fenêtre |
| Partager silencieusement une session entre consoles | Une validation ou annulation de transaction affecte un autre onglet |
