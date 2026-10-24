<!-- oxyn-translation source="docs/adr/0062-elasticsearch-opensearch-rest-json.md" sha256="e49c278bb6fa" -->

> Traduction française de [docs/adr/0062-elasticsearch-opensearch-rest-json.md](../../../../docs/adr/0062-elasticsearch-opensearch-rest-json.md). **La version anglaise fait foi.**

# ADR-0062 — Elasticsearch et OpenSearch utilisent un pilote REST/JSON borné

**Statut :** proposé · **Date :** 2026-10-22

## Contexte

Elasticsearch et OpenSearch exposent la même famille REST/JSON, et l'ADR-0003 interdit une crate par produit. Le pilote ne doit pas présenter une réponse de recherche comme une table relationnelle : un mapping est dynamique, un alias peut combiner des mappings incompatibles et un champ peut être absent ou multivalué. Les pages sont bornées avant le décodage JSON : les méthodes pratiques de `reqwest` collectent le corps entier.

Le workspace épingle déjà `reqwest` 0.13.4 avec Rustls sur aws-lc-rs ; la vérification est dans [RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md#elasticsearch-and-opensearch--checked-on-2026-10-22). Elastic documente `search_after` et un point dans le temps comme alternative à `from`/`size`, limité par défaut à 10 000 hits. OpenSearch documente `POST /_tasks/<task_id>/_cancel` et précise qu'une tâche doit être annulable.

## Décision

`oxyn-driver-elasticsearch` utilise directement le client `reqwest` déjà présent, avec une petite couche REST typée plutôt qu'un client propre à un produit. Il ouvre une URL, désactive proxy et redirections à cette frontière de secrets, puis découvre produit, distribution et version avec `GET /` ; OpenSearch n'est jamais déduit de l'URL. Basic, clé API et bearer viennent de `Credentials`, jamais d'une URL persistée ou d'un log.

Le plancher initial est Elasticsearch 8.19 et 9.x, et OpenSearch 2.10 et 3.x. Elasticsearch 9 est testé en CI avec l'image exacte 9.5.5 ; OpenSearch 3.0.0 est exercé localement avant livraison tant qu'un second service CI n'est pas approuvé. Un serveur sous ce plancher est refusé après `GET /`.

Le catalogue est plat : index, alias et flux de données occupent le niveau de relation et le mapping produit les champs. Les multi-fields `text`, propriétés `nested` et `object`, champs d'exécution, `dense_vector`, `geo_point`, `date`, `scaled_float`, `unsigned_long` et types inconnus gardent leur type serveur dans `Field::raw_type`. Un mapping incompatible devient inconnu, jamais choisi en silence.

La console accepte exactement une requête de type Kibana : méthode HTTP en majuscules et chemin API absolu sur la première ligne, puis un corps JSON facultatif. Une réponse arbitraire va dans une colonne Arrow `json`; une réponse `_search` devient des lignes `_index`, `_id`, `_score` et clés `_source`, chaque valeur JSON étant encodée sans perte en UTF-8 JSON. Cela préserve `unsigned_long` et les nombres JSON sans passage par `f64`.

Chaque page est lue par morceaux et plafonnée à un MiB avant décodage. Un aperçu d'index ouvre un point dans le temps, utilise `search_after` avec `_shard_doc`, et ferme ce point à la fin ou à l'annulation ; il n'utilise ni pagination profonde `from`/`size` ni scroll. Une source désactivée, un champ absent ou des valeurs incompatibles donnent une erreur explicite.

Une requête est classée par son **endpoint**, puis par son verbe, car les verbes mentent dans les deux sens : `POST /logs/_search` lit, `GET /_refresh` modifie l'index. L'endpoint est le premier segment du chemin qui commence par `_` (un nom d'index, d'alias ou de flux de données ne le peut pas), après décodage des pourcentages, puisque le serveur route `%5Fdelete_by_query` vers `_delete_by_query` ; les préfixes `_plugins/_x` et `_opendistro/_x` d'OpenSearch nomment l'API qui les suit. Les tables sont dans `classify.rs` :

* lectures quel que soit le verbe : recherches, `_count`, `_msearch`, `_mget`, `_explain`, `_validate`, `_analyze`, `_field_caps`, vecteurs de termes, `_sql` et `_query` d'ES|QL (deux langages en lecture seule), ouverture et fermeture de point dans le temps et de scroll, toute `_simulate` de modèles et de pipelines, `_cluster/allocation/explain`, `_has_privileges`, `_authenticate` ; un `GET` ou `HEAD` sur une API connue de supervision ou de structure (`_cat`, `_cluster`, `_nodes`, `_tasks`, `_mapping`, `_settings`, modèles, flux de données, ILM, ISM, pipelines, scripts, snapshots, sécurité) ;
* écritures : documents, `_bulk`, `_update`, opérations par requête, `_reindex`, `_tasks/…/_cancel`, exécution d'une politique enrich ;
* DDL : création et suppression d'index, tout appel non lecture sur une API de structure, et les opérations d'index quel que soit leur verbe (`_open`, `_close`, `_refresh`, `_forcemerge`, `_rollover`, `_shrink`…), restauration de snapshot comprise ;
* privilèges : tout appel non lecture sur `_security` ou `_plugins/_security` ;
* le SQL d'OpenSearch n'est une lecture que si son instruction commence par `SELECT`, `SHOW` ou `DESCRIBE` : il peut supprimer quand `plugins.sql.delete.enabled` est activé. PPL reste `Unknown` tant que sa surface en lecture seule n'est pas vérifiée ;
* tout le reste — un endpoint absent des tables, un verbe qu'un endpoint connu ne prend pas, un chemin qui ne se décode pas — est `Unknown`, donc mutant, **`GET` compris**.

Un script dans un corps de recherche est du code, mais les scripts de recherche s'exécutent isolés sur les doc values et ne peuvent modifier aucun document : la recherche reste une lecture. Un script dans `_update_by_query` ou `_update` est de toute façon une écriture.

Le risque (`Session::classify_risk`, ADR-0066) est donné après une écriture, un DDL ou un privilège prouvés, de façon prudente : `DropObject` pour un `DELETE` d'index ou de motif, et pour tout `DELETE` sur une API de structure (flux de données, modèle, pipeline, politique de cycle de vie, script, snapshot, alias) ; `UnboundedDelete` / `UnboundedUpdate` pour une requête par requête dont la requête peut tout couvrir (absente, `match_all`, un `bool` dont les clauses requises couvrent tout et qui n'exclut rien, `q=*`, une forme illisible) ; `DropObject` pour `_close`, la variante existante la plus proche — un index fermé refuse toute lecture et écriture jusqu'à sa réouverture. Aucune variante de `MutationRisk` n'est ajoutée ; le texte de raison partagé est formulé pour SQL (« DROP: the object and its data are removed »), exact pour un index supprimé, approché pour un index fermé.

