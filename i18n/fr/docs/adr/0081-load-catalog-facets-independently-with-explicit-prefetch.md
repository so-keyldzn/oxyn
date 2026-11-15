<!-- oxyn-translation source="docs/adr/0081-load-catalog-facets-independently-with-explicit-prefetch.md" sha256="9be5256025c8" -->

# ADR-0081 — Charger les facettes indépendamment et rendre le préchargement explicite

**Statut :** accepté · **Date :** 2026-11-14

**Précise :** [ADR-0018](0018-apercu-ddl.md),
[ADR-0022](0022-rafraichissement-automatique.md),
[ADR-0036](0036-l-assistant-complete-le-catalogue.md) et
[ADR-0057](0057-editor-completion-reads-the-catalog-cache.md), sur la granularité
des métadonnées et le préchargement explicite. Les plafonds existants de catalogue,
de complétion et d’assistant sont conservés ; des gardes bornent les entrées et
leur publication.

## Contexte

Au commit `6647180`, `CatalogRefreshScope::Relation` lit description,
index et clés étrangères sortantes dans un patch atomique. Index et clés
sortantes partagent aussi la fraîcheur IPC de la description. La complétion
peut donc demander des informations qu'elle ne consomme pas. En affichage large,
la vue d'objet demande une définition à côté de ses onglets de métadonnées.
L'issue [#375](https://github.com/so-keyldzn/oxyn/issues/375) demande un contrôle
du chargement, mais les lectures hiérarchiques et par objet existent déjà.

Les [parcours de recherche](../implementation-plan/schema-research-journeys.md)
identifient tous les consommateurs concernés. Un réglage visuel appliqué
uniquement à l'arbre laisserait complétion, DDL adjacent et assistant à l'écart.

## Décision

`oxyn-catalog` conserve valeur, fraîcheur et état de prise en charge indépendants
pour description, index, clés sortantes, contraintes, clés entrantes et définition.
La description conserve champs et garanties d'aperçu existants de `Relation` ;
séparer la liste des index ne retire pas les preuves de pagination du driver.

`CatalogRefreshScope` gagne les lectures explicites `Description`, `Indexes`
et `OutgoingForeignKeys`. Elles réutilisent les méthodes correspondantes de
`CatalogProvider`. La requête `Relation` reste un agrégat de compatibilité :
toutes les lectures constituantes prises en charge réussissent avant publication
du patch. Une description seule ne suffit pas à rendre cet agrégat frais.
Les consommateurs migrent vers la requête précise dont ils ont besoin.

Une facette indépendante est publiée atomiquement, conserve sa valeur antérieure
en cas d'erreur et porte son issue d'annulation. Les dépendances sont validées
avant publication : une réponse tardive pour un objet retiré ou remplacé ne
peut s'attacher à son successeur. Une invalidation DDL touche les facettes
concernées ; relister ne rend pas frais des détails non lus. L'éviction conserve
la distinction entre jamais lu et vide connu. L'exécuteur valide les réponses
indépendamment du driver.

`MetadataLoadingPolicy` contrôle le préchargement explicite, pas l'accès ni
la confidentialité IA. Elle est conservée par connexion avec surcharges
facultatives par espace de noms, séparément des paramètres du driver pour qu'un
réglage d'affichage/chargement ne renouvelle pas les identifiants. Les anciens
workspaces utilisent le chargement à la demande par défaut.

| Préréglage | Ce que demande explicitement `Preload this scope` |
|---|---|
| Minimal | Noms des objets du périmètre choisi |
| Structure | Noms, descriptions, index et clés sortantes pris en charge |
| Étendu | Structure et définitions d'objets prises en charge |

Changer le réglage enregistre la préférence sans lancer de lecture en masse.
Développer l'arbre reste une demande d'un niveau. Ouvrir une facette, utiliser
la complétion, questionner l'assistant ou ouvrir une carte demande explicitement
les besoins bornés de ce consommateur, même au-delà du préréglage. L'interface
explique cette distinction : un préréglage ne promet jamais qu'un type de
métadonnées ne sera pas lu.

L’hôte réutilise les chemins natifs et les périmètres de facettes typés.
`MetadataLevel::preload_facets` définit les préréglages ; chaque consommateur
nomme les facettes nécessaires. Les demandes traversent les commandes de
rafraîchissement ; le préchargement est admis et audité comme `PreloadMetadata`,
avec le même chemin de lecture/publication de l’exécuteur. Une demande visible de l'utilisateur précède le
préchargement en attente. Une génération remplacée est annulée ; son résultat
ne peut être publié après la fin de la génération de connexion/objet dont elle
dépend. L'ordonnancement conserve l'attribution de l'acteur et de l'audit.

