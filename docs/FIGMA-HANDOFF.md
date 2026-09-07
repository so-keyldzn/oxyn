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
Cliquer `Restore` ou `Skip` dans la première colonne change la sélection ;
`Restore selected drafts` est désactivé lorsque la sélection est vide.

Les vues de moteurs avancés de la page 05 et leurs copies dans le prototype
portent une étiquette de phase 4. Cette étiquette ne prouve pas la disponibilité
du driver ou de la fonctionnalité.

## Variables et composants

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

Les textes protégés restent localisés dans `86:4218`, `44:4045`, `190:1163`,
`47:6811`, `57:1547` et `91:3545`. Leur sens n'est pas remplacé par un changement
de style ou de structure. Les repères permanents suivent
[UX-SPEC](UX-SPEC.md#repères-permanents).

Les vérifications de maquette portent sur les captures, dimensions, propriétés
et destinations des interactions Figma. Elles ne valident pas l'exécution
GPUI, le réseau, une base réelle ni la restauration après plantage.
