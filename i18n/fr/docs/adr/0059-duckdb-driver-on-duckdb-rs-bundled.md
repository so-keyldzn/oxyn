<!-- oxyn-translation source="docs/adr/0059-duckdb-driver-on-duckdb-rs-bundled.md" sha256="ca0714c57db6" -->

> Traduction française de [docs/adr/0059-duckdb-driver-on-duckdb-rs-bundled.md](../../../../docs/adr/0059-duckdb-driver-on-duckdb-rs-bundled.md). **La version anglaise fait foi.**

# ADR-0059 — Le driver DuckDB embarque le moteur par `duckdb-rs`, reconstruit son Arrow 58 en Arrow 59, et ouvre chaque fichier avec l'accès externe coupé

**Statut :** proposé · **Date :** 2026-10-23

## Contexte

DuckDB est le premier moteur analytique des bases quotidiennes du mainteneur
([IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md)). Comme SQLite, il tourne
dans le processus d'Oxyn et une connexion est un fichier ; contrairement à
SQLite, son SQL atteint le système de fichiers et le réseau, son système de
types a des entiers de 128 bits, des types imbriqués et des unions, et il
calcule en parallèle. Vérifié le 2026-10-23
([RESEARCH-NOTES](../RESEARCH-NOTES.md#moteur-duckdb-et-bibliothèque-cliente--vérifié-le-2026-10-23)) :

* **Le client.** `duckdb` (`duckdb-rs`, MIT) est le binding que publie
  l'organisation de DuckDB. `1.10506.1` embarque DuckDB `v1.5.6`. Sa
  fonctionnalité `bundled` compile le moteur depuis le C++ avec `cc` ;
  `parquet` et `json` lient ces deux extensions statiquement et n'exigent rien
  de plus ; `icu` exige `bundled-cmake`, donc CMake sur chaque machine de
  compilation. `libduckdb-sys` déclare `links = "duckdb"`, sans conflit avec le
  `sqlite3` de `libsqlite3-sys` ([ADR-0010](0010-contraintes-natives-sqlite.md)).
* **Son Arrow n'est pas le nôtre.** Toutes les versions de `duckdb` jusqu'à
  `1.10506.1` dépendent d'Arrow `^58` ; `1.20000.0`, publiée le 2026-10-22,
  d'Arrow `^60`. Aucune de l'Arrow 59 du workspace
  ([ADR-0002](0002-arrow-result-model.md)). Deux versions d'Arrow sont deux
  jeux de types que le compilateur tient séparés.
* **Ce qui est en flux.** `Statement::stream_arrow` exécute dans le mode flux
  de DuckDB ; `Statement::step` lit ensuite un bloc à la fois — jusqu'à
  2 048 lignes — par `duckdb_fetch_chunk` et le rend en `StructArray` Arrow 58,
  ou une erreur. L'itérateur que rend `stream_arrow` panique sur une erreur de
  lecture, et le profil release s'arrête net sur une panique.
* **Ce qui panique dans le binding.** `column_logical_type` panique quand
  DuckDB ne rend aucun type pour un indice ; `schema` suppose une instruction
  exécutée ; `LogicalTypeHandle::child` et `child_name` paniquent hors limites ;
  `get_alias` déballe un alias en UTF-8 ; l'interface par lignes finit sur
  `unreachable!` pour un porteur inattendu.
* **Ce que DuckDB exporte par défaut.** `HUGEINT` et `UHUGEINT` en
  `Decimal128(38, 0)` — un `UHUGEINT` au-delà d'`i128::MAX` arrive négatif —,
  `TIMESTAMP WITH TIME ZONE` étiqueté du fuseau de la session, `TIME WITH TIME
  ZONE` sans son décalage, `BIT` en octets internes de DuckDB, `JSON` en texte
  sans son alias.
* **Ce que le SQL atteint.** `read_csv`, `read_parquet` et `read_json` lisent
  des chemins et des URL, `COPY … TO` et `EXPORT DATABASE` écrivent des
  fichiers, `ATTACH` ouvre d'autres fichiers, `INSTALL` et `LOAD` récupèrent et
  exécutent du code d'extension, et avec `autoinstall_known_extensions` une
  instruction télécharge une extension d'elle-même.
  `enable_external_access = false` arrête tout cela ; DuckDB refuse de le
  rallumer tant que la base tourne, refuse de changer `allowed_directories` une
  fois qu'il est coupé, et refuse de charger toute extension tant qu'il l'est.
* **Un fichier, un processus.** DuckDB verrouille un fichier de base contre les
  autres processus ; le verrou est un verrou d'enregistrement POSIX qu'un
  second moteur du même processus ne voit pas, et deux moteurs qui écrivent un
  même fichier le corrompent.
* **L'annulation.** `duckdb_interrupt` lève sur une connexion un drapeau que la
  requête en cours lit entre deux vecteurs ; la requête suivante l'abaisse.
* **Le coût.** Une compilation à neuf du moteur a pris 1 224 s de CPU ; avec
  informations de débogage, son archive pèse 1,80 Go par répertoire cible,
  73 Mo sans.

## Décision

1. **`oxyn-driver-duckdb` dépend de `duckdb` `=1.10506.1`**, fonctionnalités
   par défaut coupées, avec `bundled`, `parquet` et `json`. Le moteur est lié
   dans Oxyn : pas de bibliothèque système, pas de téléchargement, le même
   moteur sur chaque machine. Le profil dev compile `libduckdb-sys` avec
   `debug = false`.

2. **Arrow 58 est un doublon transitif qui ne franchit jamais la crate.**
   Aucun élément public du driver ne nomme un type Arrow 58. Chaque bloc venu
   de `step` est reconstruit en Arrow 59 en code sûr : chaque tampon est copié,
   la disposition est reconstruite par `ArrayData::builder` et validée par
   `build()` — décalages, longueurs, UTF-8 —, si bien qu'un tampon malformé est
   une erreur de décodage permanente, jamais un comportement indéfini ni une
   panique. Le type logique DuckDB de la colonne, lu une fois par colonne et
   possédé (`DuckType`), décide à chaque niveau d'imbrication : `HUGEINT` et
   `UHUGEINT` deviennent `Decimal256(39, 0)` — les bits d'un `UHUGEINT` relus
   comme non signés — ; `TIMESTAMP WITH TIME ZONE` est étiqueté `UTC`, l'instant
   que DuckDB stocke ; `BIT` devient le texte `'0101…'` qu'imprime DuckDB.
   `TIME WITH TIME ZONE` et un type que le binding ne connaît pas sont refusés
   par leur nom avant toute ligne lue, avec « cast it to VARCHAR » : leur
   porteur par défaut perd le décalage, et une supposition montrerait une
   valeur fausse comme juste. Tout autre type garde son porteur — entiers de
   toute largeur, `DECIMAL(w, s)` avec sa largeur, `TIMESTAMP` sans fuseau,
   `INTERVAL` en mois-jours-nanosecondes, `ENUM` en dictionnaire, listes,
   tableaux, maps, structs et unions avec leurs enfants convertis de même — et
   la métadonnée de champ `oxyn.duckdb.type` porte le nom DuckDB du type.
   Chaque bloc converti est ensuite découpé jusqu'à ce que chaque morceau tienne
   dans la borne en octets ; une ligne seule plus grande que la borne voyage
   seule.

3. **Aucune panique n'est atteignable depuis les données du moteur.** Du
   binding, le driver appelle `Connection::open_with_flags`,
   `open_in_memory_with_flags`, `try_clone`, `interrupt_handle`,
   `execute_batch`, `prepare`, `Statement::stream_arrow` (dont l'itérateur est
   abandonné sans être lu), `execute`, `step`, `schema` — seulement après une
   exécution réussie —, `column_logical_type` — seulement sous le nombre de
   colonnes du schéma de cette exécution —, `parameter_count`, et de
   `LogicalTypeHandle` : `try_id`, `raw_id`, `num_children`, `child` et
   `child_name` — seulement pour un identifiant qui a des enfants, sous
   `num_children` —, `decimal_width` et `decimal_scale`. Jamais `get_alias`,
   jamais l'interface par lignes, jamais les itérateurs Arrow. Une imbrication
   au-delà de 64 niveaux est refusée. Les requêtes de catalogue du driver
   passent par la même conversion et lisent les cellules par les accesseurs
   vérifiés d'Arrow. Une nouvelle version de `duckdb` est réauditée contre
   cette liste.

4. **Un moteur par fichier dans le processus, un thread porteur par session.**
   La première session d'un fichier ouvre le moteur ; les autres y prennent une
   connexion par `try_clone`, ce qui est la façon dont DuckDB partage une base
   dans un processus. Une connexion d'ancrage inactive garde le moteur tant
   qu'une session vit ; la dernière session à se fermer libère le fichier. Un
   fichier déjà ouvert dans Oxyn sous un autre mode d'accès ou un autre dossier
   est refusé, pas ouvert deux fois. `:memory:` est une base privée par
   connexion configurée, partagée par ses sessions. Chaque session possède un
   thread qui possède sa connexion, parce qu'une instruction en flux l'emprunte
   d'un `await` à l'autre ; le thread calcule au plus un lot d'avance sur le
   curseur ([I-05](../../CLAUDE.md#i-05)).

5. **Chaque moteur est ouvert avec l'accès externe coupé, et un seul dossier
   au plus est accordé.** À l'ouverture : `autoinstall_known_extensions` et
   `autoload_known_extensions` coupés ; puis, quand la connexion nomme un
   dossier (`file_access`), `allowed_directories` fixé sur lui ; puis
   `enable_external_access = false`, avant que la session n'existe. Le SQL ne
   lit et n'écrit alors de fichiers que dans ce dossier, n'atteint aucun
   réseau, et ne charge aucune extension. Parquet et JSON n'en demandent
   aucune. Une instruction qui exige une extension que cette compilation ne lie
   pas échoue avec le message de DuckDB, classé permanent ; Oxyn ne propose pas
   d'en télécharger une. La politique est celle de la connexion, pas de
   l'instruction : un `SET` tapé ne peut pas la lever.

6. **La lecture seule est celle du moteur.** Une connexion en lecture seule
   ouvre le moteur en `AccessMode::ReadOnly` et déclare `READ_ONLY_SESSION` sans
   `DDL`, `DML`, `TRUNCATE` ni `TRANSACTIONAL_DDL`. Une exécution en lecture
   seule (`ExecLimits::read_only`, qu'`oxyn-exec` pose sur toute lecture en
   production) est tenue deux fois : chaque instruction doit se classer comme
   lecture avec le profil DuckDB d'`oxyn-query` — donc `COPY … TO`, qui écrit un
   fichier, est refusé —, et l'exécution tourne dans `BEGIN TRANSACTION READ
   ONLY`, annulée ensuite, que DuckDB fait respecter quoi que tente une fonction
   dans un `SELECT`. Dans une transaction ouverte par l'utilisateur, cette
   enveloppe terminerait sa transaction : l'exécution est refusée à la place.

7. **Le driver découpe un lot avec `oxyn-query` et l'exécute instruction par
   instruction**, pour qu'une interruption soit vérifiée avant chaque
   instruction et qu'un échec après une instruction appliquée ne soit jamais
   transitoire. Un `USE` tapé est refusé : le contexte appartient à
   `Session::set_context` ([ADR-0019](0019-contexte-de-session.md)). Une
   écriture de données sans `RETURNING` rend son nombre de lignes touchées ; le
   reste est en flux.

8. **L'interruption vise une tâche.** Chaque tâche confiée au thread porteur a
   une identité ; `duckdb_interrupt` n'atteint le moteur que pendant que cette
   tâche tourne, vérifié sous un verrou, et un drapeau de la tâche survit à
   DuckDB qui abaisse le sien entre deux instructions. Une lecture arrêtée est
   `Cancelled` ; tout le reste est `OutcomeUnknown` : DuckDB annule la
   transaction de l'instruction, mais pas un fichier que `COPY` a commencé
   d'écrire ni un `ATTACH`. La session ne déclare **pas** `SERVER_SIDE_CANCEL`,
   comme SQLite : il n'y a pas de serveur, et l'interface lit ce drapeau comme
   « le bouton Annuler atteint un serveur ». La preuve de l'arrêt est celle du
   moteur : la même session répond aussitôt, et une fois fermée, un autre
   processus peut verrouiller le fichier.

9. **Les erreurs sont classées d'après les noms d'exception de DuckDB.** L'API C
   ne rend que le message mis en forme, dont les premiers mots sont le type
   d'exception tiré d'une table fixe (`EXCEPTION_MAP`). Le driver lit ce
   préfixe une fois et porte la classe comme une donnée : refus permanents ;
   conflit de transaction transitoire pour une lecture et ambigu pour une
   écriture ; erreurs d'E/S, de mémoire et fatales ambiguës pour une écriture ;
   fichier tenu par un autre processus permanent, avec ce qu'il faut faire.
   Aucun chemin n'apparaît dans une erreur ([I-03](../../CLAUDE.md#i-03)).

## Conséquences

* **+** Chaque utilisateur d'Oxyn a le même moteur et le même SQL, avec Parquet
  et JSON, sans rien installer.
* **+** Une requête qui lit un CSV ne peut rien faire d'autre à la machine : la
  politique est posée avant la première instruction et le moteur refuse de la
  lever.
* **+** Les valeurs survivent : entiers de 128 bits, `UBIGINT` au-delà de 2^53,
  décimaux avec leur largeur, types imbriqués, horodatages sans fuseau sans
  fuseau inventé.
* **−** **La compilation est plus lourde.** Le moteur représente environ
  1 200 s de CPU à neuf, et ajoute l'exigence d'une chaîne C++ que chaque
  plateforme remplissait déjà pour SQLite. La CI la met en cache ; un cache
  froid la paie dans chaque job Rust.
* **−** **Deux versions d'Arrow sont compilées**, et chaque bloc est copié une
  fois de plus qu'avec un binding sur Arrow 59. La copie est linéaire dans le
  lot, sous la borne en octets ; elle n'a pas été mesurée à part du test de
  flux de bout en bout (2 964 Mo en flux, mémoire résidente en hausse de
  164 Mo).
* **−** **Aucune extension au-delà de Parquet et JSON** : ni `httpfs`, ni
  `spatial`, ni `icu` (fonctions de fuseau comme `AT TIME ZONE 'Europe/Paris'`),
  ni `iceberg`, ni `postgres_scanner`. Chacune est une décision de liaison, et
  `icu` exige CMake.
* **−** **`TIME WITH TIME ZONE` est refusé**, converti en texte par
  l'utilisateur, et une colonne `JSON` de résultat est étiquetée `VARCHAR` :
  l'export par défaut de DuckDB perd le décalage et l'alias, et l'accesseur
  d'alias du binding peut paniquer.
* **−** **Un fichier s'ouvre sous un seul réglage à la fois dans Oxyn** : une
  connexion en lecture seule et une connexion en écriture sur un même fichier
  ne peuvent être ouvertes ensemble.
* **−** **Accorder un dossier est tout ou rien en son sein** : le SQL peut y
  lire et écrire n'importe quoi, et le dossier se tape — le sélecteur du
  formulaire de connexion choisit des fichiers.
* **−** **`ATTACH` échappe au registre « un moteur par fichier ».** Une
  instruction qui attache un fichier du dossier accordé, déjà ouvert comme une
  autre connexion Oxyn, l'ouvre une seconde fois dans le processus, où le verrou
  d'enregistrement de DuckDB ne voit pas le premier : deux écrivains peuvent
  alors le corrompre. Le registre garde ce qu'Oxyn ouvre, pas ce que le SQL
  attache ; un fichier destiné à être attaché n'est pas aussi ouvert comme
  connexion.

**Coût de sortie :** modéré. Le binding est derrière les propres types de la
crate — aucun type `duckdb` ne franchit son API publique —, si bien qu'un autre
binding remplace `open`, `worker`, `stream`, `query` et `logical` (environ
1 200 lignes) et garde les tests de la conversion. Passer à un binding sur
Arrow 59 ou plus récent retire la copie de la décision 2 et garde les
corrections par type logique.

**À reconsidérer si** une version de `duckdb` dépend de la version majeure
d'Arrow du workspace (revérifié par `/versions`), et la copie part ; si l'API C
de DuckDB expose le type d'exception, et le préfixe du message n'est plus lu ;
si un besoin d'`icu`, `spatial` ou `httpfs` l'emporte sur CMake dans chaque
compilation ; ou si le binaire livré dépasse ce que le budget de release
accepte une fois mesuré.

## Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| `duckdb` `1.20000.0` | Publiée la veille de cette décision, une nouvelle lignée majeure du moteur, et son Arrow est `^60` : toujours un doublon, avec le moins de temps d'exécution derrière elle |
| La `libduckdb` du système (`duckdb` sans `bundled`) | Chaque utilisateur installerait DuckDB, dans une version qu'Oxyn ne choisit pas : le SQL et le format de fichier changeraient d'une machine à l'autre |
| La CLI DuckDB en sidecar ([ADR-0007](0007-driver-sidecar.md)) | Du texte ou du JSON sur un tube au lieu d'Arrow, un second processus par session, et aucune interruption d'une instruction |
| Écrire la FFI vers l'API C de DuckDB | De l'`unsafe` dans toute une crate, refusé par le workspace ([SECURITY](../SECURITY.md#politique-unsafe)), pour ce que le binding fait déjà |
| L'interface par lignes du binding (`query_map`, `Row::get`) | Le décodage des valeurs finit sur `unreachable!` pour un porteur inattendu, et un `SET produce_arrow_string_view = true` tapé dans une console fait de chaque `VARCHAR` un tel porteur |
| `arrow_lossless_conversion = true` à l'ouverture | Il change la façon dont voyagent un `UUID` et un `BOOLEAN` pour chaque session du fichier, et le rendu des cellules d'Oxyn ne lit pas encore les types d'extension Arrow de DuckDB |
| Déclarer `SERVER_SIDE_CANCEL` | L'interface le lit comme « le bouton Annuler atteint un serveur » ; SQLite, qui interrompt de la même façon, ne le déclare pas non plus |
| Un moteur par session sur un fichier partagé | Deux moteurs qui écrivent un même fichier dans un processus le corrompent, et le verrou de DuckDB ne le voit pas |
