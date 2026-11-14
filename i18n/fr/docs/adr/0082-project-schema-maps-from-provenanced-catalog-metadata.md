<!-- oxyn-translation source="docs/adr/0082-project-schema-maps-from-provenanced-catalog-metadata.md" sha256="8880480a7ff2" -->
# ADR-0082 — Construire les cartes de schéma depuis des métadonnées de provenance explicite

**Statut :** accepté · **Date :** 2026-11-14

**Précise :** [ADR-0003](0003-driver-capabilities.md),
[ADR-0041](0041-registre-d-actions-menus-et-raccourcis.md) et
[ADR-0081](0081-load-catalog-facets-independently-with-explicit-prefetch.md).

## Contexte

La carte autonome doit couvrir les sources relationnelles, documentaires et
les espaces de clés. L'assistant utilise déjà React Flow/Dagre, mais son
assembleur séparé perd les types d'objet, la provenance et les identités
complètes des propriétaires. Hiérarchies natives, clés étrangères, mappings et
champs déduits ont des sens différents ; la [matrice des moteurs](../implementation-plan/schema-research-engines.md)
décrit les métadonnées disponibles sans promettre toutes les fonctions serveur.

## Décision

La carte est un onglet ordinaire, disponible sans fournisseur IA, ouvert depuis
la source sélectionnée ou un catalogue/espace/objet natif. Ouvrir, choisir le
périmètre, dessiner, filtrer, sélectionner, déplacer et zoomer ne lance aucune
lecture native de métadonnées ou de lignes. Charger, rafraîchir, développer les
voisins immédiats d'un objet sélectionné et ouvrir Data/Structure/définition
sont des gestes humains séparés. Le fichier et la session propriétaires restent
les mêmes : aucune connexion ni pièce jointe n'est ouverte pour remplir la
carte. Un fichier SQLite supprimé ou une source éphémère fermée n'est jamais
remplacé par une nouvelle base vide.

Core porte la commande typée réservée à l'humain `LoadSchemaMap`. `oxyn-exec`
possède les lectures bornées et annulables, via la file de métadonnées de la
session et les contrôles atomiques de génération existants. Une demande peut
passer devant un préchargement en attente, mais ne peut publier dans une source
modifiée. Le passage de 60 secondes attend le nettoyage d'annulation ; il garde
les parties disponibles et signale séparément échecs et indisponibilités. Le
pont desktop contrôle fenêtre/session et renvoie un IPC validé.

`oxyn-catalog` possède une `SchemaMapProjection` pure, sans E/S ni coordonnées.
Un adaptateur/rendu frontend commun dessine cette projection dans la carte et
l'assistant. L'assistant ne garde que la résolution des noms déjà listés et
l'adaptation de requête ; il ne tranche pas les ambiguïtés et ne développe pas
automatiquement les voisins. Sa couverture indique les parties non lues,
périmées, omises et les extrémités non chargées. La carte n'envoie rien à une IA ;
tout contexte destiné à l'assistant passe toujours par la passerelle de confidentialité.

Les clés d'objet comprennent connexion, session d'origine et niveaux natifs
optionnels complets. Les clés de champ gardent les segments exacts, dont les
points littéraux. Les membres composites ordonnés distinguent les clés sans
nom. Les libellés ne sont jamais des identités. Types d'objet, provenance et
fraîcheurs indépendantes sont conservés. Seules des clés primaires déclarées et
fraîches prouvent l'unicité ; l'absence de preuve reste une cardinalité inconnue.
Les liens périmés sont signalés.

Les liens distinguent clé étrangère rapportée, dépendance technique,
appartenance native et imbrication de champs. Le canevas dessine les clés
étrangères ; liste accessible, fil natif et inspecteur exposent aussi
appartenance et imbrication. Des membres composites incomplets ne sont jamais
réparés par supposition. Inverser les clés sortantes chargées ne donne des liens
entrants que dans cet ensemble. Les références non chargées restent visibles
dans la liste/l'inspecteur. Aucun nom ni DDL analysé n'invente de lien. Les
dépendances techniques structurées restent explicitement indisponibles tant
que le contrat fournisseur ne les expose pas.

