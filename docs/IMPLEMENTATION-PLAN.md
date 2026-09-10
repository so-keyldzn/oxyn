# Plan de mise en œuvre

> **Autorité** : l'ordre des phases et la porte de sortie de chacune.
> C'est le seul document qui parle de ce qui **reste à faire** — les documents
> d'autorité décrivent ce qui est décidé.

État au 2026-09-10 : les quinze crates existent, avec une application GPUI,
un formulaire de connexion et un parcours d'exécution SQL. L'intégration UI/UX
reprend la maquette Figma : sidebar repliable, thèmes clair/sombre, Hugeicons et
Geist embarqués, catalogue réel chargé par paliers à travers le command bus.
Les corrections d'interaction souris, de saisie native, de session et
d'annulation sont en place.
Les critères de performance de la phase 0 restent à mesurer ; l'existence du
code ne valide pas à elle seule les portes de sortie ci-dessous.

Le premier workspace propose l'éditeur SQL, l'exploration des métadonnées et
l'aperçu automatique des 200 premières lignes d'une table SQL. Historique visible, requêtes sauvegardées et reprise des brouillons SQL sont
raccordés comme détaillé ci-dessous. L'aperçu ne propose pas encore de pagination ni de filtre
des lignes côté serveur.
La lecture asynchrone des pages de résultats débordées passe maintenant par
`ReadResultPage`, avec annulation, erreur explicite et refus des réponses
obsolètes. Le cache de pages partage le budget du résultat selon ADR-0012.
La borne en octets du cache de métadonnées, le budget global des résultats tenus par les vues
au remplacement/à la fermeture et les mesures RSS restent à compléter.

Le cadre table/console reprend maintenant les barres denses et les onglets,
la pilule d'environnement et le seuil compact de 1200 px. La préférence de
sidebar large est conservée, et le catalogue compact reste accessible dans
un panneau superposé. La confirmation focalise `Cancel`, retient le focus dans
la revue et refuse Entrée seule. L'aide de l'édition désactivée et l'export
réel de l'aperçu sont raccordés ; ce dernier distingue un aperçu complet d'une
réception tronquée et conserve son annulation pendant un changement de résultat.

L'alignement complet sur [ADR-0011](adr/0011-structure-commune-workspace.md)
reste en cours : restauration des onglets objets, cas DDL PostgreSQL avancés,
filtres/tri et menu compact complet restent
à intégrer. Les onglets Indexes et Relations lisent maintenant les index et clés étrangères
sortantes réellement présents dans le cache, avec chargement par le bus, état
vide distinct et explication des capacités absentes. Ils passent dans `More` en
largeur compacte. Une demande de métadonnées faite pendant un autre chargement
est mise en attente ; annuler retire aussi cette demande.
L'onglet Relations ouvre maintenant les références entrantes PostgreSQL/SQLite,
avec bascule vers les clés sortantes. Les colonnes From / To / Cardinality
suivent Figma `229:33051` (340/300/200, lignes de 32 px). Le statut d'unicité
reste inconnu pour les comparaisons complexes non établies. Entrée ou Open
source table consulte la source ; si elle n'était pas encore dans le catalogue,
le bus la décrit puis charge son aperçu, sans changer ni exécuter le brouillon.
Le chemin inverse permet aussi d'ouvrir une cible depuis les clés sortantes.
L'onglet Constraints lit à la demande les contraintes PostgreSQL et SQLite par un scope
spécifique du bus. Il distingue non lu, vide, chargement annulable et erreur ;
la sélection au clavier et à la souris permet de consulter et copier la
définition du moteur sans ouvrir ni exécuter du SQL. Le contrôle rejoint `More`
en largeur compacte. SQLite conserve le texte des clauses stockées dans le
schéma, sans inventer de noms ni perdre `ON CONFLICT`, les références ou les
CHECK. La colonne Status suit Figma `229:7998` : état réel de validation
PostgreSQL, ou `Not reported` lorsqu'il n'est pas fourni. Les colonnes reprennent
les proportions 280/220/200/140 et Geist 13 px ; l'onglet Constraints a une
largeur minimale de 113 px. La recette visuelle native reste à réaliser.
L'inspecteur de ligne est raccordé aux grilles de console et d'aperçu, avec
choix du champ au clavier et lecture complète paginée par `InspectResultValue`.
Le menu Columns masque/réaffiche les colonnes localement et conserve toutes
les données pour l'export. En mode compact, l'inspecteur s'ouvre en superposition
et ces actions passent dans Actions. La largeur initiale de l'inspecteur est
celle de Figma (`190:1872`, 280 px). La poignée de 8 px redimensionne le panneau
à la souris et au clavier, avec sauvegarde en fin de geste. Replier la sidebar
conserve désormais la table active au lieu de renvoyer à la console.
Les repères de confidentialité doivent refléter le fournisseur
réel lorsqu'il sera configuré. Aucun niveau Cloud fictif n'est affiché.

Les corrections de maquette relevées dans [FIGMA-HANDOFF](FIGMA-HANDOFF.md)
précisent aussi la sélection de reprise par cases, maintenant raccordée aux brouillons SQL persistés.
Les deux préréglages de lecture sont raccordés à Settings et à la barre des
résultats, avec persistance versionnée du thème, des panneaux, de l'indicateur
NULL et du groupement numérique selon ADR-0013. Les erreurs de sauvegarde sont
visibles et les écritures anciennes ne remplacent pas les nouvelles.
La fermeture de la dernière fenêtre attend les écritures engagées ; les autres
chemins d'arrêt natifs et le délai du hook GPUI restent à recetter.
Les cellules absentes utilisent maintenant `∅ NULL` ; les cellules de grille
reprennent Geist 13 px du nœud `190:1652`, ou 14 px en lecture confortable.
Les en-têtes reprennent la ligne unique du nœud `190:1639`. Le prototype Figma ne valide ni la
persistance ni les interactions du produit.

