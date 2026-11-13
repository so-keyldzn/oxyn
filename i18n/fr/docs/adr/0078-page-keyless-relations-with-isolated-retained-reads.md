<!-- oxyn-translation source="docs/adr/0078-page-keyless-relations-with-isolated-retained-reads.md" sha256="2a480e7af391" -->

> Traduction française de [l’ADR-0078](../../../../docs/adr/0078-page-keyless-relations-with-isolated-retained-reads.md). **La version anglaise fait foi.**

# ADR-0078 — Paginer les relations sans clé par des lectures conservées isolées

**Statut :** accepté · **Date :** 2026-11-13

**Précise :** [ADR-0069](0069-turso-driver-hrana-over-http.md), [ADR-0068](0068-a-preview-comes-in-key-order.md) et [ADR-0077](0077-retain-preview-streams-for-native-pagination.md). L’absence de clé fiable ne limite plus une relation interrogeable à une page si son driver peut conserver une lecture bornée.

## Contexte

Les vues atteignent déjà Data, mais PostgreSQL, MySQL, SQLite, DuckDB, Turso et MongoDB préparent encore des lectures indépendantes limitées. Vues ordinaires ou matérialisées, tables sans clé et vues/séries temporelles MongoDB ne peuvent donc pas avancer. Une clé déclarée ne suffit pas si NULL, l’héritage ou des contraintes informatives invalident l’unicité.

Conserver un curseur SQL sur la session du catalogue bloque son worker, son mutex ou son pool. Sonder l’état transactionnel après une page peut aussi attendre derrière ce curseur. Rouvrir depuis les derniers réglages enregistrés est dangereux : ceux-ci peuvent avoir changé alors que la session vivante vise toujours l’ancienne base.

La réponse pipeline Turso est un document JSON entier. Hrana 3 définit aussi une réponse de curseur délimitée par des sauts de ligne ; son protocole et son nettoyage sont sourcés dans [RESEARCH-NOTES](../RESEARCH-NOTES.md).

## Décision

`PreviewRelation` reste la frontière de politique, d’audit et d’annulation. Le front conserve ses tailles de pages et continuations opaques de résultats. `PreviewShape::continuable` vaut false par défaut : les échantillons gardent des requêtes indépendantes bornées. Data active la continuation. `Relation::preview_key` prouve l’unicité réelle séparément de la garantie de coût `key_order_preview` ; seules les clés prouvées permettent des pages indépendantes. Les autres relations de lignes utilisent une lecture conservée, y compris pour les tris comportant des égalités. Aucune clé artificielle, requête COUNT ou OFFSET non ordonné n’est ajouté.

La préparation distingue l’exécution bornée, l’exécution conservée sur un curseur protocolaire indépendant et l’exécution conservée sur une session dédiée. Cette dernière passe par `Session::open_preview_session` : cible, identifiants, options d’accès et contexte de noms viennent de la session vivante, pas de réglages modifiés. Le driver refuse un espace de noms local qu’il ne peut reproduire ; il ne substitue jamais une base ou un objet homonyme. Les routines à arguments ou à effets restent hors des aperçus automatiques.

L’ordonnanceur réserve une place avant l’ouverture : au plus 16 lectures actives ou en ouverture. Les sessions dédiées sont privées, ni consoles ni sessions de repli du catalogue. Les continuations gardent leur propriétaire logique ; l’annulation cible la session réelle. Aucun sondage transactionnel ne suit une page conservée en lecture seule ; les garanties ordinaires de la console restent identiques.

Une tâche détenue par le registre garde la session pendant l’ouverture et la fermeture. Elle détruit le curseur avant d’attendre la fermeture, conserve la capacité jusqu’au nettoyage et est rejointe à la fermeture de la session/connexion. EOF libère la ressource distante tout en gardant les pages déjà lues. Erreur, abandon, libération du dernier résultat, expiration et déconnexion annulent et ferment. La durée de cinq minutes et les budgets Arrow existants restent en vigueur. Les écritures autorisées libèrent les aperçus conservés de leur connexion avant exécution pour qu’une lecture SQLite inactive ne bloque pas l’écriture d’Oxyn. Un écrivain externe peut encore rencontrer les verrous SQLite : aucun changement implicite de WAL ou d’isolation.

