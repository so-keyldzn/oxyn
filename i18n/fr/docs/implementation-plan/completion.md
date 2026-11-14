<!-- oxyn-translation source="docs/implementation-plan/completion.md" sha256="695e4728b15f" -->

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
| #373 | `feat/multi-base-backend` | Implémenté et validé automatiquement sur `ce58839` ; `make qualite` réussi. Le commit `a3f5961` suit la recherche `356c8d6` ; acceptation personnelle attendue. |
| #308 | `feat/f6-navigation` | Créée depuis `a3f5961` après un commit #373 propre. Cycle visible via registre implémenté ; Terminé : 93 tests d’actions, cinq stories navigateur, gate complet et revalidation frontend finale réussis. Acceptation personnelle/native attendue. |
| #375 | `feat/metadata-loading` | Créée depuis `4c80bfa` (#308), propre et validé. Implémentation commune selon capacités terminée et validée automatiquement : make qualite passe 4733 tests Rust / 2391 frontend. Acceptation personnelle après intégration finale sur dev. |
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

Le contrôle suivant a trouvé un résidu de formatage dans le test de reprise
ajouté en fin de lot. `a75a5f2` le corrige après une vérification Prettier ciblée ;
la branche #373 inactive a avancé de `a3f5961` vers ce descendant. Aucun historique
réécrit ni code d’issue transporté entre branches. Le gate F6 revalide l’ensemble.

## Validation courante de #308

F6/Maj+F6 sont des actions du registre, placées dans View et reprises dans la
palette et la fiche. Les zones visibles réutilisent les marqueurs shadcn de la
barre latérale et des marqueurs explicites d’éditeur, résultats et panneaux.
Les cartes optionnelles utilisent la même déclaration générique, sans import
ni dépendance à leur livraison. Aucun second écouteur clavier, ouverture de
panneau, activation de connexion ou lecture de données n’a été ajouté.

93 tests d’actions/registre/dispatch et cinq stories Chromium passent.
Elles exercent une vraie sélection CodeMirror et une grille en modes large,
compact, panneaux fermés et sous dialogue modal. Le retour conserve sélection
et texte. Un contexte périmé de workspace masqué est également couvert.
TypeScript et ESLint passent. `make qualite` a passé 4 707 tests Rust et
2 368 tests frontend, correctif de formatage commun inclus. La dernière revue
a ajouté le garde-fou d’onglet sélectionné : un onglet mémorisé devenu inactif
n’est plus une cible valide et ne doit pas être réactivé par F6. Sa régression
et les 93 tests d’actions passent. Le `make front` final a passé 2 370 tests,
formatage, lint, typage et build après ce raffinement purement UI. La cinquième
story appelle aussi la vraie palette et vérifie le retour du focus après sa
fermeture. `make socle` réussit sans avertissement après cette mise à jour. Les touches natives
attendent l’instance finale dev et l’acceptation personnelle. Aucun Rust ou IPC
n’a changé lors du dernier raffinement de focus.

## Plan d’implémentation de #375

Core porte politique et périmètre purs ; store porte migration 21, JSON borné et
comparaison de révision. L’exécuteur possède politique/audit, facettes séparées,
générations, annulation, priorité des demandes et comptage borné. Le desktop
expose IPC typé, contrôle de session/fenêtre et progression ; les features
frontend raccordent réglages et préchargement shadcn. Cache, coordination des
lectures et UI des facettes existants sont réutilisés.

Ordre : persistance/commandes ; descriptions/index/clés séparés et admission
explicite des échantillons ; préchargement/progression ; UI et consommateurs ;
validation/documentation. Aucun protocole ni paquet externe ajouté ; les versions Serde/Serde JSON du
workspace sont réutilisées pour borner la publication.
`INFERRED_SCHEMA` identifie déjà les descriptions échantillonnées.

Échecs à prouver : révisions et publications tardives, distinction non lu/vide/
périmé/indisponible/erreur, héritage exact, sauvegarde sans appel driver,
annulation/remplacement, éviction, admission explicite sans valeurs, demandes
justifiées du DDL/complétion/assistant. Une source fermée ne se reconnecte pas.
Mesurer appels provider, octets de métadonnées et temps sur sources jetables ;
aucun seuil automatique ni exemption générale SQLite.

La livraison exige tests de toutes les couches, stories accessibles par état,
revues locales, `cargo fmt --all`, `make qualite`, contrats anglais/français
cohérents et branche propre commitée. L’acceptation personnelle reste réservée
à l’instance finale dev intégrée.

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

### Mesure SQLite jetable (2026-11-14)

Une base mémoire privée, trois tables, une clé sortante et un index secondaire ;
chaque mode démarre sans avoir lu les noms/champs de cet espace de noms.