| Métadonnées disponibles | Représentation |
|---|---|
| Relations avec introspection des FK | Cartes d'objet et liens de clés rapportées |
| Relations sans introspection des clés | Cartes d'objet avec clés indisponibles |
| Collections avec champs échantillonnés explicitement | Collections, champs imbriqués et provenance échantillonnée |
| Mappings serveur | Hiérarchie de champs rapportée, sans relations inventées |
| Espaces de clés | Cartes clé/type et provenance de disposition d'aperçu, sans prétendre un schéma documentaire déclaré |

La disponibilité suit les capacités de la session d'origine, pas une liste de
moteurs ou le drapeau `GRAPH`. Une facette optionnelle manquante n'efface pas
les objets utiles. Aucune exemption SQLite générale, introspection de toutes
les bases, requête IA automatique ou inférence documentaire implicite n'est
ajoutée. Une confirmation humaine séparée nomme objet/source et limites de
documents/octets/temps déclarées avant l'échantillon. Seules les métadonnées de
champs/types entrent dans la carte, jamais les valeurs échantillonnées.

Les limites existantes de 40 objets et 12 champs affichés par objet sont
conservées ; les champs imbriqués partagent ce total. Publication/rendu bornent
aussi les liens à 200. Omissions d'objets/champs/liens, descriptions non lues ou
périmées et références non chargées sont visibles ; un passage borné terminé
n'est jamais appelé schéma complet. Les types affichés sont limités à 256
caractères avec indication explicite. Identités/membres natifs trop grands sont
omis sans être tronqués en une autre identité. La garde commune de 16 Mio de
métadonnées JSON couvre aussi les cartes dont les clés répètent des chemins.
Si nécessaire, liens puis ensembles de champs sont réduits avec comptes
d'omissions, en conservant les identités d'objet. Une identité qui ne tient pas
produit une erreur explicite.

Un rafraîchissement préserve les positions des identités complètes survivantes.
Les filtres masquent les objets retenus sans reconstruire leurs identités.
Dagre ne recalcule les positions que lorsque le graphe projeté change. Le
[bilan](../implementation-plan/completion.md) conserve mesures et contrôles
réellement effectués. F6 réutilise l'enregistrement des zones visibles.

Dispositions durables, export d'image, copie hors ligne et édition DDL graphique
restent des lots séparés. Cette décision n'ajoute aucun format de carte persisté.

## Conséquences

- Assistant et carte partagent la même interprétation pour tous les moteurs.
- Sessions d'origine et facettes indépendantes restent la frontière d'autorité.
- Provenance, états partiels et références non chargées complexifient modèle et interface.
- Dépendances natives et vues graphe/vecteur/séries demandent des métadonnées fournisseur.
- Un graphe borné peut omettre objets/champs ; le placement initial peut dépasser
  un budget d'image. Cela ne justifie pas d'élargir les limites.

**Coût de sortie :** remplacer projection, DTO du bus et deux adaptateurs ;
garder lectures natives et propriété. Pas de migration de disposition ni de
nouveau paquet de rendu.

**Réexaminer si** les tailles mesurées imposent un autre rendu, ou si des copies
hors ligne/liens manuels deviennent nécessaires. Ces changements touchent
persistance/provenance et demandent une décision séparée.

## Alternatives rejetées

| Alternative | Motif |
|---|---|
| Demander le schéma à un LLM | Rend l'inspection déterministe dépendante de l'inférence et d'une destination IA |
| Présenter tous les moteurs comme des tables SQL avec FK | Déforme documents et espaces de clés |
| Garder plusieurs assembleurs frontend | Fait diverger identités, fraîcheur, provenance et clés composites |
| Déduire les dépendances du DDL ou des noms | Duplique les dialectes et peut inventer des liens |
| Persister le graphe comme vérité courante | Fige des métadonnées partielles contre le contrat de cache mémoire |
| Échouer totalement pour une facette optionnelle absente | Empêche une inspection utile des sources moins capables |
