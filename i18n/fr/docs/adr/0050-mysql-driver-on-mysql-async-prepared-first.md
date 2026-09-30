<!-- oxyn-translation source="docs/adr/0050-mysql-driver-on-mysql-async-prepared-first.md" sha256="79786a4a0b79" -->

> Traduction française de [docs/adr/0050-mysql-driver-on-mysql-async-prepared-first.md](../../../../docs/adr/0050-mysql-driver-on-mysql-async-prepared-first.md). **La version anglaise fait foi.**

# ADR-0050 — Le driver MySQL repose sur `mysql_async`, prépare d'abord chaque instruction, et décode tout type que le serveur envoie

**Statut :** proposé · **Date :** 2026-09-30

## Contexte

`oxyn-driver-mysql` est le troisième driver de la porte de sortie de la
phase 2, reporté après la phase 3
([IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md)).
[ADR-0003](0003-driver-capabilities.md) tranche déjà qu'il sert MySQL et
MariaDB : un protocole, une crate. Reste ouvert, et coûteux à défaire une fois
le driver écrit : la bibliothèque cliente, la façon dont le SQL de
l'utilisateur passe sur le fil, et la façon dont les valeurs deviennent de
l'Arrow. Vérifié le 2026-09-30 ([RESEARCH-NOTES](../RESEARCH-NOTES.md)) :

