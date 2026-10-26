<!-- oxyn-translation source="docs/adr/0068-a-preview-comes-in-key-order.md" sha256="f70de72863c8" -->

> Traduction française de [docs/adr/0068-a-preview-comes-in-key-order.md](../../../../docs/adr/0068-a-preview-comes-in-key-order.md). **La version anglaise fait foi.**

# ADR-0068 — Un aperçu arrive dans l'ordre de la clé, partout où le driver se porte garant que cet ordre est une lecture bornée

**Statut :** accepté · **Date :** 2026-10-26

**Remplace :** [ADR-0028](0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md),
sur ses points de décision 1 (« aucun ordre n'est imposé ») et 2 (« aucune page
n'est offerte tant que l'ordre n'est pas total »), pour une relation qui a une
clé primaire et dont le driver se porte garant. Il **rétablit** le remède
qu'[ADR-0020](0020-apercu-trie-filtre-parcouru.md) proposait à l'origine :
« sans tri demandé, le driver ordonne par la clé primaire seule ». ADR-0028
reste la référence pour le reste : aucune page sans ordre total
(`NoUniqueKey`), et l'ordre vérifié contre la forme dont viennent les lignes
affichées.

**Précise :** [ADR-0063](0063-mongodb-driver-database-commands-in-extended-json.md),
sur une phrase : l'aperçu d'une collection ordinaire n'est plus « une commande
`find` … sans ordre imposé » — il est trié sur `_id` quand aucun tri n'est
demandé.

## Contexte

Le 2026-10-26, l'utilisateur a ouvert une table SQLite, vu 200 lignes et aucun
contrôle de page, et demandé pourquoi il n'y en avait pas. C'est, mot pour mot,
la deuxième condition de réexamen d'ADR-0028 : « des utilisateurs signalent que
l'absence de contrôle de page sur un aperçu non trié se lit comme un défaut
plutôt que comme une règle ».

ADR-0028 avait gardé l'argument d'ADR-0020 — un `OFFSET` sur un ordre non
garanti duplique et omet des lignes en silence — et rejeté son remède pour son
coût : « une lecture de métadonnées à chaque aperçu, pour découvrir la clé ».
Ce coût est réel, et il n'est pas le même partout :

| Moteur | Allers-retours de métadonnées par aperçu, avant → après |
|---|---|
| SQLite | un `PRAGMA table_info` local, sans serveur |
| PostgreSQL | aperçu simple 2 → 2 (la lecture des colonnes typées porte désormais aussi la clé dans l'ordre de l'index, `relkind` et `relhassubclass`) ; trié ou paginé 6 → 2 ; Redshift inchangé |
| MySQL/MariaDB | aperçu simple 0 → 2 (`describe_relation` : l'en-tête, puis les colonnes, chacun en prepare/execute/close) ; compté dans le code, non mesuré sur un serveur réel |

La clé seule ne rend pas l'ordre bon marché. `ORDER BY clé LIMIT n` n'est une
lecture bornée que là où le moteur sait parcourir la clé dans l'ordre. Une table
`vec0` de SQLite déclare sa colonne de clé comme clé primaire, et pourtant
aucun index ne sert son ordre : SQLite lit chaque ligne, vecteurs compris, dans
un b-tree temporaire avant que la première ne sorte
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ordre-par-défaut-dun-aperçu--vérifié-le-2026-10-26)).
Ouvrir une telle table pour y jeter un œil ne doit pas payer cela.

Une clé composite ne rend pas non plus tout ordre de colonnes bon marché. Sur
PostgreSQL 17.11, avec `PRIMARY KEY (b, a)` sur une table déclarée `(a, b)`,
`ORDER BY a, b LIMIT 200` s'est exécuté en parcours séquentiel suivi d'un tri
de toute la table, tandis qu'`ORDER BY b, a` utilisait l'index
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ordre-par-défaut-dun-aperçu--vérifié-le-2026-10-26)).

## Décision

