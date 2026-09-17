# Index de la maquette pour l'implémentation

État du relevé : 2026-09-07. Fichier Figma : `Yviemi4brBczzdRdBp1ONv`.
Cet index localise les planches ; les comportements font autorité dans
[UX-SPEC](UX-SPEC.md), et le périmètre de livraison dans
[IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md).

## Référence commune

La page 22 porte le workbench retenu par
[ADR-0011](adr/0011-structure-commune-workspace.md). La sidebar est toujours
inset, repliable en icônes, selon la page 01. Les écrans des pages 04 à 09 ont
été migrés vers cette structure, en conservant leurs identifiants et contenus.

| Planche | Nœud Figma |
|---|---|
| Table, état peuplé | [190:1163](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=190-1163) |
| Console SQL, état peuplé | [191:1521](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=191-1521) |
| Structure d'objet | [193:1814](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=193-1814) |
| Largeur compacte, 1024 px, sombre | [215:3279](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=215-3279) |
| Largeur compacte, 1024 px, claire | [235:10352](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=235-10352) |
| Table avec sidebar repliée | [233:9139](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=233-9139) |
| Palette ouverte dans le workbench | [229:6949](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=229-6949) |
| Contraintes | [229:7637](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=229-7637) |
| Relations | [229:32690](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=229-32690) |

## Barre de la console

Relevé au serveur Figma le 2026-09-10. La frame `191:1958` mesure 1272 × 32 et
se pose à (12, 12) dans la console.

| Élément | Nœud | x | Largeur |
|---|---|---|---|
| Groupe Run / Stop / Explain | [273:37036](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=273-37036) | 0 | 286 (trois segments de 96) |
| Parameters · N | [191:1993](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=191-1993) | 294 | 144 |
| Mention de lecture seule | `191:2002` | 446 | 558 |
| Sélecteur connexion / schéma | [191:2003](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=191-2003) | 1012 | 260 |

Le segment sans objet est estompé : sur la planche, c'est Stop, parce que rien
ne tourne. Le sélecteur montre `commerce-prod / public` — un exemple, non une
connexion réelle ; ce qu'il change est tranché par
[ADR-0019](adr/0019-contexte-de-session.md).

## Barre de connexion

Relevé au serveur Figma le 2026-09-10, dans `190:1163`. La barre mesure
1296 × 48 (`190:1543`).

| Élément | Nœud | x | Largeur |
|---|---|---|---|
| `Toggle sidebar · ⌘B` | `221:4945` | 12 | 32 |
| Contexte `commerce-prod / commerce / public` | `190:1544` | 52 | 868 |
| Badge d'environnement | `190:1545` | 928 | 108 |
| Badge de confidentialité | `190:1547` | 1044 | 136 |
| `Ask AI` | `190:1549` | 1188 | 96 |

L'entrée du workspace IA est donc **dans cette barre**, à côté du badge qui
annonce le niveau de confidentialité — les deux se lisent ensemble. Sur la
planche de récupération (`232:9100`), `Ask AI` et ce badge sont marqués
`hidden` : l'entrée est conditionnelle, elle n'existe pas quand il n'y a rien à
demander. Aucun niveau ne s'affiche tant qu'aucun fournisseur n'est configuré
([ADR-0006](adr/0006-ai-privacy-tiers.md)).

Relevé au serveur le 2026-09-10, le bouton `190:1549` mesure 96 × 32, posé à
(1188, 8) : icône 16 × 16 à x = 16, libellé 40 × 20 à x = 40. Même gabarit que
les badges qui le précèdent.

### Ce que la maquette ne montre pas

**Il n'existe aucune planche du panneau de conversation ni de l'écran de
configuration des fournisseurs.** La maquette s'arrête au point d'entrée. Ce qui
existe, ce sont les briques : la page 23 (`Input Group`, saisie avec action
intégrée), la page 26 (`Message`, variantes utilisateur / assistant / outil), la
page 27 (`Attachment`) et les logos de fournisseurs de la page 02 (`244:1402`).