Les budgets existants de catalogue, définition, complétion et assistant restent
les plafonds. Une tâche distingue facettes réussies, indisponibles, échouées et
non lues ; elle ne boucle pas pour contourner l'éviction et ne promet pas un
catalogue complet en mémoire. Aucun seuil automatique de nombre d'objets,
indexation de fond illimitée ou exception SQLite n'est introduit. Précharger
une description déduite d'échantillons suit l'annonce explicite d'accès aux
données de la proposition de carte des schémas.

### Persistance et frontière d’exécution

Un `CatalogGenerationId` transitoire est renouvelé lors du relistage, de
l’invalidation, de l’oubli et de l’éviction. Il n’est pas persisté comme contenu.
La publication compare le jeton sous le verrou de publication de la source ;
éviction de budget et application constituent ensuite une mise à jour vérifiée.
Une lecture étroite conserve un résumé observé : l’éviction n’impose ni lecture
de colonnes ni type d’objet deviné.

Avant publication, un compteur JSON en flux refuse les patches dépassant
16 Mio sans allouer une seconde copie sérialisée. Les types logiques imbriqués
sont bornés avant encodage. Cela borne la publication, pas le plan du serveur ni
tous les buffers de réception des protocoles. Serde et Serde JSON déjà présents
dans le workspace deviennent des dépendances directes de l’exécuteur pour ce
comptage ; aucun nouveau paquet externe.

La politique de connexion a un défaut Minimal et des dérogations exactes par
catalogue/espace de noms. La migration 21 stocke un JSON lisible versionné dans
`metadata_loading_policies`, lié à la connexion existante, séparément de
l’authentification et de l’affichage. Sans ligne, la révision vaut zéro ; les
écritures comparent la révision observée et utilisent un compteur monotone exact
en JavaScript. Maximum : 32 dérogations distinctes, 1024 octets UTF-8 par segment
d’identifiant et 128 Kio encodés. Ce sont des gardes d’entrée, pas des seuils de
déclenchement.

`ReadMetadataPolicy`, `WriteMetadataPolicy` et `PreloadMetadata` sont des commandes
typées réservées à l’humain. Un préchargement nomme une session propriétaire
ouverte et un périmètre exact ; il vérifie la révision avant toute lecture.
Il n’ouvre aucune connexion, n’attache aucun fichier et ne change aucun contexte.
Au plus huit tâches et 256 lectures de métadonnées en attente sont admises.
Chaque page explicite a une échéance de 60 secondes et décrit au plus
128 relations, en distinguant travail restant,
indisponible et échoué, sans contourner le budget existant de 1024 périmètres /
50000 objets. Modifier la politique annule les préchargements sans fermer les
éditeurs. Les demandes interactives passent avant les lectures anticipées en
attente ; la publication refuse les générations invalidées. La continuation
utilise un curseur explicite lié à la génération exacte de publication ; relister,
invalider ou évincer le retire. Un identifiant dépassant la borne rend le curseur
indisponible, sans troncature. Les détails d’échec sont limités à 16 entrées et
des identités bornées, sans diagnostic brut du fournisseur. Les compteurs de
périmètres soumis incluent attentes et annulations ; un périmètre peut appeler
plusieurs méthodes du fournisseur. Les octets JSON encodés ne mesurent pas le réseau.

`INFERRED_SCHEMA` déclare déjà les descriptions issues d’échantillons. Demande
ordinaire, complétion, assistant et presets refusent cette lecture implicite.
Un périmètre explicite `SampledDescription`, humain seul, annonce la borne fixe
du driver et ne renvoie que les champs déduits, jamais les valeurs. Index et
définitions restent indépendants. L’ancien agrégat `Relation` ne contourne pas
cette information. Le fournisseur expose `schema_sample_limits`, sans I/O,
capturé à l’ouverture uniquement si `INFERRED_SCHEMA` est déclaré. MongoDB
annonce ses bornes effectives : 1000 documents retournés, 8 Mio retenus et
10 secondes de traitement serveur. Les lots réseau et le plan serveur peuvent
lire davantage ; le nettoyage a une durée distincte. Un premier document trop
grand échoue avant sa copie.