Le backend de la bibliothèque Figma `47:7638` possède maintenant les commandes
paginées de documents et d'historique, la lecture séparée du texte complet,
les brouillons et copies nommées versionnés, la fermeture avec abandon et les
marqueurs de suppression (ADR-0014, migration 5). La réouverture d'un résultat
retenu contrôle sa connexion d'origine sans driver ni nouvelle exécution.
Les recherches locales peuvent être annulées pendant leur parcours SQLite.
La navigation GPUI ouvre maintenant History / Saved queries / Recent results
par la sidebar ou `⌘⇧H`. Les listes paginées, filtres par menus, recherche avec
délai de 250 ms et inspection en lecture seule passent par le bus. Les copies
nommées et de travail se consultent séparément ; cette vue ne remplace pas la
console et ne soumet aucune exécution. Recent results filtre les entrées ayant
une référence de résultat ; cela ne garantit pas que leur tampon est encore
retenu. Les menus de connexion exposent les configurations connues à l'ouverture
et la connexion active ; compléter leurs choix depuis l'historique global reste
nécessaire pour les connexions supprimées ou d'autres workspaces.
Les consoles disposent maintenant de contrôleurs indépendants : `⌘T` ouvre une
session dédiée, `Ctrl+Tab` / `Ctrl+Shift+Tab` changent de console, `⌘W` ferme la
console active après choix explicite si elle porte du SQL ou une opération.
Chaque onglet conserve éditeur, résultat, confirmation, pages et export, y compris
hors écran. La première console est aussi séparée de la session réservée au
catalogue et à l'aperçu. `CloseSession` ferme uniquement la session désignée et
annule sa préparation ou son drainage avant libération (ADR-0015).
Les workspaces de connexion sont conservés dans la fenêtre ; revenir à une
connexion restaure toutes ses consoles. Une copie du brouillon vers une nouvelle
connexion est signalée, sans exécution, et conserve l'original. Les préférences
communes sont réappliquées lorsqu'un workspace retenu redevient visible.
La sauvegarde explicite depuis l'éditeur est raccordée : nom modifiable,
`⌘S`, annulation, distinction entre version acquittée et modifications ultérieures,
et choix Save and close / Discard and close / Cancel. La fermeture d'un document
sauvegardé met aussi à jour son état ouvert dans le store, sans supprimer sa copie
nommée. Un conflit propose une nouvelle identité pour préserver les deux textes.
La dernière fenêtre attend les écritures locales de documents déjà engagées,
comme celles des préférences ; cela ne valide pas tous les chemins natifs d'arrêt.
Les éditions de texte et de nom soumettent maintenant une autosauvegarde locale
par une file bornée : une opération active, le dernier brouillon en attente,
une sauvegarde nommée et une fermeture. Le backend termine la file déjà soumise
même si la vue disparaît. Un titre de travail vide est conservé, tandis que la
sauvegarde nommée exige toujours un nom. Les révisions attendues sont contrôlées
dans la transaction ; un conflit arrête les écritures de ce contrôleur.
La fermeture protège aussi un document dont la première écriture n'a pas encore
commencé. Annuler une fermeture remet son dernier brouillon en attente, sans
nécessiter le retour de la vue (ADR-0016).
La bibliothèque ouvre maintenant une copie SQL sur la connexion explicitement
nommée par l'action, sans exécution. Sur la connexion d'origine, Resume working
query reprend le texte de travail, sa copie nommée et sa révision ; si le
document est déjà ouvert, sa console et ses modifications sont conservées.
Pour éditer l'original sur une autre connexion, il faut d'abord sélectionner
cette connexion ; la copie sur la connexion courante reste disponible.
Les entrées nécessitant une réconciliation n'offrent pas d'ouverture éditable.
La reprise sélective des brouillons SQL apparaît désormais au démarrage lorsque
des copies de travail ouvertes existent. La sélection est paginée ; les corps
sont chargés à la demande dans des éditeurs sans session. Ils restent modifiables
et sauvegardables hors ligne. Le choix explicite d'une connexion transfère le
même éditeur et sa file d'écriture ; une autre connexion crée une copie. Aucune
requête ne part pendant la restauration ni lors de ce raccordement.
Les positions des onglets d'objet, la qualification persistante d'un arrêt
anormal et la provenance persistante des textes d'agents restent à réaliser.
La bibliothèque ouvre maintenant les tampons retenus sans session ni SQL, avec
lecture des pages IPC et export du même résultat. Un résultat tronqué, incomplet
ou incertain ne devient pas exportable. Une référence expirée affiche une
indisponibilité sans proposer de rejeu implicite. Le retour à la bibliothèque
laisse un export engagé continuer ; la fermeture du résultat exige sa fin ou
son annulation.
Le registre évince les résultats sans lecteur selon ADR-0017 : au plus 16,
256 Mio résidents/cache et 1 Gio de débordement pour cette catégorie. Les vues
et exports restent protégés par leurs références. Le contrôle s'effectue après
exécution et toutes les secondes dans le backend ; il ne plafonne pas la mémoire
de toutes les vues actives. Le message de récupération annonce donc des
copies disponibles, sans prétendre avoir détecté un crash. La recette
visuelle native de cette bibliothèque reste à effectuer.

