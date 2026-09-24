# Plan de mise en œuvre

> **Autorité** : l'ordre des phases et la porte de sortie de chacune.
> C'est le seul document qui parle de ce qui **reste à faire** — les documents
> d'autorité décrivent ce qui est décidé.

État au 2026-09-18 : l'interface est l'application Tauri (`apps/desktop` et
`crates/oxyn-desktop`) ; l'interface GPUI a été retirée ce jour-là, voir
[Migration vers l'interface Tauri](#migration-vers-linterface-tauri). Les états
datés qui suivent décrivent le produit à leur date — souvent l'interface GPUI :
ce qu'ils disent fait ne l'est pas forcément dans `apps/desktop`, et la porte de
cette section tient la liste de ce qui y manque.

État au 2026-09-10 : les quinze crates existent, avec une application GPUI,
un formulaire de connexion et un parcours d'exécution SQL. L'intégration UI/UX
reprend la maquette Figma : sidebar repliable, thèmes clair/sombre, Hugeicons et
Geist embarqués, catalogue réel chargé par paliers à travers le command bus.
Les corrections d'interaction souris, de saisie native, de session et
d'annulation sont en place.
Une première campagne de mesure a eu lieu le 2026-09-10 et couvre le **code
pur** : conversion ligne-à-lot du driver SQLite, latence du premier lot, analyse
des requêtes ([PERFORMANCE](PERFORMANCE.md#campagne-de-mesure-du-2026-09-10)).
Elle n'a contredit aucun budget et n'en a amendé aucun. Restent à mesurer, et
aucun ne se mesure avec `criterion` : la trame, le démarrage à froid, le
développement d'un nœud de catalogue en cache, et la RSS — budget du
`ResultBuffer` compris, dont seuls les déclenchements de débordement sont
couverts, par des tests aux budgets minuscules. L'existence du code ne valide
pas à elle seule les portes de sortie ci-dessous.

Le premier workspace propose l'éditeur SQL, l'exploration des métadonnées et
l'aperçu automatique des 200 premières lignes d'une table SQL. Historique visible, requêtes sauvegardées et reprise des brouillons SQL sont
raccordés comme détaillé ci-dessous. L'aperçu accepte maintenant un prédicat
écrit par l'utilisateur, un tri par colonnes et une page suivante lorsque l'ordre
est total ([ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md)).
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
reste en cours : restauration de l'emplacement d'objet, cas DDL PostgreSQL
avancés et menu compact complet restent à intégrer. Les onglets Indexes et Relations lisent maintenant les index et clés étrangères
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
Le menu Columns, au pied des grilles de console et d'aperçu, masque et réaffiche
les colonnes localement par indice Arrow, annonce le nombre de colonnes
visibles et propose `Show all` ; la grille ne virtualise que les colonnes
visibles, la copie les ignore, et l'export conserve toutes les colonnes, ce que
le menu indique. En mode compact, l'inspecteur s'ouvre en superposition
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
chemins d'arrêt natifs et le délai du hook GPUI restaient à recetter — ce hook
a disparu avec l'interface GPUI le 2026-09-18.
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
anormal et la provenance persistante des textes d'agents sont réalisées :
`ObjectLocation` porte l'emplacement restauré, la table `app_sessions`
(migration 6) distingue un arrêt propre d'un plantage par le battement, et la
colonne `provenance` des documents retient qui a écrit un texte.
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

## Migration vers l'interface Tauri

[ADR-0029](adr/0029-interface-tauri-shadcn.md) remplace GPUI. État au 2026-09-15 :
`apps/desktop` et `crates/oxyn-desktop` existent et passent `make qualite` ; l'écran
de connexion, la console SQL (exécution, annulation, approbation, export), la grille
paginée, l'arbre du catalogue et la vue d'objet (Data, Structure) fonctionnent dans
la fenêtre Tauri.

La liste qui restait alors à faire avant de supprimer `oxyn-ui` et `oxyn-app` :

- bascule compacte sous 1 200 px et menu `More` — **en partie** : la bascule
  existe (`COMPACT_BELOW_PX`, `features/workspace/use-compact.ts`), le menu `More`
  et le menu `Actions` non, voir la porte ci-dessous ;
- ~~réglages d'affichage des cellules (`FormatOptions`) et thème clair commutable~~ — fait (`features/settings/`) ;
- ~~restauration des brouillons après arrêt brutal~~ — fait (`features/recovery/`) ;
- campagne de mesure des budgets de [PERFORMANCE](PERFORMANCE.md) **dans la webview**,
  qui est la condition de reconsidération de l'ADR-0029 — **non faite**. Elle
  ouvre des fenêtres et prend la machine sous instrument (trame, démarrage à
  froid, RSS, défilement d'un million de lignes), ce que la consigne « aucune
  fenêtre ouverte sur mon écran » exclut sur le poste de travail : elle demande
  **une session ou une machine dédiée, désignée par l'utilisateur** (décision du
  2026-09-24). Tant qu'elle n'est pas désignée, la campagne ne peut pas démarrer ;
- vérification du rendu sous WebView2 et WebKitGTK — **non faite**.

**Porte de sortie — rouverte le 2026-09-25.** Elle exigeait que chaque parcours
de l'interface GPUI ait son équivalent dans `apps/desktop`, avec ses stories.
Elle a été déclarée franchie le 2026-09-18 (commit `6ecb8ce`), qui a retiré
`oxyn-ui`, `oxyn-app` et la dépendance `gpui` dans un même commit, avec les
cibles `make app` et `make lancer` — `make desktop-dev` lance désormais
l'application, et `.claude/verifier_socle.py` refuse `gpui` partout. Le retrait
tient ; **la parité, non**. L'audit du 2026-09-24, re-vérifié dans le code le
2026-09-25, trouve des parcours que ce plan déclare faits, ou tenus par des tests
GPUI que `6ecb8ce` a supprimés, et qui n'existent pas dans `apps/desktop` :

| Parcours | Ce que le plan en disait | État dans `apps/desktop` au 2026-09-25 |
|---|---|---|
| Menu `Actions` en largeur compacte | Columns, Export et l'inspecteur passent dans `Actions` en compact (état au 2026-09-10) | **absent.** La colonne de droite passe bien en superposition sous 1 200 px (`components/oxyn/workspace-layout.tsx`), mais Columns et Export restent dans la barre, contre [UX-SPEC](UX-SPEC.md) |
| Menu `More` | Indexes, Relations et Constraints y passent en compact ; porte cochée plus haut | **absent.** `object-view-frame.tsx` affiche les onglets à plat, qui défilent à l'horizontale |
| Préférences de panneaux persistées | sidebar large conservée, poignée de l'inspecteur « avec sauvegarde en fin de geste », persistance des panneaux selon ADR-0013 | **backend seul.** `read_preferences` porte `sidebar_collapsed`, `inspector_open`, `inspector_width`, mais l'écran démarre toujours sidebar ouverte, l'aside est figé à 320 px et rien n'est écrit |
| Bibliothèque en lecture seule | History / Saved queries / Recent results, inspection en lecture seule sans exécution | **en partie.** History et Saved existent, paginés, filtrés, recherche à 250 ms ; ouvrir une entrée ouvre une copie dans une console, pas une inspection ; la bibliothèque n'existe que dans un workspace connecté |
| Recent results | une vue dédiée aux résultats retenus ; tenu par `history_and_recent_results_read_the_same_execution_without_replaying_it` | **absent** comme vue. Un résultat retenu se rouvre sans réexécution depuis une ligne d'History (`features/library/retained-result-tab.tsx`) ; aucune commande ne liste les résultats retenus |
| Restauration hors ligne des éditeurs | les brouillons repris restent modifiables et sauvegardables sans session, puis se rattachent à une connexion choisie | **absent.** La reprise sélective existe (`features/recovery/`), mais le texte ne rouvre qu'une fois une connexion choisie, dans un workspace |
| Export d'un résultat retenu | la bibliothèque exporte le tampon retenu sans le rejouer | **absent.** `ExportMenu` n'est monté que dans la console et l'aperçu, pas dans `retained-result-view.tsx` — qui affiche pourtant « Opening, scrolling and exporting do not rerun the query » |
| Panneau DDL latéral | panneau DDL en lecture seule (`193:2433`), redimensionnable | **absent** comme panneau : le DDL est un onglet de la vue d'objet (`RelationDefinition`) |
| `Edit rows…` désactivé et son infobulle | exigé par [UX-SPEC](UX-SPEC.md) dans l'aperçu en lecture seule | **absent** : aucune occurrence dans `apps/desktop` |
| La bibliothèque suit les exécutions | la bibliothèque ouverte se relit après une exécution (lot rafraîchissement automatique, ADR-0022) | **absent.** `LIBRARY_QUERY_KEY` n'est invalidée que par Refresh et les actions de la bibliothèque ; le commentaire « a run invalidates it » n'a pas de code derrière lui |
| Restauration de l'emplacement d'objet | « réalisée (`ObjectLocation`) », tenue par `a_restored_location_and_its_sub_tab_come_back_without_reading_anything` | **absent côté front.** `ObjectLocation` est stocké par `oxyn-store`, mais `ipc/settings.rs` l'exclut délibérément de ce qui traverse l'IPC, et rien ne le relit |

Le menu Columns, lui, est livré (`ef2ada8`, `components/oxyn/columns-menu.tsx`)
dans la console, l'aperçu et le résultat retenu. Chaque lot qui livre un de ces
parcours remet sa ligne à « fait » ici, avec le test ou la story qui le prouve.

Ce qui reste ouvert après le retrait, sans qu'aucun de ces points ne soit tranché
ici :

- **les deux derniers points de la liste ci-dessus.** Ils étaient annoncés
  « avant de supprimer » ; la porte de sortie, elle, ne les exigeait pas, et le
  retrait a eu lieu sans eux. Tant que la campagne n'est pas faite, la condition
  de reconsidération d'ADR-0029 n'a pas été évaluée, et les verdicts de trame, de
  démarrage à froid et de mémoire au repos de
  [PERFORMANCE](PERFORMANCE.md#confrontation-aux-budgets) portent sur
  l'interface retirée ;
- ~~**le statut d'ADR-0009.**~~ — porte `remplacé` depuis le 2026-09-24 ; le
  statut d'ADR-0029 lui-même relève de la
  [question ouverte n° 3](#3-le-statut-des-adr--revue-du-2026-09-24) ;
- **ce que l'interface GPUI faisait et qu'`apps/desktop` ne fait pas**, relevé en
  réalignant [ARCHITECTURE](ARCHITECTURE.md) : le titre de fenêtre qui signalait
  un `--temporary-workspace`, et le sélecteur de fichiers par `⌘O`, ou `⌘N` pour
  une base SQLite à créer — le front n'offre qu'un `Browse…` vers un fichier
  existant. Aucun document d'autorité hors ARCHITECTURE ne les exigeait : les
  porter ou y renoncer est à décider ;
- **`assets/fonts/` et `assets/ui/`**, que seul `oxyn-ui` lisait, n'ont plus de
  lecteur ;
- ~~**des commentaires de code nomment encore `oxyn-app`, `oxyn-ui` ou GPUI**~~ —
  fait le 2026-09-25 : le dernier, dans les tests de `oxyn-core/src/value.rs`,
  renvoie désormais au rendu `∅ NULL` de
  `apps/desktop/src/components/oxyn/cell-value.tsx`.
  `grep -rn -i "gpui\|oxyn-ui\|oxyn_ui\|oxyn-app\|oxyn_app" crates drivers apps/desktop/src Cargo.toml`
  ne rend plus rien.

Les validations datées de ce document qui citent un « test GPUI » ou un test
d'`oxyn-ui` décrivent l'état de leur date : ces tests ont été supprimés avec les
deux crates. Côté front, [I-01](../CLAUDE.md#i-01) est désormais tenu par
`.claude/hooks/code_interdit.py`, qui refuse tout appelant d'`invoke` hors de
`apps/desktop/src/lib/ipc/client.ts`.

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
prouvés par ces nouveaux tests. L'absence de nextest et les deux avertissements
Rust sur des dépendances amont restent signalés par la porte. Le socle, lui, ne
signale plus rien : `make socle` rend 42/42 cas conformes et 0 avertissement.

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
Les avertissements préexistants (absence de nextest, dépendances amont) restent
identiques ; le socle, lui, ne signale plus rien. Cette porte ne remplace pas une
recette de pixels.

La portée de Run est maintenant alignée sur l'aide de la console : sélection
explicite ou instruction courante. Le helper `oxyn_query::current_statement`
préserve les corps composés et refuse les frontières ambiguës. Le classificateur
existant reste inchangé. Le texte périmé de la sidebar prétendant que
l'historique était indisponible a été retiré.

Parcours importants encore à raccorder, vérifiés dans le code :

| Parcours | Travail restant |
|---|---|
| Accueil : recette native | La superposition est corrigée et testée sur les rectangles rendus ; l'observer à l'écran, à plusieurs largeurs et dans les deux thèmes, reste à faire |
| Aperçu de table | Réalisé, selon [ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md) : `PreviewSort` et `PreviewFilter` sont dans la commande, les capacités `PREVIEW_SORT` et `PREVIEW_FILTER` sont déclarées, les deux drivers composent la traduction citée, et la page suivante existe quand l'ordre est déterministe. Une réserve d'arbitrage subsiste sur le tri par défaut — voir les questions ouvertes en fin de document |
| Reprise de session | Le marqueur d'arrêt est en place ([ADR-0021](adr/0021-marqueur-d-arret.md)) : un `⌘Q` ne déclenche plus la reprise, une session sans fermeture au battement vieilli si. La restauration de l'emplacement d'objet qu'[UX-SPEC](UX-SPEC.md#restauration-après-un-arrêt-brutal) promet était réalisée dans l'interface GPUI (`ObjectLocation`) ; `apps/desktop` ne la relit pas — voir la [porte de la migration](#migration-vers-linterface-tauri) |
| Bibliothèque inter-workspaces | Réalisé : les filtres portent les connexions historiques supprimées ou extérieures au workspace courant, par `Command::ListHistoryConnections` |
| Workspace IA | La fuite [I-04](../CLAUDE.md#i-04) est fermée : `ToolOutcome::Failed` porte un rapport aux champs privés dont le seul constructeur exige le niveau de la connexion, et le filtre s'applique **à la construction** — sous `Local`/`Metadata`, le message du serveur n'entre jamais dans la structure. `PrivacyTier` vit désormais sur la `ConnectionConfig`, comme sa documentation l'affirmait déjà. La configuration des fournisseurs, l'entrée conditionnelle `Ask AI` (`190:1549`), les propositions par le bus et la provenance persistante des documents sont réalisées, et `oxyn-ai` est une dépendance déclarée d'`oxyn-app` — d'`oxyn-desktop` depuis le retrait de GPUI |
| Recette produit | Rendu natif complet, accessibilité et mesures de performance, distincts des tests GPUI sans GPU — aujourd'hui des stories Storybook, qui ne remplacent pas davantage la recette |

Validation du lot rafraîchissement automatique : `make qualite` passe avec
1 536 tests réussis, aucun échec et 39 ignorés. Les vues se relisent seules après
une exécution réussie, selon [ADR-0022](adr/0022-rafraichissement-automatique.md) :
un DDL relit l'explorateur, un DDL ou une écriture relit l'aperçu **visible** en
conservant le prédicat, le tri et la page appliqués, et la bibliothèque ouverte
suit. Rien après une erreur, rien sur une autre connexion, rien sur un onglet
caché, et jamais l'instruction de l'utilisateur — c'est la lecture d'aperçu
qu'Oxyn compose qui est réémise. L'invalidation du cache de catalogue après DDL
et la publication de `CatalogUpdated` existaient déjà côté exécuteur ; ce qui
manquait était l'intention dans `Event::Completed` et les abonnés.

Deux constats de ce lot méritent d'être retenus. Le premier est un défaut
préexistant du chemin **manuel** : `refresh_catalog` ignorait une demande de même
portée arrivée pendant une lecture en vol, si bien qu'un DDL exécuté pendant un
`Refresh` laissait l'arbre périmé indéfiniment. Le second porte sur la
bibliothèque : un `reload()` déclenché dans le dos de l'utilisateur reviendrait à
la première page et jetterait le détail ouvert, donc la relecture automatique
s'abstient dès qu'une entrée est sélectionnée, qu'un résultat retenu est affiché
ou qu'on a quitté la première page. La coalescence est un drapeau par vue, effacé
au départ d'une lecture et consommé à son arrivée, et seulement si cette arrivée
a ramené des lignes — sinon une relecture recouvrirait l'erreur. Elle est
vérifiée par mutation.

Ce que ce lot ne prouve pas : la branche `RecvError::Lagged` elle-même, dont la
provocation fiable reviendrait à fabriquer la panne plutôt qu'à l'observer ;
seule la fonction de rattrapage est testée directement. Et la fenêtre entre le
départ d'une lecture et son arrivée au serveur reste ouverte — une écriture
dispatchée juste avant peut s'appliquer juste après, et rien ne la redemande.
C'est inhérent à un rafraîchissement sans horloge partagée ; l'ADR ne promet pas
d'instantané.

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
restent non mesurés. Les avertissements préexistants de la porte (absence de
nextest, deux dépendances amont) sont inchangés ; le socle ne signale plus rien.

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
celui du présent document, qui fait autorité sur l'ordre des phases — le §11
y renvoie depuis le 2026-09-25 ;
la formulation périmée de PERFORMANCE affirmant l'absence de code a été
corrigée. Les budgets restent à mesurer.

Validation du lot restauration d'objet et filtres de bibliothèque :
`make qualite` passe. La restauration rouvre l'onglet d'objet, son chemin
sélectionné et son sous-onglet sans écraser un brouillon. `ListHistoryConnections`
rend la page des connexions **telles que l'historique les a enregistrées**, ce
qui permet de filtrer par une connexion supprimée depuis ou appartenant à un
autre workspace ; chaque entrée dit si le workspace la possède encore. Le champ
de menu porte une `ConnectionId` et non un rang, parce que la liste s'allonge
quand la page arrive — un rang aurait désigné une autre connexion. Un test
traverse la vue au clavier et vérifie **zéro événement d'exécution** : filtrer
n'ouvre aucune session. `deny_unknown_fields` a été retiré de
`WorkspacePreferences` et d'`ObjectLocation` : une version antérieure d'Oxyn
refusait un fichier écrit par une version plus récente et échouait au
démarrage, ce qui est consigné dans ADR-0013.

Validation du lot d'intégration IA : `make qualite` passe avec **1 635 tests
réussis**, sortie vérifiée. Le lot est décrit par
[ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md).

État constaté avant le lot, qui en justifie l'ampleur : `oxyn-ai` (4 480 lignes)
et `oxyn-llm` (5 988 lignes) étaient complets et testés, et **aucune crate ne les
déclarait en dépendance**. `oxyn-exec/src/sink.rs` exposait l'`ExecutorSink` en
disant explicitement que la traduction revenait à `oxyn-app` ; elle n'y était
pas écrite.

Ce qui est tenu, et par quoi :

- **le workspace IA n'existe que configuré** — l'entrée `Ask AI` et le badge de
  confidentialité sont absents tant qu'aucun fournisseur n'est déclaré, et un
  test traverse les quatre couches (l'écran émet, le workspace traduit en
  `Command`, le store écrit, la lecture reclassée revient, l'entrée apparaît
  sans redémarrage). Il rougit dès qu'un maillon saute, vérifié par sabotage ;
- **le classement local/distant n'est jamais persisté** : `Reach` n'a pas de
  colonne et se recalcule à chaque ouverture, sur le pool bloquant. Une réponse
  DNS d'hier appliquée à un envoi d'aujourd'hui est le piège du mandataire que
  AI-PROVIDERS demande d'éviter ;
- **l'asymétrie utilisateur / modèle est portée par le type** : l'observateur de
  conversation reçoit les faits entiers — message du serveur compris — pendant
  que l'invite ne reçoit que ce que `ToolOutcome::from_dispatch` a filtré sous
  le niveau de la connexion. L'écart est **constaté en comparant les deux
  valeurs**, jamais en rejouant la règle du filtre : une règle recopiée diverge
  en silence le jour où l'originale change ;
- **la provenance est écrite de bout en bout**, avec une règle unique dans le
  dépôt : une provenance absente veut dire « rien de neuf à écrire », jamais
  « personne ». Une autosauvegarde ordinaire ne l'efface donc pas. Les deux sens
  ont été vérifiés par sabotage.

Trois défauts trouvés en cours de lot, hors du plan :

1. **`controls.rs` affirmait le faux sur un point bloquant d'accessibilité** —
   que GPUI activerait un contrôle focalisé sur Entrée et Espace. Une sonde le
   réfute, avec un témoin qui compte les touches *reçues* : sans lui, un zéro
   d'activations se lirait aussi bien comme « GPUI n'active pas » que comme
   « les touches ne sont jamais arrivées ». Trois boutons du panneau de
   conversation en étaient inutilisables au clavier. Le commentaire est corrigé
   et le fait figé par un test, qui rougirait aussi si GPUI se mettait à activer
   sur Entrée — ce serait alors une régression de sécurité, un bouton
   d'approbation d'écriture en production ne devant jamais être activable à la
   touche Entrée ([I-02](../CLAUDE.md#i-02)) ;
2. **une classification réduite à un booléen** écrasait la distinction entre
   « distant » et « non résolu », dont l'écran de configuration a besoin — il
   dit « non résolu » plutôt que d'affirmer une mesure qui n'a pas eu lieu. La
   laisser aurait imposé une seconde campagne de résolution DNS et un second
   endroit où les deux réponses peuvent diverger ;
3. **une divergence de langue** : toute l'interface est en anglais, `format_settings`
   en était la seule exception. Tranché pour l'anglais, les deux écrans suivent,
   et `Reach::as_str` cesse de rendre du français dans du code source.

Deux relectures indépendantes ont suivi — invariants et sécurité —, chacune
avec pour consigne de vérifier les affirmations contre le code plutôt que de
croire les commentaires. Elles ont convergé sur un maillon manquant et trouvé
cinq défauts que le lot n'avait pas vus. Tous sont corrigés, et `make qualite`
repasse avec **1 640 tests réussis**.

1. **Le niveau de confidentialité n'était pas persisté.** `ConnectionConfig` le
   portait depuis le début ; la table `connections` n'avait pas la colonne. Toute
   connexion relue repartait donc à `Metadata`, ce qui rendait `Local`
   inatteignable d'une session à l'autre : un réglage pris sur une base client
   était perdu à la fermeture, **sans message**, et le premier `Ask AI` suivant
   envoyait le DDL et les noms de colonnes chez un fournisseur distant. Migration
   8, avec deux replis distincts qui sont la décision : colonne **absente** →
   défaut d'ADR-0006, parce que la ligne a été écrite par un binaire qui ignorait
   ce réglage et que l'utilisateur n'en a donc jamais choisi ; valeur
   **illisible** → `Local`, le plus contraignant, parce qu'un réglage dont le
   sens s'est perdu n'obtient pas le bénéfice du doute. Les deux sens sont
   testés et vérifiés par sabotage.
2. **La provenance était émise puis jetée.** Tout existait — la colonne, le
   `coalesce`, le type, l'événement — et aucun chemin applicatif ne posait jamais
   autre chose que `None`. Le dépôt affirmait donc formellement une
   contre-vérité sur exactement la ligne qu'ADR-0023 existe pour marquer. Le
   maillon est posé : `AiEvent::Started` → `Assistant::provenance` →
   `OpenQuery::Copy` → la console → ses deux écritures. Une copie d'historique,
   elle, n'hérite d'aucune marque — marquer par excès ferait passer pour écrit
   par un agent un texte que l'utilisateur avait écrit lui-même.
3. **Le seul type portant le message serveur non filtré dérivait `Debug`.**
   `DispatchOutcome::Failed` porte `Key (email)=(dupont@example.com)` par
   construction : c'est son objet, et l'utilisateur a le droit de le lire. Mais
   un `Debug` dérivé le rendait recopiable par un `tracing::debug!` ajouté plus
   tard pour diagnostiquer autre chose — le mode de fuite exact que le
   corollaire vérifiable d'I-03 nomme. `Debug` écrit à la main : la classe et la
   **longueur** du message, jamais le message. Le repli de traduction de
   l'adaptateur journalisait de même un rapport entier ; il ne journalise plus
   que l'identifiant de commande.
4. **Une écriture de fournisseur n'était pas attendue à la fermeture.** Un ⌘Q
   dans la seconde suivant un « Save » inscrivait un arrêt **propre** sur un
   travail perdu : au redémarrage, pas de fournisseur, pas d'entrée, une clé
   orpheline au trousseau, et rien nulle part qui l'explique. Les deux commandes
   rejoignent les écritures locales attendues.
5. **Une provenance illisible rendait le document inouvrable.** Ajouter une
   famille de fournisseur ne demande aucune migration : un Oxyn plus récent peut
   écrire, sur un schéma que celui-ci accepte, une marque qu'il ne sait pas lire
   — le piège que `deny_unknown_fields` avait déjà tendu aux préférences
   (ADR-0013). Elle est désormais écartée avec un cri, et **rien n'est détruit** :
   la valeur reste en base, protégée par le `coalesce`, et un binaire qui saura
   la lire la retrouvera. Un test le vérifie sur la colonne brute.
6. **L'identité d'une déclaration était frappée deux fois** — une fois pour la
   référence de trousseau, une fois dans la fabrique — et réconciliée par une
   ligne d'affectation qu'aucun test ne gardait. La supprimer compilait, passait
   la porte, et produisait une déclaration dont la clé était introuvable,
   signalée au premier message et longtemps après un enregistrement annoncé
   réussi. Elle est frappée une fois et passée en argument.

Ce que les deux relectures ont regardé et trouvé sain, et qui vaut d'être
consigné : le chemin d'exécution d'un agent ne comporte aucune seconde API
(I-01, I-07) ; le point de passage unique du contexte n'est pas contournable et
l'observateur ne peut pas refermer la boucle vers une invite (I-04) ; trousseau,
DNS et SQLite passent tous par le pool bloquant (I-05) ; trois barrières
indépendantes empêchent un agent de s'élever — refus d'usurpation d'acteur avant
l'ordonnanceur, refus de politique sur `production` et sur la déclaration d'un
fournisseur, périmètre d'outils qui ne laisse choisir ni la connexion ni le point
d'accès (I-02) ; aucun `unsafe` dans les huit crates.

Réserves de ce lot, explicitement ouvertes : le choix du fournisseur quand
plusieurs sont déclarés suit le premier utilisable sous le niveau, faute de
spécification — un sélecteur est une décision de produit, et ADR-0023 affirme au
présent que l'utilisateur choisit, ce que le code ne fait pas encore ;
l'indisponibilité sur une session sans `Capabilities::SQL` est expliquée plutôt
que masquée, par analogie avec ADR-0003, mais aucun document ne la nomme ; aucune
icône d'assistant n'existe dans `assets/ui` et le glyphe employé est un emprunt ;
aucun écran ne permet encore de **régler** le niveau de confidentialité, qui se
persiste désormais mais ne s'édite pas. Les réserves natives et de performance
restent ouvertes.

Validation d'accessibilité — lot clavier : un recensement des contrôles du dépôt
a montré que **six** boutons étaient atteignables au Tab et inertes, tous pour la
même raison. `oxyn_ui::control` pose `tab_stop`, donc l'atteignabilité, et rien
de plus ; un commentaire de `controls.rs` affirmait que GPUI activait de lui-même
sur Entrée et Espace, ce qui est faux — une sonde le réfute, avec un témoin qui
compte les touches **reçues** pour que le test ne puisse pas passer faute de
touches plutôt que faute d'activation.

Le plus gênant des six est le bouton **Annuler d'une exécution en cours**.
UX-SPEC pose que « l'état *en cours* porte toujours un moyen d'annuler » ; ce
moyen demandait une souris. Deux choses l'avaient rendu invisible : son
commentaire décrivait l'intention et non ce qui était fait, et il n'avait aucun
`debug_selector` — donc aucun test ne pouvait l'atteindre, ni pour son existence
ni pour son clavier. Les cinq autres : les trois boutons du panneau de
conversation, `Add` dans l'éditeur de paramètres, et les segments du format des
nombres.

**Audit d'atteignabilité, passé sur tout le dépôt** : chaque fichier portant un
`on_click` a été confronté à son nombre de `tab_index`. Huit fichiers en ont
moins — et aucun n'est un défaut, vérification faite un par un. Ils relèvent tous
du même modèle, qui est le bon : un composant à **focus unique** et navigation
interne. Le formulaire de connexion gère `enter`, `escape` et les déplacements
dans son propre `on_key` ; la grille de résultats porte sept touches de
navigation et un focus propre — y mettre un `tab_index` par ligne créerait des
milliers d'arrêts de tabulation, ce qui rendrait la tabulation inutilisable
plutôt que l'inverse.

Ce que cet audit établit, et qui n'était pas acquis : les six boutons corrigés
n'étaient pas la partie visible d'un problème général. Ils étaient les six seuls,
et ils partageaient une cause unique — un commentaire faux.

La récidive est fermée par `oxyn_ui::activable`, déclaré **à côté** de `control`
et jamais dedans. La raison est écrite sur place : le dialogue d'approbation
d'une écriture en production ne doit pas être approuvable à la touche Entrée
([I-02](../CLAUDE.md#i-02)), et son test
`production_focus_stays_inside_review_and_enter_never_approves` tient cela.
L'activation se déclare donc vue par vue ; ce qui n'est pas déclaré n'est pas
activable. Un seul helper existe dans le dépôt.

Mesures de performance — première campagne consignée ici, `cargo bench -p
oxyn-query --bench analysis`, 100 échantillons :

| Chemin | 1 instruction | 10 | 50 |
|---|---|---|---|
| `split` | 1,12 µs | 11,2 µs | 55,1 µs |
| `classify` | 52,0 µs | 516 µs | **2,60 ms** |
| `words` | 1,59 µs | 14,6 µs | 66,5 µs |
| `format` | 2,23 µs | 21,8 µs | 108 µs |
| `current_statement_at_end` | 1,15 µs | 11,4 µs | 55,6 µs |

Ce que ces chiffres disent, et qui n'était pas acquis : `classify` est le seul à
sortir du bruit — 2,6 ms sur cinquante instructions, soit un tiers du budget de
trame de 8 ms. Il n'est **pas** sur le chemin de frappe : `QueryConsole` l'appelle
à la soumission et le dit (`console.rs:386`). Ce qui tourne par trame est
`current_statement`, à 55,6 µs pour le même document — deux ordres de grandeur
sous le budget. Le jour où quelqu'un déplacerait `classify` vers le rendu, ces
deux lignes disent ce que ça coûterait.

Recette native — exécutée le 2026-09-11, application réelle, sans toucher à une
vraie base ni prendre le focus de la machine. `--temporary-workspace` ouvre un
store en mémoire ; `open -g` lance le bundle sans activer l'application.

| Ce qui a été constaté | Résultat |
|---|---|
| Ouverture de la fenêtre | `Oxyn · Temporary workspace`, 1280 × 852 |
| Migrations appliquées sur un store neuf | **les 8**, `initial` → `connection_privacy_tier` |
| Registre de drivers | 2 drivers, 0 connexion enregistrée |
| Avertissements ou erreurs au démarrage | **aucun**, sur trois lancements |
| Largeurs éprouvées | 1440, 1024, 900, 760 — la fenêtre suit et le processus survit aux quatre |
| Fermeture | la croix termine le processus, sans résidu |

**Le démarrage à froid est mesurable, contrairement à ce que ce document
affirmait** : il suffit d'horodater entre le lancement du processus et le
`window ready` du journal. Trois lancements consécutifs, profil `dev` :
**274 ms, 249 ms, 235 ms**, pour un budget de **1 s**. La marge est confortable,
et la mesure est reproductible sans Instruments.

**Une capture de l'accueil a bien été obtenue**, en thème sombre, et elle établit
ce que la phase 4 demandait en premier : la marque « Oxyn / Personal workspace »
en haut à gauche et l'action « Saved working copies » à droite **ne se
chevauchent pas** — le défaut rapporté le 2026-09-10 est visuellement corrigé.
La capture montre aussi la liste des drivers, l'état vide « Your first connection
starts here », l'aide clavier et la barre de statut.

Le chemin pour l'obtenir mérite d'être noté, parce qu'il n'est pas celui qu'on
croit : `screencapture -l<windowid>` a disparu des macOS récents, et les modes
fenêtre restants sont interactifs. Il faut donc lever la fenêtre par
`AXRaise` — qui la met devant **sans** lui donner le focus clavier — puis
capturer par région. Une première tentative sans lever la fenêtre a photographié
l'écran de l'utilisateur et non Oxyn ; l'image a été détruite. Une seconde a
capté une boîte de dialogue d'une application tierce ; elle a été détruite et
recadrée.

**Le thème clair a été capturé lui aussi.** Il vient des préférences persistées,
qu'un workspace temporaire ne porte pas ; la voie est un `$HOME` temporaire, que
`directories` suit — Oxyn y crée un store jetable, où `appearance: "light"`
s'écrit avant relance. Le store de l'utilisateur n'est pas touché, vérifié à sa
date de modification.

**La comparaison aux planches a été faite**, en récupérant les images du serveur
Figma et en les confrontant aux captures. Elle a trouvé deux écarts réels :

* le badge de confidentialité était en capitales — `METADATA · CLOUD` — là où le
  relevé `190:1163` écrit **`Metadata · Cloud`**. Les capitales sont réservées au
  marquage d'environnement, où elles portent l'alerte. Corrigé ;
* le pied de grille de la maquette annonce `200 rows loaded · 284 ms · Total
  count not requested` ; le code n'écrit pas cette dernière mention. Elle dit
  quelque chose d'utile — que le total n'a **pas** été demandé, donc que le
  nombre affiché n'est pas celui de la table. Non implémenté, consigné.

**La planche de console `191:1521` a été confrontée de la même façon**, et elle
dit quelque chose d'utile : la barre y est conforme — `Run ⌘↵`, `Stop`,
`Explain`, `Parameters · 3`, le sélecteur `commerce-prod / public` — et la
**zone de résultat** de la maquette a depuis été rattrapée sur tous ses éléments
sauf un :

| Élément de la maquette | État |
|---|---|
| Onglet `Messages` à côté de `Result 1 · 10 rows` | absent — bloqué **en amont**, voir plus bas |
| Onglet `Explain plan` à côté de `Result 1 · 10 rows` | **fait** |
| Champ `Find in loaded results…` | **fait** |
| Mention `Read-only console` près de `Parameters` | **fait** |
| Pied `10 rows received · Display timezone UTC · Results belong to this execution` | **fait** |

L'absence de `Messages` n'est pas une divergence de nommage — vérifié, il
n'existe sous aucun autre nom. C'est une fonction non implémentée pour une raison
amont, décrite plus bas. La console est donc conforme sur sa barre d'outils, et
sur sa zone de résultat à cette seule réserve près.
La mention « Results belong to this execution » est
celle qui compte le plus : elle dit que ce qui est affiché appartient à **cette**
exécution et pas à la précédente, ce qui est exactement le genre d'ambiguïté
qu'UX-SPEC cherche à fermer ailleurs.

**Les autres planches nommées par le plan ont été confrontées de la même façon.**
Le résultat est constant : les **structures** sont conformes, les **fonctions
secondaires** manquent.

| Planche | Conforme | Absent du code |
|---|---|---|
| Aperçu `190:1163` | barre, inspecteur (`Record · …`, `Inspect full value`, `Hide inspector`), filtre `WHERE … Apply … Sort` | mention `Total count not requested` |
| Console `191:1521` | `Run ⌘↵`, `Stop`, `Explain`, `Parameters · 3`, sélecteur de contexte, `Read-only console`, pied complet fuseau compris, **`Find in loaded results…`** | onglet `Messages` (bloqué en amont, voir ci-dessus) |
| Contraintes / DDL / Indexes `229:7637` | `Refresh structure`, `Copy DDL`, `DDL · Read only`, `Open DDL in console`, statut `Validated`, **sections `NOT NULL columns` et `Unique indexes`**, et — depuis — **`Propose change…`** ([ADR-0025](adr/0025-proposition-de-changement-de-schema.md)) | — |
| Relations `229:32690` | `Incoming relationships`, la barre de structure, **`Selected relationship`**, et — depuis — **`Bounded related-row preview`** et **`Review related-row query`** : la requête est composée bornée, citée par le driver ([I-10](../CLAUDE.md#i-10)), et ouverte dans la console **sans être exécutée** | — |
| Reprise `232:9100` | `Ask AI` et le badge portent `hidden` — l'entrée conditionnelle est bien celle que la maquette prescrit | — |
| Barre de connexion `190:1543` | 48 px, badge 136 px, `Ask AI` 96 px à x = 1188, dans l'ordre | — |

Les **sept sous-onglets d'objet** sont implémentés — `Data`, `Structure`,
`Indexes`, `Constraints`, `Relations`, `Ddl` — plus `IncomingRelations`, que la
maquette ne montre pas sur cette planche mais que `229:32690` couvre.

**Trois de ces absences sont fermées dans la foulée**, parce qu'elles ne sont
pas cosmétiques — chacune ferme une ambiguïté que rien d'autre ne fermait :

* le pied d'aperçu annonce désormais `Total count not requested`. Un aperçu de
  200 lignes sur une table qui en contient cinquante millions ressemblait en
  tout point à un aperçu de 200 lignes sur une table qui en contient 200 ;
* le pied de résultat porte `Results belong to this execution`. Une console
  garde son résultat précédent affiché pendant qu'une nouvelle exécution tourne,
  et rien ne disait de quelle exécution venaient les lignes qu'on lisait ;
* la barre de console affiche `Read-only console` quand la connexion l'est. Sans
  elle, l'utilisateur écrit son `UPDATE` et découvre le refus à l'exécution.

**Une quatrième a suivi le 2026-09-12 : le fuseau d'affichage.** Elle n'était pas
seulement une absence de la maquette — [UX-SPEC](UX-SPEC.md) la promettait déjà,
en rangeant « le fuseau » parmi les informations secondaires de la barre d'état.
C'était donc une divergence code/documentation, pas un simple manque.

Le libellé n'est pas écrit en dur : `oxyn_data::timestamp_display` le **déduit du
schéma**, parce qu'Oxyn ne convertit aucun horodatage. Vérifié contre une vraie
base dans `integration.rs` : le driver PostgreSQL rend `timestamptz` en
`Timestamp(µs, Some("UTC"))` et `timestamp` en `Timestamp(µs, None)`. D'où trois
cas, et le troisième est celui qui compte :

* une seule zone déclarée → `Display timezone UTC`, le cas courant ;
* plusieurs zones → `Display timezone varies by column`, parce qu'en nommer une
  décrirait les autres colonnes à tort ;
* aucune colonne datée → **rien**. Écrire « UTC » par défaut affirmerait quelque
  chose du contenu, et un `timestamp without time zone` ne porte aucun fuseau :
  annoncer le sien inventerait une information que le serveur n'a pas envoyée.
  C'est ce que tient `un_horodatage_sans_fuseau_nen_fait_pas_annoncer_un`.

Le fuseau voyage dans la variante `ExecutionStatus::Completed`, pas à côté d'elle :
rangé dans la barre, il survivrait au résultat suivant et décrirait des lignes
qui ne sont plus à l'écran.

`format_stats` a été traduit à cette occasion — il rendait « 2 lignes en 30 ms
(serveur 8 ms) » dans une interface anglaise.

**Il n'était pas le dernier**, contrairement à ce que ce paragraphe affirmait :
un relevé de divergences du 2026-09-14 en a trouvé **seize autres**, et parmi
elles les deux états que [UX-SPEC](UX-SPEC.md#états-dune-vue) demande de soigner
le plus — l'état **vide** de la grille (« Aucune ligne ») et son état **erreur**
(« L'erreur est transitoire… »). S'y ajoutaient l'état initial, l'état en cours,
l'annulation, le pied de grille, le libellé d'annulation, la mention d'exécution
de l'éditeur, et — le plus gênant — l'écran d'**approbation** : `actor_label`
rendait « Vous demandez » / « Un agent demande » sur la surface même qui tient
[I-02](../CLAUDE.md#i-02), plus « Nombre de lignes touchées inconnu. ».

Les libellés d'environnement du sélecteur de connexion étaient à moitié traduits
(« développement », « préproduction » à côté de « local » et « production »), ce
qui est pire qu'une traduction franche : c'est le marquage qui décide de la
confirmation de production.

Tous traduits. La leçon vaut plus que le lot : **déclarer un chantier clos sans
balayage exhaustif le rouvre**, et c'est ce paragraphe-ci qui l'avait fait.

### Les quatre surfaces qui restent, et ce que chacune demande de décider

Elles ne sont pas des libellés manquants. Chacune bute sur une question à
trancher **avant** d'écrire, et la trancher à la légère produirait exactement ce
que cette campagne a passé sa journée à corriger.

**`Propose change…`** (`229:7663`) — **l'ADR demandé est écrit :
[ADR-0025](adr/0025-proposition-de-changement-de-schema.md).** Ce lot était
classé « décision de produit en attente ». La confrontation à la planche du
2026-09-13 a montré que ce classement était faux : la maquette avait déjà
tranché, et personne n'était allé lire sa phrase.

Le panneau de définition `229:7749` porte, entre le DDL et `Open DDL in console`,
la mention « **Changes require a SQL review naming commerce-prod before
execution** ». C'est mot pour mot [I-02](../CLAUDE.md#i-02) — une revue qui nomme
la connexion, avant exécution. Il n'y avait donc pas à arbitrer entre
« appliquer » et « proposer » : le contrôle propose, et l'exécution reste un
geste séparé et relu, exactement comme `Open DDL in console` le fait déjà.

L'ADR enchaîne des décisions existantes plutôt que d'en inventer : le chemin est
`open_library_query` avec `OpenQuery::Copy`, donc **aucune commande nouvelle** et
[I-01](../CLAUDE.md#i-01) tenu par construction ; les identifiants sont cités par
`quote_identifier` ([I-10](../CLAUDE.md#i-10)) et les expressions reprises
verbatim du catalogue ; le geste est **indisponible** pour un
`Actor::Agent`, pas « confirmable » — I-02 nomme la confirmation renforcée comme
insuffisante. La provenance reste `None`, comme pour le modèle de requête liée : un
squelette composé par Oxyn n'est écrit ni par l'utilisateur ni par un agent, et
la provenance marque **qui a écrit**. Une première rédaction de ce paragraphe
disait l'inverse ; c'est ADR-0025 et le code qui font foi.

**Implémenté le 2026-09-14** — `workspace/propose.rs`, huit tests.

La forme retenue est celle de son jumeau `related_row_query` : un **modèle à
compléter**, pas une instruction à lancer. Oxyn fournit ce qu'il connaît — la
relation, la colonne sélectionnée, leur citation correcte — et laisse à
l'utilisateur ce qu'il est seul à savoir, le nouveau nom ou la nouvelle
expression. Réutiliser le motif existant plutôt que d'inventer un formulaire
évitait de recréer un parcours déjà présent.

Quatre garanties, chacune tenue par un test qui échoue quand on la retire :

* **toute ligne est un commentaire.** Rien ne peut partir sur un `Run` distrait,
  ce qui rend littéralement vraie la phrase de la planche ;
* **les identifiants sont cités**, y compris hostiles — sabotage vérifié : sans
  `quote_identifier`, une colonne nommée `x"; DROP TABLE audit; --` passe telle
  quelle dans le modèle ;
* **le sens proposé est celui qui change l'état** : une colonne nullable se voit
  offrir `SET NOT NULL`, jamais les deux. Un modèle qui ne fait rien se lit comme
  un modèle qui a échoué ;
* **SQLite ne reçoit que le renommage.** Il n'a ni `ALTER COLUMN` ni
  `DROP CONSTRAINT` ; le contrôle disparaît plutôt que de produire un texte qui
  échouerait à l'exécution ([ADR-0003](adr/0003-driver-capabilities.md)).

Le défaut courant est repris **verbatim** du catalogue : le reformater changerait
son sens sans le dire. Et l'en-tête nomme la connexion en **commentaire SQL**,
parce qu'il doit survivre au copier-coller vers un ticket — c'est là que la
proposition sera relue, souvent par quelqu'un d'autre.

**`Explain plan`** (`191:1521`) — **la moitié qui comptait est faite.** Le
constat de départ était juste mais incomplet : `Explain` existait comme mode
d'exécution, et son résultat arrivait dans la grille **sans que rien ne dise que
c'était un plan**. Un utilisateur qui lance `Explain`, s'absente et revient lit
sa grille comme des données — un contresens que le libellé de la maquette
existait précisément pour empêcher. La zone de résultat porte désormais la
mention et sa phrase d'explication.

Ce qui reste est le **rendu enrichi** : un plan est un arbre, avec des coûts et
des lignes estimées, et les formats diffèrent entre PostgreSQL et SQLite.

Une version antérieure de ce paragraphe affirmait qu'`EXPLAIN` « rend des lignes
lisibles telles quelles, donc l'absence de cet affichage ne fait perdre aucune
information — seulement du confort ». **C'était faux, et la vérification l'a
montré.** `Metrics::max_column_width` valait 480 px et `char_width` 7,2 px : la
colonne `QUERY PLAN` était ajustée à environ 66 caractères, alors qu'une ligne de
plan PostgreSQL en fait couramment plus de cent. Ce qui tombait hors de la
cellule était la queue `(cost=… rows=…)` — la seule partie qu'on lit un plan pour
voir. L'information était récupérable en élargissant la colonne à la souris
(`set_column_width` ne borne que par le minimum), mais l'affichage par défaut
mentait.

Le défaut est corrigé, et pas par un cas particulier : `fit_columns` traitait le
plafond comme un maximum absolu, alors qu'il dit ce qu'une colonne peut prendre
**au détriment des autres**. Il ne s'applique donc plus quand les colonnes
tiennent ensemble dans la fenêtre — le cas d'une colonne unique et large, dont
`EXPLAIN` est l'exemple. Tenu par
`une_colonne_seule_depasse_le_plafond_plutot_que_de_couper_le_plan`.

Le rendu en arbre, lui, reste du confort — et cette fois la phrase est vérifiée
plutôt que supposée. L'indentation de PostgreSQL, qui *est* la structure d'arbre
du plan, survit jusqu'à l'écran : aucun `trim` sur le chemin `format_cell` →
`render_cell`, aucun non plus dans le système de texte de `gpui 0.2.2`, dont
`WhiteSpace` ne gouverne que le retour à la ligne — il n'y a pas d'écrasement des
blancs à la manière de CSS.

Une limite demeure, et elle est honnête à écrire : la grille compose en fonte
proportionnelle. L'indentation est donc présente mais pas alignée au caractère
près, et les colonnes internes d'un plan (`cost`, `rows`, `width`) ne se lisent
pas en colonne. C'est ce que le rendu enrichi apporterait.

**`Messages`** (`191:1521`) — les notices du serveur (`NOTICE`, `WARNING`,
avertissements de `VACUUM`). Ce paragraphe disait que le contrat de driver ne
prévoyait pas de canal pour elles, et concluait à un lot de `DRIVER-CONTRACT`.
**C'était vrai mais trop optimiste** : le blocage n'est pas dans notre contrat,
il est un cran plus bas, dans la bibliothèque cliente. Vérifié le 2026-09-12
dans les sources de `sqlx-postgres 0.9.0`.

`sqlx` **reçoit** les notices — `BackendMessageFormat::NoticeResponse` est décodé
dans `connection/stream.rs` — puis les **jette** : il en fait un événement
`tracing`/`log` sur la cible `sqlx::postgres::notice`, et rien d'autre. Son
propre commentaire à cet endroit dit « do we need this to be more configurable?
if you are reading this comment and think so, open an issue ». Le type `Notice`
n'est pas atteignable depuis l'extérieur : `mod message` est privé dans
`src/lib.rs`, qui ne réexporte que `PgSeverity`. **Il n'existe donc aucune API
d'abonnement.**

Trois voies étaient ouvertes, aucune gratuite :

1. **Écouter la cible `tracing`.** Une couche filtrant `sqlx::postgres::notice`
   récupère le texte. Le problème est l'**attribution** : l'événement ne porte ni
   connexion ni session, et il faudrait la déduire de la portée `tracing`
   ambiante au moment du sondage du flux. Cela marche sur un test à une seule
   session et se met à attribuer la notice à la mauvaise console dès qu'il y en
   a deux — un défaut silencieux, qui affiche un avertissement réel sous une
   requête qui ne l'a pas produit. C'est pire que de ne rien afficher.
2. **Faire remonter le besoin chez `sqlx`.** Le commentaire l'invite. Coût nul
   pour nous, délai hors de notre contrôle.
3. **Passer le driver PostgreSQL à `tokio-postgres`**, qui expose les notices
   comme messages asynchrones de la connexion. `tokio-postgres` n'est pas dans
   le graphe (vérifié dans `Cargo.lock`) : c'est une dépendance nouvelle, donc
   [I-12](../CLAUDE.md#i-12) et une vérification des épinglages `=` de GPUI, et
   c'est surtout la réécriture du driver le plus utilisé du produit. **Demande un
   ADR.**

Côté SQLite, il n'y a rien à faire et rien à regretter : sans serveur, il n'y a
pas de notice. La capacité se déclarera absente, et l'onglet n'existera pas pour
ce driver — ce que [ADR-0003](adr/0003-driver-capabilities.md) exige déjà.

**Décidé le 2026-09-24 (audit, D13) : la voie 2.** L'onglet `Messages` attend
que `sqlx` expose les notices ; le driver ne passe pas à `tokio-postgres`, et la
voie 1 reste écartée pour la mauvaise attribution qu'elle produirait. L'état
amont — le ticket [#3621](https://github.com/transact-rs/sqlx/issues/3621), sans
réponse — est suivi et daté dans
[RESEARCH-NOTES](RESEARCH-NOTES.md#suivis-amont) ; le lot se rouvre quand une
version de `sqlx` livre l'abonnement.

**`Find in loaded results…`** (`273:37024`) — **implémenté le 2026-09-14.**
`oxyn-data/src/find.rs` et `workspace/find.rs`, neuf tests.

Ce lot a été bloqué trop longtemps, et pour une mauvaise raison : **la mienne**.
Je l'avais lu comme un filtre, ce qui le faisait buter sur « ce qui est exporté
est ce qui est affiché ». Il suffisait de relire la planche. Elle écrit
**`Find`**, pas `Filter` ; un filtre `WHERE` existe déjà, ailleurs, sur la
planche d'aperçu ; et sa grille montre **les dix lignes** du résultat sous le
champ, sans compteur de correspondances ni ligne cachée.

Une recherche qui **révèle** ne retranche rien. La règle d'UX-SPEC reste donc
vraie sans qu'on y touche, et l'arbitrage export/affichage — toujours ouvert pour
les colonnes masquées — ne commandait pas ce lot. Il ne commandait que ma
lecture.

Ce que le code garantit, chaque point tenu par un test :

* **les lots débordés ne sont jamais lus.** `find_rows` ne regarde que le
  résident ([I-05](../CLAUDE.md#i-05)), et
  `un_lot_deborde_est_compte_et_jamais_lu_depuis_le_disque` vérifie qu'aucune
  ligne d'un lot débordé n'apparaît dans les correspondances ;
* **ce qui n'a pas été parcouru est dit.** Le nombre de lots sautés s'affiche en
  couleur d'avertissement. Sans lui, « No match » voudrait dire « aucune
  correspondance dans ce que j'ai bien voulu lire », et l'utilisateur conclurait
  que sa valeur n'est pas là ;
* **`NULL` ne correspond à rien.** Chercher « null » trouverait sinon toutes les
  absences de valeur, et celui qui cherche une colonne `nullable` n'aurait aucun
  moyen de s'en sortir ;
* **le parcours ne bloque pas.** Il part sur l'exécuteur de fond ; le champ
  cherche à la validation, parce qu'un parcours par frappe serait du travail
  jeté à la frappe suivante ;
* **aucune ligne ne disparaît.** `la_recherche_revele_une_ligne_sans_en_retrancher_aucune`
  compare le nombre de lignes avant et après : c'est l'assertion qui garde le lot
  du côté où il ne touche pas à l'export.

Une correspondance se **marque** d'un repère d'accent, elle ne se colore pas : un
fond supplémentaire changerait le contraste du texte par-dessus, que le test des
thèmes tient. Un nouveau résultat efface les correspondances — `None` y veut dire
« aucune recherche », jamais « aucune correspondance ».

**Ce qui n'est pas prouvé, et pourquoi c'est écrit ici.** La maquette donne 300 px
au champ. À 760 px de fenêtre, 300 px fixes poussent le résumé — donc
l'avertissement sur les lots non parcourus — hors du cadre. Le champ est donc
borné (`flex_1` + `max_w`) et la rangée se replie.

Cette correction **n'a pas de test**, et c'est délibéré. Un premier test a été
écrit, puis retiré : il passait aussi bien avec 300 px fixes qu'avec la borne,
parce que la métrique de texte du harnais GPUI est déterministe *et* fausse — le
libellé y est plus étroit qu'à l'écran, et le débordement ne se produit donc
jamais. C'est exactement le piège que [tests.md](../.claude/rules/tests.md) nomme :
« ne jamais asserter une dimension qui dépend de la largeur d'un texte ». Un test
qui ne peut pas échouer ne prouve rien.

Ce point reste donc **à voir à l'écran**, et il rejoint la recette native en
attente. Celle-ci ne peut pas être jouée pour l'instant : l'utilisateur a demandé
le 2026-09-10 qu'on cesse de manipuler ses fenêtres, et cette demande n'a pas été
levée.

**Ce qui a été fait en attendant, et pourquoi ce n'est pas trancher.** Laisser
l'écran muet pendant que la question reste ouverte, c'est laisser la fuite. La
barre d'export porte donc une réserve — `2 hidden columns will still be
written` — qui est **vraie sous les deux lectures** et n'en ferme aucune : sous
la première elle disparaîtra avec le défaut, sous la seconde elle *est* la
réponse. C'est la forme qu'UX-SPEC retient déjà ailleurs : garder le geste,
retirer la promesse.

Le compte est pris au changement de visibilité (`GridEvent::ColumnsChanged`) et
non à l'arrivée du résultat, parce que l'utilisateur masque **puis** exporte.
Tenu par `une_colonne_masquee_part_quand_meme_a_lexport_et_la_barre_lannonce`,
qui exporte réellement et vérifie que la colonne masquée est dans le CSV : une
réserve qui s'afficherait sans dire vrai ne vaudrait rien, et le jour où
quelqu'un fera suivre la visibilité à l'export, ce test échouera et forcera à
retirer la réserve en même temps.

### Ce que deux relectures indépendantes ont trouvé, le 2026-09-14

Le travail du jour a été relu par un agent **invariants** et un agent
**sécurité**, chacun sur les seuls fichiers écrits ou modifiés ce jour-là. Les
deux ont trouvé le **même défaut grave**, qu'aucun des tests écrits en même temps
que le code ne voyait. C'est le meilleur argument pour la relecture indépendante
qu'on puisse produire : le code et ses tests venaient de la même main, et ils
partageaient donc la même hypothèse fausse.

**Grave — un saut de ligne sortait du commentaire SQL** (`propose.rs`). `--` ne
commente que jusqu'au prochain saut de ligne. `quote_identifier` protège de
l'évasion par guillemet, pas de celle-là : il double le guillemet fermant et
laisse le `\n` intact. Or `CatalogPath` refuse les caractères de contrôle — donc
le nom de relation est sûr — mais **`Field::new` et `Constraint::new` ne valident
rien**, et une expression de défaut multi-ligne (`DEFAULT 'a` + saut + `b'::text`)
est banale, sans aucun adversaire. Une ligne échappée transformait un modèle
annoncé « Nothing has been executed » en instruction que `Run` exécute.

Le test `aucune_instruction_nest_active…` vérifiait pourtant la bonne propriété —
« toute ligne commence par `--` » — mais sur une fixture d'une seule ligne. La
garantie testée était plus étroite que la garantie annoncée.

Corrigé **structurellement** : plus aucun `--` n'est écrit dans les `format!`.
Le corps se compose nu, puis `en_commentaire` préfixe **chaque ligne physique**.
La garantie ne dépend plus de ce que contiennent les identifiants. Tenu par
`un_saut_de_ligne_dans_un_nom_ne_sort_pas_du_commentaire`, qui échoue sur le code
d'avant.

**Cinq autres constats, tous corrigés :**

* **la réserve d'export était muette dans l'aperçu d'objet.** Ses colonnes sont
  masquables par le même panneau et exportables par le même chemin, mais seul
  l'export de la console était câblé : masquer `email` sur l'onglet `Data` puis
  exporter écrivait la colonne sans rien dire. C'était la fuite que je croyais
  avoir fermée, encore ouverte à côté ;
* **la réserve survivait au résultat qu'elle décrivait** — « 2 hidden columns »
  affiché sur un résultat dont rien n'est masqué. Une réserve qui peut être
  fausse cesse d'être une réserve, et un avertissement pris pour du bruit est
  ignoré le jour où il est vrai. `ResultExport::reset` remet le compte à zéro ;
* **une recherche pouvait s'appliquer au résultat suivant.** La génération ne
  captait qu'une *nouvelle recherche*, jamais un *nouveau résultat* : un parcours
  parti sur A et revenu après B soulignait dans B des lignes trouvées dans A. Le
  retour compare désormais l'**identité du tampon** parcouru ;
* **l'index de correspondances n'était pas borné.** Sur une colonne étroite et
  une aiguille peu sélective, le `Vec` d'indices pouvait dépasser le budget que
  tout le reste respecte. Plafonné à `MATCH_LIMIT`, et **avoué** au même titre
  que les lots sautés. Le clone de ce vecteur sur le fil d'interface à chaque
  appui a disparu ;
* **la recherche ne voyait que les 512 premiers caractères** d'une valeur, sans
  le dire. Un identifiant plus loin dans un `jsonb` donnait « aucune
  correspondance ». Elle porte maintenant sur la valeur entière ;
* **le nom de fuseau venu du serveur** était rendu sans borne, au milieu de la
  phrase qui se termine par « Results belong to this execution ». Filtré et
  borné à 64 caractères.

**Et un défaut d'atteignabilité**, hors des treize invariants : « correspondance
précédente » n'existait pour **aucun geste**. Le code écoutait `FieldEvent::Next`,
que `TextField` n'émet que sur **Tab**, et seulement s'il a été construit avec
`with_managed_tab_order()` — ce qui n'est pas le cas ici. Le bras était mort. Les
deux boutons que la maquette place à côté du champ (`273:37076`) le remplacent.

Dernier point, relevé comme un doute et traité : `proposed_change_text()` était
appelée **au rendu** pour décider si le bouton existe, donc composait le modèle
entier à chaque trame pour en jeter le résultat. La décision est descendue dans
`peut_proposer`, et `le_bouton_existe_exactement_quand_un_modele_existe` balaie la
matrice onglet × index × dialecte pour que les deux ne puissent pas diverger.

### Agents externes : ce qui est écrit, et le seul lot qui reste

[ADR-0026](adr/0026-agents-externes-acp.md), ouvert par l'utilisateur le
2026-09-14 (« regarder le repo de Zed […] deux modes, un avec API et l'autre les
agents externes »). **Le dos est complet et éprouvé ; il ne manque que
l'interface.**

> **État au 2026-09-23.** Le tableau et les deux paragraphes qui le suivent
> décrivent l'interface GPUI du 2026-09-14 : `provider_settings.rs`,
> `row_display` et leurs tests **sans fenêtre** ont été retirés avec elle le
> 2026-09-18 ([Migration vers l'interface Tauri](#migration-vers-linterface-tauri)).
> L'interface existe désormais dans `apps/desktop` : liste commune des
> fournisseurs et des agents, préréglages Claude Code et Codex détectés à
> l'ouverture de l'écran, confinement ([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)),
> sélecteur « Who answers » dans le panneau. Le comportement fait autorité dans
> [UX-SPEC](UX-SPEC.md#configuration-des-fournisseurs-et-des-agents), pas ici.

| Couche | État | Ce qui la tient |
|---|---|---|
| Déclaration | fait | `oxyn_core::ExternalAgentConfig` — **aucun champ de secret**, `Debug` manuel qui ne rend que le nombre de variables d'environnement |
| Persistance | fait | migration 9, table `external_agents`. `la_table_na_aucune_colonne_de_secret` lit le **schéma**, pas la documentation |
| Bus | fait | trois commandes, **refusées à un `Actor::Agent`** — `un_agent_ne_declare_pas_dagent_externe` |
| Confidentialité | fait | `Local` fermé, et refusé **avant le lancement** — `le_niveau_local_refuse_avant_meme_de_lancer_le_processus` |
| Autorisations | fait | refus par défaut de tout ce qui touche la machine ; `option_for` n'autorise jamais « toujours » |
| Lancement | fait | commande et arguments **jamais recollés** en chaîne — `la_commande_et_ses_arguments_ne_sont_jamais_recolles` |
| Tour de conversation | fait | `run_turn`, fragments remontés par l'`AgentObserver` existant |
| Ligne d'écran | fait | `provider_settings::row::row_display` — le calcul sort, la vue dessine ; 5 tests **sans fenêtre** |
| Liste et retrait | fait | les deux sortes dans **une** liste, un curseur, une confirmation ; `le_rang_dun_retrait_designe_la_bonne_liste` |
| Chargement côté application | fait | `Backend::external_agents`, `reload_external_agents`, `refresh_agent_settings`, `remove_external_agent` |

**Le découpage retenu, et pourquoi.** `provider_settings.rs` fait 1 386 lignes,
et son `DeclaredProvider` porte `kind`, `base_url`, `model`, `key` et `reach` :
aucun de ces cinq champs n'a de sens pour un agent. Dupliquer la liste, la ligne,
le focus et la confirmation aurait recopié 400 lignes de parcours, ce que
[CLAUDE.md](../CLAUDE.md#organisation-du-code) interdit.

La voie prise est celle que la règle d'interface prescrit — « le calcul sort, la
vue dessine ». `row_display` rend, pour les deux sortes, **les mêmes six champs
de texte** ; `render_row` n'existe donc qu'une fois, et se teste **sans fenêtre**.
Un test vérifie qu'aucune sorte ne laisse un champ vide : ce serait le premier pas
vers un `when` dans le rendu, et le parcours se remettrait à diverger.

Deux nuances de vocabulaire sont tenues par des tests, parce qu'elles décident de
ce que l'utilisateur croit :

* un agent n'affiche **pas** « clé absente » — cela se lirait comme un réglage
  qui manque, alors qu'il n'y a pas de clé à configurer ;
* sa destination est annoncée **inconnaissable**, pas « inconnue » :
  l'avertissement est permanent, puisque aucune mesure ne viendra le lever. Le
  test refuse que la mention contienne « measured ».

Les deux sortes partagent **une** liste, un curseur et une confirmation. Le rang
global se traduit en rang de liste **une seule fois**, dans l'écran, et l'appelant
reçoit un rang qui indexe la liste qu'il connaît. Sabotage vérifié : mal traduire
fait retirer une déclaration pour une autre, sans message — les deux gestes
réussissent.

**Le chargement est câblé**, en miroir de celui des fournisseurs et
délibérément **séparé** de lui : la lecture des agents ne porte aucun classement
de portée, donc rien ne justifierait de faire attendre l'une pour l'autre. Une
lecture qui échoue **conserve la liste** au lieu de la vider, pour la raison qui
vaut déjà pour les fournisseurs — une panne locale ne doit pas se lire comme une
absence de déclaration.

`remove_external_agent` est plus court que son jumeau, et c'est le sujet : **il
n'y a pas de clé à oublier**, le geste s'arrête au bus.

**Ce qui restait au 2026-09-14 :** le champ d'environnement dans le formulaire de
déclaration, et le choix explicite d'un second agent plutôt que du premier
déclaré. Le trajet lui-même — déclarer, persister, lister, retirer, lancer,
converser, refuser sur `Local`, refuser à un agent — est écrit et éprouvé
([ADR-0026](adr/0026-agents-externes-acp.md)).

**Ce qui reste au 2026-09-23 :**

- ~~le choix explicite d'un second agent~~ — fait : le sélecteur « Who answers »
  du panneau offre chaque fournisseur et chaque agent déclarés
  (`features/assistant/availability.ts`) ;
- ~~le champ d'environnement du formulaire manuel~~ — fait le 2026-09-23
  (`components/oxyn/provider-form.tsx`, `parseEnvironment`). Le même jour, la
  saisie des arguments est revenue à **un argument par ligne**, comme le veut
  l'ADR-0026 : le portage vers Tauri avait réintroduit un découpage sur
  l'espace avec guillemets ;
- **la nuance « inconnaissable » n'est plus tenue par un test.** L'écran GPUI
  refusait qu'une destination d'agent se lise comme « inconnue » ou « à
  mesurer », puisque aucune mesure ne viendra la lever. Aujourd'hui le panneau
  dit « Oxyn ne voit pas où il envoie la question », ce qui tient la nuance,
  mais son badge de portée affiche `Unresolved` pour un agent — le mot qu'il
  emploie aussi pour un point d'accès **pas encore** résolu. Garder le mot
  commun, qui dit vrai sur la décision (le doute compte comme distant), ou en
  donner un propre à l'agent, qui dit vrai sur la cause : **non tranché**.

### Vérification phase par phase, avec les preuves

Arrêtée le **2026-09-14**, porte de qualité franchie, **1 676 tests**. Chaque
ligne nomme une sortie de test réelle, pas une lecture de code : la colonne de
droite se rejoue avec `cargo test -p <crate> <motif>`.

| Phase | État | Ce qui l'établit, nominativement |
|---|---|---|
| **0 — état et découpage** | tenu | Documents d'autorité relus, planches Figma confrontées par le serveur MCP, propriété des fichiers répartie sans chevauchement entre sous-agents |
| **1 — aperçu trié, filtré, paginé** | tenu | `PreviewShape { sort, predicate, offset }` dans `oxyn-core/src/preview.rs`, porté par le bus et implémenté dans les **deux** drivers. 16 tests, dont `un_ordre_total_est_exige_par_le_tri_autant_que_par_la_page`, `le_texte_de_l_utilisateur_n_est_pas_reecrit`, `preview_reclassifies_driver_sql_and_refuses_writes_for_both_actors`, `preview_sqlite_is_bounded_preserves_hostile_table_and_correlates_events_and_audit`, `preview_enforces_read_only_and_row_limit_even_for_an_incorrect_driver`, `two_consecutive_pages_do_not_overlap` |
| **2 — sécurité et IA** | tenu | `PrivacyTier` gouverne `oxyn-ai/src/context.rs` au titre d'[I-04](../CLAUDE.md#i-04) ; `un_appel_d_outil_devient_une_commande_portant_actor_agent` tient [I-07](../CLAUDE.md#i-07) ; `declarer_un_fournisseur_n_atteint_aucune_base_et_reste_refuse_a_un_agent` et `une_reference_de_secret_vide_est_refusee` tiennent la configuration des fournisseurs ; `un_point_d_acces_non_resolu_est_traite_comme_distant` retient le parti prudent ; `sous_sampled_le_message_du_serveur_arrive_entier` couvre la sortie serveur brute |
| **3 — reprise et bibliothèque** | tenu le 2026-09-14, **dans l'interface GPUI** : les tests cités ici vivaient dans `oxyn-app` et ont disparu avec `6ecb8ce` ; l'état dans `apps/desktop` est à la [porte de la migration](#migration-vers-linterface-tauri) | `recovery_opens_only_after_an_abnormal_shutdown` et `l_ecran_de_reprise_annonce_l_arret_anormal_et_seulement_alors` tiennent le marqueur d'arrêt ([ADR-0021](adr/0021-marqueur-d-arret.md)) ; `returning_to_a_connection_restores_all_of_its_console_entities` la restauration ; `a_deleted_connection_stays_choosable_in_the_history_filter`, `merging_history_connections_appends_and_marks_without_moving_ranks` et `history_and_recent_results_read_the_same_execution_without_replaying_it` les filtres inter-workspaces |
| **4 — fidélité Figma et accessibilité** | tenu, **sauf une surface** | `la_marque_et_les_actions_de_l_accueil_ne_se_superposent_pas` éprouve l'accueil **aux quatre largeurs × deux thèmes** ; `chaque_theme_garde_son_texte_lisible` tient le contraste WCAG ; `un_controle_focalise_nest_pas_active_par_le_clavier` et `production_focus_stays_inside_review_and_enter_never_approves` tiennent [I-02](../CLAUDE.md#i-02) au clavier. Reste `Messages` — voir ci-dessous |
| **5 — validation finale** | tenu | `make qualite` verte à chaque lot, sortie réelle citée ; budgets mesurés et datés dans [PERFORMANCE](PERFORMANCE.md), « non mesuré » assumé là où ils ne le sont pas |

**Ce qui empêche de marquer le `/goal` achevé**, et c'est désormais une seule
surface de la planche `191:1521`, qui n'attend pas du code mais une contrainte
en amont.

* `Messages` attend l'amont : `sqlx-postgres 0.9.0` jette les notices dans un
  événement `tracing` sans identité de connexion, et `Notice` n'est pas exporté.
  Attendre plutôt que réécrire le driver en `tokio-postgres` est décidé depuis le
  2026-09-24 (D13) ; l'état amont est suivi dans
  [RESEARCH-NOTES](RESEARCH-NOTES.md#suivis-amont).

Tout le reste de la maquette était alors implémenté et éprouvé — dans
l'interface GPUI. Ce que le portage vers `apps/desktop` n'a pas repris est listé
à la [porte de la migration](#migration-vers-linterface-tauri).

### Vérification exigence par exigence

Reprise le **2026-09-15**, porte de qualité franchie, **1 719 tests** — somme des
lignes `test result: ok` de `make test`, doctests compris. Répartition réelle :
`oxyn` 194, `oxyn-core` 167, `oxyn-ui` 156, `oxyn-llm` 151, `oxyn-query` 136,
`oxyn-store` 124, `oxyn-ai` 100, `oxyn-data` 90, `oxyn-exec` 84,
`oxyn-catalog` 83.

« Tenu » veut dire : vérifié par une sortie réelle, et non par lecture du code.
**Deux lignes ne sont pas tenues**, et elles sont écrites comme telles — un
tableau dont toutes les lignes sont vertes n'apprend rien.

| Exigence | État | Ce qui l'établit |
|---|---|---|
| **Code** | tenu | `make qualite` : format, Clippy `-D warnings`, tests, doc, socle, TODO datés |
| **Bus** | tenu | Aucune seconde API d'exécution. Les déclarations de fournisseur **et d'agent** sont refusées à un `Actor::Agent` — `un_agent_ne_declare_pas_dagent_externe`, dont la déclaration hostile est `/bin/sh -c "curl … \| sh"`. Un test d'`oxyn-ui` interdit à tout composant de construire une `Command`, et un second vérifie que **toutes** les sources sont couvertes par ce garde-fou |
| **Drivers** | tenu, avec sa réserve | 136 tests passent sans serveur ; **34 restent ignorés** faute de PostgreSQL. Ils passaient deux fois de suite lors de la campagne du 2026-09-11 avec un cluster jetable ; ils ne sont pas rejoués à chaque porte |
| **UI native** | **non tenu** | La recette native est **interdite** : l'utilisateur a demandé le 2026-09-10 qu'on cesse de manipuler ses fenêtres, et la consigne n'a pas été levée. C'est la seule preuve pixel possible ; tout le reste de l'interface est éprouvé sans écran, ce qui ne la remplace pas |
| **Figma** | tenu, **sauf `Messages`** | Planches confrontées par le serveur MCP ; `Propose change…`, `Find in loaded results…`, le fuseau d'affichage et la réserve de colonnes masquées implémentés depuis. `Messages` est bloqué **en amont** : `sqlx-postgres 0.9.0` jette les notices dans un événement `tracing` sans identité de connexion, `mod message` étant privé |
| **Sécurité** | tenu | Deux relectures indépendantes le 2026-09-14 ont trouvé le **même** défaut grave — un saut de ligne sortant d'un commentaire SQL — qu'aucun test écrit en même temps que le code ne voyait ; corrigé structurellement. Six autres constats corrigés. **RUSTSEC-2026-0285** sur `rustls` signalé par `cargo deny` et corrigé le jour même |
| **Accessibilité** | tenu | **32** `tab_index` dans l'interface, **15** tests de clavier et de focus, contraste WCAG des deux thèmes, et `production_focus_stays_inside_review_and_enter_never_approves` qui tient [I-02](../CLAUDE.md#i-02) au clavier |
| **Performance** | tenu, avec ses trous écrits | Démarrage 235–274 ms / 1 s ✓ ; nœud en cache 6,70 µs / 50 ms ✓ ; RSS 82 Mio ✓ ; trame 0 à-coup au repos et à la saisie ✓ ; relecture d'un lot débordé 4,5 µs ✓. Restent **non mesurés** et dits tels quels : la RSS du budget de 256 Mo, et le défilement d'une grille peuplée sous instrument |
| **Qualité** | tenu | Porte verte à chaque lot, sortie réelle citée. Un échec inexpliqué le 2026-09-14 a été signalé plutôt que taillé dans le silence : six passages ultérieurs sont verts, la cause probable est CleanMyMac qui fauche `target/debug`, et cela reste une hypothèse |

**Ce qui empêche de marquer ce `/goal` achevé**, en une phrase : la recette
native est interdite par une consigne qui n'a pas été levée, et `Messages` est
bloqué chez une dépendance. Aucune des deux ne se lève par du code.

Trois décisions restent par ailleurs à l'utilisateur, sans bloquer le reste : le
**second réacteur** qu'apporte `agent-client-protocol`, l'**arbitrage
export/affichage** des colonnes masquées, et le choix entre attendre `sqlx` ou
réécrire le driver PostgreSQL en `tokio-postgres`.

Les ADR y font référence sans les définir. Les jalons ci-dessous marqués
**[ADR]** sont des contraintes déjà tranchées, extraites des ADR :

| Contrainte | Source |
|---|---|
| L'UI conditionnelle aux capacités est une discipline **dès la phase 0** | [ADR-0003](adr/0003-driver-capabilities.md) |
| ~~La bascule vers egui reste possible jusqu'à la fin de la phase 1~~ — sans objet depuis le remplacement de GPUI | [ADR-0001](adr/0001-ui-toolkit.md), remplacé par [ADR-0029](adr/0029-interface-tauri-shadcn.md) |
| Aucun driver des **phases 0 à 3** n'a besoin du sidecar | [ADR-0007](adr/0007-driver-sidecar.md) |
| Les plugins WASM arrivent en **phase 4**, après 6+ drivers natifs | [ADR-0005](adr/0005-wasm-plugins.md) |
| Le sidecar arrive en **phase 4** | [ADR-0007](adr/0007-driver-sidecar.md) |

Le reste — le contenu de chaque phase et sa porte de sortie — est **proposé** et
demande validation. Il n'a pas valeur d'autorité tant que le premier commit de
code ne l'a pas éprouvé.

> **Ce que le §11 d'ARCHITECTURE ordonnait et que ces phases ne placent pas**,
> recopié à son retrait le 2026-09-25 pour ne pas le perdre. Rien n'y est
> tranché : ces éléments attendent d'être rangés dans une phase, ou abandonnés
> explicitement.
> - avant l'IA, un client SQL qui se suffit à lui-même : DuckDB, ClickHouse,
>   export, historique, **édition de données avec prévisualisation du DML** ;
> - après l'IA, une phase « au-delà du relationnel » — MongoDB, Redis,
>   Elasticsearch/OpenSearch — placée **avant** les plugins parce que c'est là
>   que le modèle de capacités et `QueryLanguage` sont mis à l'épreuve : s'ils
>   sont mal conçus, mieux vaut le découvrir maintenant qu'au vingtième driver ;
> - en élargissement : Neo4j, Qdrant, Cassandra, DynamoDB, Influx, agents
>   restants, diagrammes ER, dictionnaires de données, comparaison de versions.

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

- fenêtre, éditeur de requête, grille virtualisée sur `RecordBatch` — livrés
  d'abord en GPUI dans `oxyn-ui`, câblés par `oxyn-app`, puis portés dans
  `apps/desktop` et `oxyn-desktop` ; les deux crates GPUI ont été retirées le
  2026-09-18 ([ADR-0029](adr/0029-interface-tauri-shadcn.md)) ;
- annulation de bout en bout, y compris côté serveur ;
- les cinq états de vue ([UX-SPEC](UX-SPEC.md#états-dune-vue)).

**Porte de sortie** : les budgets de [PERFORMANCE](PERFORMANCE.md) sont
**mesurés**, pas supposés — c'est la première campagne de mesure, et elle
confirme ou amende les budgets par un ADR.

> **Repris du §11 d'ARCHITECTURE**, retiré le 2026-09-25 au profit de ce plan.
> Son critère de sortie éprouve la porte ci-dessus sur un cas précis : un
> `SELECT` de 10 M de lignes, premier affichage dans le
> [budget de PERFORMANCE](PERFORMANCE.md#budgets-dinteraction), mémoire stable,
> `Échap` qui annule vraiment. **Mesuré**
> ([PERFORMANCE](PERFORMANCE.md#confrontation-aux-budgets)) : le premier lot en
> 2,6 ms quelle que soit la taille de la table, et 2 Gio qui traversent un tampon
> de 256 Mo pour 195 Mio de croissance RSS. **Reste à produire** : ce `SELECT` de
> bout en bout, et le défilement d'une grille peuplée sous instrument, désormais
> dans la webview. L'éditeur est CodeMirror 6, coloré par dialecte, avec la
> complétion de base de `basicSetup` ; une complétion nourrie du catalogue n'y est
> pas branchée.

> **[ADR]** ~~C'est la dernière phase où la bascule vers egui reste une réécriture
> de deux crates ([ADR-0001](adr/0001-ui-toolkit.md)). Après, le coût change de
> nature. Si GPUI doit être remis en cause, c'est ici.~~ — GPUI a été remis en
> cause par [ADR-0029](adr/0029-interface-tauri-shadcn.md) et retiré le
> 2026-09-18 ; ce sont désormais les conditions de reconsidération d'ADR-0029
> qui valent.

## Phase 2 — Les protocoles qui comptent

- `oxyn-driver-postgres`, `oxyn-driver-mysql` — **ce dernier reporté**, voir
  ci-dessous ;
- `oxyn-catalog` : introspection, cache, arborescence ;
- marquage d'environnement des connexions et stockage au trousseau
  ([SECURITY](SECURITY.md)).

**Porte de sortie** : la [liste de contrôle driver](../.claude/checklists/revue-driver.md)
passe intégralement sur les trois drivers, annulation côté serveur comprise.

**`oxyn-driver-mysql` est reporté après la porte de sortie de la phase 3** —
décision du 2026-09-24. La phase 3 a un lot daté au plus tard au 2026-10-31
(ci-dessous) ; le driver MySQL s'écrit juste après, par
[`/driver`](../.claude/commands/driver.md) et sa liste de contrôle,
`KILL QUERY` compris. **Raison :** la phase 3 est engagée et a une échéance
datée ; ouvrir un troisième protocole en même temps disperserait le travail,
alors que deux drivers livrés, PostgreSQL et SQLite, suffisent à éprouver les traits de
[`oxyn-driver`](../crates/oxyn-driver/src/traits.rs). Tant que MySQL manque, la
porte de sortie de la phase 2 n'est **pas franchie** : elle nomme trois drivers,
et ce report ne la réduit pas à deux.

## Phase 3 — Le workspace IA

- `oxyn-ai` : fournisseurs local et distant, point de passage unique ;
- niveaux de confidentialité par connexion ([ADR-0006](adr/0006-ai-privacy-tiers.md)) ;
- agents proposant des `Command` portant `Actor::Agent`.

**Porte de sortie** : Oxyn reste un client complet, sans dégradation, avec zéro
fournisseur configuré — vérifié par un test, pas par conviction.

**Fait le 2026-09-24 — l'échantillon ne lit que les colonnes cochées.** La
lecture d'un échantillon, épinglé ou demandé par un agent
([ADR-0034](adr/0034-echantillon-pour-toute-destination.md)), est un
`PreviewRelation` dont la forme porte une projection : `PreviewShape::columns`,
des noms que le driver vérifie contre la relation et cite
([I-10](../CLAUDE.md#i-10), [DRIVER-CONTRACT](DRIVER-CONTRACT.md#6-il-échappe-tout-identifiant-quil-compose)).
Le serveur ne rend que les colonnes approuvées, PostgreSQL et SQLite compris ;
rien de non coché n'entre dans le `ResultBuffer`.

**Reste à faire, noté le 2026-09-24 — le journal des commandes ne montre pas la
projection.** `audit_journal` n'enregistre aucun texte pour un
`PreviewRelation` : ni tri, ni prédicat, ni colonnes. La projection se retrouve
dans la `Command` et dans l'audit `ai_egress`, relié à l'entrée du journal par
`command_id`, mais pas dans le journal lui-même. Y écrire le SQL composé ferait
entrer le prédicat de l'utilisateur au journal : c'est une décision à part
entière, pas un effet de bord de la projection. **Ce qui le débloque** : cette
décision, prise pour tous les aperçus et non pour le seul échantillon.
**Échéance** : avant la porte de sortie de cette phase, et au plus tard le
2026-10-31.

## Phase 4 — Extension et isolation

**[ADR]** Ce qui a été délibérément reporté ici :

- `oxyn-plugin` : hôte wasmtime, interfaces WIT ([ADR-0005](adr/0005-wasm-plugins.md)) ;
- `oxyn-driverd` : sidecar pour Oracle, Couchbase, SDK cloud ([ADR-0007](adr/0007-driver-sidecar.md)) ;
- élargissement aux familles NoSQL, vectorielle, graphe, séries temporelles.

**Condition d'entrée**, et non de sortie : au moins six drivers natifs livrés.
Ouvrir une frontière d'extension sur des traits que trop peu d'implémentations
ont éprouvés fige des erreurs qu'il faudra ensuite supporter indéfiniment.

## Vérification exigence par exigence — 2026-09-15

Chaque ligne nomme **le test ou le code qui la tient**, pas une impression. Une
exigence sans preuve nommée est marquée non tenue, même si le code semble la
couvrir.

### Aperçu de table — tenu

| Exigence | Ce qui la tient |
|---|---|
| `PreviewSort`, `PreviewFilter`, pagination dans la commande | `oxyn-core/src/preview.rs` (`PreviewShape` : `sort`, `filter`, `offset`), porté par `Command::PreviewRelation` |
| Capacités | `Capabilities::PREVIEW_SORT`, `PREVIEW_FILTER` |
| SQL composé avec identifiants cités, les deux drivers | `quote_identifier` dans `drivers/oxyn-driver-sqlite/src/preview.rs` et `.../postgres/src/preview.rs` |
| Tri déterministe | `a_page_is_offered_only_where_the_order_is_total` — une page suivante n'est offerte que si l'ordre est total |
| Annulation | `escape_in_the_filter_field_still_stops_the_read_at_the_server` |
| Pas de réexécution involontaire | `a_stale_answer_never_overwrites_a_newer_filter` |
| État vide | `a_predicate_that_matches_nothing_is_empty_not_failed`, `an_empty_answer_says_whether_a_filter_caused_it` |
| État en cours | `typing_dispatches_nothing_and_the_bar_says_the_draft_is_not_in_force` |
| État erreur | `an_invalid_predicate_shows_the_server_message_and_keeps_no_false_rows` |
| Filtré / trié | `a_predicate_filters_the_real_rows_and_leaves_the_sql_draft_alone`, `a_sort_changes_the_order_the_server_returns` |
| Page suivante | `two_consecutive_pages_do_not_overlap` |
| Capacité absente | `a_session_without_the_capabilities_has_no_bar_and_dispatches_nothing` |

### Sécurité et IA — tenu, sauf une garantie de type

| Exigence | Ce qui la tient |
|---|---|
| Refus d'écriture production à un agent | `oxyn-core/src/policy.rs` : `actor.is_agent() && mutating && env.is_production()` rend `Decision::deny` — un refus, pas une confirmation ([I-02](../CLAUDE.md#i-02)). Test : `un_agent_n_est_jamais_moins_restreint_qu_un_humain` |
| Point de passage unique du contexte | `ContextBuilder::build` (`oxyn-ai/src/context.rs`), seule fabrique d'`AgentContext` |
| Aucune sortie serveur brute | `FailureReport` filtre **à la construction**, sous le niveau de la connexion |
| Aucune sortie IA exécutée | `open_proposal` ouvre une console et n'exécute rien ; refuse même d'écrire sans provenance |
| Provenance persistante | colonne `documents.provenance`, `coalesce` à l'écriture, visible sur l'onglet **et** dans la bibliothèque |
| Test de sentinelle | `aucune_sentinelle_natteint_le_fichier_de_workspace` — balaie **toutes** les tables et colonnes via `sqlite_master`, sans en nommer aucune |
| Porte d'I-04 typée pour **les deux** destinations | **tenu depuis le 2026-09-15.** `run_turn` prend un `AgentPrompt`, dont le seul constructeur exige le niveau ([ADR-0027](adr/0027-porte-unique-pour-les-deux-destinations.md), option B). `run_turn` garde sa propre vérification : la porte protège l'assemblage, la vérification protège le lancement. Tests : `sous_local_aucune_invite_ne_se_compose`, plus deux `compile_fail` qui interdisent tout constructeur naïf |
| I-03 — canal journal | Test **préventif** : rien ne formate un secret sur ce chemin aujourd'hui, donc retirer une garde ailleurs ne le fait pas rougir tout seul ; vérifié à la main en ajoutant un `tracing::debug!` qui formate une clé de fournisseur dans `save_ai_provider`, en confirmant le rouge, puis en revenant en arrière. `no_sentinel_reaches_the_journal` (`crates/oxyn-desktop/src/sentinel_tests.rs`) — toute la sortie tracing du binaire à `OXYN_LOG=trace`, à travers les deux filtres réels de `logging::layer`, pendant une connexion refusée et une édition de fournisseur |
| I-03 — canal erreur affichée | Éprouve le retrait d'une garde existante — `redact_key` (`oxyn-llm`). `no_sentinel_reaches_an_error_shown_to_the_front` (`crates/oxyn-desktop/src/sentinel_tests.rs`) — `IpcError` sérialisée et tous les messages du `Channel<AiUpdate>`, avec un fournisseur hostile qui recopie la clé reçue et une URL de fournisseur qui porte un secret |
| I-03 — canal invite IA | Test **préventif** : `ContextBuilder::build` ne prend ni la connexion ni le fournisseur en argument, donc rien n'y formate le secret aujourd'hui ; vérifié à la main en faisant porter la clé du fournisseur dans la question assemblée par `converse` (`backend/ai/conversation.rs`), en confirmant le rouge, puis en revenant en arrière. `no_sentinel_reaches_the_prompt_sent_to_a_provider` (`crates/oxyn-desktop/src/sentinel_tests.rs`) — le corps exact de la requête `chat/completions` envoyée au fournisseur, prompt assemblé par `ContextBuilder` compris |
| I-03 — canal presse-papiers | Éprouve le retrait d'une garde existante — le choix de l'appelant de `copyToClipboard` de ne jamais lui passer un identifiant de connexion ou de session. `describe("copying from an open connection's object inspector")` / `it("copies the qualified name only, never the connection or session that opened it")` (`apps/desktop/src/features/metadata/clipboard.test.tsx`) — limité aux copies qui passent par `copyToClipboard` (`src/features/metadata/clipboard.ts`) ; les copies directes d'`assistant-panel.tsx` et de `result-grid.tsx` n'y sont pas couvertes |
| I-03 — canal rapport de plantage | **N'existe pas.** Aucune crate de rapport de plantage (`sentry`, `minidump`, `crashpad` absents de `Cargo.lock`), aucun `std::panic::set_hook`, et `panic = "abort"` en profil `release` (racine `Cargo.toml`). Le seul texte d'un plantage est le message de panique par défaut sur `stderr` : rien à balayer |

### Reprise et bibliothèque — tenu en partie

Relu le 2026-09-25 : les tests marqués « supprimé » vivaient dans `oxyn-app` et
ont disparu avec `6ecb8ce`, sans équivalent dans `apps/desktop`. Leur ligne
n'est plus tenue ; la [porte de la migration](#migration-vers-linterface-tauri)
dit ce qui manque.

| Exigence | Ce qui la tient |
|---|---|
| Marqueur d'arrêt propre/anormal | `PreviousShutdown { Never, Clean, Abnormal }` ([ADR-0021](adr/0021-marqueur-d-arret.md)) |
| Tests de crash/restart | `une_premiere_ouverture_ne_signale_aucun_arret_anormal`, `une_fermeture_ordinaire_ne_declenche_pas_la_reprise`, `une_session_laissee_ouverte_et_muette_est_un_arret_anormal`, `une_instance_qui_bat_encore_n_est_pas_un_plantage`, `un_battement_ne_ressuscite_pas_une_session_fermee` (`oxyn-store/src/sessions.rs`) |
| Emplacement d'objet et sous-onglet restaurés sans lecture | **non tenu** — `a_restored_location_and_its_sub_tab_come_back_without_reading_anything` supprimé |
| Restauration sans écraser les données | **non tenu** — `a_restored_object_that_vanished_is_explained_and_never_erased` supprimé |
| Connexions supprimées ou hors workspace dans les filtres | **non tenu côté interface** — `a_deleted_connection_stays_choosable_in_the_history_filter` et `merging_history_connections_appends_and_marks_without_moving_ranks` supprimés |
| Pagination bornée | `document_pages_are_bounded_literal_and_do_not_open_oversized_bodies` (`oxyn-store`) |
| Inspection en lecture seule | **non tenu** — `saved_and_working_copies_are_inspected_without_modification` et `history_and_recent_results_read_the_same_execution_without_replaying_it` supprimés |

### Ce qui reste non tenu, et pourquoi

> **Une exigence a été écartée par son commanditaire, et c'est une contradiction
> qu'il faut lire comme telle.** Le plan de travail de cette session demandait
> « lancer la recette native de l'application à plusieurs tailles et dans les
> deux thèmes » **et**, dans la même phrase, « sans perturber l'ordinateur de
> l'utilisateur ». Interrogé le 2026-09-15, celui-ci a maintenu son instruction
> du 2026-09-10 : aucune fenêtre ne s'ouvre sur son écran.
>
> Les deux moitiés de l'exigence ne peuvent donc pas être satisfaites ensemble
> sur cette machine. Ce n'est pas un arbitrage qu'un agent peut rendre à sa
> place, et il a été rendu : **la seconde moitié l'emporte**. Les trois lignes
> ci-dessous en découlent, et n'ont pas vocation à être comblées tant que cette
> décision tient.


| Exigence | État |
|---|---|
| **UI native** — recette à plusieurs tailles et dans les deux thèmes | **non tenu, et c'est une décision.** Interrogé le 2026-09-15, l'utilisateur a **maintenu** son instruction du 2026-09-10 : aucune fenêtre n'est ouverte sur son écran. Ce n'est donc pas un reste à faire qu'on aurait oublié, mais un arbitrage assumé entre la preuve pixel et le fait de ne pas interrompre son travail. Aucun test GPUI ne la remplace : le harnais installe une métrique de texte déterministe *et* fausse |
| **Accessibilité** — recette clavier complète | partiel. Les points bloquants de [revue-ui](../.claude/checklists/revue-ui.md) ont leurs tests ; le `Close` d'onglet a été rendu atteignable **sans** test, pour une raison écrite à côté du code |
| **Performance** — défilement d'une grille peuplée | non mesuré : demande `xcrun xctrace` attaché au processus, donc l'application lancée. La **valeur 256 Mo du tampon**, elle, est désormais mesurée — 2 Gio poussés, RSS +195 Mio, 1,84 Gio sur disque ([PERFORMANCE](PERFORMANCE.md)) ; seule son **automatisation** reste bloquée par [I-03](../CLAUDE.md#i-03) ou `unsafe_code = "deny"` |
| **Figma** — fidélité visuelle | les neuf planches sont confrontées *en mesures et en textes* ; la fidélité **pixel** relève de la recette native |

## Journal de validation du 2026-09-15

Trois relectures indépendantes à périmètres disjoints — invariants, sécurité,
divergences code/docs — puis confrontation des frames Figma au serveur. Ce qui
suit est le relevé de ce qui a été **trouvé**, pas de ce qui a été parcouru.

### Défauts de code trouvés et corrigés

Chacun est fermé par un test, et chaque test a été éprouvé par sabotage : la
garde retirée, le test rougit. Les sabotages ont été défaits.

| Défaut | Pourquoi il ne rougissait nulle part |
|---|---|
| **Le mode agent externe était inatteignable** — la garde de `ask_assistant` et `provider_for` décidaient séparément, avec des conditions qui se recouvraient : passé la garde, `provider_for` rendait toujours `Some`, et un utilisateur n'ayant déclaré qu'un agent voyait « aucune IA » | tout le lot ACP compilait, et sa sécurité — permissions, refus par défaut, contrôle de niveau — n'était exercée par aucun chemin réel |
| **Le bouton d'arrêt d'un tour d'agent ne coupait rien** : le jeton était créé, rendu au panneau, jamais transmis à `run_turn` | l'affichage passait bien en « arrêt en cours » ; seul le sous-processus continuait |
| **Le panneau mourait après un tour d'agent** — pas de remise à zéro d'`active` | le chemin fournisseur, lui, l'avait ; rien ne comparait les deux |
| **`Debug` d'`AiProviderConfig` rendait l'URL entière**, en se fiant à `validate()` — qui n'est appelée qu'à l'autre bout du bus | une clé collée dans le champ « point d'accès » vivait en clair dans une valeur en transit |
| **Les valeurs d'environnement d'un agent n'étaient ni bornées ni assainies** | un NUL y est tronqué en silence au lancement : l'agent reçoit autre chose que ce qui est affiché **et** persisté |
| **La provenance d'agent manquait dans la bibliothèque** — `DocumentSummary` n'avait aucun champ pour elle | la marque était visible sur l'onglet, c'est-à-dire là où l'on n'en a pas besoin, et absente là où l'on relit un texte un an plus tard ([ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md)) |
| **`Metrics::toolbar_height` valait 52 ; la maquette dit 48** | documenté, sourcé, daté **et** ancré par un test vert — mais le champ n'avait aucun lecteur, la vue écrivant `48` en dur. Cas d'école d'[I-12](../CLAUDE.md#i-12) |
| `Open Indexes` absent (`229:8021`), titre de l'avertissement d'écriture absent (`232:9100`), infobulle d'`Edit rows…` absente, message à vingt espaces | écarts d'interface qu'aucune compilation ne voit |
| **Le bandeau d'[I-13](../CLAUDE.md#i-13) de la bibliothèque n'existait pas** (`268:36866`) — le texte était là, en fin d'une ligne grise, après deux autres mentions | ce que cette garantie permet, c'est de parcourir son historique **sans craindre qu'un clic rejoue une écriture** ; une note de bas de ligne ne porte pas cela, et c'est précisément l'entrée à l'issue inconnue qu'on a le plus besoin d'inspecter |
| **Le `Close` d'un onglet de console était inutilisable au clavier** — visible, cliquable, sans `tab_index` | `⌘W` ne le couvre pas : il ferme la console **active**, et seulement depuis le panneau SQL. Fermer un autre onglet n'avait donc aucun chemin clavier — le point bloquant de [revue-ui](../.claude/checklists/revue-ui.md). Corrigé **sans test** : le vérifier demanderait d'atteindre ce bouton par tabulations depuis un point connu, or leur nombre dépend des onglets ouverts ; un test qui poserait le focus par un clic serait vert sans rien prouver |

### Mesure de mémoire

400 Mio poussés dans un `ResultBuffer` borné à 4 Mio : croissance du RSS de
**4,25 Mio**, 404 Mio débordés sur disque. Contrôle de sensibilité : le même
scénario sous un budget de 400 Mio fait croître le RSS de 317 Mio. La mesure
discrimine donc d'un facteur ~75, et [I-06](../CLAUDE.md#i-06) tient sur la
mémoire du **processus**, pas seulement sur la comptabilité du tampon. Détail et
raison de sa non-automatisation : [PERFORMANCE](PERFORMANCE.md).

### Frames confrontées au serveur Figma

Aperçu et inspecteur (`190:1163`), contraintes (`229:7637`), relations
(`229:32690`), reprise (`232:9100`), barre de console (`191:1958`), barre
latérale (`221:4573`), préférences (`47:8222`). Relevés et écarts assumés :
[FIGMA-HANDOFF](FIGMA-HANDOFF.md).

La planche de bibliothèque (`47:7638`) a d'abord été crue introuvable : la
liste de pages rendue par le serveur n'en montrait que trois, alors que le
fichier en compte trente. Elle a été retrouvée en énumérant `figma.root.children`
par l'API Plugin, en lecture seule. **Les neuf planches sont donc confrontées.**

### Ce que cette session n'a pas pu faire

- **La recette native**, donc toute preuve pixel. L'instruction de l'utilisateur
  du 2026-09-10 — « les interactions de bureau ont été arrêtées à sa remarque »
  — n'a pas été levée, et elle n'a pas été contournée. « UI native » reste donc
  **non tenu**, et aucun test GPUI headless ne le remplace : le harnais installe
  une métrique de texte déterministe *et* fausse
  ([tests.md](../.claude/rules/tests.md#les-tests-dinterface)).
- Les questions ci-dessous, qui demandent un arbitrage.

## Questions ouvertes — non tranchées

Relevées le **2026-09-15** en confrontant les documents d'autorité au code.
**Aucune n'est tranchée ici** : chacune oppose deux documents d'autorité, ou un
document d'autorité au code, et c'est un arbitrage qui les départage — pas une
correction de rédaction. Elles sont listées pour qu'elles cessent d'être
invisibles, pas pour être résolues dans ce fichier.

### 1. Le tri par défaut d'un aperçu : l'ADR dit l'inverse du code

[ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md) affirme deux fois que, sans
tri demandé, le driver ordonne par la clé primaire seule, et compte parmi ses
conséquences « un tri imposé par défaut sur la clé primaire ».

**Le code fait l'inverse.** Les deux drivers ne composent aucun `ORDER BY` en
l'absence de demande — le test `un_apercu_sans_demande_ne_compose_ni_where_ni_order_by`
le tient explicitement dans `oxyn-driver-sqlite` comme dans `oxyn-driver-postgres`.
[UX-SPEC](UX-SPEC.md) confirme le code : « sans tri demandé, l'ordre des lignes
n'est pas garanti ».

**Ce qui n'est pas tranché.** Laquelle des deux positions est la bonne. L'ADR
justifiait l'ordre par défaut par la correction de `OFFSET` : paginer sur un ordre
non garanti duplique et omet des lignes en silence. Cet argument n'a pas été
réfuté — il a été contourné. Un ADR ne se réécrit pas en douce : c'est un
**nouvel ADR** qui doit dire lequel des deux comportements est voulu, et pourquoi
la pagination reste correcte dans le cas retenu.

**Répondue par [ADR-0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)**,
le nouvel ADR que ce paragraphe demandait : aucun ordre imposé, aucune page
offerte tant que l'ordre n'est pas total — c'est ainsi que la pagination reste
correcte. Il porte `accepté` depuis la
[revue du 2026-09-24](#3-le-statut-des-adr--revue-du-2026-09-24), qui cite ses deux
tests. Le statut d'ADR-0020, qu'il précise, relève de cette même revue.

### 2. Deux valeurs pour le seuil de premier affichage

**Résolue.** [ARCHITECTURE](ARCHITECTURE.md) ne chiffre plus ce seuil depuis le
2026-09-15 et renvoie aux
[budgets d'interaction de PERFORMANCE](PERFORMANCE.md#budgets-dinteraction) :
**300 ms après la première réponse du serveur**. Son §11, qui portait ce renvoi,
renvoie lui-même à ce plan depuis le 2026-09-25. Le constat d'origine suit.

[ARCHITECTURE](ARCHITECTURE.md) fixe le critère de sortie de la phase 0 à un
premier affichage **sous 100 ms** et déclare le critère non mesuré.
[PERFORMANCE](PERFORMANCE.md), qui **fait autorité sur les budgets chiffrés**,
fixe **300 ms** après la première réponse du serveur, et déclare le budget
confirmé pour SQLite (2,6 ms mesurées).

Les deux valeurs n'ont peut-être pas la même origine de comptage — l'une part du
lancement de la requête, l'autre de la première réponse du serveur. C'est
précisément ce qui rend l'écart inexploitable : **aucune régression ne peut être
arbitrée** tant que le seuil et son point de départ ne sont pas uniques.

**Ce qui n'est pas tranché.** Quelle valeur, comptée depuis quel instant. Une fois
décidé, un seul document porte le chiffre et l'autre y renvoie.

### 3. Le statut des ADR : revue du 2026-09-24

**Tranché le 2026-09-24.** Critère : un ADR passe `accepté` quand sa décision,
**telle qu'écrite**, est implémentée et tenue par au moins un test. ADR-0009
porte `remplacé` ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).

| ADR | Statut | Test qui la tient, ou raison du maintien |
|---|---|---|
| [0021](adr/0021-marqueur-d-arret.md) | accepté | `une_session_laissee_ouverte_et_muette_est_un_arret_anormal` (`oxyn-store/src/sessions.rs`), `an_abandoned_session_is_reported_as_abnormal` (`oxyn-desktop/src/backend/recovery.rs`) |
| [0023](adr/0023-fournisseurs-declares-et-provenance.md) | accepté | `aucune_url_porteuse_d_identifiants_n_atteint_le_disque` (`oxyn-store/src/providers.rs`), `un_point_d_acces_non_resolu_ne_beneficie_pas_du_doute` (`oxyn-llm/src/reach.rs`) |
| [0026](adr/0026-agents-externes-acp.md) | accepté | `un_agent_externe_ne_sert_jamais_une_connexion_locale` (`oxyn-ai/src/privacy.rs`), `la_table_na_aucune_colonne_de_secret` (`oxyn-store/src/agents/tests.rs`) |
| [0027](adr/0027-porte-unique-pour-les-deux-destinations.md) | accepté | `sous_local_aucune_invite_ne_se_compose` (`oxyn-ai/src/external/prompt/tests.rs`), `le_niveau_local_refuse_avant_meme_de_lancer_le_processus` (`oxyn-ai/src/external/tests.rs`) |
| [0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md) | accepté | `un_apercu_sans_demande_ne_compose_ni_where_ni_order_by` (les deux drivers), `no_page_is_offered_without_a_total_order` (`oxyn-desktop/src/ipc/metadata.rs`) |
| [0029](adr/0029-interface-tauri-shadcn.md) | accepté | `controler_graphe_dependances` de `.claude/verifier_socle.py` (par `make socle`), `a_window_crosses_batches_and_stays_bounded` (`oxyn-desktop/src/backend/results.rs`). La campagne de mesure est sa condition de **reconsidération**, pas un préalable |
| [0031](adr/0031-validation-des-reponses-ipc.md) | accepté | « rejects what the grid could not draw, rather than letting it through » (`apps/desktop/src/lib/ipc/types.test.ts`), « degrades an unknown ending instead of refusing the event » (`ai.test.ts`) |
| [0032](adr/0032-agent-externe-confine-au-lancement.md) | accepté | `claude_agent_keeps_no_tool_of_its_own` (`oxyn-ai/src/external/confine.rs`), `a_confined_agent_is_put_in_its_mode_first_and_cut_off_as_soon_as_it_leaves` (`external/session/tests.rs`) |
| [0033](adr/0033-couches-de-configuration-codex.md) | accepté | `every_readable_layer_is_switched_off_by_name_each_once` (`oxyn-ai/src/external/confine/codex_layers.rs`) |
| [0034](adr/0034-echantillon-pour-toute-destination.md) | proposé | **écart code/ADR** : son § 2 dit que les colonnes « ne rejoignent aucune instruction », alors que depuis `d6cf80b` elles entrent dans le `PreviewRelation` par `PreviewShape::columns`, citées par le driver ([Phase 3](#phase-3--le-workspace-ia)). Le reste est tenu (`an_external_agent_receives_only_the_columns_the_user_ticked_and_only_after`) ; la phrase est à préciser avant d'accepter |
| [0005](adr/0005-wasm-plugins.md) | proposé | WIT et instanciation des composants renvoient « phase 4 » (`oxyn-plugin/src/host.rs`) |
| [0007](adr/0007-driver-sidecar.md) | proposé | aucun sidecar, reporté en phase 4 |
| [0020](adr/0020-apercu-trie-filtre-parcouru.md) | proposé | son ordre par défaut sur la clé primaire est contredit par le code et par ADR-0028, qui le précise |
| [0022](adr/0022-rafraichissement-automatique.md) | proposé | un DDL ne relit pas l'aperçu visible, alors que l'ADR le prévoit ; `RefreshSignal::of` n'a aucun test |
| [0024](adr/0024-autosauvegarde-au-repos-de-frappe.md) | proposé | implémenté (`DRAFT_IDLE_MS`), mais ni le délai ni les trois écritures immédiates ne sont testés |
| [0025](adr/0025-proposition-de-changement-de-schema.md) | proposé | le composeur Rust est testé, mais le geste n'est pas monté : `useSchemaProposal` n'a aucun appelant |
| [0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) | proposé | **écart code/ADR** : le § 2 bis promet une écriture par question, `WriteGate` applique « une demande en attente à la fois » (`nothing_runs_while_a_request_waits_even_from_an_earlier_question`) — à arbitrer |

ADR-0035 était déjà `accepté` ; ADR-0036, écrit le jour même, n'a pas été revu.

**Ce qui reste signalé, sans être corrigé ici.** Plusieurs ADR acceptés citent
encore `oxyn-app` ou GPUI comme lieu d'implémentation (0021, 0023, 0026, 0029) :
leur décision tient, leur texte a vieilli, et un ADR accepté ne se réécrit pas.
Les deux doctests `compile_fail` d'ADR-0027 (`prompt/tests.rs`) vivent dans un
module `#[cfg(test)]` que rustdoc ne parcourt pas : ils ne s'exécutent jamais.

Ce qui suit est l'état relevé le 2026-09-15, conservé.

Seuls six ADR étaient alors au statut `accepté` (0008, 0010, 0014, 0015, 0016,
0017) ; les vingt autres restaient `proposé`, alors que la décision de la plupart
d'entre eux était implémentée et éprouvée par des tests.

**Pourquoi ce n'est pas cosmétique.**
[.claude/rules/documentation.md](../.claude/rules/documentation.md) fait reposer
sur ce statut la protection « un ADR **accepté** ne se réécrit pas ». Tant que
tout est `proposé`, cette protection ne s'applique **nulle part**. Ce n'est pas
théorique : c'est par réécriture successive qu'[ADR-0026](adr/0026-agents-externes-acp.md)
s'est retrouvé à porter, sur une même page, une affirmation et son contraire au
sujet de la dépendance `agent-client-protocol`.

**Ce qui n'était pas tranché** — et l'est depuis, voir le tableau ci-dessus.
Quels ADR passent à `accepté`, et selon quel critère — la décision prise, ou la
décision implémentée et mesurée. Le passage au
statut `accepté` verrouille la réécriture : c'est une décision de gouvernance, pas
un balayage de champs à faire en lot sans relecture.

### 4. Le budget des conversations est écrit, mais rien ne l'applique

Relevé le **2026-09-23**. [PERFORMANCE](PERFORMANCE.md) fixe le budget disque de
l'historique de l'assistant — 200 fils, 90 jours d'inactivité, 32 Mio de
transcript par workspace — « appliqués par `Conversations::prune` ».
`Conversations::prune` et `RetentionPolicy::default()` existent dans
`oxyn-store` et sont éprouvés par ses tests, mais **aucun appelant hors de ces
tests** : l'application n'élague jamais. Le budget ne tient donc pas, et c'est
précisément la croissance qu'il devait empêcher.

**Corrigé le 2026-09-23.** `Backend::prune_conversations` applique
`RetentionPolicy::default()` une fois par lancement, sur le pool bloquant, sans
retarder l'ouverture. On élague au lancement plutôt qu'après chaque échange : le
budget borne des mois d'usage, pas une session, et une suppression en pleine
conversation pourrait emporter le fil que l'utilisateur lit.

**Reste ouvert :** faut-il dire à l'utilisateur qu'un fil a été élagué ?
[UX-SPEC](UX-SPEC.md#une-conversation-reste-dans-le-workspace-pas-ce-quon-lui-a-montré)
ne le prévoit pas. Aujourd'hui, seul un journal `info` le consigne.

## Ce qui n'a pas sa place ici

Les décisions. Une phase qui a besoin d'un arbitrage écrit un ADR
([`/adr`](../.claude/commands/adr.md)) ; elle ne tranche pas dans ce fichier.
