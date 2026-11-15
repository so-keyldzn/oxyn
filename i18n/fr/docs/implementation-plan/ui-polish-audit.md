<!-- oxyn-translation source="docs/implementation-plan/ui-polish-audit.md" sha256="02c7eeeb15a5" -->

# Audit d’interface et polish du chargement à la demande

Date : 2026-11-15. Périmètre : interface desktop du worktree dev désigné.
Demande : retirer les doublons, améliorer la sidebar, employer shadcn et Hugeicons,
afficher les schémas choisis sans parcours de préparation.

## Direction

Conserver le workspace professionnel compact, sa typographie et sa palette
sémantique. Séparer connexions, objets de la source choisie et outils. Une action/
icône reconnaissable par usage, détails de maintenance progressivement dévoilés,
états de lecture/erreur/couverture partielle lisibles. Aucun thème décoratif neuf
ni préchargement automatique de toutes les bases.

## Constats et corrections

| Surface | Constat/preuve | Correction |
|---|---|---|
| Carte | Sélection puis Charger redondant | Lecture du périmètre visible automatique, actualisation/annulation conservées |
| Couverture | Badges à zéro et long pied technique concurrencent la carte | Couverture/omissions utiles visibles, détails dépliables |
| Sidebar | Boutons génériques concurrencent la navigation | Outils sidebar shadcn, rail accessible et Hugeicons sémantiques |
| Onglets | Même dashboard pour carte/transfert/résultat | Icônes distinctes relation/transfert/table |
| Groupes serveur | Contrôles de gestion concurrencent la sélection | Maintenance regroupée en menu ; sélectionner/développer/activer distincts |
| Métadonnées | Préchargement et enregistrement semblent également nécessaires | Lecture à la demande ; préchargement facultatif sous Avancé replié |
| Transferts | Termes d’implémentation trop présents | Texte centré sur le résultat, progression lisible, revue/actualisation concise |
| Structure/DDL/index | Repli Charger présent, mais les consommateurs visibles appellent ensure | Garder la demande, sans charger les facettes masquées ni échantillonner |
| Tables/connexions/réglages/assistant | Composition shadcn existante ; inventaire insuffisant pour certifier le rendu | Revue visuelle et interactions sur stories synthétiques larges/compactes |

## Limites de comportement

L’[ADR-0084](../adr/0084-load-visible-schema-scopes-on-demand.md) précise le geste.
Plusieurs choix natifs exigent encore de sélectionner le périmètre : le backend
n’agrège pas tous les namespaces d’une base. Un seul enfant entièrement listé peut
être suivi ; une liste partiellement en cache ne permet pas de deviner le défaut.
Aucune carte complète multi-namespace n’est ainsi prétendue.

Identité source/session/génération, annulation et bornes objets/champs/liens/temps/
octets conservées. Capacités réelles de chaque moteur respectées. Aucune réouverture
SQLite/DuckDB, échantillons documentaires toujours explicites et confirmés.
Préchargement global facultatif conservé et annulable ; enregistrer ou déplier
Avancé ne le lance jamais.

## Validation

Agents répartis entre carte, métadonnées/sidebar et revue visuelle générale.
Contrôles et limites visuelles sont rapportés après assemblage. Une story synthétique
réussie ne valide ni tous les parcours natifs ni les budgets de frame. L’application
dev temporaire utilise le hot reload ; main et branches distantes restent hors
modifications. Les agents d’audit ne créent aucun commit.


### Preuves visuelles élargies

La sonde de stories synthétiques stabilisées produit 33 captures sur 11 routes
en 1440/1100/600 pixels, sans erreur de page. 111 stories d’interaction/accessibilité
couvrent gestion/dialogues de connexion, workspace, objets/tables, apparence et
assistant. Les fixtures workspace de largeur fixe ne prouvent pas un défaut
responsive sous leur taille déclarée ; minimum natif 960 pixels. Contrôles
complémentaires workspace en 960/1100/1440.

