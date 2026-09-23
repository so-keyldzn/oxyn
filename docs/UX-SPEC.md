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

## Ce qui se met à jour tout seul

Une vue qui montre des données périmées sans le dire est un défaut silencieux :
l'utilisateur lit un état d'avant et ne le sait pas. Après une exécution **qui a
réussi**, sur la connexion concernée :

- un DDL fait relire l'explorateur de catalogue ;
- un DDL ou une écriture fait relire l'**aperçu visible**, en conservant le
  prédicat, le tri et la page demandés — la relecture montre les mêmes lignes à
  jour, pas un retour au début ;
- toute exécution fait relire la bibliothèque, si elle est ouverte.

Rien ne se relit après une **erreur** : une relecture qui suit un échec masque
l'échec. Rien ne se relit sur une autre connexion, ni sur un onglet qui n'est pas
à l'écran — il se relira quand on y reviendra. Et l'instruction de l'utilisateur
n'est **jamais** rejouée : ce qui est réémis est la lecture d'aperçu qu'Oxyn
compose, bornée et en lecture seule.

`Refresh` reste, et garde son sens : forcer une relecture même quand rien n'a
changé du côté d'Oxyn — par exemple quand quelqu'un d'autre a écrit dans la base
([ADR-0022](adr/0022-rafraichissement-automatique.md)).

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

Une revue ne contient les opérations que d'une seule connexion. Une transaction
ouverte sur une autre connexion dispose de sa propre vue, nommant sa connexion
et son environnement ; ses actions ne figurent pas dans la confirmation courante.

## Annulation

