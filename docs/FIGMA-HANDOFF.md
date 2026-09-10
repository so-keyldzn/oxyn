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
