# Comportements d'interface

> **Autorité** : ce que fait l'interface dans les situations où le comportement
> se décide, pas se devine. Ce document ne parle pas d'apparence.

Invariants concernés : [I-02](../CLAUDE.md#i-02), [I-05](../CLAUDE.md#i-05).

## États d'une vue

Toute vue qui dépend d'une opération distante a **cinq** états, et les cinq sont
dessinés avant d'être codés. Celui qu'on oublie est toujours le même — le vide —
et c'est le premier que voit un nouvel utilisateur.

| État | Ce qu'il montre |
|---|---|
| Initial | avant toute action ; explique quoi faire |
| En cours | progression et **moyen d'annuler** ; jamais un simple gel |
| Peuplé | le résultat |
| Vide | résultat légitimement vide ; se distingue visiblement d'une erreur |
| Erreur | ce qui a échoué, si c'est retentable, et l'action suivante |

## Ce qui n'est jamais optimiste

L'affichage optimiste — montrer le résultat avant confirmation du serveur — est
interdit pour toute opération qui écrit. Il est acceptable pour ce qui est
purement local (replier un nœud, réordonner des onglets).

**Panne concrète :** l'interface affiche la ligne comme mise à jour, le serveur
rejette pour cause de contrainte, et le message d'erreur est manqué. L'utilisateur
repart convaincu que sa correction est enregistrée.

## Les opérations destructrices

Une confirmation qui se clique par réflexe ne protège personne. Sur une
connexion marquée `production` ([SECURITY](SECURITY.md#marquage-des-connexions)),
la confirmation d'une écriture ou d'un DDL **nomme la connexion** et affiche le
SQL exact. Le bouton par défaut n'est jamais l'action destructrice.

## Annulation

Toute opération dépassant le budget de 300 ms
([PERFORMANCE](PERFORMANCE.md#budgets-dinteraction)) est annulable, et
l'annulation atteint le serveur
([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-il-expose-lannulation-et-lannulation-coupe-vraiment)).
Un bouton « Annuler » qui ne fait qu'abandonner l'affichage est un mensonge :
il laisse une requête tourner et une connexion prise.

## Les erreurs s'adressent à un professionnel

Le public d'Oxyn lit les messages de PostgreSQL. Un message d'erreur montre le
message du serveur — code compris — et pas une paraphrase rassurante. Ce qui
s'ajoute autour, c'est ce que le serveur ne dit pas : quelle connexion, quelle
requête, est-ce retentable.

Ce qui n'apparaît jamais dans un message : un identifiant de connexion, une
valeur liée ([I-03](../CLAUDE.md#i-03)).

## L'état vit dans le workspace, et il est lisible

Un onglet, une requête non exécutée, une connexion : ce qui est restauré au
redémarrage est écrit dans un format ouvert et documenté
([I-11](../CLAUDE.md#i-11)), lisible sans Oxyn.

## Ce qui n'est pas encore tranché

- la restauration après un arrêt brutal ;

## Structure commune du workspace

La structure retenue est le **workbench dense** de la page Figma
[22 · Database workspace](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=189-1163)
(arbitrage du 2026-09-07, [ADR-0011](adr/0011-structure-commune-workspace.md)).
Elle associe explorateur latéral, contexte de connexion, onglets d'objets et de
consoles, sous-onglets de l'objet, zone de travail et barre d'état. Un inspecteur
latéral peut compléter cette zone. Un en-tête de page de 88 px ne se rajoute pas
à cette structure.

La sidebar reste **inset avec repli en rail d'icônes** sur tous les écrans,
comme dans la page Figma `01 · Sidebar`. Ouverte à 280 px ou repliée à 64 px,
elle conserve le même composant, la même navigation globale et un contenu
adapté à la connexion. Le panneau de travail reste dans son encart avec une
marge de 8 px. Le workbench ne crée pas une seconde famille de sidebars et
le repli ne masque jamais entièrement la navigation.

Les pages Figma 04 à 07 **spécifient les contenus, parcours et états, pas la
structure de la fenêtre**. Leurs titres et sous-titres identifient les scénarios ;
ils ne prescrivent pas un second modèle de navigation. Les anciens exemples
des pages 01, 08 et 09 restent des références de composants ou d'interactions ;
la page 22 prévaut pour la structure commune. Les documents d'autorité du dépôt
prévalent sur toute planche pour les comportements métier.

### Repères permanents

L'environnement est présenté dans la barre supérieure par une **pilule
cerclée**, portant le libellé complet `PRODUCTION` pour la production.
Le nom de connexion reste visible à côté de ce repère et dans toute revue
d'écriture ; la couleur seule ne porte jamais cette information.

Le niveau IA utilise une forme commune dans les barres supérieures :
`Metadata · Cloud` pour un fournisseur distant au niveau Metadata. Le nom du
fournisseur figure dans les détails de contexte et de confidentialité, pas
dans cette étiquette. Les autres niveaux et les états bloqués restent
explicitement distingués ; cette présentation ne modifie pas les règles de
[AI-PROVIDERS](AI-PROVIDERS.md).

Dans la confirmation d'une écriture en production, `Cancel` reçoit le focus
initial visible. L'action qui nomme la connexion a un style destructif et
n'est pas focalisée. Entrée seule ne valide pas l'écriture.

### Largeur réduite

En dessous de **1200 px de largeur de fenêtre**, la disposition compacte
s'applique : sidebar repliée à **64 px**, inspecteur de ligne fermé et
actions secondaires regroupées dans un menu. Le contenu de la grille conserve
son défilement horizontal ; aucune colonne de données n'est supprimée.

Les sous-onglets `Data` et `Structure` restent visibles ; `Indexes`,
`Constraints`, `Relations` et `DDL` rejoignent le menu `More`. La connexion,
l'environnement et le niveau IA restent visibles dans la barre supérieure.
La barre d'état garde connexion et état d'exécution ; les informations
secondaires telles que le fuseau et la sauvegarde passent dans les détails.

L'action `Inspect row` ouvre l'inspecteur à la demande en panneau superposé,
sans réduire davantage la grille. En revenant à une largeur d'au moins
1200 px, les préférences de disposition large sont restaurées. Les changements
de largeur ne déclenchent ni requête ni changement de connexion.

## Navigation du premier workspace

La sidebar reprend le [design Oxyn](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=13-291).
Elle mesure 280 px ouverte et 64 px repliée. Son bouton de repli et `⌘B`
partagent la même action ; `⌘J` ramène à l'éditeur SQL. Les icônes viennent de
Hugeicons Stroke Rounded. Les variantes claire et sombre partagent les mêmes
dimensions et comportements.

L'arbre présente les objets de la connexion réellement ouverte. Déplier un
nœud demande le palier correspondant par le command bus ; aucune table de
démonstration ne remplace une réponse manquante. La recherche porte sur les
objets déjà chargés et annonce cette portée. Les fonctionnalités non disponibles
ne déclenchent pas de requête simulée.

Le formulaire propose uniquement les drivers enregistrés. Toute nouvelle
connexion commence en `production` jusqu'à changement explicite. Pendant un
changement de connexion, l'ancien workspace reste accessible ; une connexion
réussie transfère le texte SQL dans le nouvel éditeur sans l'exécuter. Les
connexions ouvertes pendant la session rejoignent immédiatement la liste.

L'aperçu d'une confirmation conserve le SQL intégral et permet son défilement
à la souris ainsi que par les flèches, Page précédente/suivante et Début/Fin.
Entrée seule ne valide jamais une écriture ; Échap refuse.

## Données d'une table sélectionnée

Sélectionner une table ou une vue SQL ouvre l'onglet **Data** et lit au plus
**200 lignes**. Le driver construit et cite l'identifiant qualifié ; le bus
applique la politique de la connexion et impose une requête en lecture seule.
Sur PostgreSQL, le catalogue désigne la base de la session, tandis que seuls le
schéma et la table rejoignent le nom SQL qualifié.

Cet aperçu a sa propre grille et sa propre annulation. Il ne remplace ni le
brouillon SQL ni les résultats de l'éditeur. Une nouvelle sélection annule
l'aperçu précédent ; ses réponses tardives sont ignorées. **Refresh data**
relit explicitement les lignes ; aucun rafraîchissement automatique ne suit une
erreur. Le chargement, le résultat vide, l'échec et l'annulation sont distincts.
L'ordre des lignes n'est pas garanti et l'aperçu ne compte pas la table entière.

Dans cet aperçu en lecture seule, `Edit rows…` est désactivé. Une infobulle,
également accessible au clavier par son déclencheur d'aide, explique que
l'édition nécessite une vue éditable, une session autorisant l'écriture et les
capacités appropriées. Une écriture en production reste soumise à la revue
définie ci-dessus ; le contrôle désactivé ne propose aucun contournement.

**Structure** charge les colonnes à travers le catalogue existant et les affiche
dans une liste virtualisée. `⌘2` donne le focus à la grille de l'aperçu et
Échap annule son chargement. Les capacités de la session et la nature de l'objet
déterminent la disponibilité de Data.