L’explorateur expose `Metadata loading…` sur la source active. Le dialogue
propose le défaut de connexion, une dérogation exacte d’espace de noms observé,
la relecture des préférences et des actions distinctes de préchargement,
continuation et annulation. Choisir un périmètre ne développe pas ses voisins.
Un enregistrement périmé exige une relecture ; fermer la source annule sa seule
commande. L’UI montre progression et couverture partielle, y compris les
facettes indisponibles. L’UI sépare préférence et préchargement. Enregistrer ne précharge rien. Le DDL
adjacent attend une demande explicite ; Structure ne charge plus sa définition
par effet de bord. Chaque consommateur demande description, index ou clés
sortantes selon son besoin ; disponibilité et fraîcheur restent indépendantes,
y compris après éviction ou échec partiel.


Changer le contexte d’une console annule son seul préchargement. Sa génération
est retirée avant l’appel au fournisseur puis à son terme, même en cas d’erreur :
une lecture lancée pendant ce changement ne publie pas dans le contexte final.
Les autres propriétaires de la connexion restent intacts. La continuation
vérifie aussi l’espace de noms exact et la révision de politique examinée.

La couverture finale est mesurée sur le cache après la page, séparément des
lectures réussies. Les compteurs de facettes fraîches/périmées/non lues restent
`null` avant mesure, jamais un faux zéro. Une éviction rend le résultat partiel
même si tous les appels ont réussi ; aucune boucle ne recharge les données
évincées. Les octets comptent les publications encodées, ni l’occupation actuelle
du cache ni le réseau. Une génération retirée avant le bilan final arrête la
tâche au lieu d’annoncer complète une couverture périmée.

### Préparation des aperçus dépendant de la structure

`Session::plan_preview` reçoit une description fraîche optionnelle du cache de
la session propriétaire exacte. Le défaut conserve les parcours natifs des
moteurs sans inférence et refuse les adaptateurs inférant des champs tant qu’ils
n’implémentent pas un planificateur sans inférence implicite. L’indication ne
vient pas du webview et n’autorise aucun échantillon supplémentaire. Aucun
second cache de métadonnées n’est créé dans le driver.

MongoDB réutilise cette description explicite pour valider tris, projections et
pages indépendantes. Absente ou périmée, elle fait échouer la préparation avec
une invitation à lire Structure explicitement ; aucun document supplémentaire
n’est échantillonné. L’aperçu simple utilise le type d’objet déclaré, sans
échantillon de schéma. Déduire les types Arrow des lignes réellement demandées
reste distinct d’une lecture exploratoire supplémentaire. La même règle protège
la projection d’un échantillon de lignes approuvé : aucune lecture silencieuse
de documents entiers pour découvrir leurs champs.

## Conséquences

* **+** Index et relations cessent d'être le coût implicite de toute lecture de colonnes.
* **+** Les réglages ont le même sens dans l'arbre, l'éditeur, la carte et l'assistant.
* **−** Davantage d'états de facettes traversent cache, IPC, interface et invalidation.
* **−** Des lectures séparées ne promettent pas un instantané unique de la base ;
  fraîcheur et erreurs restent visibles. Le mode Étendu peut finir partiellement sous les budgets.

**Coût de sortie :** réunir les facettes dans l'agrégat, migrer les préférences
et simplifier chaque consommateur. Les méthodes des providers restent utilisables ;
aucun catalogue sur disque ni nouveau protocole serveur n'est introduit.

**À reconsidérer si** les mesures montrent que les allers-retours supplémentaires
coûtent plus que les lectures évitées, ou si des usages répétés demandent un autre
déclencheur. Tout nouveau déclencheur explicite périmètre et annulation ; élargir
un budget nécessite une décision mesurée distincte.

## Alternatives rejetées

| Alternative | Motif du rejet |
|---|---|
| L1/L2/L3 uniquement dans l'arbre | Les autres consommateurs continueraient silencieusement le travail supposé désactivé |
| Interdire toute lecture explicite au-delà du niveau choisi | Fait échouer Structure ou DDL même lorsque l'utilisateur les demande |
| Garder une seule fraîcheur pour tous les détails | Une lecture de colonnes réussie cacherait les index/clés manquants ou périmés |
| Tout charger pour les fichiers locaux | La localité ne démontre pas un nombre d'objets, contenu ou coût de rendu borné |
| Copier les seuils automatiques de l'issue | Aucune mesure ne les justifie pour les drivers et consommateurs d'Oxyn |
