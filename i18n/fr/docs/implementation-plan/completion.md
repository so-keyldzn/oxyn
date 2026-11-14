<!-- oxyn-translation source="docs/implementation-plan/completion.md" sha256="f3aad58d9447" -->

# Suivi d'achèvement des outils de bases de données

Actualisé le 2026-11-14. Cette annexe suit la demande d'achèvement des issues
#308, #373, #374, #375 et de la carte autonome. Implémentation, validation et
acceptation personnelle restent distinctes.

## Git et dépendances

Le worktree actif unique est
`/Users/nicolasboromee/.codex/worktrees/oxyn-schema-research/oxyn`.
`dev` a avancé vers `ce58839` depuis `origin/main`. Le worktree de fonctionnalité
a ensuite avancé vers cette base et le travail existant y a été réappliqué.
La sauvegarde explicite est le stash
`7c23ae3e3d83b51f331a2cd9e84545439194654f`, antérieur à la nouvelle consigne
interdisant le transport des lots par stash. Il reste uniquement un secours.
Aucun autre stash ne transportera un lot entre branches.

L'ADR accepté de main sur les aperçus temporaires occupe 0079. Les quatre ADR
proposés de recherche ont été renumérotés 0080–0083 avec liens et miroirs.
Les prompts historiques conservent leurs identifiants de génération.

| Lot | Branche | Dépendance / état actuel |
|---|---|---|
| Recherche commune | commit documentaire initial sur #373 | Parcours, matrice, ADR proposés et maquettes sont un prérequis commun. Un commit documentaire transversal est nécessaire pour conserver ce socle sans duplication. |
| #373 | `feat/multi-base-backend` | Backend/API existants conservés ; raccordement et coordination commencés ; validation UI et compatibilité en attente. |
| #308 | `feat/f6-navigation` | Non créée ; partira de #373 terminé pour la disposition réelle. L'inscription de la carte au focus restera optionnelle. |
| #375 | `feat/metadata-loading` | Non créée ; héritera de #308/#373. Les lectures paresseuses et facettes existantes ne constituent pas encore la politique configurable. |
| Carte | `feat/schema-map` | Non créée ; dépend de couverture/politique #375 et du focus #308. L'ERD de l'assistant est un rendu réutilisable, pas une carte autonome. |
| #374 | `feat/native-backup-restore` | Non créée ; héritera de la pile terminée et réutilisera propriété, opérations et invalidation. Aucun transfert natif implémenté. |

Cette pile linéaire volontaire évite les doublons de commits. Avant chaque
bascule : commits cohérents, bilan de validation et worktree propre. Vérifier
les branches avant création. Main n'est ni cible d'écriture ni d'intégration ;
aucun push n'est autorisé.

## Inventaire initial non commité

Avant la première bascule de lot, le travail conservé comprend :

- Conception commune : annexes, ADR proposés, matrice, PNG et prompts de
  provenance, index anglais/français.
- Core/store #373 : profils, appartenance, migration 20, enfants atomiques,
  protections de destination/identifiants et tests ; aucun nouveau schéma issu
  de la synchronisation de main.
- Driver/exécuteur #373 : découverte optionnelle et PostgreSQL, capture des
  destinations ouvertes, politique/audit et tests. Les rôles console/catalogue
  et réservations de session de main sont conservés.
- Desktop/IPC #373 : API typées, confirmation native, nettoyage des annulations
  et vérifications de déconnexion commencés.
- Frontend #373 : MultiBaseExplorer, DatabaseConnectionDialog,
  ServerProfileDialog, coordination et données par fenêtre, première console
  et raccordement du workspace/sidebar. Ils restent commencés jusqu'à
  satisfaction des critères et contrôles.
- Socle réutilisé : facettes et lectures coordonnées, ERD assistant, cycle de
  vie des workspaces, registre d'actions et dialogues.

Ces changements restent non commités pendant stabilisation. Les contrôles du
2026-11-13 valident l'ancienne base, pas le code synchronisé ni le nouvel écran.

## Validation et acceptation

Chaque lot consignera formatage, lint, typage, tests, stories, build et
`make qualite` réellement exécutés, sources jetables, échecs, exclusions,
outils absents et plateformes non vérifiées. Aucun délai ni assertion ne sera
affaibli pour obtenir du vert. Les revues locales suivront les checklists du dépôt.

Après tous les lots, vérifier dev et les ascendances, intégrer sans perdre les
nouveautés de dev ni rejouer les commits communs, puis revalider l'ensemble.
Ensuite seulement, lancer une nouvelle instance `make desktop-dev` sur un
workspace temporaire depuis un checkout dev propre, préparer des données
jetables et vérifier processus et commits. Ne pas réutiliser une ancienne
instance ni proposer une instance de fonctionnalité/main pour les essais personnels.

L'acceptation personnelle n'est **pas obtenue**. Le scénario final couvrira
F6/Maj+F6, découverte/ouverture, répétitions, A→B→A, brouillons, erreurs/annulation,
niveaux de métadonnées, carte, export/restauration PostgreSQL jetables.
Attendre le retour de l'utilisateur avant d'annoncer son accord manuel.