**L'utilisateur a choisi une pagination toujours disponible plutôt qu'une
première ouverture plus rapide.** Un aperçu sans tri demandé arrive dans
l'ordre de la clé primaire, et offre des pages, partout où le driver se porte
garant que cet ordre est une lecture bornée.

1. **Le driver se porte garant, dans la description de la relation.**
   `Relation::key_order_preview: bool` (`oxyn-catalog`, valeur serde par défaut
   `false`) est posé par le driver dans `describe_relation`.
   `Relation::previews_in_key_order()` est vrai quand ce drapeau est posé
   **et** qu'une clé primaire existe. Un drapeau absent vaut `false` : un driver
   qui ne dit rien ne voit jamais ses aperçus ordonnés dans son dos.

2. **Un drapeau, deux lecteurs.** Sans tri demandé, le driver compose
   `ORDER BY <clé> ASC` — chaque colonne de la clé, citée — exactement quand
   `previews_in_key_order()` est vrai, et rien sinon. Le desktop lit le même
   drapeau pour décider si une page est offerte. Il n'y a pas de bit de
   capacité : une seconde source de vérité laisserait l'ordre et le contrôle de
   page se contredire, ce qui est la panne silencieuse elle-même. L'onglet Data
   charge la description de la relation une fois, à son ouverture ; tant
   qu'elle n'est pas lue, ni avis ni paginateur ne s'affichent.

   **Les colonnes de la clé viennent dans l'ordre de l'index, pas dans celui de
   la table.** `Field::primary_key_rank` (`oxyn-catalog`) porte le rang donné
   par le moteur, et `Relation::primary_key()` ordonne d'abord par lui : SQLite
   d'après la colonne `pk` de `PRAGMA table_info`, MySQL/MariaDB d'après
   `SEQ_IN_INDEX` de l'index `PRIMARY` dans `information_schema.STATISTICS`,
   PostgreSQL d'après l'index lui-même (`unnest(indkey) WITH ORDINALITY`).