Le serveur n'a pas de mode lecture seule à l'échelle de la session : Oxyn refuse une requête classée mutante sous cette borne et ne déclare pas `READ_ONLY_SESSION`.

Chaque requête en cours a un `X-Opaque-Id`. Le pilote retrouve cette tâche par l'API Tasks et n'annule que celles que le serveur déclare annulables ; une lecture n'est `Cancelled` qu'après preuve. Une écriture arrêtée après envoi est `OutcomeUnknown`. HTTP 429 et 503 ne sont transitoires que pour une lecture ; timeout, perte de transport ou 5xx pendant une mutation sont ambigus. Une réponse bulk 200 contenant des échecs reste une erreur.

## Conséquences

* **+** Une petite couche client sert les deux produits et garde le contrôle des bornes, de l'annulation et de la rédaction.
* **+** La console expose l'API native sans faire croire que Search DSL est SQL.
* **−** Les valeurs document sont du texte JSON dans la première tranche ; la reconstruction Arrow riche et la complétion restent des suivis.
* **−** Une seule URL exclut sniffing, identifiants Elastic Cloud, CA personnalisées et certificats client de cette première tranche.
* **−** Une recherche de tâche échouée ne prouve pas l'annulation ; le résultat reste ambigu.

**Coût de sortie :** remplacer la couche REST modifie authentification, assainissement d'erreurs, corrélation d'annulation et tests de mapping ; les traits publics et le `DriverId` persisté restent inchangés.

**Réexaminer si** un client Rust maintenu et neutre expose streaming borné, annulation `X-Opaque-Id` et les deux produits sans seconde pile TLS, ou si le plancher ne propose plus PIT avec `search_after`.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Crate officielle `elasticsearch` | Elle cible la surface générée d'Elastic, pas une couche neutre, et ajoute une abstraction inutile ici. |
| Crate OpenSearch séparée | Contredit l'ADR-0003 : les différences sont des capacités de session, non un second décodeur. |
| `from`/`size` ou scroll | La pagination profonde est limitée par `index.max_result_window` et scroll n'est pas le mécanisme retenu. |
| `Response::json` | Un grand `_source` s'allouerait sans borne avant l'application de I-06. |
