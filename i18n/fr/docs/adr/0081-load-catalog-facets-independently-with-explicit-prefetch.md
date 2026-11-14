<!-- oxyn-translation source="docs/adr/0081-load-catalog-facets-independently-with-explicit-prefetch.md" sha256="4e767ce879e2" -->

# ADR-0081 — Charger les facettes indépendamment et rendre le préchargement explicite

**Statut :** proposé · **Date :** 2026-11-13

**Précise :** [ADR-0018](0018-apercu-ddl.md),
[ADR-0022](0022-rafraichissement-automatique.md),
[ADR-0036](0036-l-assistant-complete-le-catalogue.md) et
[ADR-0057](0057-editor-completion-reads-the-catalog-cache.md), sur la granularité
des métadonnées demandées. Cette proposition ne change aucun budget actuel ni
comportement livré.

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

L'hôte assemble demandes et préchargements à partir d'une même politique pure
du catalogue ; les lectures traversent toujours `RefreshCatalogScope` et
la porte de politique. Une demande visible de l'utilisateur précède le
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