Toute opération dépassant le budget de 300 ms
([PERFORMANCE](PERFORMANCE.md#budgets-dinteraction)) est annulable, et
l'annulation atteint le serveur
([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-il-expose-lannulation-et-lannulation-coupe-vraiment)).
Un bouton « Annuler » qui ne fait qu'abandonner l'affichage est un mensonge :
il laisse une requête tourner et une connexion prise.

**Quand la session ne déclare pas `SERVER_SIDE_CANCEL`**, il reste trois issues
possibles, et une seule est acceptable :

| Issue | Pourquoi elle est écartée, ou retenue |
|---|---|
| Promettre quand même | c'est le mensonge ci-dessus |
| Masquer le bouton | l'opération devient inarrêtable, ce que le budget de 300 ms interdit |
| **Garder le bouton et retirer la promesse** | retenu : couper le flux reste utile, et la réserve dit ce que le bouton ne fait pas |

La réserve **accompagne** le bouton, elle ne le remplace pas, et elle n'affirme
que ce que le drapeau absent prouve : qu'aucune annulation ne part vers un
serveur. Elle ne conclut pas que l'instruction continue de tourner — SQLite ne
déclare pas cette capacité faute de serveur, et `sqlite3_interrupt` arrête bien
l'instruction ([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-il-expose-lannulation-et-lannulation-coupe-vraiment)).

## Ce qui est exporté est ce qui est affiché

Un export ne s'offre que sur un résultat **entier**. Un tampon encore ouvert est
refusé par `ExportOptions` ; un tampon **clos mais tronqué** — arrêté par la
limite de lignes ou par le budget mémoire ([I-06](../CLAUDE.md#i-06)) — ne l'est
pas, et c'est le cas dangereux : il a fini de charger, donc rien à l'écran ne le
distingue d'un résultat complet.

**Panne concrète :** `SELECT * FROM commandes` sur cinquante millions de lignes,
le tampon s'arrête à deux millions, l'utilisateur exporte, et repart avec un CSV
qu'il croit être la table. Rien dans le fichier ne dit qu'il en manque
quarante-huit millions.

Un format que le produit ne sait pas encore écrire s'affiche **indisponible**,
pas absent : le proposer ferait échouer l'écriture après le choix du fichier, en
laissant un fichier vide sur le disque ; le masquer ferait croire que le produit
ne l'aura jamais.

L'aperçu d'une table utilise le libellé `Export preview…` et annonce le nombre
de lignes incluses. Un résultat complet de la requête d'aperçu limitée à
200 lignes peut être exporté ; cela ne représente pas la table entière.
Un tampon interrompu, encore en cours ou tronqué par le budget de réception
reste non exportable. Déplacer l'action dans un menu ne change pas cette règle.

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

## Restauration après un arrêt brutal

**Un arrêt anormal se constate, il ne se devine pas.** Oxyn inscrit sa fermeture
quand elle est propre, et laisse un battement pendant qu'il travaille. Au
lancement suivant, une session laissée sans fermeture et dont le battement a
vieilli est un arrêt anormal ; tout le reste n'en est pas un
([ADR-0021](adr/0021-marqueur-d-arret.md)).

L'écran de reprise n'apparaît donc **pas** après une fermeture ordinaire, même
s'il reste des copies de travail : celles-ci se retrouvent dans la bibliothèque.
Un écran montré à chaque démarrage cesse d'être lu, et c'est le jour où une
écriture a réellement été interrompue qu'il faut qu'il le soit.

Au redémarrage après un arrêt anormal, Oxyn présente les onglets et brouillons
locaux récupérables. L'utilisateur choisit les éléments à restaurer ou démarre
avec un workspace vide. Sans sélection, l'action de restauration est désactivée.
La sélection utilise des cases à cocher, accessibles au clavier avec leur nom
d'élément. Le bouton final annonce le nombre d'éléments sélectionnés ; cocher
ou décocher un élément ne le restaure pas immédiatement.
La restauration reste hors ligne : elle ne reconnecte aucune session et ne
réexécute aucune requête. Un onglet d'objet restauré conserve son emplacement,
sans charger ses données avant une reconnexion explicite.

Une écriture interrompue peut avoir un résultat inconnu. La reprise conserve cet
avertissement et demande d'inspecter l'état du serveur après reconnexion ; elle
ne retente jamais l'écriture ([I-13](../CLAUDE.md#i-13)).

## Écran d'accueil

Avant toute connexion, l'écran porte une barre de titre d'une seule rangée : la
marque Oxyn et ses deux lignes de titre à gauche, les actions de fenêtre à
droite — reprendre les copies de travail locales, et revenir au workspace resté
ouvert lorsqu'il y en a un. Ces actions appartiennent à la barre ; aucune n'est
posée en calque au-dessus de l'écran, où elle recouvrirait la marque. Une action
sans objet — un retour vers un workspace qui n'existe pas — n'est pas affichée
désactivée : elle est absente. La marque est unique : une seule image de logo,
jamais deux variantes côte à côte.

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

Les écrans des pages Figma 04 à 09 reprennent cette structure commune. Leurs
titres et sous-titres identifient les scénarios dans les onglets et le contexte ;
ils ne prescrivent pas un second modèle de navigation. La page 01 définit le
composant de sidebar et la page 22 la structure de référence du workbench.
Les documents d'autorité du dépôt prévalent sur toute planche pour les
comportements métier.

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
`Columns` et `Export preview…` figurent uniquement dans `Actions` à cette
largeur, sans doublon dans la barre. `Read-only preview` reste sur une ligne.

Les sous-onglets `Data` et `Structure` restent visibles ; `Indexes`,
`Constraints`, `Relations` et `DDL` rejoignent le menu `More`. La connexion,
l'environnement et le niveau IA restent visibles dans la barre supérieure.
La barre d'état garde connexion et état d'exécution ; les informations
secondaires telles que le fuseau et la sauvegarde passent dans les détails.

L'action `Inspect row` ouvre l'inspecteur à la demande en panneau superposé,
sans réduire davantage la grille. En largeur normale, la poignée de l'inspecteur
se déplace à la souris ou par les flèches gauche/droite une fois focalisée ;
Début restaure 280 px. Les bornes de 240 à 480 px sont définies dans ADR-0013.
La largeur est sauvegardée à la fin du glissement, pas à chaque trame. En revenant à une largeur d'au moins
1200 px, les préférences de disposition large sont restaurées. Les changements
de largeur ne déclenchent ni requête ni changement de connexion.

### Inspection du DDL

Le panneau de définition est en lecture seule. Il distingue chargement
annulable, définition disponible, erreur et capacité absente ; la provenance
stockée/reconstruite et les limites restent visibles. Le bouton Copy DDL copie
le texte affiché. Open DDL in console crée un document distinct et conserve les
consoles existantes ; aucune exécution n'est déclenchée par cette préparation.
Un texte conservé après un rafraîchissement échoué est signalé comme
potentiellement périmé, y compris lors de sa copie dans une console.

En disposition large, la définition accompagne les vues de métadonnées dans
un panneau initialement large de 424 px. Sa poignée de 8 px fonctionne à la
souris et au clavier (320–640 px, Début : 424 px). Cette largeur reste dans le
workspace ouvert. En disposition compacte, le sous-onglet DDL présente la
définition sur la largeur disponible ; changer la largeur de fenêtre ne
relance pas sa lecture.

### Lisibilité et hauteur de grille

La grille occupe la hauteur disponible ; son statut et sa portée d'export
restent en bas de sa zone. La quantité de lignes dessinées dans une planche
ne définit pas une limite du viewport de l'application.

Deux préréglages de lecture sont proposés dans les préférences : `Compact`
(texte principal 13 px, mentions secondaires 11 px, lignes de données 24 px)
et `Comfortable` (14 px, 12 px et 28 px). Le choix est indépendant du thème
et du seuil de largeur de fenêtre ; il ne relance aucune requête. La barre
de résultat permet également de changer la taille du texte. Le choix est
sauvegardé dans le workspace avec le thème et les préférences de panneaux,
selon [ADR-0013](adr/0013-preferences-workspace.md). Une erreur de sauvegarde
laisse le réglage appliqué localement et affiche une action de reprise
explicite ; elle n'annonce jamais une sauvegarde réussie.

Une valeur absente utilise le jeton `null` et le repère `∅ NULL`. Une chaîne
contenant le texte `NULL`, ou l'expression SQL `NULL` d'une valeur par défaut
dans Structure, conserve son traitement de texte ou de code.

Les pièces jointes de contexte IA conservent leur nom sur une seule ligne,
avec une ellipse de fin si nécessaire. Le nom complet reste consultable dans
les sources jointes et la revue du contexte sortant ; le retrait reste local.

## Colonnes et inspection des valeurs

`Columns` règle la visibilité locale des colonnes, sans modifier leurs indices
Arrow, les lignes reçues ou le SQL. Le menu annonce le nombre de colonnes
visibles et permet de toutes les réafficher. Cette visibilité ne projette pas
l'export : le résultat exporté conserve toutes ses colonnes, ce que le menu
indique explicitement. Le contrôle reste accessible dans `Actions` en largeur
compacte.

L'inspecteur suit la ligne sélectionnée et présente ses champs en lecture
seule. Les flèches sélectionnent un champ ; Entrée ouvre sa valeur complète.
Sans ligne sélectionnée, il explique comment commencer. La lecture d'une page
manquante utilise le résultat existant, jamais une nouvelle requête.

`Inspect full value` ouvre une vue en lecture seule nommant connexion, colonne
et numéro de ligne. La représentation source est fournie en pages d'au plus
16 Kio, aux frontières UTF-8 ; le compteur indique les octets de texte rendus,
pas la taille de stockage native. Précédent et Suivant naviguent dans cette
représentation. Une valeur absente, une valeur vide et le texte `NULL` restent
visiblement distincts. Fermer pendant le chargement l'annule ; une réponse
ancienne ne rouvre pas la vue. L'état d'erreur conserve un moyen de fermer et
reprendre explicitement l'inspection, sans nouvelle exécution SQL.

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

La barre d'état du formulaire nomme la connexion en préparation et distingue
`Not tested`, le résultat du test et l'enregistrement. Elle n'annonce pas
`Connected` pour une connexion non testée ; une autre session ouverte conserve
son propre contexte explicite.

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
Sans tri demandé, l'ordre des lignes n'est pas garanti, et l'aperçu ne compte
jamais la table entière.

### Filtrer, trier, parcourir

La barre sous les actions de données porte un champ précédé de `WHERE` et un
bouton `Apply`, puis un bouton `Sort` (Figma `190:1618`). Ces contrôles
n'existent que pour une source qui déclare savoir filtrer ou ordonner un
aperçu ; ailleurs ils sont absents, pas désactivés.

Le champ `WHERE` reçoit un **prédicat que l'utilisateur écrit**, transmis tel
quel : ni analysé, ni réécrit, ni complété. Un champ vide ne filtre rien.

Un prédicat invalide échoue de deux façons, et l'aperçu ne les confond pas. Un
prédicat qu'Oxyn ne parvient pas à lire comme une condition — `id >< 3` — est
refusé **avant tout envoi** : ce qui n'est pas classé compte pour une écriture,
et cette prudence est ce qui protège. Le message le dit alors en ces termes, et
renvoie vers la syntaxe du prédicat ; l'annoncer comme un défaut de lecture
seule enverrait chercher un droit manquant là où il y a une faute de frappe. Un
prédicat bien formé mais faux pour cette table — une colonne qui n'existe pas —
part au serveur et revient avec **son** message, code compris.

L'aperçu peut donc désormais échouer pour une raison de syntaxe, ce qui n'était
pas le cas avant. Il ne peut en revanche rien écrire : le texte final est
reclassifié, la session est tenue en lecture seule par le serveur, et la borne
de lignes s'applique.

`Sort` choisit des colonnes, pas une expression : c'est Oxyn qui compose cette
partie de la requête, donc il en répond. Une colonne inconnue de la relation est
refusée avant l'envoi.

La page suivante est une **nouvelle exécution**, pas un défilement : faire
défiler les lignes déjà reçues ne déclenche jamais de requête. Elle n'est
offerte que si l'ordre est total — le tri demandé, complété par une clé unique
que le catalogue déclare. Sans clé connue, l'aperçu reste à sa première page et
dit pourquoi : un `OFFSET` sur un ordre incertain montrerait deux fois la même
ligne et en omettrait une autre, sans rien signaler. Entre deux pages, les
données du serveur ont pu changer ; l'aperçu ne prétend pas être un instantané
([ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md)).

Dans cet aperçu en lecture seule, `Edit rows…` est désactivé. Une infobulle,
également accessible au clavier par son déclencheur d'aide, explique que
l'édition nécessite une vue éditable, une session autorisant l'écriture et les
capacités appropriées. Une écriture en production reste soumise à la revue
définie ci-dessus ; le contrôle désactivé ne propose aucun contournement.

**Structure** charge les colonnes à travers le catalogue existant et les affiche
dans une liste virtualisée. `⌘2` donne le focus à la grille de l'aperçu et
Échap annule son chargement. Les capacités de la session et la nature de l'objet
déterminent la disponibilité de Data.


## Consultation locale des requêtes

La bibliothèque distingue historique, requêtes sauvegardées et références de
résultats récents. Ouvrir cette bibliothèque et sélectionner une ligne ne
remplace ni le brouillon ni le résultat de la console courante. Le texte complet
s'affiche séparément en lecture seule : navigation, sélection et copie sont
possibles ; saisie, collage, composition native et `⌘Entrée` ne modifient ni
n'exécutent ce texte.

Les filtres de recherche s'appliquent avant pagination. Une réponse ancienne
ne remplace pas une recherche ou une sélection plus récente. L'annulation est
explicite et reste distincte d'une liste vide. Une copie de travail modifiée
peut être consultée sans modifier la copie nommée. Une référence de résultat
dans l'historique ne prouve pas sa disponibilité après redémarrage ; sa
réouverture doit vérifier la rétention sans rejouer la requête. Une écriture
sans issue certaine conserve son avertissement de réconciliation.


## Portée de Run dans une console SQL

`⌘Entrée` exécute la sélection explicite lorsqu'elle existe ; sinon, seule
l'instruction sous le curseur est soumise. La position est calculée en octets
UTF-8 à partir du curseur Unicode de l'éditeur. Un point-virgule final reste
rattaché à son instruction ; du code suivant au même emplacement devient
prioritaire, mais un commentaire ne fait pas basculer vers une écriture plus
loin. Les espaces sans instruction n'envoient aucune commande.

Les corps PostgreSQL entre dollar-quotes et les triggers SQLite avec leurs
blocs CASE/END restent entiers. Si le texte est incomplet ou si ses frontières
sont ambiguës, la console demande une sélection explicite. Le découpage ne
modifie ni le texte du document ni les règles de classification et de revue.
Une sélection multi-instructions conserve les limites déclarées par la session :
elle n'est pas transformée silencieusement en exécutions successives.
Un éditeur en lecture seule annonce sélection/copie, sans raccourci d'exécution.

La barre de la console présente Run, Stop et Explain comme trois contrôles
distincts. Celui qui n'a rien à faire est estompé **et** inerte : Run pendant
une exécution, Stop en dehors. Un seul bouton dont le sens bascule serait à un
clic manqué de relancer ce qu'on voulait arrêter. Stop atteint l'annulation
réelle, jusqu'au serveur.

## Explain

Explain décrit l'instruction courante ; il ne l'exécute jamais. Le préfixe est
posé sur une instruction unique : un lot en refuse la portée, une instruction
qui commence déjà par `EXPLAIN` aussi, et un dialecte sans plan textuel le dit
plutôt que d'envoyer une syntaxe que le serveur rejetterait. `ANALYZE` n'est
jamais ajouté : il exécuterait réellement la requête analysée, suppression
comprise. Le brouillon de la console reste celui que l'utilisateur a écrit, et
le plan revient comme un résultat ordinaire, sans revue d'écriture.

## Contexte de session d'une console

Le sélecteur de la barre lit `<connexion> / <schéma>` : le nom donné par
l'utilisateur, puis l'endroit où la session résout les noms qu'une instruction
ne qualifie pas. Il n'existe que pour une source qui sait porter ce contexte ;
ailleurs, les schémas se qualifient dans le SQL, et aucun contrôle désactivé ne
le laisse croire possible.

Tant que rien n'a été déclaré, la barre dit que le serveur a placé la session à
l'ouverture — elle ne nomme pas un schéma qu'Oxyn n'a pas demandé. Le choix
traverse le réseau : pendant ce temps la barre annonce la cible, rappelle où la
session résout **encore**, et propose une annulation qui atteint le serveur.
L'affichage ne change qu'à la réponse, et montre ce que la session rapporte,
jamais ce qui a été demandé. Un refus montre le message du serveur tel quel,
dit que rien n'a bougé, et dit s'il vaut la peine d'être retenté.

Le contexte appartient à la console, comme sa session : changer d'onglet montre
celui de cet onglet, et une console n'hérite pas de sa voisine. Le retour au
défaut du serveur est toujours offert. Le SQL écrit par l'utilisateur n'est
jamais réécrit : le contexte change ce que le serveur résout, pas le texte
soumis ([ADR-0019](adr/0019-contexte-de-session.md)).

L'explorateur de catalogue **ne suit pas** ce contexte : il montre un arbre
qualifié. La console peut donc travailler dans `analytics` pendant que la
sidebar montre `public` ; c'est un écart visible, préféré à un catalogue qui se
déplace sans qu'on le lui ait demandé.

## Valeurs liées d'une console

Une console porte ses propres valeurs liées, affichées par le contrôle
`Parameters` et son panneau. Elles n'existent que dans la fenêtre : elles ne
sont ni sauvegardées avec la requête, ni écrites dans l'historique ou le
journal, ni recopiées dans un message d'erreur, et le champ de saisie refuse la
copie comme l'extraction de texte native ([I-03](../CLAUDE.md#i-03)).

Elles partent dans la requête d'exécution, **jamais** dans le texte SQL : rien
n'est concaténé ni substitué, et le SQL écrit par l'utilisateur reste intact
([I-10](../CLAUDE.md#i-10)). Chaque console a les siennes ; ouvrir une console
voisine n'en hérite pas.

Chaque valeur déclare son type. Une ligne neuve vaut `NULL` et sa saisie est
close tant qu'un type n'est pas choisi. Une valeur que son type ne sait pas
convertir arrête la soumission avant tout envoi : le panneau s'ouvre sur la
ligne fautive, et le message nomme la position et le type attendu, jamais le
texte saisi. Les bornes du produit sont 128 valeurs et 1 Mio de texte cumulé.

## Consoles indépendantes

Une nouvelle console conserve la connexion choisie et établit sa propre session
avant toute exécution. L'ouverture est annulable ; une réponse devenue obsolète
ne crée pas d'onglet et sa session est libérée. Changer d'onglet conserve le texte,
le résultat, la lecture de pages, la confirmation et l'export de chaque console.
Une confirmation d'un onglet masqué reste rattachée à cet onglet.

`⌘T` ouvre une console, `Ctrl+Tab` et `Ctrl+Shift+Tab` parcourent les consoles,
et `⌘W` vise uniquement la console active lorsque le panneau SQL est affiché.
Fermer une console vide et inactive est immédiat. Si elle contient du SQL non
sauvegardé ou une opération, un dialogue nomme la console, explique ce qui sera
abandonné et place le focus sur Annuler. Fermer ne rejoue aucune requête et
n'annule pas les opérations d'une autre console. Fermer la dernière laisse un
état vide avec l'action d'ouverture, tandis que le catalogue reste disponible.

La fenêtre conserve aussi les workspaces de connexion déjà ouverts. Sélectionner
une connexion déjà présente rend son workspace et ses consoles visibles, sans
remplacer leurs textes. Une copie vers une nouvelle connexion est annoncée et
n'exécute rien ; la console d'origine reste disponible.


## Sauvegarde d'une console

La sauvegarde explicite porte sur le document entier et son nom, pas uniquement
sur la sélection exécutée. `⌘S` et le bouton de sauvegarde transmettent la même
commande locale. Le nom est borné à 256 octets UTF-8 : un remplacement trop long
est refusé sans couper le texte ni perturber la composition native.

L'éditeur reste utilisable pendant une sauvegarde. L'acquittement porte sur le
texte demandé : des modifications plus récentes restent signalées comme non
sauvegardées. Une seule sauvegarde est engagée par console à la fois, avec un
moyen de l'annuler. Si le store contient une version concurrente, la nouvelle
sauvegarde doit créer une autre identité et préserver la version existante.

Le dialogue de fermeture propose sauvegarder puis fermer, abandonner les
modifications, ou annuler. Il attend l'acquittement de sauvegarde puis celui de
fermeture avant de retirer l'onglet. Annuler la sauvegarde empêche sa réponse
tardive de fermer la console. Pendant la fermeture locale, texte et nom restent
lisibles mais non modifiables ; son annulation conserve la vue si l'écriture
n'avait pas déjà terminé. La copie nommée reste dans la bibliothèque après un
abandon de modifications.


## Ouvrir depuis la bibliothèque

L'ouverture d'une copie nomme la connexion de destination avant l'action. Elle
crée une console indépendante et n'exécute rien. Son indication d'origine
rappelle notamment si le texte vient d'une entrée d'historique d'agent. Une
écriture dont l'issue nécessite une réconciliation reste consultable mais
n'offre pas cette ouverture éditable.

La reprise d'une requête sauvegardée sur sa connexion d'origine ouvre le texte
de travail et conserve son identité et son langage de stockage. Elle ne remplace
pas un brouillon par la copie nommée : si la console existe déjà, elle devient
visible avec ses modifications actuelles. Sur une autre connexion, l'utilisateur
ouvre une copie ou sélectionne d'abord la connexion d'origine pour reprendre le
document. Une ouverture de session annulée ne crée pas de console tardive.

Un conflit de stockage ou une copie illisible pendant la sauvegarde est traité
sans écraser le document existant. Fermer une console en conflit laisse son
record stocké intact ; une sauvegarde sous une nouvelle identité conserve les
deux versions.


## Autosauvegarde des brouillons

Les changements de texte et de nom sauvegardent la copie de travail localement,
sans modifier la copie nommée et sans exécuter de SQL. Un nom de travail vide
reste récupérable ; seul l'enregistrement d'une copie nommée exige un nom. Une
édition hors des bornes est signalée comme non sauvegardée, et une réponse
relative à un état antérieur ne peut pas annoncer cet état comme récupéré.

Le dernier brouillon en attente remplace les brouillons intermédiaires. Les
sauvegardes nommées gardent leur instantané, et la fermeture garde sa propre
annulation. Si une fermeture est annulée avant sa validation, son brouillon en
attente reprend sa place. Si elle termine, aucune ancienne écriture ne rouvre
le document. Un conflit avec une révision modifiée ailleurs arrête la file et
conserve le texte de l'éditeur pour une décision de copie séparée.


## Restauration sélective au démarrage

Lorsque des copies de travail ouvertes sont disponibles, l'écran de reprise les
présente par pages avec des cases de sélection. La sélection ne charge pas tous
les corps : seul le document affiché est lu. Continuer sans restaurer laisse les
copies stockées intactes. Il est possible de revenir à la sélection pour ajouter
d'autres documents sans remplacer les éditeurs déjà repris.

Les éditeurs repris sont hors ligne : texte, nom, annulation d'édition,
autosauvegarde et sauvegarde nommée fonctionnent, tandis que l'exécution attend
une connexion. Le choix de la connexion est un geste séparé. La connexion
originale reprend le document ; une autre produit une copie et conserve le
document initial. Le raccordement ne lance pas le SQL et conserve l'entité de
l'éditeur. Annuler le choix de connexion laisse le brouillon hors ligne.

L'écran **dit ce qu'il a constaté**. Quand le marqueur d'arrêt révèle une
session abandonnée, il annonce qu'Oxyn ne s'est pas fermé normalement ; ouvert à
la demande depuis la bibliothèque, il ne l'annonce pas — ce serait affirmer un
plantage qui n'a pas eu lieu ([ADR-0021](adr/0021-marqueur-d-arret.md)). Il
rappelle conditionnellement qu'une écriture interrompue exige une inspection du
serveur, et ne rejoue rien pour effectuer cette inspection.


## Résultats conservés

L'historique et Recent results permettent d'ouvrir un tampon encore retenu.
Cette ouverture n'établit aucune session et ne soumet aucun SQL. La grille et
l'export utilisent le résultat existant, y compris ses pages débordées sur disque.
Un résultat incomplet, tronqué ou associé à une issue incertaine reste consultable
mais ne peut pas être présenté comme un export complet.

Une référence expirée affiche une indisponibilité explicite. Le retour à la
bibliothèque ne coupe pas un export engagé ; un résultat en cours d'export doit
être terminé ou annulé avant remplacement ou fermeture de sa vue. La rétention
sans lecteur est bornée par ADR-0017, et n'implique pas une conservation entre
redémarrages. Aucun résultat évincé n'est recréé par rejeu de la requête.

## Le workspace IA n'existe que s'il a été configuré

Tant qu'aucune destination n'est déclarée — ni fournisseur, ni agent externe —,
`Ask AI` **n'est pas affiché**, aucun badge de confidentialité n'apparaît, et rien
ne suggère qu'une fonction manque. Un agent externe déclaré seul suffit à faire
exister l'entrée. Oxyn est un client complet sans IA (ADR-0006). L'accès à la
configuration passe par les réglages, jamais par un appel à l'action dans la
barre de connexion : un bouton grisé qui invite à configurer est une publicité,
pas une fonctionnalité.

La première destination enregistrée fait apparaître l'entrée **sans
redémarrage**, et la dernière supprimée la fait disparaître — en fermant le
panneau s'il était ouvert. Cela se fait **sans nouvelle variante du bus d'exécution** : la vue qui
écrit la déclaration connaît le sort de son écriture et recharge la liste
elle-même. Y publier un événement ferait ce qu'[ADR-0022](adr/0022-rafraichissement-automatique.md)
refuse — un second endroit où penser à publier
([ADR-0023](adr/0023-fournisseurs-declares-et-provenance.md)).

### Le niveau se lit avant de parler, pas après

Le badge de confidentialité et `Ask AI` sont voisins dans la barre de connexion
et se lisent ensemble. Le badge annonce le niveau de **la connexion courante** —
jamais un réglage d'application — et il change quand on change de connexion,
même conversation ouverte.

Sur une connexion en `Local` dont aucun fournisseur déclaré n'est local,
`Ask AI` reste **visible et désactivé**, avec la raison : ce niveau n'admet
qu'un fournisseur résolu sur la machine, et un agent externe ne peut pas le
servir non plus, faute de savoir où il envoie
([AI-PROVIDERS](AI-PROVIDERS.md#un-agent-externe--la-portée-nest-pas-inconnue-elle-est-inconnaissable)).
Faire disparaître l'entrée se lirait comme un défaut, et ADR-0006 demande
qu'une fonctionnalité indisponible s'explique. Il en va de même sur une session
qui ne parle pas SQL : l'assistant n'écrit que du SQL, et l'entrée le dit.

### Qui répond se choisit dans le panneau

L'en-tête du panneau porte un sélecteur, « Who answers », qui range les
destinations en deux groupes : les fournisseurs, puis les agents externes. Une
destination que le niveau de la connexion refuse y reste, désactivée, avec sa
raison. Sans choix de l'utilisateur, la question part au **premier fournisseur
utilisable** dans l'ordre de déclaration, et à un agent seulement s'il n'y en a
aucun : un fournisseur est la seule destination dont Oxyn sait vérifier la
portée (ADR-0026). Le choix tient tant que la destination reste offerte ;
retirée, la question retombe sur ce défaut.

Le sélecteur est inerte pendant qu'une réponse s'écrit. Changer de destination
entre deux questions ne transmet pas les échanges précédents : le panneau le dit
par une ligne (`destinationChanged`), puisque les réponses antérieures n'étaient
pas celles du nouveau destinataire.

Un agent choisi est accompagné de deux mentions permanentes : « Oxyn ne voit pas
où il envoie la question », et, pour un agent qu'Oxyn ne confine pas, qu'il
**ne peut pas l'empêcher de lancer des commandes ni de modifier des fichiers de
la machine de lui-même**
([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)). Pour un agent
confiné, le sélecteur de mode de l'agent n'existe pas : Oxyn le maintient dans
son mode le plus strict, et un mode dangereux ne doit pas pouvoir se choisir à
l'écran. Un agent non confiné garde les modes et options qu'il déclare.

Un point d'accès qu'Oxyn n'a pas su résoudre compte comme distant. La
configuration l'affiche comme « non résolu », pas comme « local » : le doute ne
profite pas à l'envoi.

### Le panneau montre ce qui se passe, y compris quand rien n'arrive

Le panneau suit les cinq états d'une vue. Ce qui les distingue ici :

* **en cours** : la réponse s'écrit au fil du flux, et l'annulation reste offerte
  pendant tout ce temps — pas seulement entre deux tours ;
* **appel d'outil** : chaque commande demandée par l'agent est montrée **avant**
  son résultat, avec son nom et la connexion visée. Un agent qui travaille en
  silence pendant huit tours est indistinguable d'un agent bloqué ;
* **erreur** : ce que l'utilisateur lit est le message entier du serveur ; ce que
  le modèle a reçu peut être moins, selon le niveau, et le panneau le dit quand
  les deux diffèrent. Cacher l'écart ferait passer une réponse mal informée pour
  une réponse fausse ;
* **réponse coupée** : une réponse arrêtée par le plafond de jetons du
  fournisseur le dit à sa dernière ligne. Une réponse tronquée qui ne l'annonce
  pas se lit comme une réponse fausse, et l'utilisateur corrige un modèle qui
  n'avait pas fini de parler ;
* **plafond de tours atteint** : dit comme tel, avec le nombre de tours. Ce n'est
  ni un succès ni une panne.

**Ce que « l'annulation reste offerte » veut dire exactement.** Le bouton reste
actif et la demande est prise en compte immédiatement ; son *effet* dépend du
fournisseur. La boucle relit la demande entre deux événements du flux et
n'interrompt jamais une lecture en cours : un futur abandonné au milieu d'une
trame laisserait le décodeur désynchronisé, et le dépôt refuse ce risque
(`.claude/rules/rust.md`, § Async). Si le fournisseur se tait, l'annulation
attend qu'il reparle. C'est le contrat de `LlmProvider` qui la rend effective —
tout fournisseur honore le jeton d'annulation dans son flux — et un fournisseur
qui ne le ferait pas rendrait l'annulation inopérante sans que rien d'autre ne
le signale.

### Les lignes d'une requête d'agent se lisent sous son appel

Quand un agent — fournisseur interne ou agent externe par MCP — exécute
`execute_query` et que la commande se termine, **l'utilisateur voit les lignes
renvoyées** sous l'appel d'outil, dans une grille compacte. Le modèle, lui, n'en
reçoit que la forme — nombre de lignes et de lots, troncature — et rien de ce
que la grille montre ([ADR-0006](adr/0006-ai-privacy-tiers.md),
[ADR-0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) §4) ; le pied
de la grille le rappelle (« shown to you only; the model got the count »).

* **Même chemin que la console.** Le `ResultId` retenu par l'ordonnanceur
  remonte dans l'événement de fin d'appel, jamais dans ce qui part au modèle.
  La grille ouvre le résultat par `OpenRetainedResult`, qui vérifie qu'il
  appartient à la connexion de la conversation, puis lit des pages bornées par
  `read_result_page` ; les cellules sont formatées en Rust. Un résultat produit
  sur une autre connexion que celle de la conversation n'est pas proposé.
* **Bornée.** La grille atteint au plus les 100 premières lignes, en affiche
  huit avant de défiler, et dit « First 100 rows of N » quand il y en a plus.
  `Open all rows` ouvre le résultat entier dans un onglet de résultat du
  workspace — le tampon retenu, **jamais une réexécution**. Cet onglet n'offre
  pas d'ouvrir l'instruction dans une console : elle n'y entre qu'avec sa
  provenance, depuis la réponse de l'agent.
* **États.** En cours de lecture ; peuplée ; **vide** (« The query returned no
  rows. », sans alerte) ; erreur (le message du backend entier, `Try again`
  seulement s'il est déclaré retentable) ; **expirée** (« Result no longer
  available », sans rien proposer qui relance la requête).

**Durée de vie.** Le résultat d'un agent est un résultat retenu sans lecteur :
il vit jusqu'à ce que la rétention l'évince selon les bornes
d'[ADR-0017](adr/0017-retention-resultats.md) — au plus 16 résultats sans
lecteur, les plus anciens partant les premiers —, jusqu'à la suppression de la
conversation — qui libère les résultats de ses appels —, ou jusqu'à la fin du
processus. Rien n'en est écrit dans le fichier de workspace. Une conversation
rouverte depuis le workspace ne montre donc aucune grille : sous un appel qui
avait renvoyé des lignes, elle dit « Result no longer available: the workspace
keeps the statement, never its rows ».

### Un bloc `erd` se dessine depuis le catalogue, pas depuis la réponse

Un bloc de code clôturé de langage `erd` dans une réponse liste des noms de
tables, un par ligne, éventuellement qualifiés `schema.table` (citations SQL
comprises : `"Ventes"."T1.totaux"`). Une fois le bloc **fermé**, le panneau le
dessine en diagramme ; tant que la réponse s'écrit, il reste du texte — un
diagramme redessiné à chaque nom arrivé se lirait comme un défaut.

* **Les noms sont des demandes, pas des faits.** Chacun est cherché par
  `search_catalog` parmi les objets qu'Oxyn a déjà lus : l'orthographe exacte
  d'abord, puis sans casse si une seule table correspond. Un nom qui désigne
  deux tables n'est pas tranché (« matches auth.users, public.users: not drawn
  until qualified ») ; un nom introuvable est **dit**, jamais dessiné d'après son
  orthographe (« Not found among the objects Oxyn has read »). La recherche
  n'introspecte pas : une table d'un schéma jamais déplié n'est pas trouvée, et
  le message le laisse entendre.
* **Ce qui est dessiné vient du backend.** Colonnes, clé primaire et colonnes de
  clé étrangère sont lues par les mêmes commandes que la vue d'objet
  (`relation_facets`, puis `refresh_relation_facet` pour une facette jamais lue) ;
  les arêtes sont les clés étrangères. Aux tables nommées s'ajoutent leurs
  **voisines directes** par une clé, sortante ou entrante, et les clés entre
  deux tables dessinées. Aucun modèle n'est consulté, et un même catalogue
  donne le même diagramme (disposition dagre, de gauche à droite : la table qui
  porte la clé avant celle qu'elle référence).
* **Borné.** Un bloc est lu jusqu'à 20 noms, un diagramme s'arrête à 40 tables
  et une table montre 12 colonnes, clés d'abord ; chaque surplus est compté et
  dit. Quatre lectures au plus sont en vol à la fois.
* **Navigable.** Déplacer à la souris, zoomer par les boutons, le pincement ou
  le clavier (flèches, `+`, `-`, `0` pour ajuster) quand le diagramme a le
  focus. La molette fait défiler la conversation, pas le diagramme. Un clic sur
  le nom d'une table l'ouvre dans le workspace, comme depuis le catalogue ; rien
  ne s'exécute. Les clés sont aussi écrites en liste pour un lecteur d'écran.
* **États.** Lecture en cours ; dessiné ; rien à dessiner (tous les noms
  introuvables) ; erreur — le message du backend entier, sans paraphrase, avec
  `Try again`, qui relit sans rien écrire. Une lecture de métadonnées qui
  exigerait une approbation est refusée et dite, comme dans la vue d'objet.
* `Show source` montre le texte du bloc tel que le modèle l'a écrit.

Pour qu'un modèle sache émettre ce bloc, l'invite système doit le décrire ;
sans cela, il n'apparaît que si l'utilisateur le demande.

### Ce que le panneau montre d'un agent externe

Un agent externe travaille avec ses propres outils, qui ne sont pas des `Command`
d'Oxyn. Le panneau en montre ce qu'il peut montrer **sans rien recopier de la
machine** :

* le texte de la réponse et le raisonnement que l'agent diffuse ;
* chaque étape de ses outils propres, réduite à **sa sorte** — lecture, édition,
  recherche… — et à son état : en attente, en cours, terminée, échouée. Le titre
  que l'agent compose pour l'étape n'est jamais affiché, ni son contenu : il
  peut citer un chemin de la machine. L'étape est dessinée comme le travail de
  l'agent, distincte d'un appel d'outil d'Oxyn, qui lui est passé par le bus ;
* son **plan**, remplacé en entier à chaque envoi et jamais fusionné avec le
  précédent : le protocole n'envoie ni différence ni identifiant d'étape. Une
  priorité inconnue s'affiche « moyenne », un état inconnu « en attente » —
  jamais « terminé » sur une supposition ;
* l'occupation de sa fenêtre de contexte, et son coût quand il le donne ;
* chaque demande d'agir sur la machine qu'Oxyn a refusée, sans bouton pour
  l'accorder
  ([AI-PROVIDERS](AI-PROVIDERS.md#ce-que-lagent-demande-lui-à-la-machine)).

Les messages de l'utilisateur renvoyés par l'agent, ses commandes disponibles et
les informations de session ne sont pas relayés ; une variante que cette version
ne connaît pas n'est rien, plutôt qu'autre chose que ce qu'elle est.

**Ceci précise [ADR-0026](adr/0026-agents-externes-acp.md)**, qui ne remontait
que les fragments de texte au motif qu'un plan ou une étape d'outil affichés
« comme les siens » feraient croire à un travail autorisé par Oxyn. Le risque est
tenu autrement : la marque distingue le travail de l'agent de celui d'Oxyn, et
rien de ce qu'elle montre ne peut citer la machine. Le cacher faisait retomber
dans l'état que ce document refuse : un agent qui travaille en silence est
indistinguable d'un agent bloqué.

### Une conversation reste dans le workspace, pas ce qu'on lui a montré

Une conversation appartient à sa connexion. Fermer la connexion arrête ce que
ses conversations exécutaient et les retire de la fenêtre ; elles restent
**dans le fichier du workspace** (tables `ai_conversations`,
`ai_conversation_turns` et `ai_conversation_nodes`) et se rouvrent depuis la
liste « Conversations » du panneau, qui montre les 64 plus récentes de la
connexion. Au lancement suivant, le panneau s'ouvre sur une conversation neuve ;
les précédentes attendent dans cette liste.

Ce qui est gardé, échange par échange : la question, la destination (libellé et
modèle, jamais de clé), le niveau de confidentialité **sous lequel cet échange a
eu lieu**, et son issue ; pour la réponse, le texte que l'utilisateur a lu, le
raisonnement, un **rendu** de chaque appel d'outil d'Oxyn — nom, instruction
SQL, issue, message affiché — et les jetons déclarés. Les versions sœurs créées
par une régénération ou une modification sont gardées avec leur arbre.

Ce qui ne l'est jamais, faute de colonne où l'écrire : les arguments d'un appel
d'outil, un résultat de requête, une valeur liée, une clé. Les étapes et le plan
d'un agent externe ne sont pas gardés non plus.

**Un échange qui a reçu un échantillon de lignes ne garde que sa question**, ses
compteurs — lignes et colonnes, jamais leurs noms — et son issue. Ni la réponse,
ni le raisonnement, ni un appel d'outil, ni un message d'erreur : chacun peut
citer les valeurs envoyées, et le fichier de workspace est l'un des six canaux
d'[I-03](../CLAUDE.md#i-03). Le fichier le refuse lui-même, et marquer l'échange
efface ce qui avait déjà été écrit pour lui
([AI-PROVIDERS](AI-PROVIDERS.md#échantillon-approuvé)). Rouvert, cet échange
l'annonce au lieu de montrer un vide.

**Rouvrir ne rend pas la mémoire au modèle.** La conversation s'affiche en
entier, mais la question suivante part d'un contexte neuf, et une ligne le dit
(`restarted`) : ce qu'on avait transmis au modèle n'a pas été écrit, et une
transcription rejouée sans les résultats d'outils lui ferait croire à un
contexte qui n'est plus là. Le raisonnement gardé reste sur le disque et ne
s'affiche pas à la réouverture.

Supprimer une conversation la retire du fichier avec tous ses échanges, dans la
même transaction ; ce qui est sorti vers un destinataire reste inscrit dans
`ai_egress`
([SECURITY](SECURITY.md#ce-qui-sort-vers-un-destinataire-ia-laisse-une-trace)).
Supprimer une connexion n'efface pas ses conversations, qui gardent le nom de la
connexion : ce que l'utilisateur a demandé ne disparaît pas avec l'outil qui a
servi à le demander. Si l'écriture échoue, la question part quand même, et le
panneau dit que ce fil n'est gardé que dans la fenêtre.

### Une proposition n'est jamais exécutée par le fait de l'être

Une requête proposée par un agent arrive dans une console comme du **texte**, et
rien de plus. Elle est exécutée par l'utilisateur, avec le même bouton et le même
chemin que ce qu'il écrit lui-même. Il n'y a pas de mode « exécution
automatique », et `Explain` sur une proposition suit la même règle : `EXPLAIN
ANALYZE` exécute réellement ce qu'il analyse.

Quand un agent soumet lui-même une commande, elle traverse le `PolicyGate` avec
`Actor::Agent`. Une écriture sur une connexion `production` lui est **refusée**,
pas soumise à confirmation : une confirmation finit par être cliquée (I-02).
L'utilisateur voit le refus dans la conversation, avec son motif.

### Ce qu'un agent a écrit reste marqué

Un document dont le texte vient d'une proposition d'agent porte sa provenance :
l'agent, le modèle et la date. La marque est visible sur l'onglet et dans la
bibliothèque.

Deux règles la gouvernent, et la seconde est celle qu'on casse sans s'en
apercevoir :

* **elle apparaît dès qu'un agent écrit**, y compris dans un document que
  l'utilisateur avait commencé lui-même ;
* **elle ne disparaît plus ensuite.** Réécrire tout le texte à la main ne
  l'efface pas, et une autosauvegarde ordinaire — qui ne sait rien de l'origine
  du texte — ne l'efface pas non plus. Une provenance absente veut dire « rien
  de neuf à écrire », jamais « personne ».

Ce que la marque affirme est donc « un agent a écrit ce texte », pas « il l'a
écrit en entier » ni « il est le dernier à l'avoir touché ». Prétendre la
seconde chose demanderait de suivre chaque frappe, ce qu'Oxyn ne fait pas et
n'a pas à faire.

Elle ne suit pas un copier-coller : le presse-papiers ne porte pas de
métadonnée. Un texte recopié à la main dans un autre document y arrive donc sans
marque, et c'est une limite assumée d'ADR-0023, pas un défaut à contourner.

### Configuration des fournisseurs et des agents

L'écran IA des réglages déclare deux sortes de destinations et les montre dans
**une** liste : les fournisseurs, avec leur clé, et les agents externes, sans
clé.

Un fournisseur se déclare par sa famille, son point d'accès, son modèle et, si
le point d'accès l'exige, une clé. La clé va au trousseau du système ; elle
n'est ni réaffichée, ni recopiée dans un message d'erreur, ni exportée avec le
workspace (I-03). Ce que l'écran remontre après enregistrement, c'est
« configurée » ou « absente », jamais la valeur.

L'écran affiche le classement local/distant **avec l'instant de sa mesure**,
parce qu'il est recalculé et jamais persisté : un point d'accès classé local
hier peut résoudre ailleurs aujourd'hui. Une URL portant des identifiants dans
son autorité est refusée à la saisie, pas nettoyée en silence.

Les fournisseurs sont communs à tous les workspaces ; le niveau de
confidentialité, lui, reste attaché à chaque connexion (ADR-0023). Il en va de
même des agents externes.

**Un agent externe se déclare de deux façons**, et aucune n'est déclarée sans que
l'utilisateur l'ait voulu :

* **par un préréglage**, pour les agents qu'Oxyn connaît — Claude Code et Codex
  ([AI-PROVIDERS](AI-PROVIDERS.md#un-agent-externe--la-portée-nest-pas-inconnue-elle-est-inconnaissable)).
  **À chaque ouverture de l'écran**, Oxyn cherche dans les emplacements
  d'installation documentés le lanceur, `node` et le programme de l'agent. C'est
  une **lecture du disque**, hors du thread de l'interface : rien n'est lancé,
  rien n'est enregistré. Un agent installé pendant qu'Oxyn tourne se retrouve
  par « Detect again ». L'écran montre la commande proposée, avec la version
  épinglée de l'adaptateur, ce qui a été trouvé ou non, et la variable `PATH`
  ajoutée pour qu'un Oxyn lancé depuis le Finder trouve Node — jamais un jeton.
  Sans lanceur trouvé, la déclaration est impossible : elle enregistrerait un
  agent qui ne peut pas démarrer. La commande d'authentification de l'agent est
  affichée à côté ; c'est l'agent qui connecte son utilisateur, Oxyn ne voit
  aucun identifiant ;
* **à la main**, par un programme, ses arguments et son environnement, lancés
  sans shell. Les arguments se tapent **un par ligne**, tels que le programme
  les reçoit : aucun guillemet, aucun découpage sur l'espace
  ([ADR-0026](adr/0026-agents-externes-acp.md)). L'environnement se tape une
  ligne `NOM=valeur` par variable ; une ligne sans nom valide refuse
  l'enregistrement plutôt que d'être ignorée. Les valeurs, qui sont souvent des
  jetons, partent une fois et quittent l'écran aussitôt, que l'enregistrement
  réussisse ou non ; la liste n'en montre que les noms.

Dans les deux cas, déclarer demande **deux confirmations** : le bouton de
l'écran, puis une boîte de dialogue **native** qui recopie la commande exacte,
environnement compris, et rappelle qu'Oxyn ne voit pas où l'agent envoie les
questions. Native, parce qu'un script injecté dans la webview peut appeler la
commande de déclaration mais pas cliquer une fenêtre qu'il ne dessine pas
([ADR-0026](adr/0026-agents-externes-acp.md)).

Un agent déclaré **exactement** comme le préréglage le propose — même adaptateur,
même version, aucun argument de plus — est confiné au lancement
([ADR-0032](adr/0032-agent-externe-confine-au-lancement.md)). Tout autre agent,
préréglage modifié compris, est traité comme inconnu : la liste le marque d'un
avertissement disant qu'Oxyn ne peut pas l'empêcher d'agir seul sur la machine,
et le panneau le répète quand il est choisi.