| Préréglage | Périmètres soumis | Octets de métadonnées encodées | Durée ms | Facettes détaillées fraîches |
|---|---:|---:|---:|---:|
| Minimal | 1 | 554 | 0 | 0 |
| Structure | 10 | 5 164 | 1 | 9 |
| Étendu | 13 | 7 082 | 0 | 12 |

Répéter chaque page n’a soumis aucune lecture et aucun octet supplémentaire.
Les tests sur fournisseur instrumenté vérifient séparément les appels exacts
aux méthodes de description/index/clés/définition. Périmètres et appels réseau
ne sont pas équivalents. Les millisecondes sont arrondies vers le bas : zéro
signifie sous la résolution du rapport, pas aucun travail. C’est une mesure de
fonctionnement locale, ni un banc réseau ni une justification de seuil ou
d’exemption SQLite. La porte complète reste nécessaire avant toute bascule.

## Validation finale actuelle de #375

La migration 21 sépare politique et authentification. Le préchargement reste sur
le propriétaire vivant exact : priorité interactive, pages explicites de
128 descriptions, échéance d’annulation de 60 secondes, rapports bornés. Scope,
révision et génération lient la continuation. Changer le contexte annule le seul
préchargement de cette console et retire sa génération avant/après l’opération.
Aucune source/session/fichier n’est ouvert implicitement.

Le dialogue shadcn sépare enregistrement, préchargement, page suivante et arrêt.
Les éditions doivent être enregistrées ; un conflit exige une relecture. Masquer
ou retirer la source libère sa seule tâche. Onglets, copies, complétion, assistant
et ERD demandent des facettes précises ; DDL explicite en large lit la définition,
Structure ne la lit pas. L’inférence exige un geste humain borné. Préparer un
aperçu réutilise les métadonnées fraîches du propriétaire ; structure absente ou
périmée n’entraîne aucun échantillon caché. Des champs déjà inférés restent
disponibles sans nouvel échantillon.

Le bilan distingue frais/périmé/non lu/évincé, indisponible et échoué. Les comptes
restent nullables avant mesure ; une éviction rend le bilan partiel sans boucle
de relecture. La publication contrôle atomiquement génération et éviction. Bornes
d’échantillonnage et génération typée appartiennent au contrat catalogue ; les
DTO du cœur transportent un UUID opaque et n’imposent aucun symbole non publié
à l’empaquetage. UUID/Serde/Serde JSON existants sont réutilisés sans nouvelle
version ou paquet externe dans le graphe.

Vérifié le 2026-11-14 :

- `make qualite` final passe : formatage, Clippy `-D warnings`, 4733 tests Rust
  (211 ignorés), doctests/rustdoc, quatre archives vérifiées séparément, 2391
  tests frontend et build. Socle : zéro avertissement.
- Scénarios capacités indépendantes, héritage exact, CAS/migrations, sauvegarde
  sans I/O serveur, facettes indépendantes, annulation contexte/politique,
  générations tardives, continuation bornée, éviction et inférence en cache.
- Quinze stories de réglages/échantillonnage passent dans Chromium avec axe ;
  régressions DDL large et remplacement de source/réponse tardive. Inspection
  des états partiels/évincés sans erreur de page.
- SQLite mémoire privée exerce les trois niveaux et le cache (mesures ci-dessus).
  MongoDB 8.0.34 jetable : 24 tests réussis, annulation observée serveur, flux
  de 1 Gio en lots bornés, échantillon trop grand refusé/curseur libéré et preuve
  par profiler que préparer l’aperçu ne soumet aucun `find` caché. Conteneur supprimé.
- Pic RSS MongoDB non vérifié sur macOS (assertion Linux uniquement). Autres
  serveurs et modèle d’embeddings de référence non exécutés dans la porte,
  faute d’URL/modèle configurés.
- `cargo-deny` absent : audits Rust licences/RUSTSEC non exécutés. `cargo-about`
  absent : notices non régénérées. Audit/licences npm réussis. Diagnostics
  ResizeObserver/React act existants conservés dans la sortie navigateur,
  assertions réussies. macOS seul vérifié ; aucune acceptation manuelle obtenue.

Revues locales invariants/frontières/sécurité/organisation/UI : aucun blocage
restant après corrections. Les grands répartiteurs bus/cache/objet/workspace
conservent leur rôle existant ; politique, ordonnanceur, inférence, comptabilité
et UI pure/contrôleur sont séparés. Aucun second cache driver, seuil automatique,
exemption SQLite, protocole ou configuration globale ajouté. L’ERD est adapté
comme consommateur de facettes/propriété, sans seconde carte autonome. Aucune
intégration dev/main ni publication effectuée.