## D'où viennent ces phases

### Intégration complète de la maquette — travail engagé le 2026-09-10

La demande porte sur les parcours frontend **et** leur comportement backend.
Le relevé Figma en direct confirme la structure de `190:1163`, avec les régions
`190:1543`, `190:1558`, `190:1588` et `190:1860`. Les modifications locales
antérieures de ce document, de UX-SPEC et de FIGMA-HANDOFF sont conservées.

Ordre d'intégration et critères de validation :

1. **Workbench table/console** (`oxyn-app`, `oxyn-ui`) : cadre dense, contexte
   permanent, navigation clavier, largeur compacte, aide de lecture seule,
   focus sûr de confirmation et export de l'aperçu par `Command::Export`.
   Les exports de console et de table gardent des identités distinctes.
2. **Exploration et résultats** (`oxyn-catalog`, `oxyn-data`, `oxyn-exec`, UI) :
   sous-onglets réels selon les capacités, inspecteur, colonnes, lecture des
   pages débordées hors thread UI et borne du cache. Aucune requête au resize.
3. **Travail local** (`oxyn-store`, `oxyn-app`, UI) : historique, documents et
   consoles, préférences persistées, restauration sélective hors ligne.
   Toute précision de format persistant nécessite un ADR avant implémentation.
4. **Connexions et workspace IA** : intégrer les écrans aux contrats de
   connexion et de confidentialité existants, avec états vides, annulation,
   refus et revue du contexte réellement transmis.
5. **Recette complète** : `cargo fmt --all`, `make qualite`, interactions GPUI
   et SQLite isolé, captures réelles aux largeurs de référence et dans les deux
   thèmes, puis mesures conformément à PERFORMANCE.

Les parcours étiquetés phase 4 dans la maquette respectent la condition
d'entrée de cette phase ; une planche ne rend pas une capacité disponible.
La fin exige des preuves pour les parcours intégrés ; une compilation ou une
capture Figma seule ne permet pas de déclarer cette intégration achevée.

Validation du premier lot : `make qualite` a passé le format, Clippy, les tests
et la documentation après correction de trois constructeurs du catalogue
(`with_default`, `with_system`, `with_inferred`). Les tests couvrent le focus
sûr, la conservation des brouillons au redimensionnement, les réponses tardives,
la borne d'aperçu et l'export CSV limité à cet aperçu. Les tests PostgreSQL
nécessitant un serveur restent ignorés ; les mesures de performance restent
absentes. La recette native a ouvert la base SQLite temporaire, son catalogue
et 200 lignes, puis basculé entre les thèmes. L'utilisateur utilisait aussi son ordinateur pendant cette recette : les
interférences de fenêtre ne sont pas des défauts de l'application. La recette
native compacte reste non concluante ; seul le test GPUI prouve ce trajet.
Les interactions de bureau ont été arrêtées à sa remarque ; la suite du travail
se poursuit sans manipuler ses fenêtres.
L'utilisateur a également confirmé le fonctionnement de l'export dans
l'application le 2026-09-10 ; le format et le fichier précis n'ont pas été
précisés. Cette validation manuelle complète les tests CSV, sans prouver les
autres formats ni tous les cas d'interruption.
Les dernières retouches de présentation doivent être reprises dans la recette
visuelle finale de l'intégration complète.

Validation du lot de relecture : test GPUI + SQLite sur un résultat de
100 000 lignes, avec budget de 512 Kio pour forcer le débordement. Atteindre
une page débordée la charge par le bus sans nouvelle exécution SQL. Les tests
vérifient aussi l'appartenance à la connexion pour humain et agent, le journal
sans cellules, l'annulation, le refus des pages trop grosses et des réponses
obsolètes, et le partage du budget de rétention. `make qualite` passe avec
1 268 tests réussis et 18 ignorés. Cela ne mesure ni la RSS ni les budgets de
trame et ne remplace pas une recette native du défilement.

Validation du lot inspecteur/colonnes : `make qualite` passe avec 1 274 tests
réussis et 18 ignorés. Les tests GPUI/SQLite parcourent une valeur réelle de
30 000 octets au clavier, annulent l'inspection lors du remplacement du résultat,
et vérifient la conservation de la préférence large à 1024 px. Les tests de
formatage reconstruisent un texte Unicode long par pages bornées sans omission
ni répétition ; l'absence, le texte vide et `NULL` restent distincts. Les tests
vérifient aussi l'index Arrow des colonnes masquées et l'appartenance de la
valeur à sa connexion pour humain et agent. La recette visuelle native et le
redimensionnement par poignée de l'inspecteur restent à effectuer.

Validation du lot résultats conservés : `make qualite` passe avec 1 331 tests
réussis, aucun échec et 18 ignorés. Le test GPUI rouvre un résultat réel de
100 000 lignes, vérifie le partage du tampon original, charge sa dernière page
IPC et l'exporte pendant que la bibliothèque est de nouveau affichée. Aucun
événement d'exécution supplémentaire n'est observé. Les tests couvrent aussi
l'indisponibilité d'une référence expirée et le refus d'export d'un résultat
tronqué. Le registre est testé sur l'éviction des plus anciens résultats sans
lecteur, la conservation d'une référence active et les limites de mémoire et
de débordement. Ces contrôles ne prouvent pas la RSS globale ni les budgets de
trame des vues ouvertes ; leur recette native reste à effectuer.