Défaut confirmé corrigé dans le formulaire de nouveau compte : hauteur flex/
pourcentage indéfinie laissant le viewport déborder derrière les actions. La
variante formulaire reçoit une hauteur bornée définie et un défilement contenu ;
les autres variantes gardent leur dimensionnement intrinsèque. Géométrie et stories
formulaire/dialogue existantes : 36 cas réussis. Preuves navigateur synthétiques,
pas acceptation native. La capture utilisateur guide séparément la sidebar dense
adaptée au volume et le retrait du sélecteur Browse session permanent.


## Sidebar de la capture et exploration de session

La capture confirme destinations/statuts répétés, boutons de ligne volumineux,
sélecteur de session permanent, navigation de connexion doublonnée et espacement
de haut de fenêtre appliqué à un en-tête au milieu de la sidebar. La liste de
connexions reste bornée en hauteur pour laisser de la place à l’arbre. Filtrage
local, maintenance contextuelle sans activation, identités homonymes distinctes.
Stories de volume : noms longs, nombreuses connexions, rail et clavier.

Le sélecteur permanent devient une action contextuelle de console. Aucun espace
vide en mode connexion. L’indicateur de session nomme la console et propose le
retour. Seules les lectures de l’explorateur changent : connexion active, brouillons,
sessions et transactions intacts. Revenir conserve les objets/aperçus/cartes dont
le propriétaire est vivant. Disparition ou transfert explique le retour et retire
uniquement les vues du propriétaire indisponible. Un ancien menu ne peut sélectionner
une session déjà fermée ou déplacée.


### Longues listes de connexions

La zone connexions est bornée indépendamment de l’arbre. Filtrer la liste chargée
n’ouvre aucune connexion ni n’introspecte ses bases. À 30/100 connexions et 500
objets, les tests exigent moins de 34 % du cadre pour les connexions et plus de
50 % pour les objets, avec des lignes d’au plus 36 pixels. Destinations homonymes
distinguées visuellement et dans les descriptions accessibles des lignes/actions.

L’action visible au clavier Aller aux objets donne accès au filtre de la source
courante en nombre constant d’étapes, sans parcourir deux contrôles par connexion.
Elle ne change que le focus. Le cas 1 000 est mesuré séparément ; DOM et timings
du navigateur dev ne constituent ni p99 natif ni garantie absolue de fluidité.
Virtualiser exige une preuve et doit conserver sélection, menus et clavier.


### Référence à 1 000 connexions

La sonde navigateur reproductible de la sidebar synthétique partagée mesure
13 338 éléments pour 1 000 lignes montées. Liste : viewport 183 pixels, contenu
35 996 pixels ; catalogue 456 pixels avec le vrai pied d’outils. Aucun débordement
ni erreur de page. Trois contextes navigateur neufs contre le serveur dev chaud :
longues tâches de montage de 1,309–1,756 seconde. Six filtrages vers une ligne
committés en 122,6–896 ms (médiane 388,6) ; six effacements vers 1 000 lignes
en 655–2 108 ms (médiane 1 593,6). Navigation initiale inclut Storybook/imports.
Diagnostics dev sur machine partagée, sans p99 natif ni certification au repos.
Ils justifient un rendu virtualisé borné, à comparer avec les mêmes contrôles
d’interaction/géométrie avant d’annoncer un gain.


### Logos et candidat virtualisé mesuré

L’identité utilise DriverLogo et un identifiant explicite ; Hugeicons reste réservé
aux actions/navigation. Les neuf protocoles ont leur marque. Aucun champ produit
serveur connecté n’est reçu : les alias ne prouvent pas MariaDB/Valkey/Redshift/
OpenSearch. Aucun nom/hôte ne permet de le deviner. Titre du formulaire et lignes
de connexions réutilisent l’identité connue ; actifs et restrictions des marques
existants sont conservés.