Turso utilise le cadrage Hrana 3, avec lignes et lots bornés en octets, schéma Arrow stable et erreur explicite pour une valeur ultérieure incompatible. Les batons restent en Rust et sont nettoyés sous délai. Les identifiants restent sur l’origine configurée, avec une exception explicite : un endpoint HTTPS Turso Cloud peut diriger vers un autre hôte HTTPS au port 443 sous `.turso.io`. Les endpoints personnalisés ne changent pas d’origine ; identifiants, paramètres et fragments dans une URL de routage sont refusés. Les redirections HTTP restent désactivées. Le pipeline borné reste disponible pour les requêtes ordinaires.

La disponibilité de pagination appartient à la session et au résultat affichés. Une réponse périmée ne s’applique pas aux lignes de remplacement ; les erreurs sont visibles et leur reprise explicite. Actualisation et changement de forme repartent au début ; Previous lit l’historique. L’interface dit que la lecture reste ouverte entre les pages, sans promettre un instantané universel.

## Couverture des objets

| Driver | Sources de lignes parcourables | Limites et objets sans lignes |
|---|---|---|
| Protocole PostgreSQL | Tables, tables partitionnées/étrangères, vues, vues matérialisées et relations système ; lecture conservée si unicité/coût non prouvé, dont héritage et clés informatives Redshift | Les espaces temporaires locaux ne sont pas reproductibles |
| Protocole MySQL, dont MariaDB | Tables, vues et relations système ; clés prouvées indépendantes, sinon curseur privé conservé | Pas d’appel automatique de routine |
| SQLite | Tables, vues, tables virtuelles chargées et relations de schéma ; curseur privé si clé nullable/non prouvée | Fichiers attachés rouverts en lecture seule ; état temporaire et mémoire attachée limités à un aperçu borné |
| DuckDB | Tables, vues, catalogues attachés et vraies vues `system` | État temporaire utilisateur borné ; vues internes natives reproductibles |
| Turso/libSQL | Tables et vues Hrana 3 via `/v3/cursor` | Ancien pipeline borné ; pages indépendantes seulement pour un alias rowid prouvé |
| MongoDB | Collections, vues, même sans `_id` ou avec doublons, et séries temporelles | Un curseur natif, sans clé unique inventée |
| ClickHouse | Tables, vues, vues matérialisées, dictionnaires et relations système | Curseur existant, sans prétendre que la clé de tri est unique |
| Elasticsearch/OpenSearch | Index, alias et flux de données | Restrictions PIT/version/ordre natif existantes explicites |
| Protocole Redis | Hashes, sets, listes, ensembles triés, streams, ensembles vectoriels, séries temporelles et membres TopK | Chaînes, JSON et résumés Bloom/cuckoo/count-min/t-digest : `single_page_preview`, sans pagination |

Fonctions, procédures, séquences et autres objets de métadonnées gardent leurs vues de métadonnées. Console, historique et assistant paginent leur tampon existant sans réexécution. Une continuation indisponible ne présente que la limite d’aperçu et sa raison ; `Pagination::NotApplicable` masque la barre des valeurs/résumés.

## Conséquences

* **+** Les objets à lignes partagent la navigation sans inventer d’unicité ni répéter du SQL arbitraire.
* **+** Catalogue et onglets indépendants n’attendent pas derrière un curseur suspendu.
* **+** Des réglages modifiés ne redirigent pas silencieusement un aperçu vivant.
* **−** Les lectures conservées consomment des connexions/curseurs, instantanés et parfois verrous, en quantité bornée.
* **−** Quota, expiration ou écriture peuvent demander une actualisation. L’état local non reproductible reste une limite explicite.
* **−** Un tri demandé peut toujours parcourir toute la source avant les premières lignes.

**Coût de sortie :** supprimer le mode isolé et son propriétaire de nettoyage, remplacer les composeurs/transports conservés. La pagination IPC et les formats du workspace ne changent pas.

**À reconsidérer si** les mesures montrent un coût en connexions/verrous excessif, ou si les moteurs exposent une continuation libérant sûrement leur connexion entre les pages.

## Alternatives rejetées

| Alternative | Raison |
|---|---|
| OFFSET sans ordre sur les vues | Les exécutions indépendantes sautent ou répètent des lignes |
| Trier toutes les colonnes | Les doublons restent égaux et ouvrir peut trier toute la relation |
| Conserver le curseur sur la session du catalogue | Bloque métadonnées, actualisation ou état transactionnel |
| Annuler lors de l’ouverture d’un nœud du catalogue | Une exploration indépendante détruit la navigation |
| Rouvrir depuis les réglages actuels | Peut lire une autre cible sous l’ancien catalogue |
| Matérialiser avant la première page | Travail non demandé, contraire au streaming borné |
| Appeler automatiquement les routines | Arguments et effets demandent une action distincte |