Validation de la restauration SQL hors ligne : `make qualite` passe avec
1 326 tests réussis, aucun échec et 18 ignorés. Les tests GPUI livrent la touche
Espace aux cases de reprise, restaurent uniquement les documents choisis,
modifient leur texte sans session et relisent leur autosauvegarde. `⌘Entrée`
ne démarre aucune exécution hors ligne. Sauvegarder puis fermer fonctionne sans
connexion. Un autre test transmet le même éditeur restauré à une connexion
explicitement ouverte, sans événement d'exécution. Le glyphe de case cochée
provient du nœud Figma `232:9100`, est embarqué et conserve ses deux couleurs.
Ces tests ne valident ni les pixels natifs, ni la récupération d'onglets d'objet,
ni la détection d'un crash ou l'inspection réelle d'une écriture interrompue.

Validation de l'autosauvegarde : `make qualite` passe avec 1 323 tests réussis,
aucun échec et 18 ignorés. Un runtime contrôlé vérifie que 200 soumissions ne
conservent que le dernier brouillon et la sauvegarde explicite intermédiaire ;
après disparition des receivers, les versions 200 et 50 sont bien relues dans
leurs colonnes respectives. Les tests couvrent deux écrivains concurrents,
le marqueur d'une fermeture avant première écriture, une fermeture annulée dont
le brouillon est repris sans vue, et le refus de mutation sous une révision
attendue périmée. Le harnais GPUI vérifie récupération du texte sans sauvegarde
nommée, conservation de la copie nommée lors d'une édition suivante, nom de
travail vide et abandon d'une console autosauvegardée. Le parcours de sélection
et restauration au démarrage, la recette native et ses chemins de fermeture
restent à valider.

Validation du lot ouverture depuis la bibliothèque : `make qualite` passe avec
1 316 tests réussis, aucun échec et 18 ignorés. Les tests GPUI vérifient l'ouverture
d'une copie sans exécuter ni remplacer le brouillon courant, le refus d'ouverture
éditable d'une entrée ambiguë, la reprise d'un texte de travail sous son identité
et son langage conservés, puis le retour à sa console déjà modifiée. Une copie
vers une autre connexion garde l'original intact. Les conflits de sauvegarde à
révision égale et à révision supérieure conduisent à une nouvelle identité.
Les titres hérités restent lisibles jusqu'à 4 Kio ; au-delà, la lecture est
refusée avant matérialisation Rust, sans changer la borne de 256 octets des
nouvelles écritures. Les validations natives et la reprise automatique ne sont
pas prouvées par ce lot.

Validation du lot sauvegarde explicite : `make qualite` passe avec 1 312 tests
réussis, aucun échec et 18 ignorés. Les nouveaux tests parcourent nom modifiable,
`⌘S`, édition après sauvegarde, retour à la version sauvegardée puis annulation
de cette édition, abandon préservant la copie nommée et Save and close attendu
jusqu'au stockage. Une annulation de sauvegarde suivie d'une réponse tardive ne
ferme pas la console. Une version concurrente est conservée lorsque la console
sauve sous une nouvelle identité. Les tests d'entrée couvrent une composition
Unicode refusée sans altérer la valeur précédente et la lecture seule pendant
la fermeture. Douze sauvegardes dont les receivers sont abandonnés conservent la
dernière version après attente du backend. Cette preuve concerne les commandes
soumises ; elle ne prouve ni autosauvegarde à chaque frappe, ni restauration au
démarrage, ni tous les chemins de fermeture natifs.

Validation du lot consoles indépendantes : `make qualite` passe avec 1 306 tests
réussis, aucun échec et 18 ignorés. Les événements GPUI vérifient deux consoles
ayant des identités de session et de résultat distinctes, leurs réponses hors
écran, l'export du premier résultat pendant que le second onglet est visible,
et le choix Annuler / Abandonner avant fermeture. Le retour à une connexion
retrouve ses deux consoles et leurs textes, sans exécution. Les tests SQLite
vérifient une transaction non validée, sa fin par fermeture d'une seule session,
et la disponibilité de la console voisine et de la session de catalogue.
Un driver contrôlé vérifie la fermeture pendant préparation et drainage pour
humain et agent, avec refus d'une mauvaise connexion. Le contrat SQLite couvre
aussi les profils en mémoire séparés, la copie partagée entre leurs sessions,
la lecture seule non levable par les limites de l'appelant, et la destruction
après fermeture de la dernière session. Les relectures locales ont couvert la
corrélation des réponses, les confirmations tardives, les ouvertures annulées,
la frontière SQLite et l'absence d'I/O dans le rendu. La recette native et les
mesures de performance restent ouvertes ; le test sans GPU ne les remplace pas.

Validation de la consultation GPUI de la bibliothèque : `make qualite` passe
avec 1 300 tests réussis, aucun échec et 18 ignorés. Les tests livrent des
événements clavier et souris aux menus, à l'éditeur en lecture seule et à la
navigation `⌘⇧H` / `⌘J`. Ils vérifient la copie nommée face au brouillon, les
réponses obsolètes, l'annulation d'une inspection, le refus de saisie et de
composition native, et l'absence d'événement d'exécution pendant la consultation.
Le texte et l'identité du résultat de la console restent inchangés. La sélection
d'un menu n'est appliquée qu'à validation ; Escape et un clic extérieur ferment
le menu sans changer le filtre. Le test des références récentes vérifie que leur
filtrage précède la pagination. Les lignes et en-têtes reprennent les 40 px de
`47:7881`, les filtres étiquetés les 74 px de `47:7868`, et le panneau inférieur
les 238 px de `47:7937`. Ces constats de source et tests sans GPU ne prouvent
pas la fidélité des pixels ni le budget de trame natif.