La même sonde du candidat virtualisé réduit les éléments sidebar de 13 338 à 482
et les lignes montées de 1 000 à 11. Médiane sur six filtrages : 237,8 à 7,7 ms ;
retour de liste : 784,6 à 15,7 ms. Médianes à deux frames : 273,9 à 27,9 ms et
844,5 à 41,0 ms. Aucun débordement/erreur. Navigation initiale bruitée incluant
bootstrap : ni p99 natif ni garantie de toutes les répartitions de groupes ouverts.
Identités stables et lignes au focus/menu épinglées ; répartitions et interactions
clavier/menu contrôlées séparément avant validation finale.


### Groupes répartis

100 groupes × 10 connexions ont montré la limite des fenêtres internes : les
petits groupes hors écran montaient encore leurs membres. Leur suspension observe
le scroller externe, garde la place et maintient les listes possédant focus/menu.
Aucun groupe replié ni sélection oubliée. La même sonde réduit les lignes montées
1 000→10 et le DOM sidebar 15 237→2 367. Médianes : filtrage 176→19,4 ms,
effacement 2 279,2→153,35 ms. Les en-têtes restent montés ; aucun DOM constant
pour un nombre illimité de groupes n’est prétendu. Aucun débordement/erreur mesuré.
Mêmes réserves de développement/concurrence et absence de certification native.


Les comparaisons temporelles précèdent le dernier remplacement par les logos.
Les captures finales confirment indépendamment lignes bornées, absence de
débordement/erreur, pied partagé et rail replié. L’indicateur de console est
contrôlé dans sa fixture réelle dédiée, sans faux bandeau ajouté à la fixture
de volume. L’acceptation dans une fenêtre native reste distincte.


### Relecture finale des invariants

Une relecture indépendante sans modification ne relève aucune violation
bloquante confirmée des invariants I-01 à I-13. Elle couvre la résolution des
menus sur les entrées vivantes, le partage par propriétaire connexion/session,
les réponses tardives de recherche et de carte, le retrait des propriétaires
disparus, le maintien du clavier/des menus et les marques du driver explicite.
Aucun nouveau chemin direct driver/IPC, exécution SQL, fermeture de transaction
ou journalisation de secret n’est introduit. Cette revue ne certifie ni
l’intégration sur une vraie base ni les parcours de fenêtre native ; le résultat
du contrôle qualité complet est consigné séparément.


### Contrôle qualité complet terminé

Le 2026-11-15, `CARGO_BUILD_JOBS=2 VITEST_MAX_WORKERS=1 make qualite
SHARD='1/1 --no-file-parallelism --maxWorkers=1'` passe. Ce shard couvre toute la
suite ; aucun test, assertion ou délai n’est retiré ou assoupli. Résultat :
276 fichiers frontend / 2 470 tests réussis ; 4 779 tests Rust réussis, 213 ignorés ;
doctests, Clippy strict, rustdoc, formatage, TypeScript, ESLint, builds de production
et contrats de packaging passent. Les contrôles ciblés couvrent aussi propriétaires
et retrait des sessions, réponses tardives, brouillons conservés, clavier/menus et
volume. Des relances concurrentes dépassaient les délais de tests assistant/vues
existants ; la suite complète séquentielle passe sans les modifier.

Limites explicites : suites serveur et référence embedding non exécutées faute
de ressources configurées. L’absence de cargo-deny laisse l’audit Rust des avis
de sécurité/licences non exécuté ; cargo-about absent laisse les mentions tierces
non générées. L’audit npm de production ne rapporte aucune vulnérabilité connue.
Le build conserve des avertissements sur les gros chunks. Les journaux navigateur
conservent des avertissements ResizeObserver et React act déjà observés dans des
contrôles précédents ; aucun cycle permanent ni nouvelle violation bloquante
n’est établi par les contrôles ciblés et la revue. Cela ne certifie ni les budgets
natifs, ni l’intégration sur de vrais serveurs, ni tous les parcours de fenêtre
native. Travail non committé sur dev ; main et les branches distantes inchangées.
