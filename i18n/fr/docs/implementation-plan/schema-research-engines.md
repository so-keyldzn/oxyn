<!-- oxyn-translation source="docs/implementation-plan/schema-research-engines.md" sha256="1a4a2ca9a0dd" -->

# Recherche sur les schémas : matrice des moteurs

Statut : **preuves statiques et couverture proposée**, au 2026-11-13.
Code examiné : `6647180` ; neuf manifestes de drivers sous `drivers/`.
Cette annexe appartient au [plan d'implémentation](../IMPLEMENTATION-PLAN.md).
À lire avec les [parcours utilisateur](schema-research-journeys.md).

**Revalidation — 2026-11-13, `f456ff1` :** la référence initiale `6647180`
est conservée ci-dessus. La comparaison statique couvre `3929418` (identité des
aperçus URI SQLite), `79726f5` (attente de l'état visible du popover de tri dans
un test) et `f456ff1` (préparation de la version 0.0.18). Les déclarations de
capacités et l'inventaire hors SQLite ne changent pas avec ces commits.
SQLite distingue désormais les cibles reproductibles lors de la description
de la prise en charge d'aperçus conservés ; voir SQ01–SQ06 ci-dessous.
Aucune contradiction bloquante avec les parcours ou les ADR proposés 0079–0082
n'a été trouvée. Les sources des tests ont été inspectées, pas exécutées.

**Revalidation d’implémentation — 2026-11-14, base `ce58839` :** le chantier
multi-base conserve la séparation catalogue/console de main et l’occupation des
sessions par les aperçus retenus. La découverte PostgreSQL reste distincte du
parcours du catalogue ; les hiérarchies natives des autres moteurs sont
conservées. Les ADR de recherche portent désormais les numéros 0080–0083,
l’ADR-0079 accepté sur main concernant les aperçus sur la session d’origine.
Les cas SQLite restent applicables, sans exemption automatique de préchargement.
L’état courant du code et des tests figure dans le [suivi](completion.md),
séparément de cet inventaire historique.

## Comment lire la matrice

Cet inventaire concerne **l'implémentation d'Oxyn**, pas une certification de
toutes les versions serveur ni la liste de tout ce que chaque produit sait
faire. Les capacités de la session, les privilèges et la réponse effective
restent déterminants. Aucun test d'intégration serveur n'a été exécuté pour
cet inventaire.

- **Présent** : une capacité et son implémentation ont été localisées.
- **Conditionnel** : variante, session ou type d'objet limite le chemin disponible.
- **Absent** : non exposé par le contrat inspecté de ce driver ; cela ne signifie
  pas que le produit de base de données le rend impossible.
- **Proposé** : cible de recherche, ni livrée ni promise.

Chaque ligne renvoie aux sources ci-dessous. Pouvoir lister une table/collection
ne prouve pas que ses champs ou liens sont lisibles. Une clé étrangère rapportée
ne démontre pas que le moteur l'a imposée à toutes les données stockées.

## Navigation et lectures structurelles existantes

FK sortantes/entrantes désigne leur introspection. La colonne Index concerne
les index d'un objet, pas un index Elasticsearch comme objet de premier niveau.

| Moteur / variante | Navigation actuelle | Champs | Index | FK sortantes / entrantes | Contraintes / définition | Preuve |
|---|---|---|---|---|---|---|
| PostgreSQL | Base connectée → schémas → tables, vues, vues matérialisées, séquences | Présent | Présent | Présent / présent | Présent / conditionnel selon serveur détecté et objet | PG |
| TimescaleDB / pgvector | Chemin PostgreSQL ; les extensions détectées ajoutent des capacités | Chemin PostgreSQL | Chemin PostgreSQL | Chemin PostgreSQL | Chemin PostgreSQL ; aucune carte dédiée séries temporelles/vecteurs | PG |
| Redshift | Variante du driver PostgreSQL ; base courante seulement | Implémentation partagée, validation en exécution nécessaire | Chemin partagé annoncé, validation en exécution nécessaire | Sortantes annoncées / entrantes absentes | Toutes deux absentes des capacités de la variante | PG |
| MySQL / MariaDB | Bases visibles au niveau espace de noms → tables/vues ; le parseur de catalogue reconnaît aussi les séquences | Présent | Présent | Présent / présent | Facette contraintes absente / définition présente | MY |
| SQLite | Espaces de noms (`main`, attachements visibles de la session catalogue) → tables/vues | Présent ; l'aperçu conservé dépend aussi de la reproductibilité de la source | Présent | Présent / présent | Présent / présent, avec refus selon l'objet | SQ ; SQ01–SQ06 ci-dessous |
| DuckDB | Catalogues attachés → schémas → tables/vues | Présent | Présent | Présent / présent | Présent / présent | DU |
| ClickHouse | Bases comme espaces de noms → tables/vues/vues matérialisées | Présent | Présent | Absent / absent | Facette contraintes absente / définition présente | CH |
| Turso Cloud | Une base par URL ; espaces de noms → tables/vues | Présent | Présent | Présent / absent | Toutes deux absentes | TU |
| MongoDB | Bases comme espaces de noms → collections/vues/collections temporelles | Observés sur un échantillon borné de documents, marqués déduits | Présent | Absent / absent | Pas de facette contraintes SQL / définition reconstruite comprenant les options de collection | MO |
| Elasticsearch / OpenSearch | Catalogue plat d'index/alias/data streams | Mappings serveur ; types divergents conservés | Pas de facette séparée d'introspection d'index | Absent / absent | Pas de facette contraintes SQL / définition présente | ES |
| Redis / Valkey | Bases numérotées → échantillon borné de clés | Description adaptée au type de clé sélectionnée, pas un schéma de collection déclaré | Absent | Absent / absent | Toutes deux absentes | RE |

La navigation par groupe serveur de J01 est nouvelle pour PostgreSQL ; elle ne
doit pas remplacer l'énumération de bases existante de MySQL, MongoDB, ClickHouse
ou Redis. Les attachements SQLite et DuckDB ne sont pas des bases d'un serveur
distant ; cette proposition n'introduit pas d'opération ATTACH automatique.

<a id="sqlite-target-cases"></a>

### Cas SQLite d'identité de cible et de récupération

Le [contrat du driver](../DRIVER-CONTRACT.md) porte le comportement. Ce tableau
ajoute des cas d'acceptation de recherche, pas une seconde spécification du
parseur URI. SQ01–SQ05 renvoient au code et aux sources de tests de `f456ff1` ;
ces tests n'ont pas été exécutés lors de la revalidation. SQ06 est une couverture
de récupération supplémentaire proposée.

| Cas | Source / action | Résultat attendu dans le parcours | Preuve / état de validation |
|---|---|---|---|
| SQ01 | Ouvrir l'aperçu d'une table/vue sans clé depuis une URI de fichier avec `mode=rw` ou `mode=rwc` | Le lecteur isolé cible le fichier d'origine en lecture seule ; la session source n'est ni redirigée ni modifiée | `writable_uri_modes_keep_read_only_preview_on_original_database` |
| SQ02 | Utiliser des noms/valeurs d'option d'accès encodés en pourcentage, des chemins relatifs et d'autres options VFS/cache ou fragments | Restreindre uniquement l'option d'accès en écriture. Préserver le nom de fichier et les autres octets ; les modes déjà en lecture seule ou mémoire ne sont pas réécrits vers une autre cible | `only_writable_mode_values_change_and_other_options_remain_byte_exact` ; des cas encodés figurent aussi dans le test SQ01 |
| SQ03 | Lire une URI mémoire partagée nommée, dont la forme testée `file::memory:?cache=shared`, tant que la source est vivante | Lire la même base et conserver son mode mémoire ; le lecteur privé refuse toujours les écritures | `a_named_shared_memory_uri_keeps_its_identity_and_memory_mode` |
| SQ04 | Lire un objet sans clé via une URI mémoire privée ou temporaire sans nom, comme `file::memory:` ou `file:` | La description reste lisible ; la prise en charge d'aperçu conservé vaut faux. Lire des lignes bornées sur la session d'origine ; refuser le lecteur isolé au lieu de lire une nouvelle base vide | `private_memory_and_temporary_uris_never_read_a_fresh_empty_database`. Cela ne décrit pas toutes les graphies de « mémoire » dans l'interface |
| SQ05 | Supprimer le fichier source avant l'ouverture du lecteur privé, y compris une URI d'origine en `mode=rwc` | Échouer explicitement sans recréer le fichier ; les métadonnées existantes ne doivent pas faire paraître l'aperçu réussi | `a_removed_uri_file_is_not_recreated_by_a_private_reader` ; la gestion de l'onglet restauré est un critère J09 supplémentaire |
| SQ06 | Fermer le dernier propriétaire d'une base éphémère ou redémarrer, puis rouvrir un onglet objet/carte enregistré | Conserver l'emplacement comme référence, pas comme preuve que l'ancienne base existe. Exiger reconnexion ou création explicite ; ne jamais réutiliser fraîcheur/continuations de l'ancienne source vivante pour une source nouvellement créée | Couverture de récupération J09 de bout en bout proposée ; non établie par le test SQ03 sur session vivante |

Ces distinctions affectent disponibilité de Données/aperçus et récupération ;
elles ne retirent pas l'introspection structurelle de SQLite et ne rendent pas
la carte dépendante de l'ouverture possible d'un lecteur isolé.

## Couverture proposée pour la carte des schémas

Aucune carte autonome n'existe dans ce code. L'ERD actuel de l'assistant est le
candidat à réutiliser pour toutes les lignes, pas la preuve que chacune est déjà
correctement représentée.

| Famille de moteurs | Première représentation utile | Travail supplémentaire / limite explicite | Parcours |
|---|---|---|---|
| PostgreSQL, SQLite, MySQL/MariaDB, DuckDB | Cartes d'objets avec colonnes et FK déclarées ; regroupement selon la hiérarchie existante | Réutiliser les lectures ; ajouter types de nœuds, couverture et erreurs par facette. Les dépendances des vues nécessitent un chemin de métadonnées typées distinct. | J02/J04 |
| TimescaleDB / pgvector | Même carte relationnelle, conservant les types de champs rapportés | Vues dédiées hypertables/chunks ou index vectoriels hors de ce lot ; ne pas les déduire d'un badge d'extension. | J04 |
| Turso | Cartes d'objets et clés sortantes | Inverser uniquement les clés chargées dans le périmètre choisi ; ne pas prétendre découvrir toutes les clés entrantes. | J04 |
| Redshift | Objets et métadonnées structurelles effectivement rapportées | Ne pas demander les facettes désactivées. Vérifier clés sortantes/index annoncés sur un serveur jetable avant de déclarer leur prise en charge. | J04 |
| ClickHouse | Tables/vues, colonnes et informations d'index disponibles | Aucun lien FK de ce driver. Les dépendances de vues/vues matérialisées nécessitent une implémentation et une validation supplémentaires. | J04 |
| MongoDB | Collections, champs observés et index ; validateurs déclarés affichés séparément une fois exposés structurellement | Options/validateurs circulent déjà dans le texte de définition reconstruite ; projection structurée des validateurs et modèle de champs imbriqués restent nécessaires. Échantillonner est un accès aux données, pas une promesse de métadonnées seules. | J05 |
| Elasticsearch / OpenSearch | Cartes d'index/mappings ; structure imbriquée ; appartenance aux alias et data streams | Objets et mappings existent ; liens d'appartenance typés et hiérarchie fidèle à exposer. Conserver les types divergents entre index couverts. | J05 |
| Redis / Valkey | Vue de l'espace de clés par base, type et regroupement facultatif par préfixe choisi | La liste de clés est échantillonnée. Le regroupement par préfixe est une convention, pas une relation. Une structure détaillée hash/JSON exige une conception de lecture bornée distincte. | J05 |

Une carte peut être utile sans aucun lien. Inconnu, non chargé, non pris en
charge et absence constatée de liens sont des états différents.

## Couverture des autres fonctions demandées

| Fonction | État actuel dans ce code | Couverture proposée / prérequis |
|---|---|---|
| Profil serveur multi-base (#373) | Pas d'identité de groupe serveur ; hiérarchie propre à chaque driver déjà présente | PostgreSQL d'abord ; politique, cache et sessions indépendants par base. [D01][d01] consigne la portée indépendante proposée des identifiants ; la migration reste à valider. |
| Introspection configurable (#375) | Facettes indépendantes, cache borné et préréglages manuels par connexion/périmètre natif ; pages de préchargement explicites uniquement (validation dans le registre d’achèvement) | Politique commune aux neuf drivers, limitée à leurs facettes disponibles. Aucune exemption générale pour les fichiers locaux. [D02][d02] consigne les déclencheurs explicites et la persistance de préférence. |
| Export/restauration natifs (#374) | Aucun wrapper `pg_dump`/`pg_restore` trouvé dans le code applicatif ; l'export de résultats conservés est distinct | Adaptateur PostgreSQL proposé uniquement. Compatibilité des variantes et autres moteurs à étudier séparément ; ne pas proposer les outils PostgreSQL simplement parce qu'un driver parle ce protocole. [D04][d04] consigne la frontière des tâches ; le transport d'authentification reste un prérequis. |
| Navigation F6 (#308) | Documentée, absente du manifeste d'actions | Action commune du workspace, indépendante du driver ; inclure une carte focalisée lorsqu'elle existe. |
| Cartes enregistrées / export d'image | Placement et rendu ERD existants ; positions manuelles désactivées et actions de réorganisation/export indisponibles | [D03][d03] d'abord ; enregistrer les choix d'affichage séparément des métadonnées et des valeurs échantillonnées. |

## Moteurs futurs : périmètre, pas prise en charge implicite

Les produits suivants figurent dans [VISION](../VISION.md), mais n'ont pas de
driver dans ce code. Leurs lignes sont des questions pour une étude future,
pas des capacités produit vérifiées. Aucun ordre de livraison n'est fixé ici.

| Famille | Produits nommés par la vision | Question de recherche avant de proposer une carte |
|---|---|---|
| Relationnel | SQL Server, Oracle | Quelles API de catalogue exposent colonnes, contraintes, dépendances et restrictions d'accès ? |
| Analytique/cloud | Snowflake, BigQuery | Quel effet ont les périmètres projet/compte/catalogue sur l'identité, les permissions et la découverte bornée ? |
| Document/colonnes larges/clé-valeur | Couchbase, Cassandra, DynamoDB | Qu'est-ce qui est déclaré, qu'est-ce qui exige des échantillons, et comment représenter valeurs imbriquées et clés ? |
| Vectoriel | Milvus, Weaviate, Pinecone, Qdrant, ChromaDB | Quels champs de collection, paramètres vectoriels, schémas de payload et métadonnées d'index sont introspectables ? |
| Graphe | Neo4j, Memgraph | Quelles relations entre labels/types/propriétés sont déclarées, observées ou estimées, et à quel coût de lecture ? |
| Séries temporelles | InfluxDB | Quels espaces de noms, mesures, tags et champs peut-on inspecter sans parcours non borné ? |

Ces lignes n'ajoutent aucune version ou limite externe. La vérification des
registres et API officielles appartient à [RESEARCH-NOTES](../RESEARCH-NOTES.md)
au démarrage de cette étude ; la matrice actuelle affirme uniquement ce que les
sources versionnées d'Oxyn établissent.

## Écarts qui affectent les parcours proposés

1. L'ERD de l'assistant demande `incomingKeys` aux nœuds de départ sans filtrer
   d'abord par capacité. L'assemblage échoue sur une lecture refusée au lieu de
   conserver une carte partielle. Sa généralisation exige des lectures adaptées
   aux capacités.
2. `ErdColumn` omet le statut déduit du champ et `ErdTable` le type d'objet ;
   ils ne portent pas la distinction proposée entre tables, mappings et
   structures échantillonnées. La portée par connexion est actuellement externe
   à la clé du nœud ; une carte combinée doit conserver explicitement cette identité.
3. Index et clés sortantes partagent lecture/fraîcheur de `detail`. Les séparer
   affecte aussi inspecteur d'objet, résumés de contraintes, complétion,
   contexte IA, actions de lignes liées et assemblage ERD.
4. Les capacités PostgreSQL annoncent `ROUTINES`, mais le catalogue inspecté
   liste les relations `pg_class` sans chemin correspondant de découverte des
   routines. C'est un écart capacité/surface à résoudre, pas la preuve qu'un
   navigateur complet de routines existe.
5. L'ERD actuel ne recherche que les objets déjà listés. Une carte autonome
   nécessite un parcours de découverte explicite ; « introuvable dans les
   métadonnées chargées » ne doit pas devenir « inexistant ».
6. Les limites actuelles bornent le nombre d'objets et certains contenus
   individuels ; elles ne prouvent ni un pic d'octets borné pour chaque liste,
   ni un rendu fluide. Mesurer lectures serveur et webview avant de les élargir.

## Inventaire reproductible des sources

L'inventaire initial a été inspecté à `6647180`, et les chemins modifiés
revalidés jusqu'à `f456ff1` comme indiqué ci-dessus. Les liens relatifs ouvrent
le checkout actuel, pas une copie immuable de l'un de ces commits. Des
changements ultérieurs exigent une nouvelle date de vérification. Déclarations
et implémentations sont liées séparément lorsque les variantes comptent.

- **PG :** [capacités des variantes](../../../../drivers/oxyn-driver-postgres/src/variant.rs),
  [catalogue](../../../../drivers/oxyn-driver-postgres/src/catalog.rs),
  [contexte de session](../../../../drivers/oxyn-driver-postgres/src/session.rs),
  [définition](../../../../drivers/oxyn-driver-postgres/src/catalog/definition.rs).
- **MY :** [capacités](../../../../drivers/oxyn-driver-mysql/src/variant.rs),
  [catalogue](../../../../drivers/oxyn-driver-mysql/src/catalog.rs).
- **SQ :** [capacités](../../../../drivers/oxyn-driver-sqlite/src/driver.rs),
  [catalogue](../../../../drivers/oxyn-driver-sqlite/src/catalog.rs),
  [gestion de la cible URI](../../../../drivers/oxyn-driver-sqlite/src/preview_uri.rs),
  [contexte de reproductibilité](../../../../drivers/oxyn-driver-sqlite/src/preview_context.rs),
  [ouverture du lecteur privé](../../../../drivers/oxyn-driver-sqlite/src/session/preview.rs),
  [cas de régression URI](../../../../drivers/oxyn-driver-sqlite/src/session/preview_uri_tests.rs).
- **DU :** [capacités](../../../../drivers/oxyn-driver-duckdb/src/driver.rs),
  [catalogue](../../../../drivers/oxyn-driver-duckdb/src/catalog.rs).
- **CH :** [capacités](../../../../drivers/oxyn-driver-clickhouse/src/variant.rs),
  [catalogue](../../../../drivers/oxyn-driver-clickhouse/src/catalog.rs).
- **TU :** [capacités](../../../../drivers/oxyn-driver-turso/src/variant.rs),
  [catalogue](../../../../drivers/oxyn-driver-turso/src/catalog.rs).
- **MO :** [capacités](../../../../drivers/oxyn-driver-mongodb/src/driver.rs),
  [catalogue](../../../../drivers/oxyn-driver-mongodb/src/catalog.rs),
  [reconstruction de définition](../../../../drivers/oxyn-driver-mongodb/src/catalog/definition.rs).
- **ES :** [capacités](../../../../drivers/oxyn-driver-elasticsearch/src/driver.rs),
  [catalogue](../../../../drivers/oxyn-driver-elasticsearch/src/catalog.rs),
  [projection du mapping](../../../../drivers/oxyn-driver-elasticsearch/src/mapping.rs).
- **RE :** [driver](../../../../drivers/oxyn-driver-redis/src/driver.rs),
  [catalogue](../../../../drivers/oxyn-driver-redis/src/catalog.rs).
- **Partagé :** [défauts du provider](../../../../crates/oxyn-catalog/src/provider.rs),
  [lectures du bus](../../../../crates/oxyn-exec/src/catalog.rs),
  [IPC des facettes](../../../../crates/oxyn-desktop/src/ipc/metadata.rs),
  [assemblage ERD](../../../../apps/desktop/src/features/assistant/erd.ts),
  [modèle ERD](../../../../apps/desktop/src/components/oxyn/erd-model.ts),
  [rendu ERD](../../../../apps/desktop/src/components/oxyn/erd-diagram.tsx).

## Validation nécessaire avant de déclarer une implémentation

Pour chaque moteur retenu, exercer un jeu jetable comprenant objets vides,
métadonnées refusées, identifiants hostiles, annulation et réponses périmées.
Les jeux relationnels ajoutent clés composites/autoréférentes/cycliques, objets
homonymes dans des espaces de noms distincts et vues sans clé. Les jeux
documentaires ajoutent types mixtes, champs absents, échantillons vides et
mappings incompatibles. Les jeux d'espace de clés ajoutent des clés non
textuelles et des listes bornées incomplètes.

SQLite ajoute [SQ01–SQ06](#sqlite-target-cases) : vérifier inspection d'objet
et navigation carte-vers-Données, en plus des cas du driver. Séparer fichiers
ordinaires, mémoire partagée nommée, mémoire privée et sources temporaires dans
l'inventaire des jeux de test ; une réussite sur l'un ne valide pas les autres.

Répéter les parcours carte/inspection en affichage large et compact, au clavier
et via la liste accessible. Vérifier que choisir une autre base ne peut pas
remplacer les résultats, métadonnées ou opérations en attente de la première.
C'est la matrice d'acceptation à exécuter ultérieurement ; aucune ligne ci-dessus
ne revendique un test d'intégration réussi ni une performance mesurée.

[d01]: ../adr/0080-group-database-connections-under-server-profiles.md
[d02]: ../adr/0081-load-catalog-facets-independently-with-explicit-prefetch.md
[d03]: ../adr/0082-project-schema-maps-from-provenanced-catalog-metadata.md
[d04]: ../adr/0083-run-native-database-transfers-as-authorized-jobs.md

### Actualisation #375 dans le worktree

Les facettes sont indépendantes. Minimal/Structure/Étendu sont des préférences
manuelles par connexion et périmètre natif ; seul le préchargement explicite
lance une page. Aucun moteur, SQLite inclus, ne bénéficie d’une exemption
automatique. La validation courante est suivie dans [completion.md](completion.md).

## Chargement des métadonnées entre moteurs (#375)

Préréglages, dérogations persistées, file de lectures, annulation, progression et
contrôle des générations sont communs à l’hôte. Les plans utilisent les
**capacités de la session vivante**, jamais une liste de noms de moteurs.
Le périmètre conserve les niveaux natifs absents ; aucun fichier n’est attaché
et toutes les bases ne sont pas ouvertes automatiquement.

| Famille | Périmètre | Ajouts Structure | Ajout Étendu | Limite importante |
|---|---|---|---|---|
| PostgreSQL et variantes compatibles | Schéma observé de la base connectée | Champs, index/clés sortantes disponibles | Définition annoncée | Capacités de la variante et privilèges font foi |
| MySQL / MariaDB | Base observée au niveau espace de noms | Champs, index/clés sortantes | Définition | Aucune facette de contraintes SQL inventée |
| SQLite | `main` ou attachement observé sur la session propriétaire | Champs, index/clés sortantes | Définition | Aucun ATTACH, fichier de remplacement ou exemption générale |
| DuckDB | Couple catalogue/schéma observé | Champs, index/clés sortantes | Définition | L’identité du catalogue reste dans le périmètre |
| ClickHouse | Base observée au niveau espace de noms | Champs et index | Définition | Aucune lecture de FK sortantes indisponibles |
| Turso | Espace de noms observé de l’endpoint connecté | Champs, index/clés sortantes | Indisponible si non annoncée | Aucune définition de remplacement inventée |
| MongoDB | Base observée au niveau espace de noms | Index ; inférence de champs exclue | Définition reconstruite disponible | Champs sur échantillon humain distinct et borné ; aucune FK |
| Elasticsearch / OpenSearch | Racine observée de la source | Champs du mapping | Définition disponible | Un index Elasticsearch est un objet, pas la facette d’index SQL optionnelle |
| Redis / Valkey | Catalogue de base numérotée observé | Faits de type de clé/disposition d’aperçu | Indisponible | Noms issus de l’échantillon de clés borné ; aucune valeur lue par la description |

Les tests hôte/fournisseur couvrent les combinaisons indépendantes : index seul,
FK seule, définition seule et description explicitement inférée. Les vérifications
SQLite et MongoDB jetables sont distinctes de cet inventaire statique ; elles ne
certifient pas toutes les versions des autres serveurs. Le [registre](completion.md)
détaille les validations réellement exécutées et leurs exclusions.