Validation du backend bibliothèque : `make qualite` passe avec 1 294 tests
réussis, aucun échec et 18 ignorés. Les nouveaux cas couvrent la migration des
anciennes copies nommées, l'ordre des sauvegardes, les conflits de révision,
l'abandon et les marqueurs de suppression, la portée du workspace, les filtres
littéraux et la pagination, le refus d'ouvrir un texte tronqué, les écritures
historiques à réconcilier et la réouverture locale d'un tampon sans driver.
L'annulation est exercée pendant un parcours SQLite et une transaction d'écriture,
avec vérification du rollback et du retrait du handler avant l'opération suivante.
La relecture locale des invariants et de la frontière de sérialisation a conduit
à protéger aussi les chemins de sauvegarde/suppression historiques et la lecture
d'une copie nommée illisible. Cette validation porte sur le backend ; les actions d'édition de la
bibliothèque et la reprise restent ouvertes, au-delà de la consultation GPUI
validée séparément.

Validation du lot préférences/panneaux : `make qualite` passe avec 1 282 tests
réussis et 18 ignorés. Les tests couvrent la réouverture du SQLite local, les
écritures reçues dans le désordre, les conflits et payloads illisibles, le refus
d'un agent et la bonne cible de workspace. Le harnais GPUI vérifie les lignes
de 24/28 px, l'indépendance vis-à-vis du thème et du viewport, la poignée de
8 px par événements souris livrés, son clavier et la conservation de la table
active au repli. Trente sauvegardes dont les receivers UI sont abandonnés
conservent la dernière révision après attente. Les chemins d'arrêt natifs hors
fermeture de la dernière fenêtre, la recette visuelle et les mesures de
performance restent ouverts.

Validation du lot contraintes PostgreSQL : `make qualite` passe avec
1 335 tests réussis et 20 ignorés. Deux tests supplémentaires ont été exécutés
sur une instance PostgreSQL 17.11 jetable, initialisée pour ce lot puis arrêtée :
clés composées, noms atypiques, CHECK, unicité, clés étrangères, NOT NULL et
refus des dépassements de limites. Les tests de cache/bus couvrent la lecture
explicite, les capacités absentes, la non-publication après annulation et la
compatibilité des anciens JSON. Le test GPUI vérifie sélection et copie sans
modifier ni exécuter le brouillon. La fidélité des pixels, les budgets de trame
et l'annulation d'une introspection déjà engagée côté serveur ne sont pas
prouvés par ces nouveaux tests. Les avertissements préexistants du socle
(`paths` des tests), l'absence de nextest et les deux avertissements Rust sur
des dépendances amont restent signalés par la porte.

Validation du lot contraintes SQLite et statut de validation : `make qualite`
passe avec 1 346 tests réussis et 21 ignorés. Les nouveaux tests SQLite portent
sur le moteur embarqué : noms atypiques, clauses conservées, contraintes
adjacentes sans virgule, absence/vide, tables virtuelles, limites et statut
inconnu malgré un CHECK déclaré. Le test GPUI de métadonnées traverse le bus et
le worker SQLite et garde le brouillon intact. Trois tests PostgreSQL sont
exécutés séparément sur une instance jetable puis arrêtée, dont la transition
réelle `NOT VALID` → `VALIDATE CONSTRAINT`. Le test de l'ancien JSON conserve
un statut inconnu. Figma `229:7998` a été relu pour les colonnes et proportions ;
cette lecture ne prouve pas les pixels du rendu natif. Les avertissements
préexistants de la porte restent identiques au lot précédent.

Validation du lot relations entrantes : `make qualite` passe avec 1 353 tests
réussis et 22 ignorés. Les tests SQLite couvrent sens des liens, ordre composite,
références implicites, séparation des espaces de noms, comparaisons d'affinité
et de collation, index partiels/d'expression et refus des listes incomplètes ou
trop grandes. Les tests du cache et du bus vérifient indépendance des sens,
capacités et non-publication après annulation. Le test GPUI charge les liens,
ouvre au clavier une source qui n'était pas dans le catalogue, reçoit son
aperçu réel et conserve le brouillon intact. Le test PostgreSQL exécuté sur
instance jetable couvre les références entre schémas, les colonnes INCLUDE et
les comparaisons non établies ; les trois essais de contraintes ont aussi été
relancés pendant ce lot. L'instance a été arrêtée après les essais. Les pixels
natifs et les budgets de trame restent à vérifier.