3. **Qui se porte garant, et qui non.**

   | Driver | Garant pour | Pourquoi, et ce dont il ne se porte pas garant |
   |---|---|---|
   | SQLite | toutes les tables sauf les tables virtuelles (`PRAGMA table_list` de type `virtual`) | la clé est l'ordre du b-tree, ou celui d'un index unique ; une table virtuelle (`vec0`…) ne s'ordonne qu'en la lisant entière dans un b-tree temporaire (voir Contexte) |
   | PostgreSQL | `relkind` `r` sans enfants d'héritage (`relhassubclass` faux), et `p` (partitionnée) | jamais les vues, vues matérialisées ou tables étrangères. Une table enfant n'a ni l'index de clé primaire du parent ni son unicité (vérifié sur 17.11), si bien qu'ordonner un parent d'héritage trie chaque ligne des enfants — les hypertables TimescaleDB sont des parents d'héritage. Un `relhassubclass` illisible compte comme « a des enfants » |
   | Amazon Redshift | rien | les clés primaires sont informatives et non appliquées ([RESEARCH-NOTES](../RESEARCH-NOTES.md#ordre-par-défaut-dun-aperçu--vérifié-le-2026-10-26)) ; ordonner est un tri sur toute la table distribuée |
   | MySQL/MariaDB | `BASE TABLE` sur InnoDB, MyISAM ou Aria, chaque colonne de la clé classée dans l'index `PRIMARY` et aucune en préfixe | MEMORY hache sa clé, FEDERATED vit ailleurs ; un index `UNIQUE NOT NULL` qui tient lieu de clé primaire absente, que le serveur rapporte comme `PRI`, n'est pas garanti |
   | DuckDB | les tables de base | voir plus bas la conséquence sur son coût |
   | MongoDB | les vraies collections (type `collection`), ordonnées par `{_id: 1}` | les vues et les collections de séries temporelles n'ont pas de clé unique. L'aperçu simple ne lit que l'entrée de `listCollections`, aucun échantillon de documents |
   | ClickHouse | non, inchangé | pas de clé unique : la clé de tri peut se répéter ([ADR-0060](0060-clickhouse-driver-over-http-rowbinary.md) point 10) — `NoUniqueKey` |
   | Elasticsearch/OpenSearch | non, inchangé | pas d'`OFFSET` au-delà de la fenêtre de résultats, pas de clé unique sans curseur point-in-time ([ADR-0062](0062-elasticsearch-opensearch-rest-json.md)) |
   | Redis | non, inchangé | ni tri ni page : les clés viennent de `SCAN` |

   Une relation sans clé — une vue, une table sans clé — reste sans ordre et
   rapporte `NoUniqueKey`, sur tous les drivers.

4. **Quatre états de pagination atteignent l'interface.**
   * `unknown` — la relation n'est pas encore décrite ;
   * `needsOrder` — une clé est connue mais la relation n'est pas ordonnée par
     défaut (une table virtuelle, Redshift) : l'utilisateur trie pour paginer,
     en payant l'ordre en connaissance de cause ;
   * `noUniqueKey` — aucune clé ne peut rendre l'ordre total ;
   * `ready { previous, next, firstRow, orderedBy }` — `orderedBy` liste les
     colonnes de la clé tant que l'ordre par défaut est en vigueur, et vaut
     `null` dès que l'utilisateur a trié.

   Ce que l'interface dit dans chaque état est dans
   [UX-SPEC](../UX-SPEC.md#filtrer-trier-parcourir) ; ce qu'un driver doit est
   dans
   [DRIVER-CONTRACT §3](../DRIVER-CONTRACT.md#3-il-produit-des-recordbatch-arrow-en-flux).

## Conséquences

* **+** Une table à clé se parcourt page par page dès son ouverture, ce que
  l'utilisateur attendait.
* **+** Les premières lignes sont déterministes : le même clic montre les mêmes
  lignes, et l'avis nomme l'ordre dans lequel elles arrivent.
* **+** La sûreté de l'`OFFSET` est inchangée : une page n'existe que sur un
  ordre total, celui par défaut compris.
* **−** Chaque aperçu lit la description : gratuit sur SQLite, inchangé pour un
  aperçu simple sur PostgreSQL (et moins cher trié ou paginé), deux
  allers-retours de plus sur MySQL/MariaDB — voir le tableau du Contexte.
* **−** L'ordre que les utilisateurs voient d'abord passe de « ce que le moteur
  renvoie » à l'ordre de la clé. ADR-0020 avait nommé ce changement visible et
  l'avait accepté ; il est de nouveau accepté ici.
* **−** L'`OFFSET` reste linéaire : la page 100 coûte 100 fois la page 1. Un
  paginateur toujours présent rend les pages profondes plus faciles à
  atteindre, donc ce coût plus facile à payer.
* **−** DuckDB n'a pas de parcours d'index ordonné — son index ART sert les
  contraintes et les recherches très sélectives
  ([RESEARCH-NOTES](../RESEARCH-NOTES.md#ordre-par-défaut-dun-aperçu--vérifié-le-2026-10-26)) :
  son ordre par défaut est un `TOP_N` sur un parcours séquentiel, élagué par les
  zone maps grâce à un filtre dynamique sur la clé. Sur 10 millions de lignes en
  mémoire : 1,9 ms dans l'ordre d'insertion, environ 25 ms mélangées, contre
  1,8–2 ms sans `ORDER BY` ; à l'`OFFSET` 5 000 000, environ 130 ms,
  430–540 ms mélangées. Une base sur fichier n'a pas été mesurée.
* **−** Lacunes connues. Une clé primaire ajoutée par `ALTER TABLE ONLY` sur un
  parent partitionné PostgreSQL laisse son index invalide jusqu'à ce que chaque
  partition soit attachée. Une collection MongoDB vide n'échantillonne aucun
  champ, si bien que sa description n'a pas de clé `_id` : aucune page n'est
  offerte tandis que l'aperçu simple trie quand même par `_id`, ce qui est sans
  danger. `system.profile` n'a pas d'index `_id`, une lacune antérieure à cet
  ADR.
* **−** Une relation `needsOrder` pagine toujours sur sa clé déclarée une fois
  que l'utilisateur a trié. Sur Redshift, cette clé n'est pas appliquée et peut
  se répéter ; c'est inchangé depuis ADR-0028, et cela reste le choix explicite
  de l'utilisateur.
* **−** Un catalogue périmé après un DDL pourrait ordonner par une clé qui
  n'existe plus ; l'invalidation
  d'[ADR-0022](0022-rafraichissement-automatique.md) le couvre, et rien de neuf
  n'a été ajouté pour cela.

**Hors périmètre, et les suites que l'utilisateur peut vouloir :** ClickHouse
(un ordre total demanderait un `ORDER BY` sur toutes les colonnes ordonnables),
Elasticsearch et OpenSearch (un curseur point-in-time et `search_after` au lieu
de l'`OFFSET`), Redis (la pagination par le curseur de `SCAN`). Chacune serait
son propre ADR.

**Coût de sortie :** faible, borné au code. Revenir à ADR-0028, c'est effacer
le drapeau dans le `describe_relation` de chaque driver garant — l'ordre et le
contrôle de page disparaissent alors ensemble, puisqu'ils lisent le même
drapeau — et revenir sur les tests qui ancrent l'ordre par défaut
(`a_preview_comes_in_key_order_only_when_the_driver_vouches_for_it_and_a_key_exists`
dans `oxyn-catalog` ; `a_vec0_table_is_not_ordered_by_default_but_can_be_sorted`,
`a_composite_key_is_ordered_as_its_index_not_as_its_table` et
`a_preview_without_request_composes_neither_where_nor_order_by_when_no_key_is_known`
dans le driver SQLite ; `a_plain_preview_is_ordered_exactly_when_the_description_says_so`
dans le driver PostgreSQL). Rien n'est persisté. Ce qui coûte plus est invisible,
et ADR-0028 l'avait déjà nommé : les utilisateurs s'habitueront à un
paginateur toujours présent, et son retrait se lirait comme une régression.

**Réexaminer si :**
* l'ordre par défaut d'un driver garant est mesuré au-delà du budget des
  premières lignes de [PERFORMANCE](../PERFORMANCE.md#budgets-dinteraction) sur
  une vraie table — ce driver cesse alors de se porter garant, comme les tables
  virtuelles le font déjà ;
* la pagination par clé (`WHERE clé > dernière valeur vue`) arrive, qui
  remplacerait l'`OFFSET` sous le même ordre total et changerait ce que coûte
  une page profonde ;
* un moteur gagne un curseur stable sans ordre déclaré, auquel cas ni l'ordre
  par défaut ni l'`OFFSET` ne sont le bon outil pour lui.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Garder ADR-0028 tel quel | Sa deuxième condition de réexamen est remplie : l'utilisateur lit l'absence de paginateur comme un défaut |
| Ordonner toute relation qui déclare une clé, sans la parole du driver | Une table `vec0` en déclare une et lirait chaque vecteur avant sa première page ; Redshift déclare des clés qu'il n'applique pas |
| Un bit de capacité « ordonne par défaut » | Par driver, pas par relation : il ne distingue pas une table SQLite d'une table virtuelle SQLite, et ce serait une seconde source pour ce que le drapeau dit déjà |
| Ordonner par `rowid` ou par l'ordre physique du moteur là où aucune clé n'existe | Pas un ordre total que le catalogue sait nommer : une table `WITHOUT ROWID` n'a pas de `rowid`, la plupart des moteurs n'ont pas d'équivalent, et un ordre physique est précisément l'ordre qui n'est pas garanti |
| Offrir des pages sans ordre, en avertissant que des lignes peuvent se répéter | Un résultat faux qui a l'air juste, averti ou non — la panne qu'ADR-0020 existe pour empêcher |
| La pagination par clé tout de suite | Elle exige aussi un ordre total, donc elle ne retire pas le besoin de cette décision ; elle change le coût d'une page profonde, ce qui est sa propre décision |