Les deux écrans sont donc **composés à partir de ces briques**, et leur
géométrie est assumée, pas relevée. Les comportements font autorité dans
[UX-SPEC](UX-SPEC.md#le-workspace-ia-nexiste-que-sil-a-été-configuré), et la
structure dans [ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md). À
reprendre au relevé le jour où les planches existent.

## Barre de l'aperçu de table

Relevé au serveur Figma le 2026-09-10, dans `190:1163`.

| Élément | Nœud | x | Largeur |
|---|---|---|---|
| Barre Data : `Refresh data` | `190:1589` | 0 | 130 |
| Groupe `Columns · N` / `Export preview…` | [272:10687](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=272-10687) | 138 | 252 (deux segments de 126,5) |
| Mention de lecture seule | `190:1611` | 398 | 598 |
| `Edit rows…` | `190:1612` | 1004 | 112 |
| `Why unavailable?` | `212:23799` | 1124 | 148 |
| Barre de filtre : champ `WHERE` + `Apply` | [272:10667](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=272-10667) | 0 | 1180 (saisie 1048, `Apply` 64) |
| `Sort` | `190:1621` | 1188 | 84 |

La barre de filtre (`190:1618`) mesure 1272 × 32 et se pose à (12, 92), sous la
barre Data. Le champ porte le préfixe littéral `WHERE` : c'est un prédicat que
l'utilisateur écrit, non un constructeur de conditions — arbitrage consigné dans
[ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md).

**Écart assumé** : la maquette ne dessine **aucun** contrôle de page. Son
`Result status` (`190:1860`) n'affiche que « 200 rows loaded · 284 ms · Total
count not requested ». La pagination décidée par l'ADR-0020 n'a donc pas de
place dessinée ; elle rejoint cette ligne d'état, à côté du compteur, faute de
frame qui la situe. À reprendre si la maquette la tranche plus tard.

### Le pied de l'aperçu, mot à mot

Relevé au serveur le 2026-09-15, par capture de `190:1860` — le nom du nœud
texte est tronqué dans les métadonnées et disait ici « not reported », ce qui
était faux. La maquette écrit exactement :

> 200 rows loaded · 284 ms · Total count not requested

Oxyn écrit « `N` rows shown · `D` ms · Total count not requested ». Deux écarts,
tous deux délibérés :

| Écart | Pourquoi |
|---|---|
| « shown » et non « loaded » | c'est le sujet même de cette ligne. Un aperçu borné à 200 lignes sur une table qui en contient cinquante millions ressemble en tout point à un aperçu complet ; « loaded » laisserait entendre que tout est là |
| pas de « Read only » | la barre Data (`190:1611`) porte déjà « Read-only preview » quarante pixels plus haut, et la maquette ne l'écrit qu'une fois. Le redire ici serait le doublon que la maquette évite |

**La durée, elle, manquait** et a été ajoutée : elle vient de la maquette, et
elle n'est pas décorative — c'est ce qui distingue une base lente d'un aperçu
qui n'a rien trouvé.

### L'inspecteur d'enregistrement, confronté le 2026-09-15

`Record inspector` (`190:1872`) mesure 280 × 640 et se pose à x = 992, après une
`ResizableHandle` (`190:1869`) de 8 px ; la grille occupe les 984 restants.
Vérifié dans le code : largeur par défaut 280 et poignée de 8 px, bornes
240–480 au clavier. `Inspect full value` et `Hide inspector` (256 × 32) sont
présents, ainsi que « Selected row · Read only ».

Les métriques de la grille concordent aussi : en-têtes de 28 px, lignes de
24 px — les valeurs du thème. Le contrôle de taille de lecture affiche
« Text · 13 px » comme la maquette, et « Text · 14 px » en lecture confortable.

## Confrontation du 2026-09-15 — contraintes, relations, reprise, console

Relevé au serveur, frame par frame, contre le code. Ce qui concorde n'est pas
listé ici sauf quand la valeur se vérifie ailleurs ; ce qui manquait l'est.

| Frame | Constat | Suite |
|---|---|---|
| Contraintes `229:7637` | proportions conformes — métadonnées 840 + poignée 8 + panneau DDL 424 = 1272, et 424 est bien la largeur par défaut du code. `Refresh structure`, `Copy DDL`, `Propose change…`, `DDL · Read only` et les deux synthèses sont là | **`Open Indexes` (`229:8021`) manquait** : le code n'avait que la phrase « listed under Indexes », qui envoie chercher sans mener. Ajouté, branché sur le `Control::Indexes` de la barre d'onglets et conditionné à la capacité |
| Relations `229:32690` | complète : « Incoming relationships », « Selected relationship », « Bounded related-row preview » et `Review related-row query`. Le modèle de requête cite ses identifiants et n'est exécuté par aucun chemin | rien à faire |
| Reprise `232:9100` | la table de choix, les cases, la phrase d'écriture interrompue et les issues sont là. Le badge de confidentialité et `Ask AI` sont masqués dans la frame, comme dans le code | **le titre « A write may have an unknown outcome » manquait** : c'est la phrase qui porte [I-13](../CLAUDE.md#i-13), et sans son titre elle se lit comme une note de bas de page entre une table et une rangée de boutons. Ajouté |
| Barre de console `191:1958` | composition conforme : groupe Run/Stop/Explain, `Parameters · N`, mention de lecture seule, sélecteur | deux écarts de libellé, ci-dessous |

### Écarts de libellé non comblés, et pourquoi

| Maquette | Oxyn | Raison |
|---|---|---|
| `Explain` (96 px) | `Explain query` | plus explicite hors contexte, mais **plus large que le segment de 96 px** que la maquette dessine. L'effet réel sur la mise en page ne se mesure pas sans pixels : le harnais GPUI a une métrique de texte fictive, et une assertion de largeur y serait verte à tort ([tests.md](../.claude/rules/tests.md#les-tests-dinterface)). À trancher à la recette native |
| `Run   ⌘↵` | `Run · ⌘Enter` | le raccourci en toutes lettres plutôt qu'en symboles ; même réserve de largeur |
| `Restore selected drafts` | `Restore N selected items` | délibéré et déjà décrit plus haut : le bouton annonce son décompte |
| `Start with an empty workspace` | `Continue without restoring` | dit ce que fait le geste plutôt que l'état d'arrivée ; rien ne se perd, les brouillons restent en base |

### La bibliothèque `47:7638`, confrontée le 2026-09-15

« Workflow / 07 · History & saved queries », page « 06 · Administration &
workflows ».

> **Comment elle a été retrouvée, et l'erreur que cela corrige.** Ce nœud était
> réputé introuvable : `get_metadata` sans `nodeId` ne rendait que **trois**
> pages, et l'index de ce document ne le citait pas. Le fichier en compte en
> réalité **trente** — la liste était partielle, pas le fichier. La planche a été
> retrouvée en interrogeant l'API Plugin en lecture seule (`use_figma`), qui
> énumère `figma.root.children`. À retenir pour la prochaine frame qu'on croira
> absente.

Concorde, et de près : les trois onglets `History` / `Saved queries` /
`Recent results` ; les cinq colonnes `Query` (440), `Connection` (300),
`Status` (180), `Duration` (136), `When` (192) ; les filtres recherche,
connexion, période et statut ; `Open retained result`, au même nom ; et le
panneau de détail, **238 px des deux côtés**.

**Ce qui manquait** : le bandeau `268:36866`, encadré en couleur de danger,
titré « Ambiguous writes are never replayed » et suivi de « An expired write may
have reached the server. History offers inspection and reconciliation, never a
retry action. » Le texte existait dans Oxyn — en fin d'une ligne grise, après
deux autres mentions. Il est désormais promu en bandeau, sur l'onglet Historique
seulement : c'est le seul qui liste des exécutions réelles, et l'avertissement
serait sans objet là où rien n'a jamais été écrit. La queue de phrase a été
retirée pour ne pas dire deux fois la même chose.

Écarts de nommage, assumés :

| Maquette | Oxyn |
|---|---|
| `Open SQL in editor` | `Open copy on <connexion>` et `Resume working query` — deux gestes distincts là où la maquette en dessine un, parce qu'ouvrir une copie et reprendre l'original n'ont pas les mêmes conséquences |
| `Save query`, `New saved query` | l'enregistrement se fait depuis la console, pas depuis la bibliothèque, qui est en lecture seule |

### L'écran de préférences `47:8222`, confronté le 2026-09-15

La frame se nomme « Workflow / 09 · Preferences & plugins ». C'est elle qui a
révélé la métrique fausse ci-dessus, par sa barre `47:8422`.

**L'écart est structurel, pas cosmétique** : la maquette organise les réglages
en **six onglets** — `Appearance`, `Editor`, `Results`, `AI providers`,
`Plugins`, `Diagnostics` — tandis qu'Oxyn les empile en une liste verticale
continue (apparence, confort de lecture, format, fournisseurs de modèles, état
d'enregistrement).

Ce qui concorde : les trois champs `Theme` / `Sidebar` / `Interface font` font
408 px, la hauteur de contrôle 38 px, et le bloc de confort de lecture existe
des deux côtés.

Absent, et **non ajouté** — chacun de ces points est une fonctionnalité :

| Élément | Ce qu'il supposerait |
|---|---|
| Onglet `Plugins` et sa table (`47:8480`) | l'hôte wasmtime, que [ADR-0005](adr/0005-wasm-plugins.md) place en phase 4 et dont la condition d'entrée — six drivers natifs livrés — n'est pas remplie. Un onglet vide serait pire que pas d'onglet |
| Onglet `Diagnostics` et `Preview diagnostics` | un rapport de diagnostic dont rien ne décrit le contenu ni la destination — or un rapport de plantage est l'un des six canaux d'[I-03](../CLAUDE.md#i-03) |
| Champ `Interface font` | le choix de la police d'interface ; le thème porte `ui_family`, mais rien ne l'expose ni ne le persiste |
| Champ `Result page size` | un réglage de taille de page ; Oxyn fige aujourd'hui l'aperçu à 200 lignes (`PREVIEW_ROWS`) |

Écart assumé : la maquette porte `Save settings` et `Cancel`. Oxyn **enregistre
au fil de l'eau** et l'annonce (« Preferences saved locally. »). Un bouton
d'enregistrement laisserait perdre un réglage en fermant la fenêtre, ce qu'une
sauvegarde immédiate rend impossible.

### La barre latérale, confrontée le 2026-09-15

Capture de `221:4573` — celle qu'emploient les planches de workspace, à ne pas
confondre avec `8:4` de la page 01, qui est la conception initiale. Les deux
sont presque identiques ; `221:4573` ajoute `AI workspace`.

**Le texte que le plan de travail nommait « Persistent workspace » n'existe
pas** : la maquette écrit « Personal workspace » sous la marque Oxyn, et c'est
exactement ce que le code affiche. Rien à corriger — la formulation attendue
était mal recopiée, et vérifier l'a montré.

Présent sous un autre nom, et c'est bien :

| Maquette | Oxyn |
|---|---|
| `Documentation` | `Workspace guide` |
| `Settings ⌘,` | `Settings · ⌘,` — et `⌘,` ouvre bien le panneau, c'est testé |
| `Query history` + `Saved queries`, deux entrées | `History & queries · ⌘⇧H`, une seule — la bibliothèque porte les deux onglets |
| `Connections` + `+` | `CONNECTIONS` + `New connection` |

Absent, et **non ajouté** : chacun de ces points est une fonctionnalité, pas un
ajustement d'affichage. Les inventer reviendrait à décider seul de parcours que
ni UX-SPEC ni un ADR ne décrivent.

| Élément | Ce qu'il supposerait |
|---|---|
| `Search anything…  ⌘K` | une recherche globale — objets, requêtes, historique — dont rien ne dit le périmètre ni le classement. `⌘K` n'est lié à rien aujourd'hui |
| `AI workspace` | une entrée de navigation vers un panneau IA plein écran ; aujourd'hui l'IA s'ouvre par `Ask AI` depuis la barre de connexion, conditionnée au niveau de la connexion (ADR-0006) |
| `Favorites` | un marquage d'objets favoris, avec sa persistance |
| `3 connections · 1 active` | un compteur : l'information existe, mais la maquette la place sous l'arbre et Oxyn dit déjà l'état de session ailleurs. À trancher avec le point suivant |
| Pied `Local workspace · Saved on this device` | Oxyn écrit `Connected · Current session` au même endroit, et la barre d'état dit déjà `UTC · Saved locally`. Reprendre le texte de la maquette **dupliquerait** cette dernière mention |

### Ce qui manque encore à la planche de reprise

`Reconnect manually before inspecting` (`232:9539`, 280 × 32) n'existe pas dans
le code. Le bouton suppose un parcours de reconnexion depuis l'écran de reprise
que rien d'autre ne décrit — ni UX-SPEC, ni un ADR. **Non implémenté
délibérément** : l'inventer reviendrait à décider seul ce qu'il fait d'une
session déjà ouverte, et sur l'écran dont tout le propos est de ne rien rejouer.

## États en situation

| État | Table | Console SQL |
|---|---|---|
| Initial | `224:29197` | `226:5719` |
| En cours, avec annulation | `224:29657` | `226:30837` |
| Vide | `224:30102` | `226:31227` |
| Erreur non retentable dans le contexte affiché | `224:30538` | `226:31615` |
| Capacité absente | `224:30974` | `226:32003` |

Ces variantes complètent les planches abstraites de la page 07. Elles ne
représentent aucune requête réellement exécutée.

## Édition et reprise

| Planche | Nœud | Statut |
|---|---|---|
| Cellule en édition, modification, ajout et suppression locaux | `231:7954` | Conception hors premier workspace en lecture seule |
| Revue DML sans transactions | `231:8531` | Conception hors premier workspace en lecture seule |
| Conflit, brouillon conservé | `232:8538` | Conception hors premier workspace en lecture seule |
| Récupération après arrêt brutal | `232:9100` | Sélection de brouillons, restauration locale hors ligne |
| Trois éléments restaurés | `282:13489` | Brouillons et emplacement d'objet, rien exécuté |
| Démarrage vide | `282:11550` | Workspace hors ligne |

Le parcours de reprise comporte huit combinaisons de sélection et leurs issues.
Les cases de la première colonne changent la sélection ; le bouton
`Restore N selected items` annonce son décompte et reste désactivé lorsque
la sélection est vide. Les cases vectorielles et leurs cellules portent
les mêmes destinations du prototype.

Les vues de moteurs avancés de la page 05 et leurs copies dans le prototype
portent une étiquette de phase 4. Cette étiquette ne prouve pas la disponibilité
du driver ou de la fonctionnalité.

## Variables et composants

Les corrections de lisibilité du 2026-09-07 conservent les collections,
les valeurs de couleur et les styles typographiques préexistants. Les quatre
états d'Attachment tronquent le nom sur une ligne ; les cellules de données
absentes utilisent la variante `Kind=Null` et le libellé `∅ NULL`.

La planche `223:29197`, page 03, présente les noms destinés à GPUI.
Les alias de couleur reprennent les valeurs de `Palette` dans
`crates/oxyn-ui/src/theme.rs`, sans modifier les couleurs préexistantes.
Les variables de dimensions portent les noms de `Metrics` ; les mesures du
workbench disposent également de noms explicites.

Les états des composants de base se trouvent dans les pages 10 à 20. `ReadOnly`
concerne les contrôles de valeur et les cellules ; un bouton ou un onglet
indisponible utilise `Disabled`. Le focus visible est un état propre.

Les pages 23 à 30 ajoutent des compositions réutilisables, employées dans les
écrans métiers :

| Page | Composant | Usage |
|---|---|---|
| 23 | Input Group | Recherche, conditions de filtre et saisie IA avec action intégrée |
| 24 | Button Group | Actions voisines et pagination |
| 25 | Item | Sources de contexte et listes compactes |
| 26 | Message | Messages utilisateur, assistant et outil |
| 27 | Attachment | Contexte joint, avec retrait local dans le prototype |
| 28 | Empty | États initiaux et résultats vides |
| 29 | Alert | Erreurs, refus et avertissements visibles |
| 30 | Accordion | Détails secondaires repliables |

Ces compositions suivent les conventions shadcn et les variables Oxyn. Les
avertissements de production et de confidentialité restent visibles ; une
confirmation destructrice n'est pas regroupée avec son annulation.

Pour un inventaire complet, lire les collections et variables locales via
`figma.variables.getLocalVariableCollectionsAsync()` et
`figma.variables.getLocalVariablesAsync()`. Une extraction limitée à un nœud
ne retourne que les variables utilisées dans ce sous-arbre.

## Logos des fournisseurs

La page 02 contient la planche `244:1402` et les composants
`Brand / Provider / …`. Quatorze logos ont été importés de
[SVGL](https://svgl.app/) via son
[dépôt source](https://github.com/pheralb/svgl/tree/main/static/library)
le 2026-09-07. Leurs géométries et couleurs de marque sont conservées.
Les versions claire et sombre suivent le mode de couleur du workspace.
Hugeicons reste la référence des actions et de la navigation.

Select et Combobox exposent `Leading icon` et `Show leading icon`. Les
points d'accès simplement compatibles avec une API gardent une identité
neutre : la compatibilité ne désigne pas le fournisseur effectif.

## Textes et limites de validation

### Corrections après analyse

| Sujet | Références |
|---|---|
| Confirmation de production sans transaction de staging imbriquée | `47:6811`, copies `62:9534` et `63:26236` |
| Panneaux de transaction staging séparés sur le canvas | `47:7046`, `62:9562`, `63:26264` |
| État cohérent du formulaire de connexion | `47:1137`, `62:8608`, `63:25091` |
| Grille occupant la hauteur disponible et export explicitement limité à l'aperçu | les dix variantes de table peuplée de la page 22 |
| Lecture confortable, large sombre / claire | `303:13195`, `303:13955` |
| Lecture confortable, 1024 px sombre / claire | `303:14712`, `303:15336` |
| Choix de lisibilité dans les préférences | `47:8222` ↔ `305:3416`, et les deux paires du prototype page 08 |
| Menu d'actions compact, thème clair | `306:14765` |

Les quatre paires de table sont reliées par le contrôle `Text · 13 px` /
`Text · 14 px`. La préférence est représentée par navigation entre variantes,
sans sauvegarde réelle dans le prototype. Les grilles peuplées contiennent
32 lignes de démonstration pour montrer l'occupation et le défilement ; le
compteur de 200 lignes reste une donnée du scénario, sans requête exécutée.

Les contrôles après correction portent sur les captures des écrans modifiés,
les dimensions, les 24 transitions de sélection des huit états de reprise,
les sept destinations de restauration et l'absence d'action de restauration
pour la sélection vide. Les huit transitions de taille de texte sont présentes.
Le contrôle final des pages 06, 08, 09 et 22 relève 1 178 liaisons sans
destination manquante. Les 15 pièces jointes des pages 04, 08 et 09 ne
débordent plus et conservent leurs 15 actions de retrait local.
Ces vérifications statiques ne constituent pas une recette au clavier dans
le lecteur Figma ou dans GPUI.

### Contrôles antérieurs et textes protégés

Les textes protégés restent localisés dans `86:4218`, `44:4045`, `190:1163`,
`47:6811`, `57:1547` et `91:3545`. Leur sens n'est pas remplacé par un changement
de style ou de structure. Les repères permanents suivent
[UX-SPEC](UX-SPEC.md#repères-permanents).

Les vérifications de maquette portent sur les captures, dimensions, propriétés
et destinations des interactions Figma. Elles ne valident pas l'exécution
GPUI, le réseau, une base réelle ni la restauration après plantage.

Recette antérieure à ces corrections : le trajet Table → Structure → Constraints
a été cliqué et contrôlé visuellement dans Figma Desktop. Le contrôle statique
des pages 08, 09 et 22 comptait alors 1 099 liaisons vers 146 destinations, sans référence
manquante. Cela ne constitue pas une recette interactive exhaustive. Les huit
sélections de reprise sont reliées à leurs issues ; leur recette complète dans
le lecteur reste à effectuer.

Le contrôle des variables préexistantes et des styles typographiques ne relève
aucune modification. Les textes des revues de production, des filtres liés et
des erreurs permanentes ou ambiguës sont conservés, y compris dans les slots
des composants composés.