Le panneau DDL en lecture seule (`193:2433`, repris dans `229:7749`), Copy DDL
et Open DDL in console sont raccordés au bus (ADR-0018). La copie crée une
console distincte sans exécution. SQLite fournit les déclarations stockées des
tables/vues, index et triggers, y compris les tables virtuelles. PostgreSQL
reconstruit tables ordinaires, racines partitionnées, vues, vues matérialisées
et séquences ; les séquences détenues, contraintes, index, règles, triggers
utilisateur et politiques RLS sont inclus. La portée exclut données, privilèges,
commentaires et dépendances externes, et ses notes restent visibles.
Les enfants de partition sont maintenant reconstruits avec `PARTITION OF`,
parent qualifié, borne native et options locales. Les sous-partitions conservent
leur `PARTITION BY` ; les contraintes, index et triggers clonés du parent ne
sont pas recréés en double. Les fixtures vérifient le lien parent et la borne
après réapplication du DDL. Sur PostgreSQL 12, une partition portant des
triggers utilisateur reste refusée faute de métadonnée native de filiation ;
aucune comparaison heuristique ne décide d'omettre un trigger.
Les objets PostgreSQL temporaires, hérités, typés ou
étrangers, ainsi que les NOT NULL non validés, sont encore refusés : leur
raccordement reste à réaliser. La persistance de la largeur DDL entre
lancements, l'alignement visuel natif complet et les mesures de trame restent
également ouverts.

Validation du lot DDL : `make qualite` passe avec 1 365 tests réussis et
25 ignorés. Les tests GPUI vérifient le refus de saisie/exécution dans l'aperçu,
la copie exacte, le clic réel ouvrant une console distincte sans exécution,
l'annulation en quittant le DDL et la poignée souris/clavier sans nouvelle
lecture lors du redimensionnement. Les tests SQLite recréent table, index,
trigger, vue et table virtuelle dans des bases temporaires. Trois tests
PostgreSQL exécutés séparément sur une instance jetable vérifient identity,
serial, colonnes générées, contraintes, index, états de triggers, RLS, vues,
séquences et racines partitionnées ; le cluster a été arrêté. Le DTO, la
publication du bus et l'éviction des définitions, même invalidées, sont testés.
Les avertissements préexistants (socle `paths`, absence de nextest, dépendances
amont) restent identiques ; cette porte ne remplace pas une recette de pixels.

La portée de Run est maintenant alignée sur l'aide de la console : sélection
explicite ou instruction courante. Le helper `oxyn_query::current_statement`
préserve les corps composés et refuse les frontières ambiguës. Le classificateur
existant reste inchangé. Le texte périmé de la sidebar prétendant que
l'historique était indisponible a été retiré.

Parcours importants encore à raccorder, vérifiés dans le code :

| Parcours | Travail restant |
|---|---|
| Accueil : recette native | La superposition est corrigée et testée sur les rectangles rendus ; l'observer à l'écran, à plusieurs largeurs et dans les deux thèmes, reste à faire |
| Aperçu de table | Tranché par [ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md) ; reste à implémenter : `PreviewSort` / `PreviewFilter` dans la commande, les capacités `PREVIEW_SORT` et `PREVIEW_FILTER`, la traduction citée et liée par les drivers, et la page suivante quand l'ordre est déterministe |
| Reprise de session | Marqueur fiable d'arrêt propre/anormal et restauration des onglets objets |
| Bibliothèque inter-workspaces | Filtres incluant les connexions historiques supprimées ou extérieures au workspace courant |
| Workspace IA | Configuration, entrée conditionnelle, confidentialité par connexion, propositions par le bus et provenance persistante des documents. **À corriger dans ce lot** : `ToolOutcome::Failed` (`crates/oxyn-ai/src/runtime.rs`) porte le message d'erreur du serveur dans la conversation sans passer par le point de passage unique, donc sans que le niveau de confidentialité soit consulté ([I-04](../CLAUDE.md#i-04)). Le chemin n'est pas atteignable aujourd'hui — `oxyn-ai` n'est encore la dépendance d'aucune crate — mais un message primaire PostgreSQL peut recopier une valeur de ligne, et la protection posée à la frontière driver ne couvre pas ce cas : une instruction composée par un agent ne porte pas de valeur liée, donc son message part entier. La rédaction des erreurs selon le niveau est justement listée comme non tranchée dans [AI-PROVIDERS](AI-PROVIDERS.md) |
| Recette produit | Rendu natif complet, accessibilité et mesures de performance, distincts des tests GPUI sans GPU |

Validation du lot contexte de session : `make qualite` passe avec 1 450 tests
réussis, aucun échec et 37 ignorés. Le sélecteur de `191:2003` est raccordé,
conditionnel à la capacité `SESSION_CONTEXT` : sur un moteur qui ne la déclare
pas — SQLite — il est **absent**, pas grisé, et un test l'ancre. Ses cinq états
sont distincts, y compris l'initial, qui dit que le serveur a placé la session
plutôt que de nommer un schéma qu'Oxyn n'a pas demandé. Rien n'y est optimiste :
l'affichage montre ce que la session rapporte.

Deux faits ont modifié la conception en cours de route et sont consignés dans
[ADR-0019](adr/0019-contexte-de-session.md). D'abord `oxyn-core` ne peut pas
nommer `CatalogPath` : la commande transporte deux `Option<String>`, comme
`PreviewRelation`. Ensuite, une session PostgreSQL est un bassin de quatre
connexions et `search_path` est un état **par connexion** : le `SET` est donc
posé à chaque exécution, sur la connexion que cette exécution emprunte, et
défait avant qu'elle reparte au bassin. Cette seconde moitié n'était pas prévue
et vient d'une relecture : `pg_get_indexdef`, `pg_get_constraintdef`,
`pg_get_expr` et `format_type` rendent leur texte **relativement au
`search_path`**, si bien qu'un même objet aurait pu être décrit différemment
d'une lecture à l'autre. Un chemin d'erreur qui ne peut pas défaire le `SET`
ferme la connexion plutôt que de la rendre.

