<!-- oxyn-translation source="docs/adr/0082-project-schema-maps-from-provenanced-catalog-metadata.md" sha256="900e7c9bd846" -->

# ADR-0082 — Construire les cartes de schéma depuis des métadonnées de provenance explicite

**Statut :** proposé · **Date :** 2026-11-13

**Précise :** [ADR-0003](0003-driver-capabilities.md) sur les capacités de
présentation et [ADR-0041](0041-registre-d-actions-menus-et-raccourcis.md) sur les
actions de carte du workspace. Cette proposition ne remplace pas le contrat ERD
livré de l'assistant et n'active aucune introspection non prise en charge.

## Contexte

La carte autonome demandée pendant la recherche doit couvrir sources
relationnelles, documentaires et espaces de clés. Au commit `6647180`,
l'assistant rend déjà un ERD avec React Flow et Dagre. Son modèle contient tables
et clés étrangères, mais omet le type d'objet et la provenance déduite des
champs ; ses lectures initiales demandent les clés entrantes même sans cette
capacité du driver.

La [matrice des moteurs](../implementation-plan/schema-research-engines.md)
montre pourquoi un graphe de style SQL ne représente pas fidèlement toutes les
sources. Clés déclarées, appartenance de mappings et champs déduits d'échantillons
ont des sens différents.

## Décision

La carte est un onglet ordinaire du workspace, disponible sans fournisseur IA.
Elle s'ouvre depuis une connexion, un espace de noms ou un objet sélectionné ;
l'assistant peut ouvrir la même carte avec un périmètre présélectionné.
Sélection et dessin n'accèdent pas au serveur par eux-mêmes. Les demandes
explicites de métadonnées utilisent le bus et
[ADR-0081](0081-load-catalog-facets-independently-with-explicit-prefetch.md).

`oxyn-catalog` possède une `SchemaMapProjection` pure : identités d'objets,
descriptions de champs, liens typés, provenance, fraîcheur et couverture.
Elle ne contient pas de coordonnées et ne fait pas d'I/O. `oxyn-desktop`
orchestre les lectures autorisées et construit une projection IPC bornée.
Une fonctionnalité front partagée possède rendu React Flow, placement Dagre et
navigation ; l'assistant conserve seulement son adaptateur de demande, pas un
second assembleur de graphe.

L'identité d'un nœud comprend `ConnectionId` et l'adresse complète validée
de l'objet. Les libellés ne sont pas des clés. Le type d'objet est conservé ;
les champs distinguent information déclarée, observée et indisponible.
Les champs déduits portent couverture/provenance, pas des valeurs de lignes exemples.

Les liens ont des types explicites : clé étrangère déclarée, dépendance d'objet
rapportée, appartenance rapportée ou inclusion de champ. Chacun porte sa source
et sa couverture. Une cardinalité inconnue reste inconnue. Inverser les clés
sortantes chargées ne démontre les liens entrants que dans cet ensemble. Un
lien vers un objet non chargé montre une extrémité non résolue au lieu d'impliquer
une découverte complète. Un nom tel que `user_id` ne crée jamais de lien.

| Métadonnées disponibles | Représentation |
|---|---|
| Relations et clés étrangères | Cartes d'objets et liens déclarés |
| Relations sans introspection des clés | Cartes d'objets ; aucune relation inventée |
| Collections et champs observés | Structure de collection avec provenance d'échantillonnage |
| Mappings et appartenance rapportée | Hiérarchie de champs et liens techniques typés |
| Espace de clés échantillonné | Vue clés/types ; regroupement facultatif par préfixe marqué comme convention |
| Métadonnées de graphe futures | Labels/types/propriétés avec certitude et couverture explicites du driver |

La capacité `GRAPH` du moteur ne conditionne pas cet écran. Sa disponibilité
dépend de l'introspection réellement exposée par la session. Une facette
facultative manquante ou un objet en échec produit une carte partielle, pas un
graphe vide inventé ni l'échec total des objets déjà lus. Les objets refusés ne
révèlent aucune métadonnée au-delà de ce que fournit la réponse autorisée.

La carte part d'un périmètre choisi et borné, puis développe explicitement les
voisins. Les limites ERD et budgets de catalogue existants restent en vigueur
jusqu'à l'acceptation de changements mesurés. Les grands périmètres affichent
les comptes dessinés/omis/non lus et proposent filtrage ou réduction. Une passe
bornée terminée n'est pas appelée schéma complet de la base.

La lecture déduite d'échantillons est annoncée avant lancement et suit la
politique de connexion existante. Une carte locale ne lance aucune requête IA ;
envoyer ses métadonnées à l'assistant traverse toujours la passerelle unique.
Actualiser annule/remplace la génération active, marque les anciennes données
périmées et conserve les positions stables lorsque les identités subsistent.
Accès clavier et liste d'objets/liens accompagnent le canevas ; F6 reste une
action ordinaire du workspace.

La persistance facultative `SchemaMapView` conserve périmètre versionné,
IDs sélectionnés, positions, filtres et cadrage dans `oxyn-store`, via une
commande du bus réservée à l'humain. Elle ne conserve ni instantané de schéma,
ni échantillons, ni identifiants d'authentification. Rouvrir affiche les choix
enregistrés et exige une lecture explicite des métadonnées actuelles.
Exporter une image est un geste humain explicite qui marque le contenu partiel
ou périmé ; son rendu ne peut élargir le périmètre ni lire d'autres données.

## Conséquences

* **+** Assistant et vue autonome partagent une interprétation des métadonnées.
* **+** L'interface expose une structure utile sans prétendre que chaque moteur a des FK.
* **−** Provenance, résultats partiels et extrémités inaccessibles complexifient le modèle.
* **−** Une disposition enregistrée n'est pas une archive de schéma hors ligne ;
  une carte bornée peut omettre des objets. De nouveaux liens propres à un moteur
  nécessitent un véritable travail dans son driver.

**Coût de sortie :** remplacer projection et adaptateurs de rendu, migrer les
choix d'affichage enregistrés et conserver les lectures du catalogue. Réutiliser
les bibliothèques actuelles évite une seconde pile graphique ; aucune nouvelle
version de paquet n'est choisie.

**À reconsidérer si** les tailles de graphe mesurées dépassent cette approche,
ou si les usages exigent un instantané hors ligne ou des liens affirmés
manuellement. Ces extensions changent persistance ou provenance et exigent
un ADR distinct.

## Alternatives rejetées

| Alternative | Motif du rejet |
|---|---|
| Demander à un LLM de reconstruire le schéma | Rend une inspection déterministe dépendante d'une inférence et d'une destination IA |
| Dessiner chaque moteur comme des tables à clés étrangères | Déforme les structures documentaires et les espaces de clés |
| Parser le DDL dans le front pour deviner les dépendances | Duplique la connaissance des dialectes et peut inventer des liens incorrects |
| Persister tout le graphe comme schéma actuel | Fige des métadonnées partielles ou sensibles et contredit le catalogue en mémoire |
| Faire échouer toute la carte pour une facette facultative indisponible | Empêche une inspection utile sur les drivers moins capables |