* **Serveurs.** MySQL a deux lignes LTS, 8.4 (8.4.11, supportée jusqu'au
  2032-04-30) et 9.7 (9.7.2, jusqu'au 2034-04-30) ; la 8.0 est en fin de vie
  depuis le 2026-04-30. Les lignes LTS de MariaDB sont 11.8 (jusqu'au
  2028-06-04) et 12.3 (jusqu'au 2029-06-12).
* **`sqlx-mysql` 0.9.0 ne sait pas lire une colonne `VECTOR`.** MySQL 9
  l'annonce avec le code de type 242 (`MYSQL_TYPE_VECTOR`) ; `sqlx-mysql`
  associe les codes à une énumération fermée et fait échouer tout le résultat
  avec `unknown column type 0xf2` — sur `main` aussi. Un `SELECT *` sur une
  table qui contient des embeddings échoue net. Il perd aussi les `decimals` de
  la colonne (l'échelle d'un `DECIMAL`), garde les octets bruts et l'état de
  transaction en `pub(crate)`, ne gère aucun des plugins `client_ed25519` et
  `parsec` de MariaDB, et exige, pour `caching_sha2_password` sans TLS, la
  crate `rsa` 0.10.0-rc.18 — une release candidate sous RUSTSEC-2023-0071, qui
  n'indique aucune version corrigée.
* **`mysql_async` 0.37.1** (MIT OR Apache-2.0) repose sur `mysql_common`, qui
  connaît `MYSQL_TYPE_VECTOR` (0.37.x), expose pour chaque colonne le type, les
  drapeaux, la longueur, les `decimals` et le jeu de caractères, implémente
  `client_ed25519` et `parsec` de MariaDB, et fait son échange RSA avec son
  propre code sur `num-bigint`, pas avec la crate `rsa`. `Conn::id()` donne
  l'identifiant de connexion, et `Conn::last_ok_packet()` les drapeaux d'état
  du serveur, `SERVER_STATUS_IN_TRANS` compris. TLS passe par `rustls` avec
  `aws-lc-rs`, le fournisseur que le workspace embarque déjà.
* **Les deux bibliothèques annoncent `CLIENT_MULTI_STATEMENTS` sans
  condition** et n'offrent aucun moyen public d'envoyer `COM_SET_OPTION`. Un
  texte envoyé par `COM_QUERY` peut donc exécuter plusieurs instructions.
  `mysql_async` annonce aussi `CLIENT_LOCAL_FILES` : le serveur peut alors
  demander au client un fichier local par son nom, et seule l'absence de
  gestionnaire le refuse.
* **Une instruction préparée est une seule instruction, prouvée par le
  serveur.** Dans `Prepared_statement::prepare` de MySQL 8.4, le lexer tourne
  avec `multi_statements = false`, donc tout ce qui suit un `;` est une erreur
  de syntaxe (`ER_PARSE_ERROR`, 1064) ; c'est seulement ensuite que
  `prepare_query` refuse ce qui ne se prépare pas (`ER_UNSUPPORTED_PS`, 1295).
  Ce qui ne se prépare pas, d'après le code source : `START TRANSACTION`/`BEGIN`,
  `SAVEPOINT`, `USE`, `LOCK TABLES`, `ALTER VIEW`,
  `CREATE PROCEDURE`/`FUNCTION`/`TRIGGER`/`EVENT`, `LOAD DATA`, `XA`. `SET`,
  `CREATE VIEW`, `COMMIT`, `ROLLBACK` et `EXPLAIN` se préparent, bien que la
  liste du manuel en omette certains. **Observé le 2026-09-30** avec
  `mysql_async` 0.37.1 contre MySQL 8.4.11 et 9.7.2 et MariaDB 11.8.9 et
  12.3.3 : les quatre refusent à la préparation un texte multi-instructions
  avec 1064 — y compris quand sa première instruction ne se prépare pas, et
  quand le `;` est dans un `/*! … */` —, alors que `COM_QUERY` l'exécute.
  MariaDB prépare toutes les instructions essayées, `USE`, `START TRANSACTION`,
  `LOCK TABLES`, `CREATE PROCEDURE` et `LOAD DATA` compris ; MySQL les refuse
  avec 1295.
* **`oxyn-query` découpe mal MySQL sur les corps composés.** Mesuré le
  2026-09-30 : `CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END` devient
  trois fragments. Le scanner connaît les corps de trigger SQLite, pas le
  `BEGIN … END` de MySQL, et il ne connaît pas le `DELIMITER` du client
  `mysql`.
* **Ce qu'envoient les serveurs, observé le même jour.** Un `VECTOR(3)` de
  MySQL 9.7 arrive en type 242, longueur 12, `f32` little-endian contigus. Un
  `VECTOR(3)` de MariaDB arrive en `VAR_STRING` avec le jeu de caractères
  `binary` — comme un `VARBINARY(12)` ; seules les métadonnées étendues de
  MariaDB, que `mysql_async` ne demande pas, les distingueraient. MySQL envoie
  `JSON` en type 245 avec le jeu de caractères `binary` (63) ; MariaDB l'envoie
  en `BLOB` `utf8mb4`. La longueur d'une colonne `DECIMAL(p, s)` vaut `p`, plus
  un pour le signe sauf si `UNSIGNED`, plus un pour le point quand `s > 0` —
  vérifié sur `(10,2)`, `(65,30)`, `(10,0) UNSIGNED`, `(5,5)`. `mysql_async`
  décode sans erreur une date nulle (`0000-00-00`, `2024-00-15`), et un `TIME`
  de `-838:59:59`.
* **Un lecteur à l'arrêt bloque le serveur.** Quand le client cesse de lire,
  le serveur attend `net_write_timeout` (60 s par défaut) puis abandonne
  l'instruction, en gardant ses verrous entre-temps.
* **Annulation.** `KILL QUERY <id>` arrête l'instruction qu'exécute une
  connexion et garde la connexion ; le drapeau est lu entre deux blocs de
  lignes. Sans `CONNECTION_ADMIN`, un compte ne peut tuer que ses propres
  threads — le cas d'une seconde connexion du même compte. Observé sur les
  quatre serveurs : un compte qui n'a que `SELECT` tue sa propre requête depuis
  sa seconde connexion, la requête échoue avec 1317, et la connexion sert
  l'instruction suivante.
* **Mode lecture seule, observé sur les quatre serveurs.** Sous
  `SET SESSION TRANSACTION READ ONLY`, `INSERT`, `UPDATE`, `DELETE`,
  `CREATE TABLE`, `DROP TABLE`, `TRUNCATE`, `ALTER TABLE`, `RENAME TABLE`,
  `CREATE INDEX` et `CREATE TEMPORARY TABLE` échouent tous avec 1792, en
  protocole texte comme binaire ; `SELECT` s'exécute. Le manuel permet le DML
  sur les tables temporaires, mais aucune ne peut être créée dans ce mode.

## Décision

1. **`oxyn-driver-mysql` dépend de `mysql_async`**, sans ses fonctionnalités
   par défaut, avec `default-rustls` (`rustls` sur `aws-lc-rs`),
   `client_ed25519` et `client_parsec`. Une session tient un `Conn`, ouvert
   avec `stmt_cache_size(0)` et sans `local_infile_handler` : une demande
   `LOAD DATA LOCAL INFILE` venue du serveur est alors refusée, aucune ligne
   n'est chargée, et la connexion est fermée — observé sur les quatre
   serveurs ; le driver le signale comme une connexion perdue, et un test du
   driver le tient. Une crate sert MySQL et MariaDB ;
   les différences sont des capacités lues à la connexion depuis la version du
   serveur, pas une seconde crate.
2. **Chaque instruction de l'utilisateur est d'abord préparée** : `Conn::prep`,
   puis `exec_iter` sans paramètres, puis `Conn::close` sur l'instruction — le
   cache désactivé, fermer revient à l'appelant, et une instruction laissée
   ouverte compte contre `max_prepared_stmt_count`. Le serveur prouve que le
   texte est une seule instruction, et les valeurs arrivent en protocole
   binaire.
3. **Le protocole texte est un repli, sur une seule erreur.** Quand la
   préparation échoue avec 1295, **et** que la requête n'a aucun paramètre lié,
   **et** qu'`oxyn-query` découpe le texte en exactement une instruction, le
   driver envoie le même texte par `query_iter`. Toute autre erreur de
   préparation est le résultat montré à l'utilisateur. Sur MariaDB, qui
   prépare tout ce qui a été essayé, le repli ne se produit pas en pratique. Un
   test d'intégration du driver garde l'observation du contexte sur chaque
   serveur testé :
   `CREATE TRIGGER x BEFORE INSERT ON t FOR EACH ROW SET @a = 1; DROP TABLE t`
   fait échouer la préparation avec 1064, pas 1295.
4. **`oxyn-query` apprend MySQL avant que le driver parte.** Son scanner MySQL
   garde en un seul fragment un corps `BEGIN … END` dans `CREATE PROCEDURE`,
   `FUNCTION`, `TRIGGER` et `EVENT`, et honore une ligne `DELIMITER` comme le
   fait le client `mysql` : une directive du lot, jamais envoyée au serveur.
5. **Le driver ne déclare jamais `MULTIPLE_STATEMENTS`.** Un `CALL` renvoie
   légitimement plusieurs jeux de résultats : le premier est le résultat, les
   suivants sont comptés et signalés, jamais abandonnés en silence. Toute autre
   instruction qui produit un second jeu de résultats signifie que le serveur a
   exécuté plus que ce qu'Oxyn a envoyé : le driver le vide, le signale comme
   une erreur qui nomme ce fait, et ferme la connexion.
6. **Réglage de session, à chaque connexion :** `SET time_zone = '+00:00'`,
   pour qu'un `TIMESTAMP` arrive en UTC, et `SET NAMES utf8mb4`. TLS est exigé
   (`SslOpts` présent, certificat vérifié) sauf si l'environnement de la
   connexion est `Environment::Local`.
7. **L'annulation est `KILL QUERY <Conn::id()>` depuis une seconde connexion
   du même compte**, envoyée par la tâche de flux pendant qu'elle tient le
   `Conn` visé, fermé ensuite — la règle du `cancel.rs` du driver PostgreSQL,
   pour la même raison : l'identifiant nomme une connexion, pas une
   instruction. Quand le curseur cesse de lire pour de bon — borne de lignes
   atteinte, résultat abandonné —, le driver annule de la même façon au lieu de
   laisser le serveur bloqué jusqu'à `net_write_timeout`. `SERVER_SIDE_CANCEL`
   n'est déclaré pour une session que si la seconde connexion s'ouvre.
8. **L'état de transaction**
   ([ADR-0039](0039-etat-de-transaction-d-une-session.md)) est lu dans
   `SERVER_STATUS_IN_TRANS` après chaque instruction — le serveur le dit, le
   driver ne le déduit pas du texte.
9. **La session en lecture seule est `SET SESSION TRANSACTION READ ONLY`.**
   `READ_ONLY_SESSION` est déclaré : les quatre serveurs refusent DDL et DML
   dans ce mode (contexte). Un test d'intégration du driver garde cette
   observation, comme
   [ADR-0042](0042-revue-sur-place-des-operations-destructrices.md) l'exige
   pour chaque drapeau.
10. **Tout type envoyé par le serveur est décodé ; aucun ne fait échouer le
    résultat.**

    | MySQL / MariaDB | Arrow |
    |---|---|
    | `TINYINT` … `BIGINT` | `Int8` … `Int64` ; `UInt8` … `UInt64` si `UNSIGNED`. `TINYINT(1)` reste `Int8` : il contient jusqu'à 127, pas un booléen |
    | `MEDIUMINT` | `Int32` / `UInt32` |
    | `FLOAT`, `DOUBLE` | `Float32`, `Float64` |
    | `DECIMAL(p, s)` | `Decimal128(p, s)` jusqu'à `p = 38`, `Decimal256(p, s)` au-delà (MySQL permet 65) ; `s` est le `decimals` de la colonne, `p` sa longueur moins les positions du signe et du point (contexte) |
    | `DATE` | `Date32` |
    | `DATETIME` | `Timestamp(Microsecond, None)` |
    | `TIMESTAMP` | `Timestamp(Microsecond, "UTC")` |
    | `TIME` | `Duration(Microsecond)` — son domaine est ±838:59:59, pas une heure du jour |
    | `YEAR` | `UInt16` |
    | `BIT(n)` | `UInt64` |
    | `CHAR`, `VARCHAR`, `TEXT`, `ENUM`, `SET` | `Utf8` ; `Binary` quand le jeu de caractères est `binary` (63). Le drapeau `BINARY` n'est pas le critère : MariaDB le pose sur du texte `utf8mb4` |
    | `JSON` | `Utf8`, bien que MySQL l'annonce dans le jeu de caractères `binary` ; sur MariaDB c'est un `BLOB` `utf8mb4`, qui relève de la ligne précédente |
    | `BINARY`, `VARBINARY`, `BLOB` | `Binary` |
    | `GEOMETRY` | `Binary` : le SRID sur 4 octets du serveur suivi du WKB, marqué `oxyn:mysql_type` |
    | `VECTOR` | MySQL : `Binary` (`f32` little-endian contigus), marqué `oxyn:mysql_type`. MariaDB : `Binary`, indiscernable de `VARBINARY` sans métadonnées étendues |
    | un code de type que le driver ne connaît pas | `Binary`, avec le code dans `oxyn:mysql_type` |

    Une date nulle ou partielle (`0000-00-00`, `2024-00-15`), que ni `Date32`
    ni `Timestamp` ne peuvent contenir, fait échouer le résultat avec une
    erreur permanente qui nomme la colonne et le remède (`CAST(… AS CHAR)`) —
    la règle que le driver PostgreSQL applique à `infinity` : ni `NULL` ni une
    date inventée. `mysql_async` lit une telle date sans erreur : le refus est
    celui du driver, à la conversion en Arrow.

## Conséquences

* **+** Un `SELECT *` arrive dans la grille quoi que contienne le serveur :
  vecteurs, géométries, et codes de type qui n'existent pas encore.
* **+** `DECIMAL` est typé et exact, là où le driver PostgreSQL doit se replier
  sur du texte pour `numeric`.
* **+** Les comptes MariaDB en `client_ed25519` ou `parsec` se connectent, et
  aucune dépendance n'est sous un avis de sécurité sans correctif.
* **+** L'essentiel du SQL de l'utilisateur — `SELECT`, DML, `SET`,
  `CREATE VIEW`, `SHOW` — est prouvé instruction unique par le serveur
  lui-même ; sur MariaDB, la totalité.
* **−** Une seconde bibliothèque SQL à côté de `sqlx` : son propre type
  d'erreur, son propre câblage TLS, son propre chemin de mise à jour, et un
  second endroit où une lacune de protocole devient la nôtre.
* **−** Sur MySQL, pour les instructions non préparables — routines, `USE`,
  `START TRANSACTION`, `LOCK TABLES` —, seul le découpage d'`oxyn-query`
  protège contre une seconde instruction cachée. Le point 4 ajoute un travail
  sur le scanner dont le driver dépend, et un découpage en désaccord avec le
  serveur reste possible en général.
* **−** Un aller-retour de plus par instruction (préparation, exécution,
  fermeture) : sur un serveur à 50 ms, environ 100 ms avant la première ligne,
  à mesurer contre le budget [PERFORMANCE](../PERFORMANCE.md) plutôt que
  supposer.
* **−** Une table héritée contenant des dates nulles ne se lit pas par
  `SELECT *` ; l'utilisateur convertit la colonne. C'est honnête, et c'est une
  friction.
* **−** `mysql_async` annonce `CLIENT_LOCAL_FILES` quelles que soient les
  options : la protection est le gestionnaire absent, tenu par un test, pas un
  drapeau que le serveur voit ; et un serveur qui demande coûte sa connexion à
  la session.
* **−** Un `VECTOR` de MariaDB s'affiche en octets bruts, sans le marquage
  qu'obtient celui de MySQL.

**Coût de sortie :** modéré. Remplacer `mysql_async` réécrit la session, le
curseur et le décodage — à peu près la taille de `session.rs`, `cursor.rs` et
`decode.rs` du driver PostgreSQL — mais pas les requêtes de catalogue, les
capacités ni les tests, qui parlent SQL et Arrow. La table des types
(point 10) est la partie coûteuse à changer : elle façonne les exports et les
résultats déjà sauvegardés ([I-11](../../../../CLAUDE.md#i-11)).

**À reconsidérer si** `sqlx-mysql` lit les codes de type inconnus sans échouer
et expose les `decimals` et les octets bruts (une seule bibliothèque SQL
redeviendrait possible) ; si l'une des bibliothèques permet de ne pas annoncer
`CLIENT_MULTI_STATEMENTS` ou d'envoyer `COM_SET_OPTION` (le protocole texte
serait alors sûr à lui seul) ; ou si l'aller-retour supplémentaire dépasse le
budget du premier lot sur un serveur distant mesuré.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| `sqlx-mysql` 0.9.0, déjà dans le workspace | fait échouer tout le résultat sur une colonne `VECTOR`, perd l'échelle des `DECIMAL`, ne sait pas authentifier `client_ed25519`/`parsec` de MariaDB, et exige sans TLS un `rsa` en release candidate sous un avis sans correctif. Une bibliothèque pour trois drivers ne pèse pas plus lourd qu'un `SELECT *` qui échoue |
| Protocole texte pour tout, gardé par le seul découpage d'`oxyn-query` | la seule défense contre les multi-instructions serait côté client, sur le chemin que prend l'IA ([I-07](../../../../CLAUDE.md#i-07)) ; la préparation donne une preuve côté serveur pour la plupart des instructions |
| Préparation seule, en refusant ce que MySQL ne sait pas préparer | un client qui ne peut pas exécuter `START TRANSACTION`, `USE` ou `CREATE PROCEDURE` n'est pas un client MySQL |
| `DECIMAL` en `Utf8`, comme le driver PostgreSQL pour `numeric` | MySQL envoie précision et échelle avec chaque colonne ; le texte renoncerait pour rien au tri et à l'export typés |
| Les dates nulles en `NULL` ou à l'epoch | invente des données ; le driver PostgreSQL refuse `infinity` pour la même raison |
| Écrire notre propre implémentation du protocole MySQL | un protocole, plusieurs plugins d'authentification et une intégration TLS à maintenir seuls |
| Un driver MariaDB séparé | contredit [ADR-0003](0003-driver-capabilities.md) : le protocole est le même, les différences sont des capacités |