Les tests PostgreSQL sur instance jetable couvrent la cohérence à travers le
bassin — quatre curseurs concurrents, quatre pids distincts relevés par
`pg_backend_pid()` dans la requête elle-même —, le retour au défaut, le refus
d'un schéma absent et d'une autre base, deux schémas réellement nommés
`oxyn_ctx"weird` et `oxyn_ctx.dotted`, la lecture seule préservée, et le fait
que le serveur atteste lui-même, par `pg_stat_activity.query`, que le texte
soumis n'a pas été réécrit. Le test d'introspection force l'emprunt d'une
connexion recyclée en immobilisant trois curseurs sur un bassin de quatre. Le
cluster a été arrêté après chaque campagne. La preuve par l'échec — désactiver
la remise au défaut pour voir le test rougir — n'a pas été faite ; la
sensibilité des rendus au `search_path` a été vérifiée séparément en `psql` sur
17.11 et le tableau des deux formes figure dans le commentaire du test.

Validation du lot accueil / Explain / valeurs liées : `make qualite` passait avec
1 431 tests réussis, aucun échec et 27 ignorés.

La superposition signalée le 10 septembre avait deux causes distinctes, toutes
deux corrigées. Les actions de l'accueil étaient dessinées par `oxyn-app` en
calques `absolute()` posés à 20 px du haut, exactement sur l'en-tête de 56 px du
formulaire ; elles appartiennent maintenant à sa barre de titre, qui partage une
seule rangée entre la marque et les actions. Le formulaire dessinait aussi deux
marques — `logo(theme.mode)` et `icon(IconName::Logo)` — alors que la migration
consignée dans `assets/ui/README.md` prévoyait le retrait de la seconde ; la
variante `IconName::Logo` n'avait plus d'autre appelant et a été supprimée. Le
test vérifie les rectangles rendus : la marque et chaque action ne se recoupent
pas, et les actions restent à droite. Une action sans objet est absente plutôt
qu'affichée inerte. La garde de `ConnectionForm::on_key` applique désormais ce
que son commentaire annonçait, ce qui rend la barre atteignable au clavier.

`ParameterEditor` existait sans être instancié nulle part. Il appartient
maintenant à chaque `QueryConsole` : valeurs isolées par console, éphémères,
transmises par `ExecRequest::params` et jamais insérées dans le texte SQL. Le
parsing est descendu dans `oxyn-core`, seul endroit où `ParameterType` existe
désormais ; UUID, dates, heures, horodatages et JSON sont réellement construits,
et les formats acceptés ont été établis par des tests avant d'être documentés.
Un horodatage sans décalage est refusé plutôt qu'interprété en UTC en silence.
La validation décimale ne passe plus par `f64`, qui acceptait `1e400`.
Les tests couvrent une valeur hostile réellement liée sur une session SQLite,
l'indépendance de deux consoles, et une valeur non convertible qui arrête la
soumission sans que le message répète la saisie.

La barre reprend le groupe `273:37036` : Run, Stop et Explain sont trois
contrôles, celui qui n'a rien à faire étant estompé **et** inerte, vérifié par
un test qui clique les deux. Le contrôle `Parameters` suit les 144 px de
`191:1993`.

La fuite relevée par la relecture est corrigée à la frontière driver. Quand une
instruction porte au moins une valeur liée de l'appelant, le message primaire du
serveur n'est plus propagé : PostgreSQL rend un message protégé suivi de son
SQLSTATE, SQLite le code de résultat étendu et son libellé canonique. L'erreur
d'origine n'est pas conservée dans le type, pour qu'aucun `Debug` ne la
ré-expose. La classe d'erreur, l'annulation et le refus de rejeu d'une erreur
ambiguë sont préservés, et un test le vérifie. Deux tests PostgreSQL exécutés
sur l'instance jetable — un cast invalide de `$1` et un déclencheur plpgsql qui
recopie la valeur — puis le cluster arrêté. Le test symétrique **sans** valeur
liée montre que le message du moteur arrive entier : c'est lui qui prouve que le
premier teste quelque chose. Le message de l'encodeur `sqlx` n'est plus
interpolé dans `bind_params`. Les erreurs de transport PostgreSQL conservent
leur texte : `sqlx` les compose sans y verser les arguments, et les retenir
coûterait un diagnostic sans rien protéger.

Ces preuves ne valent pas recette de pixels : le rendu natif de l'accueil
corrigé et de la barre n'a pas été observé à l'écran, et les budgets de trame
restent non mesurés. Les avertissements préexistants de la porte (`paths` des
tests du socle, absence de nextest, deux dépendances amont) sont inchangés.

Validation du lot partitions/instruction courante : `make qualite` passe avec
1 377 tests réussis et 25 ignorés. Les tests de résolution couvrent UTF-8,
dollar-quotes, commentaires, séparateurs, SQL incomplet et corps de triggers.
La relecture a détecté puis fait corriger une fausse fermeture sur une colonne
SQLite nommée END : la fermeture doit être à une frontière d'instruction, et
le préfixe CREATE TRIGGER est strict. Le test GPUI vérifie une revue portant
sur tout le trigger, jamais sur son UPDATE interne, et l'absence d'exécution
de l'INSERT suivant. Un autre test montre que les écritures voisines ne partent
qu'après sélection explicite du script. Trois fixtures PostgreSQL réelles ont
été réexécutées pour les partitions, RLS et autres définitions ; le cluster
jetable a été arrêté. Le fallback de métadonnées PG12 est vérifié structurellement,
mais n'a pas été exécuté sur un serveur PG12. Les réserves natives et de
performance ainsi que les avertissements préexistants de la porte restent
ouverts.

