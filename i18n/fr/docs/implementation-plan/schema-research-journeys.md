<!-- oxyn-translation source="docs/implementation-plan/schema-research-journeys.md" sha256="580ff07fc069" -->

# Recherche sur les schémas : parcours utilisateur

Statut : **proposition de recherche, pas un contrat produit accepté**. Préparée
le 2026-11-13 sur le code au commit `6647180`. Ce document est une annexe du
[plan d'implémentation](../IMPLEMENTATION-PLAN.md), pas une seconde feuille de
route. La [matrice des moteurs](schema-research-engines.md) porte l'inventaire
des capacités.

**Revalidation — 2026-11-13 :** comparaison de `6647180..f456ff1` après
synchronisation avec `main` local. La correction des URI/lecteurs privés
SQLite, l'ajustement du test du popover de tri et la préparation de la version
0.0.18 n'introduisent aucune contradiction bloquante avec ces parcours ou les
ADR proposés 0079–0082. Le code initial indiqué ci-dessus reste une preuve
historique. Les cas SQLite sont enrichis ci-dessous et dans la matrice.
Cette revalidation est statique : les nouveaux tests SQLite ont été lus, pas
exécutés ; elle ne démontre pas la réussite du contrôle qualité applicatif.

Périmètre : issues [#308](https://github.com/so-keyldzn/oxyn/issues/308),
[#373](https://github.com/so-keyldzn/oxyn/issues/373),
[#374](https://github.com/so-keyldzn/oxyn/issues/374),
[#375](https://github.com/so-keyldzn/oxyn/issues/375), et carte autonome des
schémas demandée. Cette étape décrit uniquement des parcours et des critères
d'acceptation observables : aucun code applicatif, accès serveur, ajout de
dépendance ou ADR accepté.

**Revalidation d’implémentation — 2026-11-14, base `ce58839` :** le chantier
multi-base conserve la séparation catalogue/console de main et l’occupation des
sessions par les aperçus retenus. La découverte PostgreSQL reste distincte du
parcours du catalogue ; les hiérarchies natives des autres moteurs sont
conservées. Les ADR de recherche portent désormais les numéros 0080–0083,
l’ADR-0079 accepté sur main concernant les aperçus sur la session d’origine.
Les cas SQLite restent applicables, sans exemption automatique de préchargement.
L’état courant du code et des tests figure dans le [suivi](completion.md),
séparément de cet inventaire historique.

## Comment lire cette proposition

**Existant** signifie trouvé par inspection statique, pas essayé sur un serveur.
**Proposé** décrit le parcours souhaité, sous réserve des décisions ci-dessous.
Les libellés anglais des écrans sont provisoires.

Les contrats applicables restent [UX-SPEC](../UX-SPEC.md),
[SECURITY](../SECURITY.md), [DRIVER-CONTRACT](../DRIVER-CONTRACT.md),
[AI-PROVIDERS](../AI-PROVIDERS.md) et [PERFORMANCE](../PERFORMANCE.md).
Cette proposition ne déclare pas leurs critères de phase satisfaits. Le plan
signale encore des mesures de performance de bout en bout non produites ; la
carte autonome concerne aussi le travail sur les diagrammes ER de la phase 4.
Modifier l'ordre des phases nécessite une décision séparée.

## J01 — Parcourir plusieurs bases d'un serveur

**Entrée :** connexion enregistrée ou profil serveur proposé ; l'humain choisit
`Browse databases`.

**Existant :** PostgreSQL introspecte sa base connectée ; plusieurs autres
drivers listent déjà plusieurs bases ou espaces de noms. L'identité du workspace
est une connexion, pas un groupe serveur.

**Parcours proposé :** afficher la destination et la portée d'accès du profil ;
découvrir les noms des bases sans toutes les ouvrir ; développer une base
choisie ; ouvrir ses schémas et objets ; ouvrir une console dont la destination
reste visible. Utiliser une connexion technique par base PostgreSQL. Conserver
la hiérarchie de chaque driver plutôt qu'ajouter partout un niveau artificiel.

La configuration choisit une connexion initiale autorisée ou en crée une.
Chaque ouverture enfant revoit ses identifiants et protections. L'explorateur
reste commun, mais chaque base garde son workspace et sa barre d'onglets.
Un clic sélectionne ; le chevron développe ; Entrée, double-clic ou Open workspace
active. En-tête et assistant suivent le workspace actif, pas la ligne choisie.
Les bases connectées inactives gardent sessions, brouillons et transactions rapportées.

**Échec et reprise :** une base refusée reste identifiable avec une erreur
locale ; les autres restent utilisables. L'annulation arrête l'ouverture en
attente. Supprimer ou modifier un profil doit tenir compte de ses bases ouvertes
et de ses identifiants d'authentification.
Suivre ouverture et annulation par enfant, réserver la capacité avant ouverture
et montrer la limite de fenêtre sans évincer le travail d'une autre base.

**Acceptation :** avec des tables homonymes dans A et B, sélectionner B ne
change ni le brouillon ni la transaction de A. La découverte n'ouvre pas une
session par base listée. Aucun ancien secret n'est réutilisé hors de sa
destination autorisée. **Décision :** D01.
Sélectionner B alors que A est active ne redirige ni menus ni exécution de A.
Ouvrir B plusieurs fois ne crée pas de sessions en double.

## J02 — Inspecter une table, une vue ou un autre objet

**Entrée :** catalogue, recherche, objet lié ou nœud de la carte.

**Existant :** Données, Structure, Index, Contraintes, Relations et DDL partagent
une vue d’objet. #375 sépare l’ancien `detail` en lectures indépendantes de
description, d’index et de clés sortantes. La définition adjacente attend une
demande explicite ; la validation figure dans le registre d’achèvement.

**Parcours proposé :** identifier connexion, base, espace de noms et objet ;
ouvrir la vue adaptée ; demander uniquement ses facettes nécessaires ; afficher
séparément fraîcheur et disponibilité ; naviguer vers un objet lié sans changer
la console d'origine. Données peut demander les métadonnées de clé nécessaires
à la pagination.

**Échec et reprise :** conserver les informations précédentes avec un marqueur
périmé ; distinguer non lu, vide, non pris en charge, inaccessible et échoué.
Une réponse tardive pour l'objet précédent ne remplace pas l'objet sélectionné.

**Acceptation :** ouvrir Index ne recharge pas les facettes sans rapport. Une
vue sans clé de pagination démontrée ne reçoit pas de clé inventée. Les résumés
des contraintes n'annoncent pas l'absence d'index uniques lorsque les index
n'ont pas été lus.

**Acceptation SQLite :** exercer [SQ01–SQ05](schema-research-engines.md#sqlite-target-cases)
à l'ouverture de Données depuis l'arbre ou la carte. Préserver l'identité de
l'URI source et ses options autres que l'accès lors de la création d'un aperçu
isolé en lecture seule. Une mémoire partagée nommée doit atteindre la même base
vivante. Pour les objets sans clé d'une source mémoire privée ou temporaire
sans nom, garder la lecture bornée sur la session d'origine : ne proposer ni
continuation conservée non prise en charge, ni base vide de remplacement.
L'impossibilité de rouvrir un fichier supprimé est une erreur, pas un aperçu
vide ni une création de fichier. Disponibilité des métadonnées et disponibilité
de l'aperçu conservé restent distinctes.
**Décision :** D02.

## J03 — Maîtriser le chargement des métadonnées

**Entrée :** `Metadata loading…` dans l’explorateur de la source active.

**Existant :** lectures hiérarchiques, facettes indépendantes, cache borné,
complétion de l’éditeur et de l’assistant. #375 ajoute les préférences manuelles
Minimal/Structure/Étendu et les dérogations exactes d’espace de noms natif ;
la validation est consignée dans le [registre d’achèvement](completion.md).

**Parcours proposé :** choisir un préréglage, voir son héritage, examiner son
périmètre et demander explicitement les détails supplémentaires nécessaires.

| Préréglage | Préchargement du périmètre choisi | Lecture explicite |
|---|---|---|
| Minimal | Noms des objets | Toute facette prise en charge à son ouverture |
| Structure | Noms, colonnes, index et clés sortantes | Définitions et autres facettes |
| Étendu | Structure et définitions disponibles | Les familles non prises en charge restent indisponibles |

Ces préréglages n’agrandissent pas les budgets et ne promettent pas de garder
la base entière en mémoire. Choisir un préréglage ne lance pas une lecture en
masse : une action distincte précharge le périmètre sélectionné. Enregistrer la
préférence ou développer un niveau ne précharge rien en masse. Chaque page
explicite s’arrête à 128 relations / 60 secondes ; la page suivante est liée à
la génération de source. Facettes indisponibles, échouées, fraîches, périmées et
non lues restent distinctes. L’annulation atteint la session d’origine. Les
préférences sont du JSON local lisible séparé de l’authentification.

**Échec et reprise :** afficher le travail en attente, annulé et incomplet ;
abaisser le niveau arrête le préchargement remplacé sans perdre l’éditeur.
Une information manquante reste distincte d’un résultat vide.

**Acceptation :** arbre, complétion, diagramme et assistant demandent leurs
facettes nécessaires ; changer la largeur ne déclenche pas le DDL adjacent.
Aucun seuil automatique ni exception SQLite. **Décision :** D02 ; les mesures
n’autorisent aucun seuil automatique.

## J04 — Ouvrir une carte relationnelle sans IA

**Entrée :** `View schema map` sur une base ou un schéma ; `Show related objects`
sur un objet ; ou ouverture du diagramme de l'assistant dans un onglet.

**Existant :** l'ERD de l'assistant utilise les métadonnées du catalogue, React
Flow et Dagre. Ses noms sources ne sont résolus que parmi les objets déjà
listés. Ce composant ne fournit pas de carte autonome, de disposition déplaçable
enregistrée ni d'export d'image.

**Parcours proposé :** choisir périmètre et types d'objets ; lister les objets
disponibles ; dessiner les objets sélectionnés ; charger progressivement les
colonnes et liens pris en charge ; développer les voisins à la demande ;
inspecter un nœud/lien ou ouvrir Structure/Données/DDL. Zoom, déplacement du
cadrage et sélection ne demandent aucune ligne. Ouvrir Données conserve le
comportement ordinaire de l'aperçu et sa politique.

**Échec et reprise :** conserver les nœuds utiles si une lecture échoue ;
signaler les liens inconnus et les extrémités non chargées. Une source sans
introspection des clés entrantes peut afficher les objets et les clés sortantes
disponibles. Inverser les clés chargées ne démontre les liens entrants qu'à
l'intérieur de ce périmètre chargé.

**Acceptation :** cycles, clés composites et objets homonymes restent distincts ;
aucun lien n'est inventé depuis un nom de colonne ; une cardinalité inconnue le
reste. Les comptes distinguent objets dessinés, omis et non lus, avec une liste
accessible. Enregistrer une disposition ne doit pas enregistrer de métadonnées
porteuses de secrets ni d'échantillons.
**Décision :** D03 ; garder les limites ERD existantes jusqu'à justification mesurée.

## J05 — Inspecter une structure documentaire ou un espace de clés

**Entrée :** même action de carte, avec une représentation choisie selon
l'introspection réellement disponible ; les rendus candidats sont dans la matrice.

**Parcours proposé :** afficher collections/mappings/groupes de clés ; exposer
les champs imbriqués lorsqu'ils sont disponibles ; distinguer structure déclarée
et échantillons observés. Pour une structure déduite, expliquer que des données
seront lues, montrer le périmètre borné et permettre l'annulation. Aucune valeur
observée n'appartient à un diagramme exporté. Consulter localement ne lance pas
de requête IA.

**Échec et reprise :** un échantillon vide signifie aucun champ observé, pas la
preuve d'un schéma vide. Les types mixtes restent mixtes. Une structure
inaccessible n'est pas remplacée par des relations devinées. Regrouper des clés
Redis suivant une convention de nommage est présenté comme un regroupement,
pas comme une contrainte de base de données.

**Acceptation :** observations MongoDB, mappings Elasticsearch et groupes de
clés Redis ne peuvent pas être confondus avec des clés étrangères SQL. Une vue
de structure peut réussir sans aucun lien. Aucun parcours exhaustif de
collection ou d'espace de clés ne démarre implicitement.
**Décision :** D03 ; les familles futures nécessitent d'abord un driver.

## J06 — Interroger, copier et questionner sur l'objet choisi

**Entrée :** menu d'objet, console SQL, complétion ou assistant.

**Parcours proposé :** copier un modèle INSERT demande les colonnes ; copier
une définition demande cette définition ; ouvrir du texte dans une console
n'exécute rien. La complétion résout les noms dans le contexte de cette console.
L'assistant indique sa connexion et la couverture des métadonnées reçues.

L'ouverture groupée démarre une nouvelle base avec une console vide.
Copy SQL to database nomme source et destination explicitement. Un dépôt d'objet
entre bases n'insère rien et propose une console sur la base source ou Annuler.
Les demandes tardives conservent connexion et génération de console destinataires ;
changer de base active ne leur fournit pas une cible de remplacement.
Cette précision figure dans l'ADR-0080 proposé ; la copie de brouillon existante
hors regroupement n'est pas annoncée comme modifiée.

**Échec et reprise :** changer de base active ne peut pas rediriger silencieusement
une commande en attente, un échantillon approuvé, un objet épinglé ou une
conversation. Les métadonnées indisponibles restent signalées comme contexte manquant.

**Acceptation :** deux objets homonymes de bases différentes n'échangent jamais
suggestions, buffers de résultats ou échantillons approuvés. Les actions
Renommer/Supprimer/Tronquer existantes restent contrôlées par capacités et
passent par leur revue et politique actuelles. Ajouter un éditeur visuel complet
de tables/vues sort de ce périmètre de recherche.
**Décisions :** D01/D02 ; passerelle IA et protections de production conservées.

## J07 — Exporter une base PostgreSQL

**Entrée :** action d'export natif proposée sur une base prise en charge.

**Parcours proposé :** identifier la source ; choisir un exécutable installé,
un format pris en charge et une destination ; revoir les options ; démarrer ;
afficher la progression sans pourcentage inventé ; annuler ou obtenir une
confirmation de fin et l'emplacement du fichier. Distinguer ce parcours de
l'export des lignes conservées d'une grille.

**Échec et reprise :** exécutable absent/incompatible, échec d'authentification,
destination refusée, disque plein ou échec du processus enfant sont explicites.
Un fichier interrompu n'est jamais présenté comme une sauvegarde réussie ; une
destination existante n'est pas écrasée silencieusement.

**Acceptation :** aucun texte de commande shell ni secret dans l'IPC ou les
journaux ; réussir exige la réussite du processus et la finalisation du fichier.
Fermer la surface de progression suit une règle explicite de cycle de vie de
tâche, sans processus orphelin.
**Décision :** D04. PostgreSQL est un premier adaptateur proposé, pas livré.

## J08 — Restaurer dans une base délibérément choisie

**Entrée :** action de restauration proposée ; fichier et cible sont explicites.

**Parcours proposé :** examiner format/options pris en charge ; montrer
l'identité cible et les effets destructeurs ; autoriser par le bus et la
confirmation native requise ; exécuter comme tâche suivie ; rapporter l'issue ;
invalider métadonnées et aperçus concernés. La cible revue doit être la cible
effective même si l'archive nomme une autre base.

**Échec et reprise :** distinguer refus, échec avant lancement, application
partielle, annulation avec résultat inconnu et réussite. Annuler promet l'arrêt
du travail, pas son annulation transactionnelle. Proposer l'inspection après
une issue ambiguë ; ne jamais rejouer automatiquement.

**Acceptation :** les cibles en lecture seule sont refusées ; la confirmation de
production nomme la connexion effective ; un agent ne peut pas écrire en
production. Après interruption, une ancienne carte est marquée périmée et non
présentée comme l'état restauré. Les diagnostics bruts des outils ne doivent
exposer ni secrets ni contenu sensible des lignes.
**Décision :** D04, notamment authentification et arrêt de l'arbre de processus.

## J09 — Reprendre après changement, déconnexion ou redémarrage

**Entrée :** workspace conservé, réouverture d'onglet ou récupération au démarrage.

**Existant :** consoles, résultats conservés et positions d'objet portent leur
connexion ; les onglets d'objet restaurés attendent un geste explicite de lecture.

**Parcours proposé :** retrouver l'identité connexion/objet enregistrée ;
signaler les cibles indisponibles ; reconnecter uniquement par le parcours
autorisé. Retrouver les positions/filtres éventuels de carte séparément des
métadonnées vivantes.

**Échec et reprise :** une base supprimée ou un profil retiré ne peut pas
basculer silencieusement vers une autre destination. Fermer une base libère
uniquement son travail ; fermer un profil tient compte de chaque session/tâche
dépendante.
Déconnecter le groupe liste d'abord bases affectées, travail non enregistré,
opérations en cours et transactions ouvertes/inconnues. Traiter les transactions
par leur revue existante avant de terminer ; Annuler ne ferme rien. Rapporter
les fermetures partielles par base. Détacher, déconnecter, supprimer le profil et
DDL de base restent distincts ; les ressources d'une autre fenêtre gardent leur propriétaire.

**Acceptation :** les anciens workspaces restent lisibles ; la récupération ne
lance pas de requête/restauration surprise. La limite existante de workspaces
conservés continue à borner les bases ouvertes.

**Acceptation SQLite :** [SQ05/SQ06](schema-research-engines.md#sqlite-target-cases)
couvrent un fichier supprimé et une source mémoire/temporaire disparue.
Rouvrir un onglet enregistré ne doit pas créer une base par effet de bord.
Une URI de mémoire partagée nommée conserve l'identité seulement tant que cette
base existe ; enregistrer son nom ne conserve pas son contenu après fermeture
ou redémarrage du processus. Une création explicite démarre une nouvelle source
et ne peut hériter silencieusement de l'ancien aperçu, de la fraîcheur du
diagramme ou d'une continuation. Les buffers de résultats conservés restent
distincts d'une base vivante. Ces cas de récupération sont des critères
d'acceptation proposés, pas des comportements démontrés par les nouveaux tests URI.
**Décisions :** D01/D03/D04 pour persistance et cycles de vie.

## J10 — Parcourir tout le workspace au clavier

**Entrée :** F6 / Shift+F6 depuis le workspace actif.

**Existant :** UX-SPEC nomme ces touches ; le manifeste d'actions ne les lie pas.

**Parcours proposé :** parcourir les zones de focus visibles dans l'ordre de
lecture, inversement avec Shift+F6. Ignorer les régions fermées ; préserver
curseur/sélection ; rester dans les boîtes de dialogue modales. La navigation
propre à la carte reste limitée à son canevas lorsqu'il a le focus.

**Acceptation :** catalogue, éditeur, résultat, carte et panneau latéral sont
accessibles lorsqu'ils existent, en mode compact aussi ; les workspaces cachés
ne reçoivent jamais le focus. Menus, palette et aide dérivent leurs libellés du
même registre. Cela corrige #308 sans nouvelle commande de base de données.

## Frontières proposées et ADR

Les ADR ci-dessous consignent des propositions concrètes, ni implémentées ni
acceptées. Ils portent les choix de conception ; ces parcours conservent le
travail d'acceptation.

| ID | ADR proposé | Responsabilité et parcours concernés |
|---|---|---|
| D01 | [0079 — Regroupement serveur sans autorité partagée](../adr/0080-group-database-connections-under-server-profiles.md) | Modèle core/store ; découverte driver ; sessions de l'exécuteur ; identifiants de l'hôte ; J01/J06/J09. Identifiants indépendants par enfant ; pas d'autorisation serveur globale dans cette révision. |
| D02 | [0080 — Facettes indépendantes et préchargement explicite](../adr/0081-load-catalog-facets-independently-with-explicit-prefetch.md) | Modèle du catalogue ; politique de l'exécuteur ; lectures des drivers ; consommateurs desktop ; J02/J03/J06. Changer la préférence ne lance pas de lecture en masse. |
| D03 | [0081 — Cartes à provenance explicite](../adr/0082-project-schema-maps-from-provenanced-catalog-metadata.md) | Projection du catalogue ; orchestration par l'hôte ; rendu partagé ; J04/J05/J09. Persistance d'affichage distincte des métadonnées. |
| D04 | [0082 — Tâches de transfert natif autorisées](../adr/0083-run-native-database-transfers-as-authorized-jobs.md) | Politique/audit dans l'exécuteur ; frontière oxyn-transfer proposée ; dialogues desktop ; J07/J08/J09. Transport d'authentification encore soumis à un prérequis. |

Les lectures existantes utilisent `RefreshCatalogScope` ; dessiner un graphe
en cache ne nécessite aucune commande d'exécution. Découverte et transferts
natifs nécessitent des contrats explicites du bus avant implémentation ; leurs
noms proposés figurent dans les ADR, pas les signatures finales de méthodes.
Les invariants concernés sont
I-01/I-02/I-03/I-04/I-05/I-06/I-07/I-08/I-09/I-10/I-11/I-13 ; versions ou
limites externes exigent aussi les preuves d'I-12.

### Prérequis de recherche et de validation

- D01 : revoir la friction de saisie des identifiants, la migration et la
  suppression de groupe face à la propriété des workspaces/fenêtres existants.
  Aucun repli implicite sur un secret partagé.
- D02 : mesurer trafic réel des facettes et pic d'octets ; démontrer fraîcheur,
  annulation et atomicité des anciennes lectures avant de changer consommateurs
  ou budgets.
- D03 : valider cartes partielles et provenance par moteur ; établir
  sérialisation et rendu bornés avant dispositions enregistrées ou exports.
- D04 : prouver un mode d'authentification et l'arrêt des descendants par
  plateforme avec des identifiants fictifs. Jusque-là, ce mode reste indisponible.
  Consigner les preuves des bibliothèques/API externes retenues dans
  RESEARCH-NOTES avant implémentation. Aucune exception de fichier en clair
  n'est supposée.

## Preuves et limite de validation

La matrice lie chaque driver. Sources transversales inspectées :

- [Vue d'objet et déclencheurs](../../../../apps/desktop/src/features/workspace/object-view.tsx),
  [hook des facettes](../../../../apps/desktop/src/features/metadata/use-relation-facets.ts),
  [schéma IPC](../../../../apps/desktop/src/lib/ipc/metadata.ts).
- [Lectures du catalogue](../../../../crates/oxyn-exec/src/catalog.rs),
  [passe de complétion](../../../../crates/oxyn-desktop/src/backend/completion_load.rs),
  [complément de l'assistant](../../../../crates/oxyn-desktop/src/backend/ai/catalog_fill.rs).
- [Assemblage ERD](../../../../apps/desktop/src/features/assistant/erd.ts),
  [rendu](../../../../apps/desktop/src/components/oxyn/erd-diagram.tsx),
  [modèle](../../../../apps/desktop/src/components/oxyn/erd-model.ts).
- [Propriété des workspaces](../../../../apps/desktop/src/features/session.ts),
  [position d'objet](../../../../apps/desktop/src/features/workspace/object-location.ts),
  [persistance des fenêtres](../../../../crates/oxyn-core/src/window_layout.rs),
  [manifeste d'actions](../../../../apps/desktop/src/lib/actions/actions.json).

Les critères ci-dessus constituent un inventaire de validation future, pas des
tests réussis. Aucun serveur réel, échantillon, benchmark ou transfert natif
n'a été essayé. Le livrable de recherche est examinable lorsque chaque parcours
a un déclencheur, un chemin d'échec/reprise, un critère d'acceptation et une
référence de capacité/décision.
