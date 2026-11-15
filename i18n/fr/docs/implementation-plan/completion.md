<!-- oxyn-translation source="docs/implementation-plan/completion.md" sha256="de9ca44a551d" -->

# Suivi d'achèvement des outils de bases de données

Actualisé le 2026-11-15. Cette annexe suit la demande d'achèvement des issues
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
| Carte des schémas | `feat/schema-map` | Créée proprement depuis `40329e4` (#375). Onglet autonome, projection hôte commune, navigation propriétaire et voisins/échantillonnage explicites implémentés. Terminée et validée automatiquement : make qualite passe 4 753 tests Rust / 2 415 frontend, documentation et quatre archives de paquets. Acceptation personnelle après intégration finale dev. |
| #374 | `feat/native-backup-restore` | Créée proprement depuis `2f24e30` (carte) ; pile complète héritée sans duplication. Implémentation et validation natives terminées ; intégration dev après le commit natif propre. |

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

## Carte autonome : implémentation et validation courantes

La demande humaine `LoadSchemaMap` réutilise session exacte, priorité
interactive et générations atomiques du cache. La projection pure est commune
à la carte et à l'assistant ; ce dernier ne garde que résolution des noms et
adaptation de requête. Un contrôle mémoire refuse aussi une projection dépassée
pendant la livraison IPC. Aucun fichier/session, appel IA ou échantillon implicite.

Source de l'explorateur, menus natifs et barre de fiche d'objet ouvrent des
onglets propriétaires conservés. Objet choisi et périmètre entier ont des
chargements distincts. Périmètre, filtres, inspection, zoom/déplacement et liste
d'objets/liens restent locaux. Les voisins se développent explicitement à un
saut. Data/Structure/définition gardent la source et changent la section d'un
objet déjà ouvert. Fermer/masquer libère seulement sa commande ; la console ne
peut se déplacer pendant sa demande. Nouvelles cartes/cartes agrandies ne
recouvrent pas les voisines. En compact, diagramme et inspecteur défilent sans
se chevaucher ; une story vérifie cette géométrie.

Niveaux natifs absents, homonymes, clés composites/cycliques/sans nom ou
incomplètes, références non chargées et provenances restent distincts. Une
cardinalité inconnue le reste ; une description périmée ne prouve pas l'unicité.
Couverture, omissions, erreurs/indisponibilités et arrêts sont visibles. Les
dépendances techniques restent indisponibles sans métadonnées fournisseur.
Aucune pièce jointe SQLite/DuckDB n'est ouverte ; aucune source supprimée ou
éphémère fermée n'est remplacée par une base vide. Dispositions durables, export
image et DDL graphique restent séparés.

Preuves ciblées du 2026-11-14 :

- Huit tests de projection et neuf tests exécuteur/publication passent :
  identités, provenance, bornes, capacités, appels exacts/cache, objets inconnus,
  humain/propriétaire, nettoyage d'annulation et générations tardives. La revue
  a aussi refusé la preuve de cardinalité pour une clé incomplète ; le contrôle
  final la revalide.
- SQLite et DuckDB jetables passent vues, clés composites et périmètres natifs :
  trois objets, quatre liens, sept lectures puis zéro à la répétition. Deux
  sources mémoire privées ont des clés distinctes. Le cas fichier supprimé/
  propriétaire fermé a été ajouté pour le contrôle final.
- Tests contrôleur/IPC : propriétaires tardifs/remplacés et générations à la
  livraison refusés, ancienne carte conservée après erreur, clics regroupés,
  aucun chargement/échantillon au montage. Le placement pur couvre ajout,
  agrandissement et remplacement de propriétaire. Cinquante-deux stories
  carte/arbre passent avec axe, dont géométrie compacte et menu ; les stories
  ERD/assistant communes passaient aussi plus tôt dans ce lot.
- Clippy de tout le workspace avec `-D warnings` passe après déplacement d'un
  réexport Core mal placé. Les premiers contrôles globaux ont trouvé du lint
  frontend et une régression de menu, corrigés sans suppression. Les comptes
  finaux seront consignés après le contrôle complet.
- `make qualite` final passe le 2026-11-15 : 4 753 tests Rust / 2 415 frontend,
  documentation et quatre paquets. Acceptation personnelle non obtenue ; pas
  d’intégration dev/main ni de push.

### Rendu exploratoire de la carte (2026-11-14)

Chromium/Playwright installé, Storybook local, vue 1 440 × 1 050 ; graphe
synthétique de 40 objets/12 champs/200 FK. Les 40 nœuds et 200 arêtes sont rendus,
sans erreur de page. Cinq placements Dagre : 181,2 / 141,4 / 133,2 / 121,3 /
117,2 ms. Le filtre relançait ce placement ; il dépend maintenant du graphe,
séparément des filtres/gestionnaires. Positions et collisions sont testées à part.

Observations entrée-vers-deux-images pour quatre filtres : 309,6 / 247,3 /
200,6 / 447,1 ms avant ; 511,6 / 113,7 / 73,1 / 226,7 ms après. La première
valeur à froid régresse. Compilation et navigateur actifs rendent ces chiffres
impropres à établir gain fiable, p99 ou respect des budgets. Ce sont des
diagnostics exploratoires, pas un benchmark de référence sur machine au repos.
Placement initial et interactions continues restent à mesurer dans Tauri ;
aucun budget ni limite n'a été augmenté. Une mesure contrôlée distincte reste
nécessaire. Un build local ne vaut pas acceptation personnelle.

### Contrôle final de la carte — 2026-11-15

`make qualite` passe : 4 753 tests Rust, 211 exclus, 2 415 tests frontend,
format/lint/types/build, doctests/rustdoc et quatre archives de paquets vérifiées
indépendamment. Les régressions fichier supprimé/propriétaire fermé et clé
incomplète passent dans cet arbre final. Le socle ne rapporte aucun avertissement.
Audit npm et licences des 415 paquets de production passent.

`cargo-deny` reste absent : audits de licences Rust/RUSTSEC non effectués.
`cargo-about` reste absent : notices non régénérées. Les suites de serveurs et
le modèle de référence n'étaient pas configurés ; seuls les essais natifs de
carte SQLite/DuckDB jetables ont été exécutés dans ce lot. Les diagnostics
existants ResizeObserver/React act n'ont pas été masqués ; les assertions
passent. Seul macOS est exercé localement.

Revues locales des invariants, frontières, sécurité, organisation et interface :
pas de blocage restant après corrections. La demande conserve un bail d'usage
de console ; son contrôle de génération mémoire n'introspecte rien. Les grands
points de composition existants gardent leur responsabilité ; projection,
placement, chargement, propriété, validation IPC et présentation sont séparés.
Acceptation personnelle non obtenue ; pas d'intégration dev/main ni de push.
Ce contrôle remplace l'état d'attente consigné pendant les vérifications ci-dessus.

## Plan des transferts natifs — 2026-11-15

La branche part du commit carte propre et validé `2f24e30`. `main` et `dev`
restent sur `ce58839`. Core porte commandes humaines typées et états lisibles.
`oxyn-transfer` possède capacités de fichiers/outils privés, adaptateurs,
processus/groupes, flux bornés et publication sans écrasement. L'exécuteur garde
vérifications d'autorité, audit, cycle persistant, annulation/attente et
invalidation après modification possible ; le store ajoute des enregistrements
versionnés bornés. Desktop possède sélecteurs natifs, confirmation critique et
baux de fenêtre. Onglet/contrôleur shadcn expose les mêmes descripteurs et états
pour chaque moteur, avec motifs explicites d'indisponibilité.

Restauration mutante toujours confirmée par l'hôte, même locale ; agents exclus
de l'inspection/exécution. Ni chemins ni argv bruts ne donnent autorité au
webview. Masquer conserve la tâche ; fermer attend son nettoyage. Fermer source
ou fenêtre rejoint ses tâches. Au redémarrage : interrompu/inconnu, sans rejeu.

ADR-0083 proposée précise premier mode utile d'authentification et décisions
Unix/fichiers. Mot de passe/TLS/SSH et restauration SQL psql restent indisponibles.
Clients corrigés préparés séparément pour essais, sans modifier Homebrew global.
Activation après tests d'authentification synthétique, arguments, outils absents/
incompatibles, fichiers/permissions, descendants, annulation, confidentialité et
export custom/SQL puis restauration d'archive sur PostgreSQL jetable. Terminer
par contrôle complet, revues, contrats bilingues et commit propre ; ensuite
seulement intégrer/revalider dev et lancer la nouvelle instance personnelle.


## Implémentation et revue des transferts natifs

Le parcours commun core/store/exécuteur/interface est implémenté ; seul
PostgreSQL possède l’adaptateur initial dans le mode restreint de l’ADR-0083.
L’historique conserve identité d’outil, nom de fichier, options de restauration,
étape, octets, dates et résultat initial, sans arguments, chemins, secrets ou stderr
brut. Droits préparés/actifs et historique lisible restent séparés ; aucun droit
exécutable n’est reconstruit au redémarrage.

Les revues locales invariants/frontière/sécurité/interface ont corrigé :
enregistrement du réveil des attentes concurrentes de nettoyage ; droits de
sélecteur arrivant après fermeture source/fenêtre ; nettoyage des réponses tardives
d’interface ; recontrôles lecture seule/destination ; admission des mutations et
fermeture des aperçus ; contrôle d’identité de publication ; options persistées ;
contrôles rognés en disposition compacte. Le contrôleur retenu coordonne états,
requêtes et fermeture sur 432 lignes ; politique outils/processus/fichiers
et présentation pure ont leurs modules. L’orchestration workspace compose seulement
les onglets attachés à leur source et leurs handles d’activité/fermeture.

Le premier contrôle complet a passé ses 2 433 tests frontend, types/lint/build,
puis arrêté sur des diagnostics Clippy de la frontière native. Construction de
commande regroupée dans un site vidant l’environnement, attente bornée de worker
documentée et fixture d’orphelin conservant l’assertion de récolte système. Aucun
lint global, assertion ou délai de test assoupli. Un contrôle ultérieur doit
valider l’arbre final avant commit/intégration.

Corrections de fixtures jetables, pas de produit : la console PostgreSQL ne
propose pas de transaction explicite ; le verrou utilise une instruction DO,
dont l’annulation reste ambiguë et non rejouable. La remise à zéro de la table
de restauration exige d’approuver la revue de risque DROP ordinaire, sans
confondre NeedsApproval et exécution terminée. L’aller-retour lit maintenant la
page de résultat bornée retenue et vérifie les valeurs restaurées.

Restent non vérifiés : exécution Linux, possession de processus Windows,
transports distant/TLS/SSH/mot de passe, restauration SQL, autres adaptateurs,
client PostgreSQL 18, gros volumes réels et acceptation utilisateur. Aucune de ces
preuves n’est déduite d’une compilation locale ou de l’interface commune.


## Acceptation personnelle sur l’instance dev intégrée

Utiliser uniquement les fichiers/bases jetables du `fixtures.json` de lancement.
La nouvelle fenêtre doit indiquer Temporary workspace et son identité de build
correspondre au commit dev intégré propre. L’acceptation reste en attente du
retour utilisateur ; les contrôles automatiques ne constituent pas ce retour.

1. **Navigation/propriété :** connecter les fichiers SQLite homonymes A/B. Lancer
   un petit SELECT dans A puis modifier un brouillon sans enregistrer. Répéter
   activations et A→B→A : retrouver exactement brouillon, résultat et console de A ;
   la première console de B reste vide.
2. **Multi-base :** sur le PostgreSQL jetable, créer un groupe serveur, sélectionner/
   développer sans activer, découvrir les bases puis ouvrir explicitement un enfant.
   Répéter l’activation et annuler découverte/ouverture. Une cible enregistrée en
   erreur conserve son entrée locale et se rouvre après correction.
3. **Focus :** sélectionner du texte ; F6 et Maj+F6 parcourent les zones visibles
   puis retrouvent la sélection. Refaire en compact, panneau fermé, carte et dialogue.
   Aucun workspace ou panneau masqué ne vole le focus.
4. **Métadonnées :** changer Minimal/Structure/Étendu et une dérogation de namespace.
   Enregistrer seul ne précharge pas la base. Demander un préchargement borné,
   l’annuler, examiner séparément non lu/vide/périmé/indisponible/erreur.
5. **Carte :** charger explicitement un périmètre SQLite/PostgreSQL, inspecter la FK
   composite, filtrer/déplacer/zoomer, passer à la liste accessible puis Data/Structure/
   DDL. Changer A/B conserve l’identité ; les gestes visuels ne lisent pas de lignes.
6. **Export :** sélectionner le `pg_dump` corrigé, archive personnalisée ou SQL et
   nouveau fichier. Préparer, inspecter puis lancer ; vérifier le fichier terminé.
   Un chemin existant est refusé. SQLite et autres adaptateurs indisponibles expliquent
   leur limite.
7. **Restauration :** sélectionner `pg_restore` corrigé et l’archive, acquitter la
   confiance, préparer la base jetable vide de destination. Annuler une fois la
   confirmation native : rien ne démarre. Préparer à nouveau et vérifier la cible
   exacte avant confirmation. Lecture seule refusée, scripts SQL indisponibles.
8. **Résultats/fermeture :** inspecter réussite ou résultat partiel possible d’une
   restauration volontairement répétée. Aucun rejeu automatique. Masquer conserve
   la tâche ; fermer propose par défaut de garder ouvert et attend annulation/
   nettoyage demandés. L’inspection humaine conserve le résultat initial.


Le deuxième contrôle complet a passé 2 434 tests frontend et Clippy workspace,
puis arrêté après 2 693 réussites Rust sur la régression existante de déconnexion
catalogue hors runtime (2 082 cas non exécutés, 213 ignorés). Le nettoyage natif
appelait un worker exigeant un runtime avant d’achever l’annulation catalogue.
La correction réutilise le repli d’arrêt hors runtime de l’écrivain local audité
pour persistance et libération associée ; les appels GUI utilisent toujours le
pool bloquant. Le test existant et ses assertions d’annulation/nettoyage sont
conservés. Un nouveau contrôle complet valide cette correction.

La frontière native passe également Clippy croisé `x86_64-pc-windows-gnu`,
toutes cibles/fonctionnalités et avertissements refusés. Imports, dépendances et
tests Unix sont conditionnés. Cela contrôle `oxyn-transfer` et sa dépendance
core ; ni l’application Windows complète ni l’exécution native Windows ne sont
validées. La possession des processus Windows reste explicitement indisponible.
Le composant a été installé via la chaîne Rust épinglée, sans modifier sa version
sélectionnée ni la configuration globale de Codex.


## Validation finale #374 — 2026-11-15

Le `make qualite` final passe 4 779 tests Rust (213 explicitement ignorés),
2 438 tests frontend, formatage, Clippy workspace sans avertissement, types,
stories/accessibilité, build frontend, doctests/rustdoc et vérification indépendante
de quatre paquets. Le frontend comprend 17 stories natives et six tests de
contrôleur : historique non lu/échoué distinct de vide connu, lectures de
disponibilité/historique annulables et annulées au démontage, conservation en
onglet masqué et propriété synchrone pendant les mises à jour React groupées.
Deux contrôles préliminaires ont échoué sur fermeture et liste exhaustive des
tables lisibles, corrigées sans affaiblir les assertions. Les passages diagnostiques
utilisant encore d’anciens artefacts ne valident pas un source modifié.

Contrôles complémentaires : clients/serveur PostgreSQL 17.12 corrigés isolés,
exports archive/SQL, valeurs restaurées lues en page bornée, refus de confirmation
native et autre fenêtre, changements lecture seule/destination après préparation,
annulation d’un dump bloqué observée dans pg_stat_activity avant libération du
verrou, invalidation après restauration possiblement partielle, absence de rejeu
et rapprochement explicite conservant le résultat. La sonde d’authentification
accepte la fixture locale sans mot de passe et refuse le défi SCRAM sans archive.
Les 14 tests frontière fichiers/processus/adaptateur passent ; leur harnais de
sous-processus reste explicitement ignoré lors des exécutions ordinaires.
Clippy croisé Windows GNU passe pour frontière et core ; exécution native Windows
indisponible. Linux partage le runner Unix mais n’a pas été exécuté localement.
Seul macOS dispose de preuves locales d’exécution et visuelles.

La revue finale corrige aussi nettoyage à capacité pleine, droit d’outil non
retourné après inspection annulée, lecture JSON bornée au démarrage et libération
d’une préparation dont la console a changé de fenêtre ou disparu. La libération
reste attachée aux tâche/connexion/fenêtre originales, sans accès à la session
déplacée ni autorité d’exécution. Les workers source/fenêtre/arrêt sont rejoints.
Revues invariants/frontière/sécurité/organisation/interface sans blocage restant
dans le périmètre implémenté.

`cargo-deny` et `cargo-about` absents : audits Rust sécurité/licences et régénération
des notices non réalisés. Audit npm et 415 licences de production passent.
Suites génériques de serveurs et modèle de référence embedding non configurés ;
fixture PostgreSQL native explicite exécutée séparément. Diagnostics navigateur
ResizeObserver/React act existants toujours visibles, sans suppression, tests verts.
Gros volumes, application Windows complète, exécution Linux, transports distant/
TLS/SSH/mot de passe, restauration SQL et autres adaptateurs natifs ne sont pas
validés ni implicites. Aucune acceptation utilisateur enregistrée.
