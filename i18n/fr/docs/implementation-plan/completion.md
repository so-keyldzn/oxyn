<!-- oxyn-translation source="docs/implementation-plan/completion.md" sha256="a162a1c9a29c" -->

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
| #373 | `feat/multi-base-backend` | Implémenté et validé automatiquement sur `ce58839` ; `make qualite` réussi. Le commit du code suit la recherche `356c8d6` ; acceptation personnelle attendue. |
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

## Validation courante de #373

Le socle commun de recherche a été commité en `356c8d6` après le succès de
`make socle`, sans avertissement. Ce commit transversal reste documentaire.

L’explorateur sépare sélection, développement et activation. Les ouvertures
répétées sont regroupées ; les réponses tardives respectent le dernier choix et
les annulations ne libèrent que leur paire de sessions. Une connexion sauvegardée
mais non ouverte garde son identité. Les comptes restent distincts et la première
console est vide. La découverte refuse une source détenue seulement par une
autre fenêtre.

Création, renommage, déplacement/dissociation et suppression des groupes
conservent les connexions enfants. La déconnexion collective montre les comptes
de consoles, brouillons et opérations ainsi que les transactions signalées, puis
sérialise les revues de fermeture existantes. Un échec ou une annulation arrête
le lot et rapporte les fermetures effectuées. Un nettoyage backend incomplet
reste visible avec reprise explicite. Fermer la base active révèle une autre
base retenue.

Les demandes de texte, d’objet et de résultat portent la génération du workspace.
Les objets privés ne s’insèrent pas dans une console sœur ; un dépôt entre
sources demande le choix de la console d’origine. Le catalogue d’objets reste
celui du workspace actif, sans introspection profonde de toutes les bases.

Vérifié le 2026-11-14 :

- 18 tests backend de fermeture/propriété passent, dont nettoyage exact,
  refus inter-fenêtres et idempotence.
- Les tests frontend ciblés passent : raccordement, activations, files,
  identité sauvegardée, comptes distincts, annulation avant envoi et brouillons.
- Les nouvelles stories passent avec axe ; Cancel reçoit le focus dans les
  revues de source et de groupe. La suite frontend complète a passé 2 349 tests
  à ce stade.
- PostgreSQL 17 jetable : découverte et ouverture indépendante, annulation
  vérifiée côté serveur et streaming borné passent. Le conteneur limité au
  loopback a ensuite été supprimé.
- `make qualite` réussit : Clippy avec `-D warnings`, 4 707 tests Rust
  (209 exclus), doctests, rustdoc et vérification des contrats empaquetés.
  `make socle` réussit sans avertissement. Les raffinements UI et trois tests de
  reprise ultérieurs passent aussi TypeScript, ESLint et les contrôles ciblés.
- Le navigateur a émis des diagnostics ResizeObserver et React act malgré les
  assertions réussies ; aucun n’a été masqué. Les nouveaux composants ciblés et
  le contrôle visuel indépendant de trois écrans n’ont émis aucune erreur de page.
- `cargo-deny` absent : audits Rust de licences/RUSTSEC non exécutés.
  `cargo-about` absent : mentions non régénérées. Les autres URL serveur et le
  modèle de référence d’embedding ne sont pas configurés : suites non exécutées.

Les revues locales des invariants, frontières, sécurité, organisation et UI
ne trouvent plus de blocage dans ce lot. Elles ont fait corriger les générations
de texte/objets/résultats, la propriété de la source et les erreurs de nettoyage.
L’orchestration existante garde ses responsabilités ; profils, coordination et
dialogues vivent dans leurs modules. Aucune dépendance, configuration globale
ou publication ajoutée.

Il s’agit de contrôles automatiques. L’acceptation personnelle reste attendue.
Seul macOS a été exercé localement ; cette entrée ne livre ni transferts natifs
ni les lots suivants.

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
