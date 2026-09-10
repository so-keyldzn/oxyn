# ADR-0013 — Persister les préférences de lecture dans le workspace

**Statut :** proposé · **Date :** 2026-09-10

**Précise :** [ADR-0004](0004-command-bus.md), pour les réglages locaux.

## Contexte

La maquette propose Compact et Comfortable, indépendamment du thème et de la
largeur. Les réglages de lecture existants disparaissent à la fermeture.
Le store local possède un schéma SQLite migré, tandis que les types GPUI
doivent rester dans l'interface. Les changements rapides ne doivent pas être
réécrits dans le désordre par des tâches asynchrones.

## Décision

- `oxyn-core` porte `WorkspacePreferences`, `ReadingDensity` et
  `PreferencesSnapshot`, sans type GPUI. Le format JSON version 1 contient
  `appearance`, `reading_density`, `sidebar_collapsed`, `inspector_open`,
  `inspector_width`, `null_text` et `group_thousands`.
- La migration SQLite 4 ajoute `workspace_preferences`, liée au workspace,
  avec `revision`, `payload` JSON et `updated_at`. Les migrations précédentes
  restent inchangées. Une ligne absente donne les valeurs par défaut.
- Les lectures et écritures passent par `ReadWorkspacePreferences` et
  `WriteWorkspacePreferences`. L'amorçage lit une fois le snapshot hors thread
  UI. Ces commandes ne contactent aucune base distante. L'écriture des réglages
  d'interface est réservée à `Actor::Human` ; un agent ne change pas l'interface
  de son utilisateur. Le refus passe par le `PolicyGate` et le journal.
- La révision augmente à chaque changement local. Une écriture plus ancienne
  ne remplace jamais une révision plus récente. Une même révision avec un autre
  contenu signale un conflit, sans écraser le fichier. L'interface distingue
  l'application locale de la sauvegarde confirmée et propose une reprise
  explicite en cas d'échec. La fermeture de la dernière fenêtre attend les
  écritures engagées avant de demander l'arrêt. Le hook natif de GPUI reçoit
  aussi cette attente, mais son délai propre de 100 ms ne suffit pas à garantir
  tous les chemins d'arrêt du système ; ces chemins demandent leur recette.
- Les données sont validées avant écriture et après lecture : version connue,
  révision dans le domaine SQLite positif, libellé d'absence limité à 64 octets,
  largeur d'inspecteur comprise entre 240 et 480 px. Ces deux bornes de largeur
  sont des garde-fous d'implémentation ; la largeur initiale de 280 px vient de
  Figma, pas les bornes.
- Les panneaux conservent leur préférence large lors d'un passage compact.
  Leur adaptation au viewport ne produit aucune écriture de préférence ni
  aucune requête. La poignée de l'inspecteur est utilisable à la souris et au
  clavier ; elle sauvegarde la largeur choisie après le geste.

## Conséquences

- **+** Les mêmes réglages sont repris après redémarrage et changement de connexion.
- **+** Le format reste lisible avec SQLite et un lecteur JSON ordinaire.
- **+** Une tâche lente ne peut pas écraser un réglage plus récent.
- **−** Une seconde instance ayant lu la même révision peut provoquer un conflit
  explicite ; l'interface doit permettre de sauvegarder de nouveau après relecture.
- **−** La sauvegarde locale a ses propres états d'erreur et son attente de fin.

**Coût de sortie :** migrer une petite table de réglages et remplacer les deux
commandes ; les connexions, secrets et données de résultats sont indépendants.

**Reconsidérer si** les préférences deviennent propres à un document, si plusieurs
fenêtres demandent des réglages divergents, ou si une synchronisation entre
machines nécessite une fusion de champs plutôt qu'une révision de snapshot.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Sérialiser `Theme` | Introduit des types GPUI et des couleurs calculées dans le domaine |
| Écrire un fichier depuis la vue | Bloque le rendu et contourne le bus |
| Sauvegarder uniquement à la fermeture | Perd les réglages lors d'un arrêt anormal |
| Accepter toutes les écritures dans leur ordre d'arrivée | Une réponse lente peut rétablir une préférence antérieure |