Écarts documentaires relevés : le phasage du §11 d'ARCHITECTURE diffère de
celui du présent document, qui fait autorité sur l'ordre des phases ;
la formulation périmée de PERFORMANCE affirmant l'absence de code a été
corrigée. Les budgets restent à mesurer.

Les ADR y font référence sans les définir. Les jalons ci-dessous marqués
**[ADR]** sont des contraintes déjà tranchées, extraites des ADR :

| Contrainte | Source |
|---|---|
| L'UI conditionnelle aux capacités est une discipline **dès la phase 0** | [ADR-0003](adr/0003-driver-capabilities.md) |
| La bascule vers egui reste possible **jusqu'à la fin de la phase 1** | [ADR-0001](adr/0001-ui-toolkit.md) |
| Aucun driver des **phases 0 à 3** n'a besoin du sidecar | [ADR-0007](adr/0007-driver-sidecar.md) |
| Les plugins WASM arrivent en **phase 4**, après 6+ drivers natifs | [ADR-0005](adr/0005-wasm-plugins.md) |
| Le sidecar arrive en **phase 4** | [ADR-0007](adr/0007-driver-sidecar.md) |

Le reste — le contenu de chaque phase et sa porte de sortie — est **proposé** et
demande validation. Il n'a pas valeur d'autorité tant que le premier commit de
code ne l'a pas éprouvé.

## Phase 0 — Charpente

Le but n'est pas d'afficher quelque chose, c'est de rendre les décisions
exécutables. Une phase 0 bâclée se paie sur toutes les suivantes.

- workspace Cargo, `rust-toolchain.toml` épinglé ([ADR-0008](adr/0008-chaine-outils-rust.md)) ;
- `oxyn-core`, `oxyn-core` avec `Command`, `Actor`, `PolicyGate` ([ADR-0004](adr/0004-command-bus.md)) ;
- `oxyn-driver` : `Driver`, `Session`, `Capabilities`, `QueryLanguage` ([ADR-0003](adr/0003-driver-capabilities.md)) ;
- `oxyn-data` : `ResultBuffer` Arrow avec débordement disque ([ADR-0002](adr/0002-arrow-result-model.md)) ;
- un driver de référence : **SQLite**, en processus, sans réseau ;
- la commande de qualité `make qualite` qui passe.

**Porte de sortie** : une `Command` de lecture émise par un test traverse le
`PolicyGate`, atteint SQLite, et revient en `RecordBatch`. Sans interface.

> Le choix de SQLite en premier n'est pas un raccourci : c'est le seul driver
> qui isole le contrat de tout ce qui vient du réseau. Un bug à ce stade est un
> bug de contrat, pas de protocole.

## Phase 1 — Premier trajet visible

- `oxyn-ui` : fenêtre, éditeur de requête, grille virtualisée sur `RecordBatch` ;
- `oxyn-app` : câblage ;
- annulation de bout en bout, y compris côté serveur ;
- les cinq états de vue ([UX-SPEC](UX-SPEC.md#états-dune-vue)).

**Porte de sortie** : les budgets de [PERFORMANCE](PERFORMANCE.md) sont
**mesurés**, pas supposés — c'est la première campagne de mesure, et elle
confirme ou amende les budgets par un ADR.

> **[ADR]** C'est la dernière phase où la bascule vers egui reste une réécriture
> de deux crates ([ADR-0001](adr/0001-ui-toolkit.md)). Après, le coût change de
> nature. Si GPUI doit être remis en cause, c'est ici.

## Phase 2 — Les protocoles qui comptent

- `oxyn-driver-postgres`, `oxyn-driver-mysql` ;
- `oxyn-catalog` : introspection, cache, arborescence ;
- marquage d'environnement des connexions et stockage au trousseau
  ([SECURITY](SECURITY.md)).

**Porte de sortie** : la [liste de contrôle driver](../.claude/checklists/revue-driver.md)
passe intégralement sur les trois drivers, annulation côté serveur comprise.

## Phase 3 — Le workspace IA

- `oxyn-ai` : fournisseurs local et distant, point de passage unique ;
- niveaux de confidentialité par connexion ([ADR-0006](adr/0006-ai-privacy-tiers.md)) ;
- agents proposant des `Command` portant `Actor::Agent`.

**Porte de sortie** : Oxyn reste un client complet, sans dégradation, avec zéro
fournisseur configuré — vérifié par un test, pas par conviction.

## Phase 4 — Extension et isolation

**[ADR]** Ce qui a été délibérément reporté ici :

- `oxyn-plugin` : hôte wasmtime, interfaces WIT ([ADR-0005](adr/0005-wasm-plugins.md)) ;
- `oxyn-driverd` : sidecar pour Oracle, Couchbase, SDK cloud ([ADR-0007](adr/0007-driver-sidecar.md)) ;
- élargissement aux familles NoSQL, vectorielle, graphe, séries temporelles.

**Condition d'entrée**, et non de sortie : au moins six drivers natifs livrés.
Ouvrir une frontière d'extension sur des traits que trop peu d'implémentations
ont éprouvés fige des erreurs qu'il faudra ensuite supporter indéfiniment.

## Ce qui n'a pas sa place ici

Les décisions. Une phase qui a besoin d'un arbitrage écrit un ADR
([`/adr`](../.claude/commands/adr.md)) ; elle ne tranche pas dans ce fichier.
