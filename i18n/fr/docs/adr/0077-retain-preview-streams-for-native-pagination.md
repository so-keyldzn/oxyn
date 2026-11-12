<!-- oxyn-translation source="docs/adr/0077-retain-preview-streams-for-native-pagination.md" sha256="c1323597ca7b" -->
# ADR-0077 — Conserver les flux d’aperçu pour la pagination native

**Statut :** accepté · **Date :** 2026-11-12

**Précise :** ADR-0028, ADR-0060, ADR-0061, ADR-0062 et ADR-0068 : les requêtes indépendantes sans ordre total restent limitées à la première page ; une exécution conservée peut être parcourue sans inventer une clé unique.

## Contexte

Data propose des pages de 50, 100 ou 200 lignes. Les clés de tri ClickHouse ne sont pas uniques ; Elasticsearch/OpenSearch et les collections Redis exposent des continuations de protocole plutôt que des offsets SQL. Réexécuter une requête sans ordre pour chaque page répéterait ou omettrait des enregistrements. Garder toute une collection dans la webview violerait le budget mémoire.

## Décision

`PreviewRelation` reste la commande et la frontière de politique et d’audit. `Session::prepare_preview` renvoie par défaut la requête bornée existante ; un driver peut déclarer une requête conservée. `Session::execute_preview` utilise par défaut l’exécution ordinaire. L’ordonnanceur reclassifie toujours la requête composée, impose la lecture seule, utilise les événements d’exécution habituels et draine une page bornée dans `ResultBuffer`.

Une continuation nomme le `ResultId` opaque d’une page précédente. L’ordonnanceur le lie à la connexion, la session, la relation, la taille, le tri, le filtre et la projection. Il refuse une forme modifiée, une identité expirée et un offset au-delà des lignes déjà reçues. La page suivante avance le même curseur ; les précédentes lisent ses lots Arrow conservés. Chaque résultat exposé contient une seule page. Une page pleine autorise la suivante, y compris la page vide après une frontière exacte. Aucun total n’est demandé.

L’historique utilise 8 Mio de mémoire et au plus 128 Mio de débordement Arrow par flux, au plus 8192 lots et 16 identités de pages conservées. Atteindre une borne produit une erreur explicite, jamais une fausse fin des données. Un flux vit au plus cinq minutes. Une tâche de nettoyage détenue annule son jeton et détruit son curseur à expiration ; libérer sa dernière page, fermer sa session, se déconnecter ou abandonner une page libère aussi le flux. Une future de curseur interrompue n’est jamais reprise.

ClickHouse conserve un résultat HTTP avec rétropression, sans OFFSET ni promesse universelle d’instantané. Elasticsearch/OpenSearch conservent un PIT et utilisent `search_after`, comptage désactivé. Elasticsearch ajoute `_shard_doc` pour départager les égalités ; OpenSearch 3.3 et les versions suivantes ajoutent `_index` puis `_shard_doc`, dont le numéro de shard est local à l’index. L’ancien OpenSearch parcourt un index/shard concret à la fois, dans l’ordre `_doc` d’un PIT restreint par la préférence `_shards` et un filtre `_index`. `_search_shards` énumère au plus 128 partitions ; les filtres et routages d’alias restent attachés à la cible originale. Un seul PIT de partition est détenu à la fois ; les partitions ouvertes ensuite peuvent voir des données plus récentes. Sur ces anciennes versions, le tri conserve l’aperçu borné de la première page ; effacer le tri réactive le parcours natif. OpenSearch 2.0–2.3 ne dispose pas de PIT et conserve son aperçu borné existant sans pagination. Cela évite de supposer `_id` unique avec un routage personnalisé, ou de charger ses fielddata. Les réponses de création de PIT et les requêtes de nettoyage ont des tâches détenues, même si la livraison sur le canal n’est pas reçue ; les identités renouvelées sont conservées avant le décodage des résultats. Les identités serveur ne traversent jamais l’IPC. Redis parcourt listes, ensembles triés, hashes, ensembles, streams, ensembles vectoriels et séries temporelles avec leurs opérations natives bornées. Les identités répétées par SCAN sont supprimées dans un budget en octets ; les restes de réponses et les champs des entrées de stream ne sont pas jetés à une frontière de page. Les modifications Redis concurrentes ne constituent pas un instantané.

L’interface transmet une continuation uniquement pour naviguer. Actualiser, modifier filtre, tri ou taille, et se reconnecter repartent à la première page. Elle conserve l’ancien résultat jusqu’à la fin de la nouvelle lecture, puis le libère. La barre explique que les pages appartiennent à une lecture conservée et qu’actualiser en démarre une autre.

## Conséquences

* **+** La même barre parcourt tous les protocoles implémentés qui renvoient des collections.
* **+** Aucune clé inventée ni matérialisation complète côté client.
* **−** La lecture détient une ressource distante jusqu’à libération ou expiration ; un grand tri peut toujours coûter un tri complet au serveur.
* **−** Une expiration ou un quota impose d’actualiser ou d’utiliser une console. L’ancien OpenSearch ouvre successivement ses partitions et ne promet ni instantané unique entre shards ni pagination sous un tri global.
* **−** Des mutations Redis pendant le parcours peuvent affecter les lignes non encore lues ; les pages retournées restent celles déjà lues.

**Coût de sortie :** remplacer l’adaptateur de flux conservé et les méthodes de préparation des drivers ; l’interface et le contrat de résultat d’une page bornée restent identiques.

**À reconsidérer si** toutes les variantes de recherche supportées offrent le même départage PIT, ou si les mesures montrent que les ressources HTTP/recherche détenues dépassent les bornes de cinq minutes et seize entrées.

## Alternatives rejetées

| Alternative | Motif |
|---|---|
| OFFSET sur une clé non unique | Aucune séquence stable entre lectures indépendantes |
| Charger toute la source avant d’afficher les pages | Latence non bornée et travail serveur inutile |
| Transmettre les curseurs serveur à la webview | Expose l’état de protocole et rend propriété et nettoyage ambigus |
| Supposer `_shard_doc` dans tous les OpenSearch | Les anciennes variantes supportées n’offrent pas cette garantie |
| Scroll | Les deux moteurs imposent un comptage exact dans ce contexte |
| PIT trié uniquement par `_index`, `_id` | Le routage personnalisé peut créer le même ID sur plusieurs shards d’un index |
